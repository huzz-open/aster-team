package mariadb

import (
	"context"
	"database/sql"
	"errors"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

func (s *Store) GetQuotationSource(ctx context.Context, reference, environment string) (commercial.QuotationSource, error) {
	if environment != "local" && environment != "production" {
		return commercial.QuotationSource{}, commercial.ErrPublicationNotAccepted
	}
	column := "id"
	if strings.HasPrefix(reference, "catalog_") {
		column = "catalog_revision"
	}
	// Exact reference, no global recent-N window. Several accepted events for
	// one catalog are considered newest first; each retains its own deadline.
	rows, err := s.db.QueryContext(ctx, publicationSelect+" WHERE "+column+"=? AND environment=? AND status='accepted' ORDER BY accepted_at DESC,id", reference, environment)
	if err != nil {
		return commercial.QuotationSource{}, err
	}
	defer rows.Close()
	for rows.Next() {
		publication, err := scanPublication(rows)
		if err != nil {
			return commercial.QuotationSource{}, err
		}
		source, err := publication.QuotationSource(environment, time.Now().UTC())
		if errors.Is(err, commercial.ErrPublicationNotAccepted) {
			continue
		}
		return source, err
	}
	if err := rows.Err(); err != nil {
		return commercial.QuotationSource{}, err
	}
	return commercial.QuotationSource{}, commercial.ErrPublicationNotAccepted
}

func (s *Store) CreateQuotationOrder(ctx context.Context, id string, input commercial.QuotationOrderInput, environment, actor string) (commercial.OrderRecord, error) {
	if err := input.Validate(); err != nil {
		return commercial.OrderRecord{}, err
	}
	// Readback is independent of current publication availability and channel
	// configuration. The transaction repeats this check for simultaneous calls.
	old, err := scanCommercialOrder(s.db.QueryRowContext(ctx, commercialOrderSelect+" WHERE operation_id=?", input.OperationID))
	if err == nil {
		if !old.MatchesQuotationRequest(id, input, actor) {
			return commercial.OrderRecord{}, commercial.ErrConflict
		}
		return old, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return commercial.OrderRecord{}, err
	}
	if environment != "local" && environment != "production" {
		return commercial.OrderRecord{}, commercial.ErrPublicationNotAccepted
	}
	publication, err := s.GetPublication(ctx, input.PublicationID)
	if err != nil {
		return commercial.OrderRecord{}, err
	}
	// This immutable deadline covers all subsequent lock waits, retries and
	// Commit. A timeout can be an unknown outcome; retry the same operation.
	deadline, err := time.Parse("2006-01-02T15:04:05.000Z", publication.Snapshot.Request.AcceptUntil)
	if err != nil {
		return commercial.OrderRecord{}, commercial.ErrInvalidPublication
	}
	if _, err := publication.ResolveQuotation(environment, input.CatalogRevision, input.PlanID, input.PlanVersion, input.Years, time.Now().UTC()); err != nil {
		return commercial.OrderRecord{}, err
	}
	ctx, cancel := context.WithDeadline(ctx, deadline)
	defer cancel()
	var result commercial.OrderRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		old, err := scanCommercialOrder(tx.QueryRowContext(ctx, commercialOrderSelect+" WHERE operation_id=? FOR UPDATE", input.OperationID))
		if err == nil {
			if !old.MatchesQuotationRequest(id, input, actor) {
				return commercial.ErrConflict
			}
			result = old
			return tx.Commit()
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		var customer string
		if err := tx.QueryRowContext(ctx, "SELECT id FROM customers WHERE id=? AND status<>'inactive' FOR UPDATE", input.CustomerID).Scan(&customer); err != nil {
			if errors.Is(err, sql.ErrNoRows) {
				return commercial.ErrNotFound
			}
			return err
		}
		current, err := scanPublication(tx.QueryRowContext(ctx, publicationSelect+" WHERE id=? FOR UPDATE", input.PublicationID))
		if err != nil {
			return err
		}
		if current.SHA256 != publication.SHA256 {
			return commercial.ErrConflict
		}
		now := time.Now().UTC().Truncate(time.Millisecond)
		snapshot, err := commercial.CreateQuotationOrderSnapshot(id, input, current, environment, now)
		if err != nil {
			return err
		}
		raw, err := snapshot.Bytes()
		if err != nil {
			return err
		}
		digest := commercial.ContentDigest(raw)
		if _, err = tx.ExecContext(ctx, `INSERT INTO commercial_orders(id,operation_id,customer_id,plan_id,plan_version,snapshot_json,content_sha256,status,created_by,created_at,updated_at) VALUES(?,?,?,?,?,?,?,'pending_payment',?,?,?)`, id, input.OperationID, input.CustomerID, input.PlanID, input.PlanVersion, raw, digest, actor, now, now); err != nil {
			return commercialConflict(err)
		}
		if err = writeCommercialAudit(ctx, tx, input.OperationID, "commercial.quotation_order_created", id, actor, digest, now); err != nil {
			return err
		}
		if !time.Now().Before(deadline) {
			return commercial.ErrPublicationNotAccepted
		}
		result = commercial.OrderRecord{Snapshot: snapshot, SHA256: digest, OperationID: input.OperationID, Status: "pending_payment", CreatedBy: actor, CreatedAt: now}
		return tx.Commit()
	})
	return result, err
}
