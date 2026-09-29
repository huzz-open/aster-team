package commercial

import (
	"bytes"
	"encoding/json"
	"os"
	"slices"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/productcatalog"
)

func testDefinition(t *testing.T) Definition {
	t.Helper()
	data, err := os.ReadFile("../../../../contracts/test-vectors/plan-definition.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var value Definition
	if err := json.Unmarshal(data, &value); err != nil {
		t.Fatal(err)
	}
	return value
}
func TestFrozenPlanOwnsAllRightsAndNormalizesSelectionOrder(t *testing.T) {
	definition := testDefinition(t)
	frozen, err := FreezePlan("test_plan", 1, definition)
	if err != nil {
		t.Fatal(err)
	}
	reversed := testDefinition(t)
	slices.Reverse(reversed.Entitlements.Features)
	slices.Reverse(reversed.Entitlements.Quotas)
	slices.Reverse(reversed.Offer.Terms)
	second, err := FreezePlan("test_plan", 1, reversed)
	if err != nil {
		t.Fatal(err)
	}
	if frozen.Digest() != second.Digest() {
		t.Fatal("selection order changed content identity")
	}
	definition.Name = "changed"
	definition.Entitlements.Features[0] = "unknown"
	*definition.Entitlements.Quotas[0].Limit.Value = 99
	definition.Offer.Terms[0].DiscountBasisPoints = 1
	projection, err := frozen.Snapshot()
	if err != nil {
		t.Fatal(err)
	}
	projection.Definition.Name = "another"
	projection.Definition.Entitlements.Quotas[0].Limit = productcatalog.Limited(999)
	raw := frozen.Bytes()
	raw[0] = '['
	if !bytes.Equal(frozen.Bytes(), second.Bytes()) {
		t.Fatal("a caller mutated the frozen snapshot")
	}
	if _, err := ParseFrozenPlan(frozen.Bytes(), frozen.Digest()); err != nil {
		t.Fatal(err)
	}
}
func TestOrderPinsVersionRightsPriceTermsAndContractDates(t *testing.T) {
	def := testDefinition(t)
	frozen, err := FreezePlan("test_plan", 1, def)
	if err != nil {
		t.Fatal(err)
	}
	starts := time.Date(2024, 2, 28, 16, 30, 0, 0, time.UTC)
	order, err := CreateOrderSnapshot("test_order", "test_customer", frozen, 1, starts)
	if err != nil {
		t.Fatal(err)
	}
	if order.EndsAt != "2025-02-27T16:30:00.000Z" {
		t.Fatalf("UTC/calendar mismatch %s", order.EndsAt)
	}
	before, _ := order.Bytes()
	def.Offer.AnnualAmountMinor = 999900
	def.Entitlements.Quotas[0].Limit = productcatalog.Limited(50)
	if _, err := FreezePlan("test_plan", 2, def); err != nil {
		t.Fatal(err)
	}
	after, err := order.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(before, after) || order.AmountMinor != 599900 || order.Plan.Version != 1 {
		t.Fatal("new revision changed purchased snapshot")
	}
	if _, err := ParseOrderSnapshot(before, ContentDigest(before)); err != nil {
		t.Fatal(err)
	}
	order.AmountMinor++
	if order.Validate() == nil {
		t.Fatal("forged order amount accepted")
	}
}
func TestCalendarClampsLeapDayAndKeepsTimezone(t *testing.T) {
	starts := time.Date(2024, 2, 28, 16, 30, 0, 0, time.UTC)
	ends, err := EndOfTerm(starts, 1, "calendar_years_clamp_day", "Asia/Shanghai")
	if err != nil {
		t.Fatal(err)
	}
	if !ends.Equal(time.Date(2025, 2, 27, 16, 30, 0, 0, time.UTC)) {
		t.Fatalf("wrong leap-day end: %s", ends)
	}
	if _, err := EndOfTerm(starts, 1, "unknown", "UTC"); err == nil {
		t.Fatal("unknown calendar accepted")
	}
}
func TestPricesMatchSharedVectorsAndRejectOverflowOrUnknownTerms(t *testing.T) {
	data, err := os.ReadFile("../../../../contracts/test-vectors/pricing.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []struct {
			Annual   int64  `json:"annual_amount_minor"`
			Years    uint32 `json:"years"`
			Discount uint32 `json:"discount_basis_points"`
			Amount   int64  `json:"amount_minor"`
		} `json:"cases"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	for _, c := range fixture.Cases {
		amount, err := CalculateAmount(c.Annual, Term{Years: c.Years, DiscountBasisPoints: c.Discount})
		if err != nil || amount != c.Amount {
			t.Fatalf("price=%d error=%v want %d", amount, err, c.Amount)
		}
	}
	for _, term := range []Term{{Years: 0, DiscountBasisPoints: 10000}, {Years: 6, DiscountBasisPoints: 10000}, {Years: 1, DiscountBasisPoints: 0}, {Years: 1, DiscountBasisPoints: 10001}} {
		if _, err := CalculateAmount(599900, term); err == nil {
			t.Fatal("invalid term accepted")
		}
	}
	if _, err := CalculateAmount(MaximumAmountMinor, Term{Years: 5, DiscountBasisPoints: 10000}); err == nil {
		t.Fatal("overflow accepted")
	}
}
func TestFreeAndContactNeverBecomeZeroPriceOrders(t *testing.T) {
	for _, offer := range []Offer{{Kind: "contact"}, {Kind: "free", Expiry: &licenseprotocol.ExpiryV2{Mode: licenseprotocol.NoExpiryV2}}, {Kind: "free", Expiry: &licenseprotocol.ExpiryV2{Mode: licenseprotocol.FixedExpiryV2, ExpiresAt: "2027-01-01T00:00:00.000Z"}}} {
		def := testDefinition(t)
		def.Offer = offer
		frozen, err := FreezePlan("test_plan", 1, def)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := CreateOrderSnapshot("order", "customer", frozen, 1, time.Now().UTC().Truncate(time.Millisecond)); err == nil {
			t.Fatal("free/contact became a priced order")
		}
	}
	def := testDefinition(t)
	def.Offer = Offer{Kind: "free"}
	if def.Validate() == nil {
		t.Fatal("missing free expiry selected an implicit default")
	}
}
func TestSnapshotsRejectMissingDuplicateOrMismatchedFields(t *testing.T) {
	frozen, err := FreezePlan("test_plan", 1, testDefinition(t))
	if err != nil {
		t.Fatal(err)
	}
	raw := string(frozen.Bytes())
	for _, pair := range [][2]string{{`"version":1`, `"version":null`}, {`"version":1`, `"version":1,"version":1`}, {`"version":1`, `"Version":1`}, {`"product":"aster-team"`, `"product":"other"`}} {
		if !strings.Contains(raw, pair[0]) {
			t.Fatal("test input missing")
		}
		if _, err := ParseFrozenPlan([]byte(strings.Replace(raw, pair[0], pair[1], 1)), frozen.Digest()); err == nil {
			t.Fatal("ambiguous snapshot accepted")
		}
	}
	if _, err := ParseFrozenPlan(frozen.Bytes(), strings.Repeat("0", 64)); err == nil {
		t.Fatal("wrong content digest accepted")
	}
}

func TestPlanMinimumVersionMatchesSharedSemanticVersionLength(t *testing.T) {
	value := testDefinition(t)
	value.MinimumVersion = "1.0.0+" + strings.Repeat("a", 58)
	if err := value.Validate(); err != nil {
		t.Fatalf("64-character version rejected: %v", err)
	}
	value.MinimumVersion += "a"
	if err := value.Validate(); err == nil {
		t.Fatal("65-character version accepted")
	}
}

func TestFeatureSetsRemainSymbolicOwnedSnapshotsAndRejectFreePlans(t *testing.T) {
	d := testDefinition(t)
	d.Entitlements.Features = []productcatalog.CapabilityID{}
	d.Entitlements.FeatureSets = []productcatalog.FeatureSetID{productcatalog.FeatureSetStandard}
	frozen, err := FreezePlan("test_sets", 1, d)
	if err != nil {
		t.Fatal(err)
	}
	d.Entitlements.FeatureSets[0] = "unknown"
	snapshot, err := frozen.Snapshot()
	if err != nil {
		t.Fatal(err)
	}
	if len(snapshot.Definition.Entitlements.Features) != 0 || !slices.Equal(snapshot.Definition.Entitlements.FeatureSets, []productcatalog.FeatureSetID{productcatalog.FeatureSetStandard}) {
		t.Fatal("symbolic grant mutated or expanded")
	}
	r := CatalogRequest{OperationID: "catalog_sets", Environment: "local", Reason: "test", Plans: []CatalogSelection{{PlanID: "test_sets", Version: 1, ExpectedSHA256: frozen.Digest()}}}
	public, err := BuildPublicCatalog("catalog_"+strings.Repeat("a", 48), r, []PlanSnapshot{snapshot})
	if err != nil {
		t.Fatal(err)
	}
	snapshot.Definition.Entitlements.FeatureSets[0] = "unknown"
	if !slices.Equal(public.Catalog.Plans[0].Entitlements.FeatureSets, []productcatalog.FeatureSetID{productcatalog.FeatureSetStandard}) {
		t.Fatal("public snapshot aliases source")
	}
	d = testDefinition(t)
	d.Entitlements.FeatureSets = []productcatalog.FeatureSetID{productcatalog.FeatureSetStandard}
	d.Offer = Offer{Kind: "free", Expiry: &licenseprotocol.ExpiryV2{Mode: licenseprotocol.NoExpiryV2}}
	if _, err := FreezePlan("test_free_sets", 1, d); err == nil {
		t.Fatal("free plan granted future set")
	}
}
