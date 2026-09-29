package mariadb

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/application"
)

// GrantCommercialAdministrator is a local database-administrator operation, never
// exposed through HTTP. Migrations deliberately do not promote existing accounts.
func (store *Store) GrantCommercialAdministrator(ctx context.Context, operatorID string, now time.Time) error {
	if operatorID == "" || len(operatorID) > 64 || strings.TrimSpace(operatorID) != operatorID {
		return errors.New("an explicit operator ID is required")
	}
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var found string
	if err := tx.QueryRowContext(ctx, "SELECT id FROM operators WHERE id=? AND status='active' FOR UPDATE", operatorID).Scan(&found); err != nil {
		return err
	}
	var granted int64
	for _, permission := range application.BootstrapCommercialPermissions() {
		result, err := tx.ExecContext(ctx, `INSERT INTO operator_permissions(operator_id,permission_code,granted_at) VALUES(?,?,?) ON DUPLICATE KEY UPDATE permission_code=VALUES(permission_code)`, operatorID, permission, now.UTC().Truncate(time.Millisecond))
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return err
		}
		granted += count
	}
	if granted > 0 {
		payload, err := json.Marshal(map[string]any{"actor_type": "local_database_administrator", "subject_operator_id": operatorID, "permissions": application.BootstrapCommercialPermissions()})
		if err != nil {
			return err
		}
		// The legacy audit schema requires an operator foreign key. The subject is
		// stored there; actor_type explicitly distinguishes this local command.
		_, err = tx.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES(?,?,'commercial.local_admin_granted','operator',?,?,?)`, operationAuditID(operatorID, fmt.Sprintf("commercial-permissions-%d", now.UnixNano())), operatorID, operatorID, payload, now.UTC().Truncate(time.Millisecond))
		if err != nil {
			return err
		}
	}
	return tx.Commit()
}
