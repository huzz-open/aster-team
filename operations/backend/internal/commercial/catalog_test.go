package commercial

import (
	"bytes"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"aster.local/team/operations/backend/internal/licenseprotocol"
)

func catalogFixture(t *testing.T) (CatalogRequest, []PlanSnapshot) {
	t.Helper()
	r := CatalogRequest{OperationID: "catalog_test", Environment: "local", Reason: "private approval reason", Plans: []CatalogSelection{}}
	plans := []PlanSnapshot{}
	for _, kind := range []string{"annual", "free", "contact"} {
		d := testDefinition(t)
		d.Code = "INTERNAL_CODE_" + kind
		d.TransferLimit = 9817
		if kind == "free" {
			d.Offer = Offer{Kind: kind, Expiry: &licenseprotocol.ExpiryV2{Mode: licenseprotocol.NoExpiryV2}}
		}
		if kind == "contact" {
			d.Offer = Offer{Kind: kind}
		}
		f, err := FreezePlan("plan_"+kind, 1, d)
		if err != nil {
			t.Fatal(err)
		}
		p, err := f.Snapshot()
		if err != nil {
			t.Fatal(err)
		}
		plans = append(plans, p)
		r.Plans = append(r.Plans, CatalogSelection{PlanID: p.PlanID, Version: 1, ExpectedSHA256: f.Digest()})
	}
	return r, plans
}
func TestPublicCatalogWhitelistOrderingAndPrice(t *testing.T) {
	r, plans := catalogFixture(t)
	p, err := BuildPublicCatalog("catalog_"+strings.Repeat("a", 48), r, plans)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := canonical(p.Catalog)
	if err != nil {
		t.Fatal(err)
	}
	if p.SHA256 != digest(raw) || len(p.Catalog.Plans) != 3 {
		t.Fatal("catalog identity or count changed")
	}
	for _, forbidden := range []string{"INTERNAL_CODE", "9817", "private approval reason", "operation_id", "approved_by", "transfer_limit", "snapshot", "plan_sha256"} {
		if bytes.Contains(raw, []byte(forbidden)) {
			t.Fatalf("internal field leaked: %s", forbidden)
		}
	}
	if p.Catalog.Plans[0].Offer.Kind != "fixed_price" || p.Catalog.Plans[1].Offer.Kind != "free" || p.Catalog.Plans[2].Offer.Kind != "contact" {
		t.Fatal("offer types or selected order changed")
	}
	for _, term := range p.Catalog.Plans[0].Offer.Terms {
		amount, err := CalculateAmount(plans[0].Definition.Offer.AnnualAmountMinor, Term{Years: term.Years, DiscountBasisPoints: term.DiscountBasisPoints})
		if err != nil || term.TotalAmountMinor != amount {
			t.Fatal("public amount differs from fixed pricing")
		}
	}
}
func TestPublicCatalogOwnsDataAndSeparatesEnvironmentAndEmptyCollection(t *testing.T) {
	r, plans := catalogFixture(t)
	id := "catalog_" + strings.Repeat("b", 48)
	p, err := BuildPublicCatalog(id, r, plans)
	if err != nil {
		t.Fatal(err)
	}
	before, _ := canonical(p.Catalog)
	plans[0].Definition.Entitlements.Features[0] = "unknown"
	for _, quota := range plans[0].Definition.Entitlements.Quotas {
		if quota.Limit.Value != nil {
			*quota.Limit.Value = 400
			break
		}
	}
	plans[1].Definition.Offer.Expiry.Mode = "changed"
	after, _ := canonical(p.Catalog)
	if !bytes.Equal(before, after) {
		t.Fatal("public projection retained caller-owned data")
	}
	r.Plans = []CatalogSelection{}
	empty, err := BuildPublicCatalog(id, r, []PlanSnapshot{})
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := canonical(empty.Catalog)
	if !bytes.Contains(raw, []byte(`"plans":[]`)) {
		t.Fatal("empty catalog must be an explicit array")
	}
	r.Environment = "production"
	production, err := BuildPublicCatalog(id, r, []PlanSnapshot{})
	if err != nil || production.SHA256 == empty.SHA256 {
		t.Fatal("environment missing from public identity")
	}
}
func TestCatalogApprovalRejectsAmbiguityTamperingAndCrossVersionSources(t *testing.T) {
	r, plans := catalogFixture(t)
	id := "catalog_" + strings.Repeat("c", 48)
	p, err := BuildPublicCatalog(id, r, plans)
	if err != nil {
		t.Fatal(err)
	}
	s := CatalogApprovalSnapshot{Schema: CatalogApprovalSchema, ID: id, Request: r, Plans: plans, PublicSHA256: p.SHA256, ApprovedBy: "operator_1", ApprovedAt: "2026-09-06T00:00:00.000Z"}
	raw, err := s.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := ParseCatalogApproval(raw, digest(raw)); err != nil {
		t.Fatal(err)
	}
	for _, changed := range [][]byte{
		append(bytes.Clone(raw), '\n'),
		bytes.Replace(raw, []byte(`"environment":"local"`), []byte(`"environment":"production"`), 1),
		bytes.Replace(raw, []byte(`"approved_by":"operator_1"`), []byte(`"approved_by":"operator_1","Approved_By":"operator_1"`), 1),
		bytes.Replace(raw, []byte(`"public_sha256":`), []byte(`"public_sha256":"`+strings.Repeat("d", 64)+`","public_sha256":`), 1),
	} {
		if _, err := ParseCatalogApproval(changed, digest(changed)); err == nil {
			t.Fatal("tampered approval accepted")
		}
	}
	r.Plans[0].Version = 2
	if _, err := BuildPublicCatalog(id, r, plans); !errors.Is(err, ErrCatalogPreviewConflict) {
		t.Fatal("cross-version source accepted", err)
	}
	r, _ = catalogFixture(t)
	r.Plans = append(r.Plans, r.Plans[0])
	if err := r.Validate(); err == nil {
		t.Fatal("duplicate public plan accepted")
	}
	for _, invalid := range []string{`{"operation_id":"x","environment":"local","reason":"x","plans":null}`, `{"operation_id":"x","environment":"LOCAL","reason":"x","plans":[]}`, `{"operation_id":"x","environment":"local","reason":"x","plans":[],"rights":[]}`} {
		var request CatalogRequest
		if err := json.Unmarshal([]byte(invalid), &request); err == nil {
			t.Fatal("invalid request accepted")
		}
	}
}
