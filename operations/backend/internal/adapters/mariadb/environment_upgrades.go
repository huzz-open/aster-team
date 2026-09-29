package mariadb

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/domain"
)

func (s *Store) RotateUpgradeCredentials(ctx context.Context, id, sealed, actor string) error {
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var data []byte
	if err = tx.QueryRowContext(ctx, `SELECT metadata_json FROM upgrade_environments WHERE id=? FOR UPDATE`, id).Scan(&data); err != nil {
		return err
	}
	var env domain.UpgradeEnvironment
	if err = json.Unmarshal(data, &env); err != nil {
		return err
	}
	env.CredentialVersion++
	data, err = json.Marshal(env)
	if err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `UPDATE upgrade_environments SET metadata_json=?,credentials_ciphertext=? WHERE id=?`, data, sealed, id); err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events (id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES (CONCAT(?, '_credentials_', ?),?,'environment.credentials.rotated','upgrade_environment',?,'{}',UTC_TIMESTAMP(6))`, id, env.CredentialVersion, actor, id); err != nil {
		return err
	}
	return tx.Commit()
}

func (s *Store) CreateUpgradeEnvironment(ctx context.Context, env domain.UpgradeEnvironment, sealed, actor string) error {
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	data, err := json.Marshal(env)
	if err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO upgrade_environments (id,installation_id,metadata_json,credentials_ciphertext,created_at) VALUES (?,?,?,?,?)`, env.ID, env.InstallationID, data, sealed, env.CreatedAt); err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events (id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES (?,?,'environment.created','upgrade_environment',?,'{}',?)`, env.ID+"_created", actor, env.ID, env.CreatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (s *Store) ListUpgradeEnvironments(ctx context.Context) ([]domain.UpgradeEnvironment, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT metadata_json FROM upgrade_environments ORDER BY created_at DESC LIMIT 200`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.UpgradeEnvironment{}
	for rows.Next() {
		var data []byte
		var item domain.UpgradeEnvironment
		if err = rows.Scan(&data); err != nil {
			return nil, err
		}
		if err = json.Unmarshal(data, &item); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (s *Store) GetUpgradeEnvironment(ctx context.Context, id string) (domain.UpgradeEnvironment, string, error) {
	var item domain.UpgradeEnvironment
	var data []byte
	var sealed string
	err := s.db.QueryRowContext(ctx, `SELECT metadata_json,credentials_ciphertext FROM upgrade_environments WHERE id=?`, id).Scan(&data, &sealed)
	if errors.Is(err, sql.ErrNoRows) {
		err = application.ErrNotFound
	}
	if err == nil {
		err = json.Unmarshal(data, &item)
	}
	return item, sealed, err
}

func (s *Store) CreateEnvironmentUpgrade(ctx context.Context, task domain.EnvironmentUpgrade) error {
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	data, err := json.Marshal(task)
	if err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO environment_upgrades (id,environment_id,active_environment_id,task_json,created_at) VALUES (?,?,?,?,?)`, task.ID, task.EnvironmentID, task.EnvironmentID, data, task.CreatedAt); err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events (id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES (?,?,'environment.upgrade.created','environment_upgrade',?,'{}',?)`, task.ID+"_created", task.CreatedBy, task.ID, task.CreatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (s *Store) ListEnvironmentUpgrades(ctx context.Context) ([]domain.EnvironmentUpgrade, error) {
	rows, err := s.db.QueryContext(ctx, `SELECT task_json FROM environment_upgrades ORDER BY created_at DESC LIMIT 100`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.EnvironmentUpgrade{}
	for rows.Next() {
		var data []byte
		var item domain.EnvironmentUpgrade
		if err = rows.Scan(&data); err != nil {
			return nil, err
		}
		if err = json.Unmarshal(data, &item); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (s *Store) GetEnvironmentUpgrade(ctx context.Context, id string, after int64) (domain.EnvironmentUpgradeDetail, error) {
	result := domain.EnvironmentUpgradeDetail{Samples: []domain.UpgradeProbeSample{}}
	var data []byte
	err := s.db.QueryRowContext(ctx, `SELECT task_json FROM environment_upgrades WHERE id=?`, id).Scan(&data)
	if errors.Is(err, sql.ErrNoRows) {
		err = application.ErrNotFound
	}
	if err != nil {
		return result, err
	}
	if err = json.Unmarshal(data, &result.Task); err != nil {
		return result, err
	}
	rows, err := s.db.QueryContext(ctx, `SELECT sequence,sample_json FROM upgrade_probe_samples WHERE task_id=? AND sequence>? ORDER BY sequence LIMIT 500`, id, after)
	if err != nil {
		return result, err
	}
	defer rows.Close()
	for rows.Next() {
		var sample domain.UpgradeProbeSample
		var sequence int64
		if err = rows.Scan(&sequence, &data); err != nil {
			return result, err
		}
		if err = json.Unmarshal(data, &sample); err != nil {
			return result, err
		}
		sample.Sequence = sequence
		result.Samples = append(result.Samples, sample)
	}
	return result, rows.Err()
}

func (s *Store) ClaimEnvironmentUpgrade(ctx context.Context, owner string) (domain.EnvironmentUpgrade, bool, error) {
	var task domain.EnvironmentUpgrade
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return task, false, err
	}
	defer tx.Rollback()
	var data []byte
	err = tx.QueryRowContext(ctx, `SELECT task_json FROM environment_upgrades WHERE active_environment_id IS NOT NULL AND (lease_until IS NULL OR lease_until<UTC_TIMESTAMP(6)) ORDER BY created_at LIMIT 1 FOR UPDATE`).Scan(&data)
	if errors.Is(err, sql.ErrNoRows) {
		return task, false, nil
	}
	if err != nil {
		return task, false, err
	}
	if err = json.Unmarshal(data, &task); err != nil {
		return task, false, err
	}
	if task.Phase != "queued" {
		task.CoverageGap = true
	}
	data, err = json.Marshal(task)
	if err != nil {
		return task, false, err
	}
	_, err = tx.ExecContext(ctx, `UPDATE environment_upgrades SET lease_owner=?,lease_until=DATE_ADD(UTC_TIMESTAMP(6),INTERVAL 45 SECOND),task_json=? WHERE id=?`, owner, data, task.ID)
	if err != nil {
		return task, false, err
	}
	err = tx.Commit()
	return task, err == nil, err
}

func (s *Store) RenewEnvironmentUpgrade(ctx context.Context, id, owner string) error {
	result, err := s.db.ExecContext(ctx, `UPDATE environment_upgrades SET lease_until=DATE_ADD(UTC_TIMESTAMP(6),INTERVAL 45 SECOND) WHERE id=? AND lease_owner=? AND lease_until>=UTC_TIMESTAMP(6) AND active_environment_id IS NOT NULL`, id, owner)
	return upgradeLeaseResult(result, err)
}

func upgradeLeaseResult(result sql.Result, err error) error {
	if err != nil {
		return err
	}
	count, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if count != 1 {
		return errors.New("environment upgrade lease lost")
	}
	return nil
}

func (s *Store) SaveEnvironmentUpgrade(ctx context.Context, task domain.EnvironmentUpgrade, owner string, samples []domain.UpgradeProbeSample, terminal bool) error {
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	data, err := json.Marshal(task)
	if err != nil {
		return err
	}
	var active any = task.EnvironmentID
	if terminal {
		active = nil
	}
	result, err := tx.ExecContext(ctx, `UPDATE environment_upgrades SET task_json=?,active_environment_id=? WHERE id=? AND lease_owner=? AND lease_until>=UTC_TIMESTAMP(6)`, data, active, task.ID, owner)
	if err = upgradeLeaseResult(result, err); err != nil {
		return err
	}
	for _, sample := range samples {
		data, err = json.Marshal(sample)
		if err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, `INSERT INTO upgrade_probe_samples (task_id,sample_json) VALUES (?,?)`, task.ID, data); err != nil {
			return err
		}
	}
	if terminal {
		payload, _ := json.Marshal(map[string]string{"upgrade_result": task.UpgradeResult, "recovery_result": task.RecoveryResult})
		if _, err = tx.ExecContext(ctx, `INSERT IGNORE INTO audit_events (id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES (?,?,'environment.upgrade.completed','environment_upgrade',?,?,?)`, task.ID+"_completed", task.CreatedBy, task.ID, payload, time.Now().UTC()); err != nil {
			return err
		}
	}
	return tx.Commit()
}
