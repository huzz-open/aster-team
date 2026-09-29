package mariadb

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/licenseprotocol"
)

const distributionSelect = `SELECT id,operation_id,plan_id,plan_version,snapshot_json,content_sha256,approved_by,approved_at,status,claims_json,document_json,document_sha256 FROM commercial_free_distributions`

func scanDistribution(row commercialScanner) (commercial.DistributionRecord, error) {
	var r commercial.DistributionRecord
	var id, plan, actor string
	var version uint32
	var approved time.Time
	var snapshot, claims, document []byte
	var digest sql.NullString
	if err := row.Scan(&id, &r.OperationID, &plan, &version, &snapshot, &r.SHA256, &actor, &approved, &r.Status, &claims, &document, &digest); err != nil {
		return r, err
	}
	if err := json.Unmarshal(snapshot, &r.Snapshot); err != nil {
		return r, err
	}
	if r.Snapshot.ID != id || r.Snapshot.Plan.PlanID != plan || r.Snapshot.Plan.Version != version || r.Snapshot.ApprovedBy != actor || r.Snapshot.ApprovedAt != approved.UTC().Format("2006-01-02T15:04:05.000Z") {
		return r, errors.New("distribution differs from stored identity")
	}
	if len(claims) > 0 {
		r.Claims = &licenseprotocol.ClaimsV2{}
		if err := json.Unmarshal(claims, r.Claims); err != nil {
			return r, err
		}
	}
	if len(document) > 0 {
		r.Document = &licenseprotocol.DocumentV2{}
		if err := json.Unmarshal(document, r.Document); err != nil {
			return r, err
		}
	}
	r.DocumentSHA256 = digest.String
	return r, r.Validate()
}

func (s *Store) GetFreeDistribution(ctx context.Context, id string) (commercial.DistributionRecord, error) {
	r, e := scanDistribution(s.db.QueryRowContext(ctx, distributionSelect+" WHERE id=?", id))
	if errors.Is(e, sql.ErrNoRows) {
		e = commercial.ErrNotFound
	}
	return r, e
}
func (s *Store) ListFreeDistributions(ctx context.Context, limit int) ([]commercial.DistributionRecord, error) {
	if limit < 1 || limit > 100 {
		limit = 50
	}
	rows, err := s.db.QueryContext(ctx, distributionSelect+" ORDER BY approved_at DESC,id LIMIT ?", limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	records := []commercial.DistributionRecord{}
	for rows.Next() {
		r, err := scanDistribution(rows)
		if err != nil {
			return nil, err
		}
		records = append(records, r)
	}
	return records, rows.Err()
}

func (s *Store) ApproveFreeDistribution(ctx context.Context, id string, in commercial.ApproveDistributionInput, actor string, now time.Time) (commercial.DistributionRecord, error) {
	var record commercial.DistributionRecord
	err := retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		old, err := scanDistribution(tx.QueryRowContext(ctx, distributionSelect+" WHERE operation_id=? FOR UPDATE", in.OperationID))
		if err == nil {
			v := old.Snapshot
			if v.ID != id || v.Plan.PlanID != in.PlanID || v.Plan.Version != in.PlanVersion || v.PlanSHA256 != in.ExpectedSHA256 || v.ApprovedBy != actor || v.Reason != in.Reason || v.NotBefore != in.NotBefore.UTC().Format("2006-01-02T15:04:05.000Z") {
				return commercial.ErrConflict
			}
			record = old
			return tx.Commit()
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		plan, err := scanCommercialPlan(tx.QueryRowContext(ctx, commercialPlanSelect+" WHERE plan_id=? AND version_no=?", in.PlanID, in.PlanVersion))
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.ErrNotFound
		}
		if err != nil {
			return err
		}
		if plan.SHA256 != in.ExpectedSHA256 {
			return commercial.ErrConflict
		}
		snapshot := commercial.DistributionSnapshot{Schema: commercial.DistributionSchema, ID: id, Plan: plan.Snapshot, PlanSHA256: plan.SHA256, NotBefore: in.NotBefore.UTC().Format("2006-01-02T15:04:05.000Z"), Reason: in.Reason, ApprovedBy: actor, ApprovedAt: now.UTC().Format("2006-01-02T15:04:05.000Z")}
		raw, err := snapshot.Bytes()
		if err != nil {
			return fmt.Errorf("%w: %v", commercial.ErrInvalidDistribution, err)
		}
		record = commercial.DistributionRecord{Snapshot: snapshot, SHA256: commercial.ContentDigest(raw), OperationID: in.OperationID, Status: "approved"}
		if err = record.Validate(); err != nil {
			return err
		}
		_, err = tx.ExecContext(ctx, `INSERT INTO commercial_free_distributions(id,operation_id,plan_id,plan_version,snapshot_json,content_sha256,approved_by,approved_at,status) VALUES(?,?,?,?,?,?,?,?,'approved')`, id, in.OperationID, in.PlanID, in.PlanVersion, raw, record.SHA256, actor, now)
		if err != nil {
			return commercialConflict(err)
		}
		if err = writeCommercialAudit(ctx, tx, in.OperationID, "commercial.distribution_approved", id, actor, record.SHA256, now); err != nil {
			return err
		}
		return tx.Commit()
	})
	return record, err
}

func (s *Store) PrepareFreeDistribution(ctx context.Context, id, keyID, actor string, now time.Time) (commercial.DistributionRecord, error) {
	var record commercial.DistributionRecord
	err := retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		record, err = scanDistribution(tx.QueryRowContext(ctx, distributionSelect+" WHERE id=? FOR UPDATE", id))
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.ErrNotFound
		}
		if err != nil {
			return err
		}
		if record.Status != "approved" {
			if record.Claims.KeyID != keyID {
				return commercial.ErrConflict
			}
			return tx.Commit()
		}
		claims, err := record.Snapshot.Claims(keyID, now)
		if err != nil {
			return err
		}
		record.Claims = &claims
		record.Status = "prepared"
		if err = record.Validate(); err != nil {
			return err
		}
		raw, err := json.Marshal(claims)
		if err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, `UPDATE commercial_free_distributions SET status='prepared',claims_json=? WHERE id=? AND status='approved'`, raw, id); err != nil {
			return err
		}
		if err = writeCommercialAudit(ctx, tx, id, "commercial.distribution_prepared", id, actor, commercial.ContentDigest(raw), now); err != nil {
			return err
		}
		return tx.Commit()
	})
	return record, err
}

func (s *Store) CompleteFreeDistribution(ctx context.Context, id string, document licenseprotocol.DocumentV2, actor string, now time.Time) (commercial.DistributionRecord, error) {
	var record commercial.DistributionRecord
	err := retryCommercial(ctx, func() error {
		tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
		if err != nil {
			return err
		}
		defer tx.Rollback()
		record, err = scanDistribution(tx.QueryRowContext(ctx, distributionSelect+" WHERE id=? FOR UPDATE", id))
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.ErrNotFound
		}
		if err != nil {
			return err
		}
		if record.Status == "approved" {
			return commercial.ErrConflict
		}
		raw, err := json.Marshal(document)
		if err != nil {
			return err
		}
		digest := commercial.ContentDigest(raw)
		if record.Status == "issued" {
			if record.DocumentSHA256 != digest {
				return commercial.ErrConflict
			}
			return tx.Commit()
		}
		record.Status = "issued"
		record.Document = &document
		record.DocumentSHA256 = digest
		if err = record.Validate(); err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, `UPDATE commercial_free_distributions SET status='issued',document_json=?,document_sha256=? WHERE id=? AND status='prepared'`, raw, digest, id); err != nil {
			return err
		}
		if err = writeCommercialAudit(ctx, tx, id, "commercial.distribution_issued", id, actor, digest, now); err != nil {
			return err
		}
		return tx.Commit()
	})
	return record, err
}
