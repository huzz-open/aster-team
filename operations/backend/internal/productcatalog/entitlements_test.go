package productcatalog

import (
	"encoding/json"
	"os"
	"reflect"
	"strings"
	"testing"
)

func entitlementVector(t *testing.T) []byte {
	t.Helper()
	data, err := os.ReadFile("../../../../contracts/test-vectors/entitlements.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func sampleEntitlements(t *testing.T) Entitlements {
	t.Helper()
	value, err := ParseEntitlements(entitlementVector(t))
	if err != nil {
		t.Fatal(err)
	}
	return value
}

func TestSharedEntitlementsVectorRoundTrips(t *testing.T) {
	source := entitlementVector(t)
	value := sampleEntitlements(t)
	encoded, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	var before, after any
	if err := json.Unmarshal(source, &before); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(encoded, &after); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(before, after) {
		t.Fatal("serialized entitlements differ from shared vector")
	}
	seats, ok := value.Quota(QuotaMemberSeats)
	if !ok || !seats.Permits(3) || seats.Permits(4) {
		t.Fatal("seat bound differs from shared vector")
	}
}

func TestMalformedEntitlementsCannotExpandRights(t *testing.T) {
	source := string(entitlementVector(t))
	for _, data := range []string{
		source + "{}", source + "garbage", "null", "{}",
		strings.Replace(source, "catalog_version", "CATALOG_VERSION", 1),
		strings.Replace(source, "features", "Features", 1),
		strings.Replace(source, "\"id\"", "\"ID\"", 1),
		strings.Replace(source, "\"limit\"", "\"Limit\"", 1),
		strings.Replace(source, "\"mode\"", "\"MODE\"", 1),
		strings.Replace(source, "\"value\"", "\"VALUE\"", 1),
		strings.Replace(source, "gateway", "unknown", 1),
		strings.Replace(source, "member_seats", "unknown", 1),
		strings.Replace(source, "runners", "member_seats", 1),
		strings.Replace(source, "runner\"", "member\"", 1),
		strings.Replace(source, "\"catalog_version\": 1,", "", 1),
		strings.Replace(source, "\"catalog_version\": 1,", "\"catalog_version\": 1, \"catalog_version\": 1,", 1),
		strings.Replace(source, "\"catalog_version\": 1,", "\"catalog_version\": 1, \"unexpected\": 1,", 1),
		strings.Replace(source, "\"value\": 3", "\"value\": 3, \"value\": 50", 1),
		strings.Replace(source, "\"value\": 3", "\"value\": -1", 1),
		strings.Replace(source, "\"value\": 3", "\"value\": 4294967296", 1),
		strings.Replace(source, "\"limited\", \"value\": 3", "\"unlimited\", \"value\": null", 1),
		strings.Replace(source, "\"limited\", \"value\": 3", "\"limited\"", 1),
	} {
		if _, err := ParseEntitlements([]byte(data)); err == nil {
			t.Errorf("accepted malformed entitlements %s", data)
		}
	}
}

func TestEntitlementIssuerScopeAndZeroAreExplicit(t *testing.T) {
	ceiling := sampleEntitlements(t)
	grant := sampleEntitlements(t)
	if err := grant.EnsureWithin(ceiling); err != nil {
		t.Fatal(err)
	}
	grant.Quotas[0].Limit = Unlimited()
	if err := grant.EnsureWithin(ceiling); err == nil {
		t.Fatal("unlimited grant passed finite ceiling")
	}
	grant.Quotas[0].Limit = Limited(4)
	if err := grant.EnsureWithin(ceiling); err == nil {
		t.Fatal("larger grant passed finite ceiling")
	}
	grant.Quotas[0].Limit = Limited(0)
	if err := grant.EnsureWithin(ceiling); err != nil {
		t.Fatal(err)
	}
	if grant.Quotas[0].Limit.Permits(1) || !grant.Quotas[0].Limit.Permits(0) {
		t.Fatal("zero is not forbidden")
	}
	ceiling.Features = []CapabilityID{}
	if err := ceiling.Validate(); err != nil {
		t.Fatal(err)
	}
	if err := grant.EnsureWithin(ceiling); err == nil {
		t.Fatal("ungranted capability accepted")
	}
	if (QuotaLimit{}).Permits(0) {
		t.Fatal("missing limit treated as valid")
	}
}

func TestSymbolicFeatureSetsCannotBeSignedByExplicitFeatureCeiling(t *testing.T) {
	explicit := sampleEntitlements(t)
	subscription := sampleEntitlements(t)
	subscription.Features = []CapabilityID{}
	subscription.FeatureSets = []FeatureSetID{FeatureSetStandard}
	if !reflect.DeepEqual(subscription.EffectiveFeatures(), explicit.Features) {
		t.Fatal("runtime projection differs")
	}
	if subscription.EnsureWithin(explicit) == nil {
		t.Fatal("explicit ceiling grants future rights")
	}
	if err := explicit.EnsureWithin(subscription); err != nil {
		t.Fatal(err)
	}
	if err := subscription.EnsureWithin(subscription); err != nil {
		t.Fatal(err)
	}
	excessive := sampleEntitlements(t)
	excessive.FeatureSets = []FeatureSetID{FeatureSetStandard}
	excessive.Quotas[0].Limit = Unlimited()
	if excessive.EnsureWithin(subscription) == nil {
		t.Fatal("symbolic set bypasses quota ceiling")
	}
	for _, raw := range []string{`null`, `["*"]`, `["unknown"]`, `["standard","standard"]`, `[{"standard":null}]`} {
		input := strings.Replace(string(entitlementVector(t)), `"catalog_version"`, `"feature_sets":`+raw+`,"catalog_version"`, 1)
		if _, err := ParseEntitlements([]byte(input)); err == nil {
			t.Fatalf("accepted %s", raw)
		}
	}
}
