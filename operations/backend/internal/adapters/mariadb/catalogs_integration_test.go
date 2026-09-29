package mariadb

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"net"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
	operationsschema "aster.local/team/operations/backend/schema"
	mysqldriver "github.com/go-sql-driver/mysql"
)

func testCatalogLifecycle(t *testing.T, ctx context.Context, db *sql.DB, store *Store, definition commercial.Definition, actor string, now time.Time, suffix string) {
	definition.Code = "public_" + suffix
	plan, err := store.FreezePlanVersion(ctx, "public_"+suffix, 0, definition, "public_plan_"+suffix, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	request := commercial.CatalogRequest{OperationID: "public_approve_" + suffix, Environment: "local", Reason: "INTERNAL_NOT_PUBLIC", Plans: []commercial.CatalogSelection{{PlanID: plan.Snapshot.PlanID, Version: 1, ExpectedSHA256: plan.SHA256}}}
	id := "catalog_" + commercial.ContentDigest([]byte(suffix))[:48]
	preview, err := store.PreviewPublicCatalog(ctx, id, request)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetCatalogApproval(ctx, id); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatal("preview saved approval", err)
	}
	input := commercial.ApproveCatalogInput{Request: request, ExpectedPublicSHA256: preview.SHA256}
	wrong := input
	wrong.ExpectedPublicSHA256 = strings.Repeat("a", 64)
	if _, err := store.ApprovePublicCatalog(ctx, id, wrong, actor, now); !errors.Is(err, commercial.ErrCatalogPreviewConflict) {
		t.Fatal("wrong preview accepted", err)
	}
	// A failure at the final audit must roll back approval and public bytes.
	auditID := operationAuditID(request.OperationID, "commercial.catalog_approved")
	if _, err := db.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES(?,?,'test.injected_failure','commercial',?,'{}',?)`, auditID, actor, id, now); err != nil {
		t.Fatal(err)
	}
	if _, err := store.ApprovePublicCatalog(ctx, id, input, actor, now); err == nil {
		t.Fatal("audit failure ignored")
	}
	if _, err := store.GetCatalogApproval(ctx, id); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatal("partial approval persisted", err)
	}
	if _, err := db.ExecContext(ctx, "DELETE FROM audit_events WHERE id=? AND action='test.injected_failure'", auditID); err != nil {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	results := make(chan error, 6)
	for range 6 {
		wg.Add(1)
		go func() { defer wg.Done(); _, e := store.ApprovePublicCatalog(ctx, id, input, actor, now); results <- e }()
	}
	wg.Wait()
	close(results)
	for err := range results {
		if err != nil {
			t.Fatal("concurrent approval", err)
		}
	}
	record, err := store.GetCatalogApproval(ctx, id)
	if err != nil {
		t.Fatal(err)
	}
	public, err := record.PublicBytes()
	if err != nil || bytes.Contains(public, []byte(request.Reason)) {
		t.Fatal("invalid public projection", err)
	}
	definition.Name = "changed after approval"
	if _, err := store.FreezePlanVersion(ctx, plan.Snapshot.PlanID, 1, definition, "public_plan_next_"+suffix, actor, now); err != nil {
		t.Fatal(err)
	}
	again, err := store.ApprovePublicCatalog(ctx, id, input, actor, now.Add(time.Hour))
	if err != nil || again.SHA256 != record.SHA256 || again.Snapshot.ApprovedAt != record.Snapshot.ApprovedAt {
		t.Fatal("retry changed source/time", err)
	}
	changed := input
	changed.Request.Reason = "another reason"
	if _, err := store.ApprovePublicCatalog(ctx, id, changed, actor, now); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("operation changed its request", err)
	}
	changed = input
	changed.Request.Environment = "production"
	if _, err := store.ApprovePublicCatalog(ctx, id, changed, actor, now); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("local approval became production", err)
	}
	if _, err := store.ApprovePublicCatalog(ctx, id, input, "other_actor", now); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("actor reuse accepted", err)
	}
	var auditCount int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE resource_id=? AND action='commercial.catalog_approved'", id).Scan(&auditCount); err != nil || auditCount != 1 {
		t.Fatal("approval audit duplicated", auditCount, err)
	}
	exportAudit := operationAuditID(id, "commercial.catalog_exported")
	if _, err := db.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES(?,?,'test.injected_failure','commercial',?,'{}',?)`, exportAudit, actor, id, now); err != nil {
		t.Fatal(err)
	}
	if _, err := store.MarkCatalogExported(ctx, id, record.Public.SHA256, actor, now); err == nil {
		t.Fatal("export audit failure ignored")
	}
	current, err := store.GetCatalogApproval(ctx, id)
	if err != nil || current.Status != "approved" {
		t.Fatal("export failure changed status", err)
	}
	if _, err := db.ExecContext(ctx, "DELETE FROM audit_events WHERE id=? AND action='test.injected_failure'", exportAudit); err != nil {
		t.Fatal(err)
	}
	for range 2 {
		current, err = store.MarkCatalogExported(ctx, id, record.Public.SHA256, actor, now)
		if err != nil || current.Status != "exported" {
			t.Fatal(err)
		}
	}
	if _, err := store.MarkCatalogExported(ctx, id, strings.Repeat("f", 64), actor, now); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("wrong export acknowledged", err)
	}
	// Stored public bytes are verified exactly, not permissively decoded.
	if _, err := db.ExecContext(ctx, "UPDATE commercial_catalog_approvals SET public_json=? WHERE id=?", append(public, '\n'), id); err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetCatalogApproval(ctx, id); err == nil {
		t.Fatal("noncanonical public file accepted")
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_catalog_approvals SET public_json=? WHERE id=?", public, id); err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_catalog_approvals SET environment='production' WHERE id=?", id); err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetCatalogApproval(ctx, id); err == nil {
		t.Fatal("row environment mismatch accepted")
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_catalog_approvals SET environment='local' WHERE id=?", id); err != nil {
		t.Fatal(err)
	}
}

func TestCatalogPermissionUpgradeIntegration(t *testing.T) {
	address := os.Getenv("ASTER_COMMERCIAL_TEST_ADDR")
	if address == "" {
		t.Skip("requires disposable local MariaDB")
	}
	host, _, err := net.SplitHostPort(address)
	if err != nil || (host != "127.0.0.1" && host != "localhost") {
		t.Fatal("local test database required")
	}
	cfg := mysqldriver.NewConfig()
	cfg.Net = "tcp"
	cfg.Addr = address
	cfg.User = "root"
	cfg.Passwd = "aster-test-only"
	cfg.ParseTime = true
	cfg.Loc = time.UTC
	connector, err := mysqldriver.NewConnector(cfg)
	if err != nil {
		t.Fatal(err)
	}
	admin := sql.OpenDB(connector)
	defer admin.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 45*time.Second)
	defer cancel()
	databaseName := fmt.Sprintf("aster_catalog_upgrade_test_%d", time.Now().UnixNano())
	if _, err := admin.ExecContext(ctx, "CREATE DATABASE "+databaseName); err != nil {
		t.Fatal(err)
	}
	defer admin.ExecContext(context.Background(), "DROP DATABASE "+databaseName)
	cfg.DBName = databaseName
	connector, err = mysqldriver.NewConnector(cfg)
	if err != nil {
		t.Fatal(err)
	}
	db := sql.OpenDB(connector)
	defer db.Close()
	db.SetMaxOpenConns(1)
	if _, err := db.ExecContext(ctx, `CREATE TABLE operations_schema_migrations(version VARCHAR(255) PRIMARY KEY,checksum CHAR(64) NOT NULL,applied_at DATETIME(6) NOT NULL)`); err != nil {
		t.Fatal(err)
	}
	paths, err := schemaFilePaths(operationsschema.Files)
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range paths {
		// Build the actual pre-catalog baseline. Later migrations may reference
		// catalog tables, so skipping only 005 would construct an impossible schema.
		if path != "init.mariadb.sql" && path >= "202609060005/" {
			continue
		}
		raw, err := operationsschema.Files.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		raw = canonicalMigrationContents(raw)
		for _, statement := range splitStatements(string(raw)) {
			if _, err := db.ExecContext(ctx, statement); err != nil {
				t.Fatal(err)
			}
		}
		sum := sha256.Sum256(raw)
		if _, err := db.ExecContext(ctx, "INSERT INTO operations_schema_migrations VALUES(?,?,UTC_TIMESTAMP(6))", path, hex.EncodeToString(sum[:])); err != nil {
			t.Fatal(err)
		}
	}
	now := time.Now().UTC().Truncate(time.Millisecond)
	if _, err := db.ExecContext(ctx, `INSERT INTO operators(id,email,normalized_email,display_name,password_hash,status,password_change_required,created_at,updated_at) VALUES('old','old@test.invalid','old@test.invalid','old','fixture','active',FALSE,?,?)`, now, now); err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, "INSERT INTO operator_permissions VALUES('old','release.read',?)", now); err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, "INSERT INTO operator_permissions VALUES('old','commercial.catalog.read',?)", now); err == nil {
		t.Fatal("old schema unexpectedly allows catalog permission")
	}
	for range 2 {
		if err := Migrate(ctx, db); err != nil {
			t.Fatal(err)
		}
	}
	store := NewStore(db)
	for _, permission := range []string{application.PermissionCatalogRead, application.PermissionCatalogApprove, application.PermissionCatalogExport} {
		allowed, err := store.HasPermission(ctx, "old", permission)
		if err != nil || allowed {
			t.Fatal("upgrade silently granted permission", err)
		}
	}
	allowed, err := store.HasPermission(ctx, "old", "release.read")
	if err != nil || !allowed {
		t.Fatal("upgrade removed old permission", err)
	}
	if err := store.GrantCommercialAdministrator(ctx, "old", now); err != nil {
		t.Fatal(err)
	}
	for _, permission := range application.BootstrapCommercialPermissions() {
		allowed, err := store.HasPermission(ctx, "old", permission)
		if err != nil || !allowed {
			t.Fatal("explicit grant failed", permission, err)
		}
	}
	if _, err := db.ExecContext(ctx, "INSERT INTO operator_permissions VALUES('old','unknown.permission',?)", now); err == nil {
		t.Fatal("registry widened to arbitrary permissions")
	}
}
