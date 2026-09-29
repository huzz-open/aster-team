package application

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"errors"
	"os"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/adapters/licensing"
	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
)

type distributionTestStore struct {
	*commercialPermissionStore
	record       commercial.DistributionRecord
	calls        int
	failComplete bool
	attempted    []byte
}

func (s *distributionTestStore) ApproveFreeDistribution(context.Context, string, commercial.ApproveDistributionInput, string, time.Time) (commercial.DistributionRecord, error) {
	s.calls++
	return s.record, nil
}
func (s *distributionTestStore) GetFreeDistribution(context.Context, string) (commercial.DistributionRecord, error) {
	s.calls++
	return s.record, nil
}
func (s *distributionTestStore) ListFreeDistributions(context.Context, int) ([]commercial.DistributionRecord, error) {
	s.calls++
	return []commercial.DistributionRecord{s.record}, nil
}
func (s *distributionTestStore) PrepareFreeDistribution(_ context.Context, _ string, key, actor string, now time.Time) (commercial.DistributionRecord, error) {
	s.calls++
	if s.record.Status == "approved" {
		claims, err := s.record.Snapshot.Claims(key, now)
		if err != nil {
			return s.record, err
		}
		s.record.Claims = &claims
		s.record.Status = "prepared"
	}
	return s.record, nil
}
func (s *distributionTestStore) CompleteFreeDistribution(_ context.Context, _ string, doc licenseprotocol.DocumentV2, _ string, _ time.Time) (commercial.DistributionRecord, error) {
	s.calls++
	raw, _ := json.Marshal(doc)
	s.attempted = raw
	if s.failComplete {
		return commercial.DistributionRecord{}, errors.New("persist interrupted after signing")
	}
	s.record.Document = &doc
	s.record.DocumentSHA256 = commercial.ContentDigest(raw)
	s.record.Status = "issued"
	return s.record, nil
}
func distributionFixture(t *testing.T) (*distributionTestStore, *Service, commercial.ApproveDistributionInput) {
	t.Helper()
	raw, err := os.ReadFile("../../../../contracts/test-vectors/plan-definition.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var definition commercial.Definition
	if err = json.Unmarshal(raw, &definition); err != nil {
		t.Fatal(err)
	}
	definition.Offer = commercial.Offer{Kind: "free", Expiry: &licenseprotocol.ExpiryV2{Mode: licenseprotocol.FixedExpiryV2, ExpiresAt: "2028-01-01T00:00:00.000Z"}}
	plan, err := commercial.FreezePlan("plan_test", 1, definition)
	if err != nil {
		t.Fatal(err)
	}
	snapshot, _ := plan.Snapshot()
	now := time.Date(2026, 9, 6, 0, 0, 0, 0, time.UTC)
	input := commercial.ApproveDistributionInput{OperationID: "approve_test", PlanID: "plan_test", PlanVersion: 1, ExpectedSHA256: plan.Digest(), NotBefore: now, Reason: "isolated approval test"}
	source := commercial.DistributionSnapshot{Schema: commercial.DistributionSchema, ID: commercialObjectID("dist", "operator_1", input.OperationID), Plan: snapshot, PlanSHA256: plan.Digest(), NotBefore: "2026-09-06T00:00:00.000Z", Reason: input.Reason, ApprovedBy: "operator_1", ApprovedAt: "2026-09-06T00:00:00.000Z"}
	raw, err = source.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	store := &distributionTestStore{commercialPermissionStore: &commercialPermissionStore{fakeStore: &fakeStore{}, allowed: map[string]bool{PermissionDistributionRead: true, PermissionDistributionApprove: true, PermissionLicenseIssueV2: true}}, record: commercial.DistributionRecord{Snapshot: source, SHA256: commercial.ContentDigest(raw), OperationID: input.OperationID, Status: "approved"}}
	private, _ := x509.MarshalPKCS8PrivateKey(ed25519.NewKeyFromSeed(bytes.Repeat([]byte{17}, 32)))
	policy := licenseprotocol.IssuerPolicyV2{Sources: []licenseprotocol.SourceKindV2{licenseprotocol.FreeDistributionV2}, Bindings: []licenseprotocol.BindingKindV2{licenseprotocol.UnboundV2}, Expiries: []licenseprotocol.ExpiryKindV2{licenseprotocol.FixedExpiryV2, licenseprotocol.NoExpiryV2}, EntitlementCeiling: snapshot.Definition.Entitlements}
	signer, err := licensing.NewV2("distribution-test", base64.RawURLEncoding.EncodeToString(private), policy)
	if err != nil {
		t.Fatal(err)
	}
	service := NewService(store, time.Hour, WithV2LicenseSigners(map[string]ports.LicenseSignerV2{"distribution-test": signer}))
	service.now = func() time.Time { return now }
	return store, service, input
}

func TestFreeDistributionRetriesFrozenClaimsAfterExpiry(t *testing.T) {
	store, s, _ := distributionFixture(t)
	store.failComplete = true
	if _, err := s.IssueFreeDistribution(context.Background(), store.record.Snapshot.ID, "distribution-test", "operator_1"); err == nil {
		t.Fatal("lost persistence accepted")
	}
	original := append([]byte(nil), store.attempted...)
	if store.record.Status != "prepared" || len(original) == 0 {
		t.Fatal("failed issuance did not retain prepared claims")
	}
	s.now = func() time.Time { return time.Date(2029, 1, 1, 0, 0, 0, 0, time.UTC) }
	store.failComplete = false
	record, err := s.IssueFreeDistribution(context.Background(), store.record.Snapshot.ID, "distribution-test", "operator_1")
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(original, store.attempted) || record.Claims.Validity.Expiry.ExpiresAt != "2028-01-01T00:00:00.000Z" {
		t.Fatal("retry changed the signed document or extended expiry")
	}
	if _, err = s.IssueFreeDistribution(context.Background(), store.record.Snapshot.ID, "distribution-test", "operator_1"); err != nil {
		t.Fatal(err)
	}
}
func TestFreeDistributionApprovalRetryVerifiesIssuedDocument(t *testing.T) {
	store, s, input := distributionFixture(t)
	_, err := s.IssueFreeDistribution(context.Background(), store.record.Snapshot.ID, "distribution-test", "operator_1")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = s.ApproveFreeDistribution(context.Background(), input, "operator_1"); err != nil {
		t.Fatal(err)
	}
	saved := store.record.Document.Signature
	store.record.Document.Signature = "AA"
	raw, _ := json.Marshal(store.record.Document)
	store.record.DocumentSHA256 = commercial.ContentDigest(raw)
	if _, err = s.ApproveFreeDistribution(context.Background(), input, "operator_1"); err == nil {
		t.Fatal("approval retry returned corrupted signed artifact")
	}
	store.record.Document.Signature = saved
	raw, _ = json.Marshal(store.record.Document)
	store.record.DocumentSHA256 = commercial.ContentDigest(raw)
	publicOnly, err := licensing.NewV2Verifier(s.licenseSignersV2["distribution-test"].Profile())
	if err != nil {
		t.Fatal(err)
	}
	s.licenseSignersV2 = nil
	s.licenseVerifiersV2 = map[string]ports.LicenseVerifierV2{"distribution-test": publicOnly}
	if _, err = s.ApproveFreeDistribution(context.Background(), input, "operator_1"); err != nil {
		t.Fatalf("public-key-only approval recovery: %v", err)
	}
	if _, err = s.IssueFreeDistribution(context.Background(), store.record.Snapshot.ID, "distribution-test", "operator_1"); err != nil {
		t.Fatalf("public-key-only issued recovery: %v", err)
	}
	s.licenseVerifiersV2 = nil
	if _, err = s.ApproveFreeDistribution(context.Background(), input, "operator_1"); !errors.Is(err, ErrV2VerifierUnavailable) {
		t.Fatalf("untrusted approval retry: %v", err)
	}
}
func TestFreeDistributionServicePermissionsPrecedeStorageAndSigning(t *testing.T) {
	store, s, input := distributionFixture(t)
	store.allowed = map[string]bool{PermissionCommercialPlanWrite: true}
	calls := []func() error{
		func() error { _, e := s.ApproveFreeDistribution(context.Background(), input, "operator_1"); return e },
		func() error {
			_, e := s.GetFreeDistribution(context.Background(), store.record.Snapshot.ID, "operator_1")
			return e
		},
		func() error { _, e := s.ListFreeDistributions(context.Background(), 10, "operator_1"); return e },
		func() error { _, e := s.ListV2IssuerProfiles(context.Background(), "operator_1"); return e },
		func() error {
			_, e := s.IssueFreeDistribution(context.Background(), store.record.Snapshot.ID, "distribution-test", "operator_1")
			return e
		},
	}
	for _, run := range calls {
		if err := run(); !errors.Is(err, ErrUnauthorized) {
			t.Fatalf("permission denial: %v", err)
		}
	}
	if store.calls != 0 || len(store.attempted) != 0 {
		t.Fatal("permission denial touched protected storage or signing")
	}
}
func TestFreeDistributionRejectsUnapprovedTamperedOrWidenedSource(t *testing.T) {
	store, s, _ := distributionFixture(t)
	store.record.Status = "draft"
	if _, err := s.IssueFreeDistribution(context.Background(), store.record.Snapshot.ID, "distribution-test", "operator_1"); err == nil {
		t.Fatal("unapproved state signed")
	}
	store.record.Status = "approved"
	store.record.Snapshot.Plan.Definition.MinimumVersion = "9.0.0"
	if _, err := s.IssueFreeDistribution(context.Background(), store.record.Snapshot.ID, "distribution-test", "operator_1"); err == nil {
		t.Fatal("changed approval signed")
	}
	if len(store.attempted) != 0 {
		t.Fatal("rejected source reached persistence")
	}
}
