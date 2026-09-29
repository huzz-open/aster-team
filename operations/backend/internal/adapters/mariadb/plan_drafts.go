package mariadb

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

const planDraftSelect = `SELECT draft_id,revision_no,snapshot_json,content_sha256,operation_id,created_by,created_at FROM commercial_plan_draft_revisions`

func scanPlanDraft(row commercialScanner) (commercial.PlanDraftRecord, error) {
	var record commercial.PlanDraftRecord
	var raw []byte
	var id string
	var revision uint32
	if err := row.Scan(&id, &revision, &raw, &record.SHA256, &record.OperationID, &record.CreatedBy, &record.CreatedAt); err != nil {
		return record, err
	}
	var err error
	record.Snapshot, err = commercial.ParsePlanDraft(raw, record.SHA256)
	if err == nil && (id != record.Snapshot.DraftID || revision != record.Snapshot.Revision) {
		return commercial.PlanDraftRecord{}, errors.New("draft content differs from stored identity")
	}
	return record, err
}

func (s *Store) GetPlanDraft(ctx context.Context, id string, revision uint32) (commercial.PlanDraftRecord, error) {
	query := planDraftSelect + " WHERE draft_id=? AND revision_no=?"
	args := []any{id, revision}
	if revision == 0 {
		query = planDraftSelect + " WHERE draft_id=? AND revision_no=(SELECT current_revision FROM commercial_plan_draft_heads WHERE id=?)"
		args = []any{id, id}
	}
	record, err := scanPlanDraft(s.db.QueryRowContext(ctx, query, args...))
	if errors.Is(err, sql.ErrNoRows) {
		err = commercial.ErrNotFound
	}
	return record, err
}

func (s *Store) ListPlanDrafts(ctx context.Context, limit int) ([]commercial.PlanDraftRecord, error) {
	if limit < 1 || limit > 100 {
		limit = 50
	}
	rows, err := s.db.QueryContext(ctx, `SELECT r.draft_id,r.revision_no,r.snapshot_json,r.content_sha256,r.operation_id,r.created_by,r.created_at FROM commercial_plan_draft_heads h JOIN commercial_plan_draft_revisions r ON r.draft_id=h.id AND r.revision_no=h.current_revision ORDER BY h.updated_at DESC,h.id LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]commercial.PlanDraftRecord, 0)
	for rows.Next() {
		record, err := scanPlanDraft(rows)
		if err != nil {
			return nil, err
		}
		items = append(items, record)
	}
	return items, rows.Err()
}

func (s *Store) SavePlanDraft(ctx context.Context, snapshot commercial.PlanDraftSnapshot, operation, actor string, now time.Time) (commercial.PlanDraftRecord, error) {
	if operation == "" || len(operation) > 128 || actor == "" {
		return commercial.PlanDraftRecord{}, errors.New("draft operation and actor are required")
	}
	raw, err := snapshot.Bytes()
	if err != nil {
		return commercial.PlanDraftRecord{}, err
	}
	digest := commercial.ContentDigest(raw)
	snapshot, err = commercial.ParsePlanDraft(raw, digest)
	if err != nil {
		return commercial.PlanDraftRecord{}, err
	}
	var record commercial.PlanDraftRecord
	err = retryCommercial(ctx, func() error {
		var attemptErr error
		record, attemptErr = s.savePlanDraftOnce(ctx, snapshot, raw, digest, operation, actor, now)
		return attemptErr
	})
	return record, err
}

func (s *Store) savePlanDraftOnce(ctx context.Context, snapshot commercial.PlanDraftSnapshot, raw []byte, digest, operation, actor string, now time.Time) (commercial.PlanDraftRecord, error) {
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return commercial.PlanDraftRecord{}, err
	}
	defer tx.Rollback()
	existing, err := scanPlanDraft(tx.QueryRowContext(ctx, planDraftSelect+" WHERE operation_id=? FOR UPDATE", operation))
	if err == nil {
		if existing.SHA256 != digest || existing.CreatedBy != actor {
			return commercial.PlanDraftRecord{}, commercial.ErrConflict
		}
		return existing, tx.Commit()
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return commercial.PlanDraftRecord{}, err
	}
	var head uint32
	err = tx.QueryRowContext(ctx, "SELECT current_revision FROM commercial_plan_draft_heads WHERE id=? FOR UPDATE", snapshot.DraftID).Scan(&head)
	if errors.Is(err, sql.ErrNoRows) {
		if snapshot.Revision != 1 {
			return commercial.PlanDraftRecord{}, commercial.ErrNotFound
		}
		if _, err = tx.ExecContext(ctx, "INSERT INTO commercial_plan_draft_heads(id,current_revision,updated_at) VALUES(?,0,?)", snapshot.DraftID, now); err != nil {
			return commercial.PlanDraftRecord{}, commercialConflict(err)
		}
	} else if err != nil {
		return commercial.PlanDraftRecord{}, err
	} else if head != snapshot.Revision-1 {
		return commercial.PlanDraftRecord{}, commercial.ErrDraftRevisionConflict
	}
	if head > 0 {
		prior, err := scanPlanDraft(tx.QueryRowContext(ctx, planDraftSelect+" WHERE draft_id=? AND revision_no=?", snapshot.DraftID, head))
		if err != nil {
			return commercial.PlanDraftRecord{}, err
		}
		if prior.Snapshot.PlanID != snapshot.PlanID || prior.Snapshot.Definition.Code != snapshot.Definition.Code || snapshot.ExpectedVersion < prior.Snapshot.ExpectedVersion {
			return commercial.PlanDraftRecord{}, commercial.ErrConflict
		}
	}
	if snapshot.ExpectedVersion > 0 {
		base, err := scanCommercialPlan(tx.QueryRowContext(ctx, commercialPlanSelect+" WHERE plan_id=? AND version_no=?", snapshot.PlanID, snapshot.ExpectedVersion))
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.PlanDraftRecord{}, commercial.ErrNotFound
		}
		if err != nil {
			return commercial.PlanDraftRecord{}, err
		}
		if base.Snapshot.Definition.Code != snapshot.Definition.Code {
			return commercial.PlanDraftRecord{}, commercial.ErrConflict
		}
	}
	if _, err = tx.ExecContext(ctx, "INSERT INTO commercial_plan_draft_revisions(draft_id,revision_no,snapshot_json,content_sha256,operation_id,created_by,created_at) VALUES(?,?,?,?,?,?,?)", snapshot.DraftID, snapshot.Revision, raw, digest, operation, actor, now); err != nil {
		return commercial.PlanDraftRecord{}, commercialConflict(err)
	}
	result, err := tx.ExecContext(ctx, "UPDATE commercial_plan_draft_heads SET current_revision=?,updated_at=? WHERE id=? AND current_revision=?", snapshot.Revision, now, snapshot.DraftID, snapshot.Revision-1)
	if err != nil {
		return commercial.PlanDraftRecord{}, err
	}
	if count, err := result.RowsAffected(); err != nil || count != 1 {
		if err != nil {
			return commercial.PlanDraftRecord{}, err
		}
		return commercial.PlanDraftRecord{}, commercial.ErrConflict
	}
	if err := writeCommercialAudit(ctx, tx, operation, "commercial.plan_draft_saved", snapshot.DraftID, actor, digest, now); err != nil {
		return commercial.PlanDraftRecord{}, err
	}
	record := commercial.PlanDraftRecord{Snapshot: snapshot, SHA256: digest, OperationID: operation, CreatedBy: actor, CreatedAt: now}
	return record, tx.Commit()
}

func (s *Store) FreezePlanDraft(ctx context.Context, id string, revision uint32, digest, operation, actor string, now time.Time) (commercial.PlanVersionRecord, error) {
	// The immutable selection is checked again inside the transaction. It never
	// accepts replacement definition/rights supplied by a freeze request.
	draft, err := s.GetPlanDraft(ctx, id, revision)
	if err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	if revision == 0 || draft.SHA256 != digest {
		return commercial.PlanVersionRecord{}, commercial.ErrConflict
	}
	frozen, err := commercial.FreezePlan(draft.Snapshot.PlanID, draft.Snapshot.ExpectedVersion+1, draft.Snapshot.Definition)
	if err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	var record commercial.PlanVersionRecord
	err = retryCommercial(ctx, func() error {
		var attemptErr error
		record, attemptErr = s.freezePlanVersionOnce(ctx, frozen, draft.Snapshot.ExpectedVersion, operation, actor, now, &draft)
		return attemptErr
	})
	return record, err
}

func validateDraftFreeze(ctx context.Context, tx *sql.Tx, draft commercial.PlanDraftRecord) error {
	var current uint32
	if err := tx.QueryRowContext(ctx, "SELECT current_revision FROM commercial_plan_draft_heads WHERE id=? FOR UPDATE", draft.Snapshot.DraftID).Scan(&current); err != nil {
		return err
	}
	if current != draft.Snapshot.Revision {
		return commercial.ErrDraftRevisionConflict
	}
	stored, err := scanPlanDraft(tx.QueryRowContext(ctx, planDraftSelect+" WHERE draft_id=? AND revision_no=? FOR UPDATE", draft.Snapshot.DraftID, current))
	if err != nil {
		return err
	}
	if stored.SHA256 != draft.SHA256 {
		return commercial.ErrConflict
	}
	return nil
}

func validateDraftFreezeRetry(ctx context.Context, tx *sql.Tx, operation string, draft commercial.PlanDraftRecord) error {
	var raw []byte
	if err := tx.QueryRowContext(ctx, "SELECT payload_json FROM audit_events WHERE id=? AND resource_id=?", operationAuditID(operation, "commercial.plan_draft_frozen"), draft.Snapshot.DraftID).Scan(&raw); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return commercial.ErrConflict
		}
		return err
	}
	var payload map[string]string
	if err := json.Unmarshal(raw, &payload); err != nil {
		return err
	}
	if payload["operation_id"] != operation || payload["content_sha256"] != draft.SHA256 {
		return commercial.ErrConflict
	}
	return nil
}
