package licenseprotocol

import (
	"bytes"
	"crypto/ed25519"
	"encoding/json"
	"os"
	"strings"
	"testing"

	"aster.local/team/operations/backend/internal/productcatalog"
)

type fixtureV2 struct {
	PublicKeySPKI string `json:"public_key_spki"`
	Cases         []struct {
		Name      string     `json:"name"`
		Canonical string     `json:"canonical_claims"`
		Document  DocumentV2 `json:"document"`
	} `json:"cases"`
}

func loadFixtureV2(t *testing.T) fixtureV2 {
	t.Helper()
	data, err := os.ReadFile("../../../../contracts/test-vectors/license.v2.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture fixtureV2
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	return fixture
}
func scopeV2(value productcatalog.Entitlements, source SourceKindV2) IssuerPolicyV2 {
	return IssuerPolicyV2{Sources: []SourceKindV2{source}, Bindings: []BindingKindV2{UnboundV2, InstallationV2}, Expiries: []ExpiryKindV2{FixedExpiryV2, NoExpiryV2}, EntitlementCeiling: value}
}
func keysV2(t *testing.T, scope IssuerPolicyV2) *TrustedKeysV2 {
	t.Helper()
	keys := &TrustedKeysV2{}
	if err := keys.Insert("test-only-v2", loadFixtureV2(t).PublicKeySPKI, scope); err != nil {
		t.Fatal(err)
	}
	return keys
}
func encodeV2(t *testing.T, value any) []byte {
	t.Helper()
	data, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return data
}
func testKeyV2() ed25519.PrivateKey { return ed25519.NewKeyFromSeed(bytes.Repeat([]byte{42}, 32)) }

func TestV2FreeIssuerCannotAlsoAuthorizeCommercialOrTrialSources(t *testing.T) {
	for _, sources := range [][]SourceKindV2{{FreeDistributionV2, CommercialOrderV2}, {ApprovedTrialV2, FreeDistributionV2}} {
		policy := scopeV2(loadFixtureV2(t).Cases[0].Document.Claims.Entitlements, FreeDistributionV2)
		policy.Sources = sources
		if err := policy.Validate(); err == nil {
			t.Fatal("mixed free and commercial/trial issuer policy accepted")
		}
		var decoded IssuerPolicyV2
		if err := json.Unmarshal(encodeV2(t, policy), &decoded); err == nil {
			t.Fatal("mixed source policy loaded from trusted configuration")
		}
	}
}

func TestV2SharedCanonicalBytesSignaturesAndVerification(t *testing.T) {
	for _, item := range loadFixtureV2(t).Cases {
		t.Run(item.Name, func(t *testing.T) {
			payload, err := CanonicalClaimsV2(item.Document.Claims)
			if err != nil {
				t.Fatal(err)
			}
			if string(payload) != item.Canonical {
				t.Fatalf("canonical mismatch: %s", payload)
			}
			signed, err := SignV2(item.Document.Claims, testKeyV2())
			if err != nil {
				t.Fatal(err)
			}
			if signed.Signature != item.Document.Signature {
				t.Fatal("cross-language signature mismatch")
			}
			keys := keysV2(t, scopeV2(item.Document.Claims.Entitlements, item.Document.Claims.Source.Kind))
			verified, err := VerifyV2(encodeV2(t, item.Document), keys)
			if err != nil {
				t.Fatal(err)
			}
			if !bytes.Equal(encodeV2(t, verified.Document()), encodeV2(t, item.Document)) {
				t.Fatal("verified document changed")
			}
		})
	}
}

func TestV2TamperingCannotChangeSignedClaims(t *testing.T) {
	doc := loadFixtureV2(t).Cases[0].Document
	mutations := []func(*ClaimsV2){
		func(c *ClaimsV2) { c.Edition = "paid" }, func(c *ClaimsV2) { c.PlanID = "another" }, func(c *ClaimsV2) { c.PlanVersion = 2 },
		func(c *ClaimsV2) { c.Serial = "another" }, func(c *ClaimsV2) { c.LicenseID = "another" }, func(c *ClaimsV2) { c.Source.DistributionID = "another" },
		func(c *ClaimsV2) { c.Entitlements.Quotas[0].Limit = productcatalog.Limited(4) }, func(c *ClaimsV2) { c.Entitlements.Features = []productcatalog.CapabilityID{} },
		func(c *ClaimsV2) { c.MinimumVersion = "1.2.4" }, func(c *ClaimsV2) { c.Validity.NotBefore = "2026-09-02T00:00:00.000Z" },
		func(c *ClaimsV2) { c.IssuedAt = "2026-08-31T00:00:00.000Z" }, func(c *ClaimsV2) { c.Validity.Expiry = ExpiryV2{Mode: NoExpiryV2} },
	}
	keys := keysV2(t, scopeV2(doc.Claims.Entitlements, doc.Claims.Source.Kind))
	for i, mutate := range mutations {
		changed := cloneDocumentV2(doc)
		mutate(&changed.Claims)
		if _, err := VerifyV2(encodeV2(t, changed), keys); err == nil || err.Error() != "invalid signature" {
			t.Fatalf("mutation %d: %v", i, err)
		}
	}
}

func TestV2FreeIssuerCannotAuthorizeCommercialOrLargerGrants(t *testing.T) {
	fixture := loadFixtureV2(t)
	doc := fixture.Cases[0].Document
	scope := scopeV2(doc.Claims.Entitlements, doc.Claims.Source.Kind)
	scope.Sources = []SourceKindV2{FreeDistributionV2}
	scope.Bindings = []BindingKindV2{UnboundV2}
	scope.Expiries = []ExpiryKindV2{FixedExpiryV2}
	keys := keysV2(t, scope)
	for _, i := range []int{1, 2, 3} {
		if _, err := VerifyV2(encodeV2(t, fixture.Cases[i].Document), keys); err == nil {
			t.Fatalf("scope accepted case %d", i)
		}
	}
	claims := cloneDocumentV2(doc).Claims
	claims.Entitlements.Quotas[0].Limit = productcatalog.Limited(4)
	over, err := SignV2(claims, testKeyV2())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := VerifyV2(encodeV2(t, over), keys); err == nil {
		t.Fatal("signed excessive quota accepted")
	}
	claims = cloneDocumentV2(fixture.Cases[2].Document).Claims
	claims.Binding = BindingV2{Mode: UnboundV2}
	if _, err := SignV2(claims, testKeyV2()); err == nil {
		t.Fatal("commercial unbound accepted")
	}
}

func TestV2RawJSONIsStrictAtEveryLevel(t *testing.T) {
	doc := loadFixtureV2(t).Cases[0].Document
	raw := string(encodeV2(t, doc))
	keys := keysV2(t, scopeV2(doc.Claims.Entitlements, doc.Claims.Source.Kind))
	replacements := [][2]string{
		{`"mode":"unbound"`, `"mode":"unbound","mode":"unbound"`},
		{`"mode":"unbound"`, `"mode":"unbound","skip":true`},
		{`"mode":"unbound"`, `"mode":"unknown"`},
		{`"mode":"unbound"`, `"Mode":"unbound"`},
		{`"mode":"limited"`, `"mode":"limited","value":3`},
		{`"plan_version":1`, `"plan_version":null`},
		{`"plan_version":1`, `"Plan_Version":1`},
		{`"catalog_version":1`, `"catalog_version":1,"catalog_version":1`},
		{`"kind":"free_distribution"`, `"kind":"free_distribution","kind":"free_distribution"`},
		{`"binding":{"mode":"unbound"}`, `"binding":null`},
	}
	for _, pair := range replacements {
		if !strings.Contains(raw, pair[0]) {
			t.Fatalf("missing test input %s", pair[0])
		}
		changed := strings.Replace(raw, pair[0], pair[1], 1)
		if _, err := VerifyV2([]byte(changed), keys); err == nil {
			t.Fatalf("accepted %s", pair[1])
		}
	}
	for _, suffix := range []string{" {}", " garbage", " false"} {
		if _, err := VerifyV2([]byte(raw+suffix), keys); err == nil {
			t.Fatal("accepted trailing input")
		}
	}
	bound := loadFixtureV2(t).Cases[2].Document
	changed := strings.Replace(string(encodeV2(t, bound)), `"transfer_sequence":0`, `"transfer_sequence":null`, 1)
	if _, err := VerifyV2([]byte(changed), keys); err == nil {
		t.Fatal("accepted null transfer")
	}
}

func TestV2TrustedScopeAndVerifiedDocumentCannotBeMutatedThroughAliases(t *testing.T) {
	doc := loadFixtureV2(t).Cases[0].Document
	scope := scopeV2(doc.Claims.Entitlements, doc.Claims.Source.Kind)
	keys := keysV2(t, scope)
	scope.Sources[0] = CommercialOrderV2
	*scope.EntitlementCeiling.Quotas[0].Limit.Value = 999
	// The input was changed after registration; the original signed document still verifies.
	original := loadFixtureV2(t).Cases[0].Document
	verified, err := VerifyV2(encodeV2(t, original), keys)
	if err != nil {
		t.Fatal(err)
	}
	returned := verified.Claims()
	*returned.Entitlements.Quotas[0].Limit.Value = 999
	returned.Entitlements.Features[0] = "unknown"
	if !bytes.Equal(encodeV2(t, verified.Document()), encodeV2(t, original)) {
		t.Fatal("projection mutated verified claims")
	}
	over := cloneDocumentV2(original).Claims
	over.Entitlements.Quotas[0].Limit = productcatalog.Limited(4)
	signed, err := SignV2(over, testKeyV2())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := VerifyV2(encodeV2(t, signed), keys); err == nil {
		t.Fatal("mutated caller policy widened trust")
	}
	if err := keys.Insert("test-only-v2", loadFixtureV2(t).PublicKeySPKI, scopeV2(over.Entitlements, over.Source.Kind)); err == nil {
		t.Fatal("duplicate key accepted")
	}
	if _, err := VerifyV2(encodeV2(t, signed), keys); err == nil {
		t.Fatal("duplicate insertion replaced previous policy")
	}
	if _, err := Verify(encodeV2(t, original), map[string]string{"test-only-v2": loadFixtureV2(t).PublicKeySPKI}); err == nil {
		t.Fatal("v1 accepted v2")
	}
}

func TestV2RejectsInvalidVersionsTimesAndPerpetualCommercialClaims(t *testing.T) {
	original := loadFixtureV2(t).Cases[0].Document
	mutations := []func(*ClaimsV2){
		func(c *ClaimsV2) { c.PlanVersion = 0 }, func(c *ClaimsV2) { c.QuotaPolicyVersion = 0 }, func(c *ClaimsV2) { c.Entitlements.CatalogVersion = 99 },
		func(c *ClaimsV2) { c.MinimumVersion = "1.02.3" }, func(c *ClaimsV2) { c.MinimumVersion = "18446744073709551616.0.0" },
		func(c *ClaimsV2) { c.IssuedAt = "2026-09-01T00:00:60.000Z" }, func(c *ClaimsV2) { c.IssuedAt = "2027-09-01T00:00:00.000Z" },
		func(c *ClaimsV2) { c.Validity.Expiry.ExpiresAt = c.Validity.NotBefore },
	}
	for i, mutate := range mutations {
		claims := cloneDocumentV2(original).Claims
		mutate(&claims)
		if _, err := SignV2(claims, testKeyV2()); err == nil {
			t.Fatalf("accepted mutation %d", i)
		}
	}
	paid := loadFixtureV2(t).Cases[2].Document.Claims
	paid.Validity.Expiry = ExpiryV2{Mode: NoExpiryV2}
	if _, err := SignV2(paid, testKeyV2()); err == nil {
		t.Fatal("accepted perpetual paid")
	}
	empty := cloneDocumentV2(original).Claims
	empty.Entitlements.Features = []productcatalog.CapabilityID{}
	signed, err := SignV2(empty, testKeyV2())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := VerifyV2(encodeV2(t, signed), keysV2(t, scopeV2(empty.Entitlements, empty.Source.Kind))); err != nil {
		t.Fatal(err)
	}
}

func TestV2RejectsNoncanonicalBase64EvenWithValidSignatureBytes(t *testing.T) {
	fixture := loadFixtureV2(t)
	original := fixture.Cases[0].Document
	keys := keysV2(t, scopeV2(original.Claims.Entitlements, original.Claims.Source.Kind))
	for _, separator := range []string{"\n", "\r", "\r\n"} {
		doc := cloneDocumentV2(original)
		doc.Signature = doc.Signature[:12] + separator + doc.Signature[12:]
		if _, err := VerifyV2(encodeV2(t, doc), keys); err == nil {
			t.Fatalf("signature accepted ignored %q", separator)
		}
		spki := fixture.PublicKeySPKI[:12] + separator + fixture.PublicKeySPKI[12:]
		var trust TrustedKeysV2
		if err := trust.Insert("test-only-v2", spki, scopeV2(original.Claims.Entitlements, original.Claims.Source.Kind)); err == nil {
			t.Fatalf("SPKI accepted ignored %q", separator)
		}
	}
}

func TestV2CannotAliasTheSamePublicKeyUnderAnotherScope(t *testing.T) {
	fixture := loadFixtureV2(t)
	original := fixture.Cases[0].Document
	scope := scopeV2(original.Claims.Entitlements, original.Claims.Source.Kind)
	keys := keysV2(t, scope)
	broader := scopeV2(fixture.Cases[2].Document.Claims.Entitlements, fixture.Cases[2].Document.Claims.Source.Kind)
	if err := keys.Insert("broader-alias", fixture.PublicKeySPKI, broader); err == nil {
		t.Fatal("same signing material accepted under broader alias")
	}
}

func TestV2LaterIssuancePreservesPurchasedDates(t *testing.T) {
	claims := loadFixtureV2(t).Cases[2].Document.Claims
	claims.IssuedAt = "2026-10-01T00:00:00.000Z"
	signed, err := SignV2(claims, testKeyV2())
	if err != nil {
		t.Fatal(err)
	}
	verified, err := VerifyV2(encodeV2(t, signed), keysV2(t, scopeV2(claims.Entitlements, claims.Source.Kind)))
	if err != nil {
		t.Fatal(err)
	}
	if verified.Claims().Validity.NotBefore != "2026-09-01T00:00:00.000Z" || verified.Claims().Validity.Expiry.ExpiresAt != "2027-09-01T00:00:00.000Z" {
		t.Fatal("issuance changed purchased period")
	}
}

func TestV2FeatureSetsRequireCommercialSourceAndAreSigned(t *testing.T) {
	var paid DocumentV2
	for _, item := range loadFixtureV2(t).Cases {
		if item.Document.Claims.Source.Kind == CommercialOrderV2 {
			paid = item.Document
			break
		}
	}
	paid.Claims.Entitlements.Features = []productcatalog.CapabilityID{}
	paid.Claims.Entitlements.FeatureSets = []productcatalog.FeatureSetID{productcatalog.FeatureSetStandard}
	policy := scopeV2(paid.Claims.Entitlements, CommercialOrderV2)
	if err := policy.Authorize(paid.Claims); err != nil {
		t.Fatal(err)
	}
	for _, source := range []SourceKindV2{FreeDistributionV2, ApprovedTrialV2} {
		if scopeV2(paid.Claims.Entitlements, source).Validate() == nil {
			t.Fatal("non-commercial issuer accepted symbolic rights")
		}
	}
	free := loadFixtureV2(t).Cases[0].Document.Claims
	free.Entitlements.FeatureSets = paid.Claims.Entitlements.FeatureSets
	if free.Validate() == nil {
		t.Fatal("free claims accepted symbolic rights")
	}
}
