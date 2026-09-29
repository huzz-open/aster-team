package mariadb

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

const publicationSelect = `SELECT id,operation_id,environment,catalog_revision,snapshot_json,content_sha256,created_by,created_at,status,evidence_json,accepted_by,accepted_at FROM commercial_catalog_publications`

func scanPublication(row commercialScanner) (commercial.PublicationRecord, error) {
	var r commercial.PublicationRecord
	var id, operation, environment, revision, actor string
	var created time.Time
	var snapshot, evidence []byte
	var acceptedBy sql.NullString
	var acceptedAt sql.NullTime
	if err := row.Scan(&id, &operation, &environment, &revision, &snapshot, &r.SHA256, &actor, &created, &r.Status, &evidence, &acceptedBy, &acceptedAt); err != nil {
		return r, err
	}
	if err := json.Unmarshal(snapshot, &r.Snapshot); err != nil {
		return r, err
	}
	raw, err := r.Snapshot.Bytes()
	s := r.Snapshot
	if err != nil || !bytes.Equal(raw, snapshot) || s.ID != id || s.Request.OperationID != operation || s.Catalog.Request.Environment != environment || s.Catalog.ID != revision || s.CreatedBy != actor || s.CreatedAt != created.UTC().Format("2006-01-02T15:04:05.000Z") {
		return r, commercial.ErrInvalidPublication
	}
	if len(evidence) > 0 {
		r.Evidence = &commercial.PublicationEvidence{}
		if err := json.Unmarshal(evidence, r.Evidence); err != nil {
			return r, err
		}
	}
	r.AcceptedBy = acceptedBy.String
	if acceptedAt.Valid {
		r.AcceptedAt = acceptedAt.Time.UTC().Format("2006-01-02T15:04:05.000Z")
	}
	return r, r.Validate()
}

func (s *Store) GetPublication(ctx context.Context, id string) (commercial.PublicationRecord, error) {
	r, err := scanPublication(s.db.QueryRowContext(ctx, publicationSelect+" WHERE id=?", id))
	if errors.Is(err, sql.ErrNoRows) {
		err = commercial.ErrNotFound
	}
	return r, err
}
func (s *Store) ListPublications(ctx context.Context, limit int) ([]commercial.PublicationRecord, error) {
	if limit < 1 || limit > 100 {
		limit = 50
	}
	rows, err := s.db.QueryContext(ctx, publicationSelect+" ORDER BY created_at DESC,id LIMIT ?", limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []commercial.PublicationRecord{}
	for rows.Next() {
		r, err := scanPublication(rows)
		if err != nil {
			return nil, err
		}
		result = append(result, r)
	}
	return result, rows.Err()
}
func (s *Store) GetPublicationHead(ctx context.Context, environment string) (string, error) {
	var id sql.NullString
	err := s.db.QueryRowContext(ctx, "SELECT active_id FROM commercial_publication_heads WHERE environment=?", environment).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		err = commercial.ErrNotFound
	}
	if err != nil || !id.Valid {
		return "", err
	}
	r, err := s.GetPublication(ctx, id.String)
	if err != nil {
		return "", err
	}
	if r.Status != "accepted" || r.Snapshot.Catalog.Request.Environment != environment {
		return "", commercial.ErrInvalidPublication
	}
	return id.String, nil
}

func (s *Store) PreparePublication(ctx context.Context, id string, in commercial.PreparePublicationInput, actor string, now time.Time) (commercial.PublicationRecord, error) {
	requested, err := in.Bytes()
	if err != nil {
		return commercial.PublicationRecord{}, err
	}
	var result commercial.PublicationRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		old, err := scanPublication(tx.QueryRowContext(ctx, publicationSelect+" WHERE operation_id=? FOR UPDATE", in.OperationID))
		if err == nil {
			stored, err := old.Snapshot.Request.Bytes()
			if err != nil {
				return err
			}
			if old.Snapshot.ID != id || old.Snapshot.CreatedBy != actor || !bytes.Equal(stored, requested) {
				return commercial.ErrConflict
			}
			result = old
			return tx.Commit()
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		catalog, err := scanCatalogApproval(tx.QueryRowContext(ctx, catalogApprovalSelect+" WHERE id=?", in.CatalogRevision))
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.ErrNotFound
		}
		if err != nil {
			return err
		}
		if catalog.Status != "exported" {
			return commercial.ErrPublicationNotAccepted
		}
		var head sql.NullString
		if err = tx.QueryRowContext(ctx, "SELECT active_id FROM commercial_publication_heads WHERE environment=? FOR UPDATE", catalog.Snapshot.Request.Environment).Scan(&head); err != nil {
			return err
		}
		if head.String != in.ExpectedActiveID {
			return commercial.ErrPublicationPreparationRejected
		}
		// Capture creation time after the operation and channel locks. Once the
		// deadline has passed, no delayed original attempt may create this event.
		now = time.Now().UTC().Truncate(time.Millisecond)
		until, err := time.Parse("2006-01-02T15:04:05.000Z", in.AcceptUntil)
		if err != nil || !now.Before(until) {
			return commercial.ErrPublicationPreparationRejected
		}
		snapshot := commercial.PublicationSnapshot{Schema: commercial.PublicationSchema, ID: id, Request: in, Catalog: catalog.Snapshot, CreatedBy: actor, CreatedAt: now.UTC().Format("2006-01-02T15:04:05.000Z")}
		raw, err := snapshot.Bytes()
		if err != nil {
			return err
		}
		result = commercial.PublicationRecord{Snapshot: snapshot, SHA256: commercial.ContentDigest(raw), Status: "prepared"}
		_, err = tx.ExecContext(ctx, `INSERT INTO commercial_catalog_publications(id,operation_id,environment,catalog_revision,snapshot_json,content_sha256,created_by,created_at,status) VALUES(?,?,?,?,?,?,?,?,'prepared')`, id, in.OperationID, catalog.Snapshot.Request.Environment, in.CatalogRevision, raw, result.SHA256, actor, now)
		if err != nil {
			return commercialConflict(err)
		}
		if err = writeCommercialAudit(ctx, tx, in.OperationID, "commercial.publication_prepared", id, actor, result.SHA256, now); err != nil {
			return err
		}
		return tx.Commit()
	})
	return result, err
}

func (s *Store) AcceptPublication(ctx context.Context, id, expectedDigest string, evidence commercial.PublicationEvidence, actor string) (commercial.PublicationRecord, error) {
	previous, err := s.GetPublication(ctx, id)
	if err != nil {
		return commercial.PublicationRecord{}, err
	}
	if previous.SHA256 != expectedDigest {
		return commercial.PublicationRecord{}, commercial.ErrConflict
	}
	if previous.Status == "accepted" {
		return previous, nil
	}
	observed, err := time.Parse("2006-01-02T15:04:05.000Z", evidence.ObservedAt)
	if err != nil {
		return commercial.PublicationRecord{}, commercial.ErrInvalidPublication
	}
	deadline, err := time.Parse("2006-01-02T15:04:05.000Z", previous.Snapshot.Request.AcceptUntil)
	if err != nil {
		return commercial.PublicationRecord{}, err
	}
	if freshness := observed.Add(time.Minute); freshness.Before(deadline) {
		deadline = freshness
	}
	// Includes lock waits, every deadlock retry and commit; an observation made
	// before a long wait cannot be accepted using the old pre-transaction time.
	ctx, cancel := context.WithDeadline(ctx, deadline)
	defer cancel()
	var result commercial.PublicationRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		result, err = scanPublication(tx.QueryRowContext(ctx, publicationSelect+" WHERE id=? FOR UPDATE", id))
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.ErrNotFound
		}
		if err != nil {
			return err
		}
		if result.SHA256 != expectedDigest {
			return commercial.ErrConflict
		}
		// A replay only restores the original receipt. It never reactivates an old
		// head, changes its deadline or requires an old site to still be live.
		if result.Status == "accepted" {
			return tx.Commit()
		}
		var head sql.NullString
		environment := result.Snapshot.Catalog.Request.Environment
		if err = tx.QueryRowContext(ctx, "SELECT active_id FROM commercial_publication_heads WHERE environment=? FOR UPDATE", environment).Scan(&head); err != nil {
			return err
		}
		if head.String != result.Snapshot.Request.ExpectedActiveID {
			return commercial.ErrConflict
		}
		now := time.Now().UTC().Truncate(time.Millisecond)
		if !now.Before(deadline) {
			return commercial.ErrPublicationNotAccepted
		}
		result.Status, result.Evidence, result.AcceptedBy, result.AcceptedAt = "accepted", &evidence, actor, now.UTC().Format("2006-01-02T15:04:05.000Z")
		if err = result.Validate(); err != nil {
			return err
		}
		raw, err := json.Marshal(evidence)
		if err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, `UPDATE commercial_catalog_publications SET status='accepted',evidence_json=?,accepted_by=?,accepted_at=? WHERE id=? AND status='prepared'`, raw, actor, now, id); err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, "UPDATE commercial_publication_heads SET active_id=? WHERE environment=?", id, environment); err != nil {
			return err
		}
		if err = writeCommercialAudit(ctx, tx, id, "commercial.publication_accepted", id, actor, commercial.ContentDigest(raw), now); err != nil {
			return err
		}
		if !time.Now().Before(deadline) {
			return commercial.ErrPublicationNotAccepted
		}
		return tx.Commit()
	})
	return result, err
}
