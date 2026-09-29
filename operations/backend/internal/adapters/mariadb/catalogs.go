package mariadb

import (
	"bytes"
	"context"
	"database/sql"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

const catalogApprovalSelect = `SELECT id,operation_id,environment,snapshot_json,content_sha256,public_json,public_sha256,approved_by,approved_at,status FROM commercial_catalog_approvals`

func scanCatalogApproval(row commercialScanner) (commercial.CatalogApprovalRecord, error) {
	var r commercial.CatalogApprovalRecord
	var id, operation, environment, publicDigest, actor string
	var approved time.Time
	var snapshot, public []byte
	if err := row.Scan(&id, &operation, &environment, &snapshot, &r.SHA256, &public, &publicDigest, &actor, &approved, &r.Status); err != nil {
		return r, err
	}
	var err error
	r.Snapshot, err = commercial.ParseCatalogApproval(snapshot, r.SHA256)
	if err != nil {
		return r, err
	}
	s := r.Snapshot
	if s.ID != id || s.Request.OperationID != operation || s.Request.Environment != environment || s.PublicSHA256 != publicDigest || s.ApprovedBy != actor || s.ApprovedAt != approved.UTC().Format("2006-01-02T15:04:05.000Z") {
		return r, commercial.ErrInvalidCatalog
	}
	r.Public, err = s.Preview()
	if err != nil {
		return r, err
	}
	raw, err := r.PublicBytes()
	if err != nil {
		return r, err
	}
	if !bytes.Equal(public, raw) {
		return r, commercial.ErrInvalidCatalog
	}
	return r, nil
}
func (s *Store) GetCatalogApproval(ctx context.Context, id string) (commercial.CatalogApprovalRecord, error) {
	r, err := scanCatalogApproval(s.db.QueryRowContext(ctx, catalogApprovalSelect+" WHERE id=?", id))
	if errors.Is(err, sql.ErrNoRows) {
		err = commercial.ErrNotFound
	}
	return r, err
}
func (s *Store) ListCatalogApprovals(ctx context.Context, limit int) ([]commercial.CatalogApprovalRecord, error) {
	if limit < 1 || limit > 100 {
		limit = 50
	}
	rows, err := s.db.QueryContext(ctx, catalogApprovalSelect+" ORDER BY approved_at DESC,id LIMIT ?", limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []commercial.CatalogApprovalRecord{}
	for rows.Next() {
		record, err := scanCatalogApproval(rows)
		if err != nil {
			return nil, err
		}
		result = append(result, record)
	}
	return result, rows.Err()
}

func previewCatalogTx(ctx context.Context, tx *sql.Tx, id string, request commercial.CatalogRequest) (commercial.CatalogPreview, []commercial.PlanSnapshot, error) {
	plans := make([]commercial.PlanSnapshot, 0, len(request.Plans))
	for _, selected := range request.Plans {
		plan, err := scanCommercialPlan(tx.QueryRowContext(ctx, commercialPlanSelect+" WHERE plan_id=? AND version_no=?", selected.PlanID, selected.Version))
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.CatalogPreview{}, nil, commercial.ErrNotFound
		}
		if err != nil {
			return commercial.CatalogPreview{}, nil, err
		}
		if plan.SHA256 != selected.ExpectedSHA256 {
			return commercial.CatalogPreview{}, nil, commercial.ErrCatalogPreviewConflict
		}
		plans = append(plans, plan.Snapshot)
	}
	preview, err := commercial.BuildPublicCatalog(id, request, plans)
	return preview, plans, err
}
func (s *Store) PreviewPublicCatalog(ctx context.Context, id string, request commercial.CatalogRequest) (commercial.CatalogPreview, error) {
	if err := request.Validate(); err != nil {
		return commercial.CatalogPreview{}, err
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelRepeatableRead, ReadOnly: true})
	if err != nil {
		return commercial.CatalogPreview{}, err
	}
	defer tx.Rollback()
	preview, _, err := previewCatalogTx(ctx, tx, id, request)
	if err != nil {
		return commercial.CatalogPreview{}, err
	}
	return preview, tx.Commit()
}
func (s *Store) ApprovePublicCatalog(ctx context.Context, id string, input commercial.ApproveCatalogInput, actor string, now time.Time) (commercial.CatalogApprovalRecord, error) {
	if err := input.Validate(); err != nil {
		return commercial.CatalogApprovalRecord{}, err
	}
	requested, err := input.Request.Bytes()
	if err != nil {
		return commercial.CatalogApprovalRecord{}, err
	}
	var record commercial.CatalogApprovalRecord
	err = retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		old, err := scanCatalogApproval(tx.QueryRowContext(ctx, catalogApprovalSelect+" WHERE operation_id=? FOR UPDATE", input.Request.OperationID))
		if err == nil {
			stored, err := old.Snapshot.Request.Bytes()
			if err != nil {
				return err
			}
			if old.Snapshot.ID != id || old.Snapshot.ApprovedBy != actor || old.Snapshot.PublicSHA256 != input.ExpectedPublicSHA256 || !bytes.Equal(stored, requested) {
				return commercial.ErrConflict
			}
			record = old
			return tx.Commit()
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		preview, plans, err := previewCatalogTx(ctx, tx, id, input.Request)
		if err != nil {
			return err
		}
		if preview.SHA256 != input.ExpectedPublicSHA256 {
			return commercial.ErrCatalogPreviewConflict
		}
		snapshot := commercial.CatalogApprovalSnapshot{Schema: commercial.CatalogApprovalSchema, ID: id, Request: input.Request, Plans: plans, PublicSHA256: preview.SHA256, ApprovedBy: actor, ApprovedAt: now.UTC().Format("2006-01-02T15:04:05.000Z")}
		raw, err := snapshot.Bytes()
		if err != nil {
			return err
		}
		record = commercial.CatalogApprovalRecord{Snapshot: snapshot, SHA256: commercial.ContentDigest(raw), Public: preview, Status: "approved"}
		public, err := record.PublicBytes()
		if err != nil {
			return err
		}
		_, err = tx.ExecContext(ctx, `INSERT INTO commercial_catalog_approvals(id,operation_id,environment,snapshot_json,content_sha256,public_json,public_sha256,approved_by,approved_at,status) VALUES(?,?,?,?,?,?,?,?,?,'approved')`, id, input.Request.OperationID, input.Request.Environment, raw, record.SHA256, public, preview.SHA256, actor, now)
		if err != nil {
			return commercialConflict(err)
		}
		if err = writeCommercialAudit(ctx, tx, input.Request.OperationID, "commercial.catalog_approved", id, actor, record.SHA256, now); err != nil {
			return err
		}
		return tx.Commit()
	})
	return record, err
}

func (s *Store) MarkCatalogExported(ctx context.Context, id, digest, actor string, now time.Time) (commercial.CatalogApprovalRecord, error) {
	var record commercial.CatalogApprovalRecord
	err := retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		record, err = scanCatalogApproval(tx.QueryRowContext(ctx, catalogApprovalSelect+" WHERE id=? FOR UPDATE", id))
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.ErrNotFound
		}
		if err != nil {
			return err
		}
		if record.Public.SHA256 != digest {
			return commercial.ErrConflict
		}
		if record.Status == "exported" {
			return tx.Commit()
		}
		if _, err := tx.ExecContext(ctx, "UPDATE commercial_catalog_approvals SET status='exported' WHERE id=? AND status='approved'", id); err != nil {
			return err
		}
		if err := writeCommercialAudit(ctx, tx, id, "commercial.catalog_exported", id, actor, digest, now); err != nil {
			return err
		}
		record.Status = "exported"
		return tx.Commit()
	})
	return record, err
}
