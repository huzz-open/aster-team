package mariadb

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/domain"
	operationsschema "aster.local/team/operations/backend/schema"
	mysqldriver "github.com/go-sql-driver/mysql"
)

func TestEnvironmentUpgradeDatabaseFencesRecoveryAndPreservesEvidence(t *testing.T) {
	dsn := os.Getenv("ASTER_OPERATIONS_TEST_DB_DSN")
	if dsn == "" {
		t.Skip("set ASTER_OPERATIONS_TEST_DB_DSN with CREATE DATABASE permission for isolated upgrade tests")
	}
	cfg, err := mysqldriver.ParseDSN(dsn)
	if err != nil {
		t.Fatal("invalid test database DSN")
	}
	cfg.ParseTime = true
	connector, err := mysqldriver.NewConnector(cfg)
	if err != nil {
		t.Fatal(err)
	}
	admin := sql.OpenDB(connector)
	defer admin.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	databaseName := fmt.Sprintf("aster_upgrade_test_%d", time.Now().UnixNano())
	if _, err = admin.ExecContext(ctx, "CREATE DATABASE "+databaseName); err != nil {
		t.Fatal(err)
	}
	defer func() {
		if _, err := admin.ExecContext(context.Background(), "DROP DATABASE "+databaseName); err != nil {
			t.Error(err)
		}
	}()
	cfg.DBName = databaseName
	connector, err = mysqldriver.NewConnector(cfg)
	if err != nil {
		t.Fatal(err)
	}
	db := sql.OpenDB(connector)
	defer db.Close()
	if err = Migrate(ctx, db); err != nil {
		t.Fatal(err)
	}
	if err = Migrate(ctx, db); err != nil {
		t.Fatal("migration replay", err)
	}
	store := NewStore(db)
	now := time.Now().UTC()
	_, err = store.CreateOperator(ctx, application.CreateOperatorParams{ID: "operator_test", Email: "operator@test.invalid", NormalizedEmail: "operator@test.invalid", DisplayName: "Test", PasswordHash: "not-a-login-fixture", CreatedAt: now})
	if err != nil {
		t.Fatal(err)
	}
	for _, permission := range []string{application.PermissionEnvironmentWrite, application.PermissionEnvironmentUpgrade} {
		ok, err := store.HasPermission(ctx, "operator_test", permission)
		if err != nil || !ok {
			t.Fatal("new permission missing", err)
		}
	}
	// Reapply the dated statements to cover interrupted-migration retries too.
	contents, err := fs.ReadFile(operationsschema.Files, "202609081100/environment_upgrades.mariadb.sql")
	if err != nil {
		t.Fatal(err)
	}
	for _, statement := range splitStatements(string(contents)) {
		if _, err = db.ExecContext(ctx, statement); err != nil {
			t.Fatal("dated migration is not restartable", err)
		}
	}
	env := domain.UpgradeEnvironment{ID: "environment_test", InstallationID: "installation_test", Name: "Test", CredentialVersion: 1, CreatedAt: now}
	if err = store.CreateUpgradeEnvironment(ctx, env, "ciphertext-v1", "operator_test"); err != nil {
		t.Fatal(err)
	}
	duplicate := env
	duplicate.ID = "environment_alias"
	if store.CreateUpgradeEnvironment(ctx, duplicate, "other", "operator_test") == nil {
		t.Fatal("installation alias allowed duplicate upgrade locks")
	}
	if err = store.RotateUpgradeCredentials(ctx, env.ID, "ciphertext-v2", "operator_test"); err != nil {
		t.Fatal(err)
	}
	updated, sealed, err := store.GetUpgradeEnvironment(ctx, env.ID)
	if err != nil || updated.CredentialVersion != 2 || sealed != "ciphertext-v2" {
		t.Fatal("rotation failed", err)
	}
	public, _ := json.Marshal(updated)
	if strings.Contains(string(public), "ciphertext") {
		t.Fatal("credential escaped public metadata")
	}
	task := domain.EnvironmentUpgrade{ID: "upgrade_test", EnvironmentID: env.ID, CreatedBy: "operator_test", Phase: "queued", CreatedAt: now, UpdatedAt: now}
	if err = store.CreateEnvironmentUpgrade(ctx, task); err != nil {
		t.Fatal(err)
	}
	other := task
	other.ID = "upgrade_duplicate"
	if store.CreateEnvironmentUpgrade(ctx, other) == nil {
		t.Fatal("parallel target upgrade accepted")
	}
	claimed, ok, err := store.ClaimEnvironmentUpgrade(ctx, "worker_one")
	if err != nil || !ok {
		t.Fatal("claim failed", err)
	}
	if _, ok, err = store.ClaimEnvironmentUpgrade(ctx, "worker_two"); err != nil || ok {
		t.Fatal("live lease was stolen", err)
	}
	claimed.Phase = "uploading"
	claimed.UpdatedAt = now.Add(time.Second)
	if err = store.SaveEnvironmentUpgrade(ctx, claimed, "worker_one", nil, false); err != nil {
		t.Fatal(err)
	}
	if _, err = db.ExecContext(ctx, `UPDATE environment_upgrades SET lease_until=DATE_SUB(UTC_TIMESTAMP(6),INTERVAL 1 SECOND) WHERE id=?`, task.ID); err != nil {
		t.Fatal(err)
	}
	if store.RenewEnvironmentUpgrade(ctx, task.ID, "worker_one") == nil {
		t.Fatal("expired lease revived")
	}
	claimed, ok, err = store.ClaimEnvironmentUpgrade(ctx, "worker_two")
	if err != nil || !ok || !claimed.CoverageGap || claimed.Phase != "uploading" {
		t.Fatal("recovery lost upload checkpoint", err)
	}
	if store.SaveEnvironmentUpgrade(ctx, claimed, "worker_one", nil, true) == nil {
		t.Fatal("stale worker completed task")
	}
	claimed.Phase = "completed"
	claimed.UpdatedAt = now.Add(2 * time.Second)
	claimed.UpgradeResult = "succeeded"
	claimed.RecoveryResult = "recovered_with_observation_gap"
	if err = store.SaveEnvironmentUpgrade(ctx, claimed, "worker_two", []domain.UpgradeProbeSample{{Kind: "stream", OK: false, ErrorCode: "interrupted"}}, true); err != nil {
		t.Fatal(err)
	}
	detail, err := store.GetEnvironmentUpgrade(ctx, task.ID, 0)
	if err != nil || len(detail.Samples) != 1 || detail.Samples[0].OK {
		t.Fatal("failed sample lost", err)
	}
	if err = store.CreateEnvironmentUpgrade(ctx, other); err != nil {
		t.Fatal("completed target not unlocked", err)
	}
}
