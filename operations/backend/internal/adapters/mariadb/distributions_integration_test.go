package mariadb

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/x509"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"errors"
	"sync"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/adapters/licensing"
	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
)

func testDistributionLifecycle(t *testing.T, ctx context.Context, db *sql.DB, store *Store, definition commercial.Definition, actor string, now time.Time, suffix string) {
	t.Helper()
	id := "dist_" + suffix
	planID := "free_plan_" + suffix
	definition.Code = "free_code_" + suffix
	definition.Offer = commercial.Offer{Kind: "free", Expiry: &licenseprotocol.ExpiryV2{Mode: licenseprotocol.FixedExpiryV2, ExpiresAt: now.Add(24 * time.Hour).Format("2006-01-02T15:04:05.000Z")}}
	plan, err := store.FreezePlanVersion(ctx, planID, 0, definition, "free_plan_op_"+suffix, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	input := commercial.ApproveDistributionInput{OperationID: "approve_" + suffix, PlanID: planID, PlanVersion: 1, ExpectedSHA256: plan.SHA256, NotBefore: now, Reason: "isolated distribution integration"}
	record, err := store.ApproveFreeDistribution(ctx, id, input, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	again, err := store.ApproveFreeDistribution(ctx, id, input, actor, now.Add(time.Hour))
	if err != nil || again.SHA256 != record.SHA256 || again.Snapshot.ApprovedAt != record.Snapshot.ApprovedAt {
		t.Fatalf("approval retry changed snapshot: %v", err)
	}
	changed := input
	changed.Reason = "different request"
	if _, err = store.ApproveFreeDistribution(ctx, id, changed, actor, now); !errors.Is(err, commercial.ErrConflict) {
		t.Fatalf("changed operation replay: %v", err)
	}
	prepared, err := store.PrepareFreeDistribution(ctx, id, "distribution-test", actor, now)
	if err != nil {
		t.Fatal(err)
	}
	// Simultaneous retries after the fixed expiry still use the already frozen
	// issued-at, never now, and never manufacture another document identity.
	var wg sync.WaitGroup
	results := make(chan error, 4)
	for range 4 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			r, e := store.PrepareFreeDistribution(ctx, id, "distribution-test", actor, now.Add(48*time.Hour))
			if e == nil && (r.Claims == nil || r.Claims.IssuedAt != prepared.Claims.IssuedAt) {
				e = errors.New("prepare changed immutable claims")
			}
			results <- e
		}()
	}
	wg.Wait()
	close(results)
	for e := range results {
		if e != nil {
			t.Fatal(e)
		}
	}
	if _, err = store.PrepareFreeDistribution(ctx, id, "different-key", actor, now); !errors.Is(err, commercial.ErrConflict) {
		t.Fatalf("prepared key changed: %v", err)
	}
	definition.Name = "Later free plan"
	if _, err = store.FreezePlanVersion(ctx, planID, 1, definition, "free_next_"+suffix, actor, now); err != nil {
		t.Fatal(err)
	}
	private, _ := x509.MarshalPKCS8PrivateKey(ed25519.NewKeyFromSeed(bytes.Repeat([]byte{28}, 32)))
	signer, err := licensing.NewV2("distribution-test", base64.RawURLEncoding.EncodeToString(private), licenseprotocol.IssuerPolicyV2{Sources: []licenseprotocol.SourceKindV2{licenseprotocol.FreeDistributionV2}, Bindings: []licenseprotocol.BindingKindV2{licenseprotocol.UnboundV2}, Expiries: []licenseprotocol.ExpiryKindV2{licenseprotocol.FixedExpiryV2}, EntitlementCeiling: record.Snapshot.Plan.Definition.Entitlements})
	if err != nil {
		t.Fatal(err)
	}
	document, err := signer.Sign(ctx, *prepared.Claims)
	if err != nil {
		t.Fatal(err)
	}
	// Force the last audit insert to fail, after the document UPDATE ran. A real
	// database rollback must leave prepared intact and no partially issued file.
	faultID := operationAuditID(id, "commercial.distribution_issued")
	_, err = db.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES(?,?,'test.injected_failure','commercial',?,'{}',?)`, faultID, actor, id, now)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = store.CompleteFreeDistribution(ctx, id, document, actor, now); err == nil {
		t.Fatal("injected persistence failure ignored")
	}
	persisted, err := store.GetFreeDistribution(ctx, id)
	if err != nil || persisted.Status != "prepared" || persisted.Document != nil {
		t.Fatalf("partial document escaped rollback: %v", err)
	}
	if _, err = db.ExecContext(ctx, "DELETE FROM audit_events WHERE id=? AND action='test.injected_failure'", faultID); err != nil {
		t.Fatal(err)
	}
	issued, err := store.CompleteFreeDistribution(ctx, id, document, actor, now.Add(48*time.Hour))
	if err != nil {
		t.Fatal(err)
	}
	replay, err := store.CompleteFreeDistribution(ctx, id, document, actor, now.Add(49*time.Hour))
	if err != nil || replay.DocumentSHA256 != issued.DocumentSHA256 {
		t.Fatalf("issuance retry changed artifact: %v", err)
	}
	if issued.Claims.PlanVersion != 1 || issued.Snapshot.Plan.Definition.Name == definition.Name {
		t.Fatal("later plan leaked into issued rights")
	}
	var count int
	if err = db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE resource_id=? AND action IN ('commercial.distribution_approved','commercial.distribution_prepared','commercial.distribution_issued')", id).Scan(&count); err != nil || count != 3 {
		t.Fatalf("duplicated issuance audit: %d %v", count, err)
	}
	service := application.NewService(store, time.Hour, application.WithV2LicenseSigners(map[string]ports.LicenseSignerV2{"distribution-test": signer}))
	if _, err = service.GetFreeDistribution(ctx, id, actor); err != nil {
		t.Fatal(err)
	}
	corrupted := document
	corrupted.Signature = "AA"
	raw, _ := json.Marshal(corrupted)
	if _, err = db.ExecContext(ctx, "UPDATE commercial_free_distributions SET document_json=?,document_sha256=? WHERE id=?", raw, commercial.ContentDigest(raw), id); err != nil {
		t.Fatal(err)
	}
	if _, err = service.GetFreeDistribution(ctx, id, actor); err == nil {
		t.Fatal("valid content hash replaced signature verification")
	}
	original, _ := json.Marshal(document)
	if _, err = db.ExecContext(ctx, "UPDATE commercial_free_distributions SET document_json=?,document_sha256=? WHERE id=?", original, issued.DocumentSHA256, id); err != nil {
		t.Fatal(err)
	}
}
