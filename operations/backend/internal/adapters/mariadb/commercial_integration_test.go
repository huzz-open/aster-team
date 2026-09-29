package mariadb

import (
	"context"
	"database/sql"
	"encoding/json"
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
	"aster.local/team/operations/backend/internal/productcatalog"
	mysqldriver "github.com/go-sql-driver/mysql"
)

// This suite uses a disposable local MariaDB container, never an Operations profile.
func TestCommercialStoreIntegration(t *testing.T) {
	address := os.Getenv("ASTER_COMMERCIAL_TEST_ADDR")
	if address == "" {
		t.Skip("set ASTER_COMMERCIAL_TEST_ADDR to a disposable local MariaDB test container")
	}
	host, _, err := net.SplitHostPort(address)
	if err != nil || (host != "127.0.0.1" && host != "localhost") {
		t.Fatal("integration database must be explicitly local")
	}
	cfg := mysqldriver.NewConfig()
	cfg.Net = "tcp"
	cfg.Addr = address
	cfg.User = "root"
	cfg.Passwd = "aster-test-only"
	cfg.DBName = "aster_contracts_test"
	cfg.ParseTime = true
	cfg.Loc = time.UTC
	connector, err := mysqldriver.NewConnector(cfg)
	if err != nil {
		t.Fatal(err)
	}
	db := sql.OpenDB(connector)
	defer db.Close()
	db.SetMaxOpenConns(1)
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	if err := db.PingContext(ctx); err != nil {
		t.Fatal(err)
	}
	if err := Migrate(ctx, db); err != nil {
		t.Fatal(err)
	}
	if err := Migrate(ctx, db); err != nil {
		t.Fatalf("migration not repeatable: %v", err)
	}
	db.SetMaxOpenConns(8)
	suffix := fmt.Sprintf("%d", time.Now().UnixNano())
	operatorID := "operator_" + suffix
	customerID := "customer_" + suffix
	planID := "plan_" + suffix
	now := time.Now().UTC().Truncate(time.Millisecond)
	if _, err := db.ExecContext(ctx, `INSERT INTO operators(id,email,normalized_email,display_name,password_hash,status,password_change_required,created_at,updated_at) VALUES(?,?,?,?,?,'active',FALSE,?,?)`, operatorID, suffix+"@test.invalid", suffix+"@test.invalid", "Test operator", "not-a-login-password", now, now); err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, `INSERT INTO customers(id,name,status,notes,created_at,updated_at) VALUES(?,?,'active','test-only',?,?)`, customerID, "Test customer", now, now); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile("../../../../../contracts/test-vectors/plan-definition.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var definition commercial.Definition
	if err := json.Unmarshal(data, &definition); err != nil {
		t.Fatal(err)
	}
	definition.Code = "code_" + suffix
	store := NewStore(db)
	for _, permission := range application.BootstrapCommercialPermissions() {
		allowed, err := store.HasPermission(ctx, operatorID, permission)
		if err != nil || allowed {
			t.Fatalf("migration granted an existing account: %v %v", allowed, err)
		}
	}
	if err := store.GrantCommercialAdministrator(ctx, operatorID, now); err != nil {
		t.Fatal(err)
	}
	if err := store.GrantCommercialAdministrator(ctx, operatorID, now.Add(time.Second)); err != nil {
		t.Fatal(err)
	}
	for _, permission := range application.BootstrapCommercialPermissions() {
		allowed, err := store.HasPermission(ctx, operatorID, permission)
		if err != nil || !allowed {
			t.Fatalf("explicit grant missing: %v %v", allowed, err)
		}
	}
	var grants int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE resource_id=? AND action='commercial.local_admin_granted'", operatorID).Scan(&grants); err != nil || grants != 1 {
		t.Fatalf("repeated grant audit: %d %v", grants, err)
	}
	if err := store.GrantCommercialAdministrator(ctx, "missing_"+suffix, now); !errors.Is(err, sql.ErrNoRows) {
		t.Fatalf("unknown grant target: %v", err)
	}
	bootstrap, err := store.CreateOperator(ctx, application.CreateOperatorParams{ID: "bootstrap_" + suffix, Email: "bootstrap_" + suffix + "@test.invalid", NormalizedEmail: "bootstrap_" + suffix + "@test.invalid", DisplayName: "Test bootstrap", PasswordHash: "not-a-login-password", CreatedAt: now})
	if err != nil {
		t.Fatal(err)
	}
	for _, permission := range application.BootstrapCommercialPermissions() {
		allowed, err := store.HasPermission(ctx, bootstrap.ID, permission)
		if err != nil || !allowed {
			t.Fatalf("initial administrator missing permission %s: %v", permission, err)
		}
	}

	first, err := store.FreezePlanVersion(ctx, planID, 0, definition, "freeze_"+suffix, operatorID, now)
	if err != nil {
		t.Fatal(err)
	}
	again, err := store.FreezePlanVersion(ctx, planID, 0, definition, "freeze_"+suffix, operatorID, now.Add(time.Hour))
	if err != nil || again.SHA256 != first.SHA256 || !again.CreatedAt.Equal(first.CreatedAt) {
		t.Fatalf("idempotent freeze changed: %v", err)
	}
	orderID := "order_" + suffix
	order, err := store.CreateCommercialOrder(ctx, orderID, "order_op_"+suffix, customerID, planID, 1, 3, now, operatorID, now)
	if err != nil {
		t.Fatal(err)
	}
	if order.Snapshot.AmountMinor != 1529745 {
		t.Fatalf("wrong three-year total: %d", order.Snapshot.AmountMinor)
	}
	definition.Name = "Revised plan"
	definition.Offer.AnnualAmountMinor = 999900
	for i := range definition.Entitlements.Quotas {
		if definition.Entitlements.Quotas[i].ID == productcatalog.QuotaMemberSeats {
			definition.Entitlements.Quotas[i].Limit = productcatalog.Limited(50)
		}
	}
	second, err := store.FreezePlanVersion(ctx, planID, 1, definition, "freeze2_"+suffix, operatorID, now.Add(time.Minute))
	if err != nil || second.Snapshot.Version != 2 {
		t.Fatalf("new version: %v", err)
	}
	current, err := store.GetCurrentCommercialPlan(ctx, planID)
	if err != nil || current.SHA256 != second.SHA256 || current.Snapshot.Version != 2 {
		t.Fatalf("current version lookup: %v", err)
	}
	if _, err := store.GetCurrentCommercialPlan(ctx, "missing_"+suffix); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatalf("missing current plan: %v", err)
	}
	if _, err := store.FreezePlanVersion(ctx, planID, 1, definition, "stale_"+suffix, operatorID, now); !errors.Is(err, commercial.ErrPlanVersionConflict) {
		t.Fatalf("absent operation with stale head must be recoverable: %v", err)
	}
	if _, err := store.FreezePlanVersion(ctx, planID, 0, definition, "freeze_"+suffix, operatorID, now); !errors.Is(err, commercial.ErrConflict) || errors.Is(err, commercial.ErrPlanVersionConflict) {
		t.Fatalf("existing mismatched operation must not be treated as absent: %v", err)
	}
	retained, err := store.GetCommercialOrder(ctx, orderID)
	if err != nil || retained.SHA256 != order.SHA256 || retained.Snapshot.AmountMinor != 1529745 || retained.Snapshot.Plan.Version != 1 {
		t.Fatalf("new plan changed old order: %v", err)
	}
	original, err := store.GetCommercialPlan(ctx, planID, 1)
	if err != nil || original.SHA256 != first.SHA256 {
		t.Fatal("old plan was overwritten")
	}
	againOrder, err := store.CreateCommercialOrder(ctx, orderID, "order_op_"+suffix, customerID, planID, 1, 3, now, operatorID, now.Add(time.Hour))
	if err != nil || againOrder.SHA256 != order.SHA256 {
		t.Fatalf("order retry changed: %v", err)
	}
	if _, err := store.CreateCommercialOrder(ctx, orderID, "order_op_"+suffix, customerID, planID, 2, 3, now, operatorID, now); !errors.Is(err, commercial.ErrConflict) {
		t.Fatalf("mismatched retry accepted: %v", err)
	}
	var wg sync.WaitGroup
	results := make(chan error, 2)
	for i := 0; i < 2; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			_, err := store.FreezePlanVersion(ctx, planID, 2, definition, fmt.Sprintf("race_%s_%d", suffix, i), operatorID, now.Add(2*time.Minute))
			results <- err
		}(i)
	}
	wg.Wait()
	close(results)
	passed, conflicts := 0, 0
	for err := range results {
		if err == nil {
			passed++
		} else if errors.Is(err, commercial.ErrConflict) {
			conflicts++
		} else {
			t.Fatalf("unexpected concurrency failure: %v", err)
		}
	}
	if passed != 1 || conflicts != 1 {
		t.Fatalf("concurrent revisions: %d successful %d conflicts", passed, conflicts)
	}
	var versions, audits int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM commercial_plan_versions WHERE plan_id=?", planID).Scan(&versions); err != nil {
		t.Fatal(err)
	}
	if versions != 3 {
		t.Fatalf("unexpected version count %d", versions)
	}
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE resource_id=? AND action='commercial.order_created'", orderID).Scan(&audits); err != nil || audits != 1 {
		t.Fatalf("order retry duplicated audit: %d %v", audits, err)
	}
	// A valid digest must not identify another row's order.
	forged := order.Snapshot
	forged.OrderID = "other_" + suffix
	forgedRaw, err := forged.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_orders SET snapshot_json=?,content_sha256=? WHERE id=?", forgedRaw, commercial.ContentDigest(forgedRaw), orderID); err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetCommercialOrder(ctx, orderID); err == nil {
		t.Fatal("different order identity accepted with a valid digest")
	}
	originalRaw, err := order.Snapshot.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_orders SET snapshot_json=?,content_sha256=? WHERE id=?", originalRaw, order.SHA256, orderID); err != nil {
		t.Fatal(err)
	}
	// Corrupt only this disposable test record; reads must fail on the persisted digest.
	if _, err := db.ExecContext(ctx, "UPDATE commercial_orders SET content_sha256=? WHERE id=?", strings.Repeat("0", 64), orderID); err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetCommercialOrder(ctx, orderID); err == nil {
		t.Fatal("corrupt snapshot accepted")
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_orders SET content_sha256=? WHERE id=?", order.SHA256, orderID); err != nil {
		t.Fatal(err)
	}
	t.Run("free_distribution_lifecycle", func(t *testing.T) { testDistributionLifecycle(t, ctx, db, store, definition, operatorID, now, suffix) })
	t.Run("plan_draft_lifecycle", func(t *testing.T) { testPlanDraftLifecycle(t, ctx, db, store, definition, operatorID, now, suffix) })
	t.Run("public_catalog_lifecycle", func(t *testing.T) { testCatalogLifecycle(t, ctx, db, store, definition, operatorID, now, suffix) })
	t.Run("publication_lifecycle", func(t *testing.T) { testPublicationLifecycle(t, ctx, db, store, definition, operatorID, now, suffix) })
	t.Run("paid_fulfillment_lifecycle", func(t *testing.T) { testPaidFulfillmentLifecycle(t, ctx, db, store, operatorID, suffix) })
}
