package mariadb

import (
	"context"
	"database/sql"
	"time"
)

func (store *Store) FinishReleaseTaskVerification(ctx context.Context, taskID string, taskError *string, now time.Time) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var status string
	if err := tx.QueryRowContext(ctx, `SELECT status FROM release_tasks WHERE id=? FOR UPDATE`, taskID).Scan(&status); err != nil {
		return err
	}
	if status != "verifying" {
		return tx.Commit()
	}
	var queued, failed int
	var packageError sql.NullString
	if err := tx.QueryRowContext(ctx, `SELECT COALESCE(SUM(verification_status='queued'),0),
		COALESCE(SUM(verification_status IN ('failed','unavailable')),0), MAX(verification_error_code)
		FROM release_task_artifacts WHERE release_task_id=? AND platform<>''`, taskID).Scan(&queued, &failed, &packageError); err != nil {
		return err
	}
	if queued != 0 {
		return tx.Commit()
	}
	status = "completed"
	if taskError != nil || failed != 0 {
		status = "failed"
		if taskError == nil {
			code := "RELEASE_ARTIFACT_UNAVAILABLE"
			if packageError.Valid {
				code = packageError.String
			}
			taskError = &code
		}
	}
	if _, err := tx.ExecContext(ctx, `UPDATE release_tasks SET status=?, phase=?, error_code=?, active_dedup_key=NULL,
		updated_at=?, completed_at=? WHERE id=?`, status, status, taskError, now, now, taskID); err != nil {
		return err
	}
	return tx.Commit()
}
