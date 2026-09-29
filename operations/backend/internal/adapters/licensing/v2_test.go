package licensing

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"os"
	"reflect"
	"testing"

	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
	"aster.local/team/operations/backend/internal/productcatalog"
)

func signerFixtureV2(t *testing.T) (licenseprotocol.ClaimsV2, string, string) {
	t.Helper()
	data, err := os.ReadFile("../../../../../contracts/test-vectors/license.v2.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Public string `json:"public_key_spki"`
		Cases  []struct {
			Document licenseprotocol.DocumentV2 `json:"document"`
		} `json:"cases"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	private, err := x509.MarshalPKCS8PrivateKey(ed25519.NewKeyFromSeed(bytes.Repeat([]byte{42}, 32)))
	if err != nil {
		t.Fatal(err)
	}
	return fixture.Cases[0].Document.Claims, base64.RawURLEncoding.EncodeToString(private), fixture.Public
}
func freeSignerScopeV2(claims licenseprotocol.ClaimsV2) licenseprotocol.IssuerPolicyV2 {
	return licenseprotocol.IssuerPolicyV2{Sources: []licenseprotocol.SourceKindV2{licenseprotocol.FreeDistributionV2}, Bindings: []licenseprotocol.BindingKindV2{licenseprotocol.UnboundV2}, Expiries: []licenseprotocol.ExpiryKindV2{licenseprotocol.FixedExpiryV2}, EntitlementCeiling: claims.Entitlements}
}
func TestScopedSignerSignsOnlyItsFixedIssuerScope(t *testing.T) {
	claims, private, public := signerFixtureV2(t)
	scope := freeSignerScopeV2(claims)
	signer, err := NewV2("test-only-v2", private, scope)
	if err != nil {
		t.Fatal(err)
	}
	claims.KeyID = ""
	signed, err := signer.Sign(context.Background(), claims)
	if err != nil {
		t.Fatal(err)
	}
	var keys licenseprotocol.TrustedKeysV2
	if err := keys.Insert("test-only-v2", public, scope); err != nil {
		t.Fatal(err)
	}
	raw, err := json.Marshal(signed)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := licenseprotocol.VerifyV2(raw, &keys); err != nil {
		t.Fatal(err)
	}
	claims.KeyID = "another-key"
	if _, err := signer.Sign(context.Background(), claims); err == nil {
		t.Fatal("wrong issuer key accepted")
	}
	claims.KeyID = ""
	// Change caller-owned configuration after construction; the signer keeps its copy.
	*scope.EntitlementCeiling.Quotas[0].Limit.Value = 999
	claims.Entitlements.Quotas[0].Limit = productcatalog.Limited(4)
	if _, err := signer.Sign(context.Background(), claims); err == nil {
		t.Fatal("caller widened signer scope")
	}
}
func TestScopedSignerRejectsUnconfiguredScopeAndCancelledRequests(t *testing.T) {
	claims, private, _ := signerFixtureV2(t)
	if _, err := NewV2("test-only-v2", private, licenseprotocol.IssuerPolicyV2{}); err == nil {
		t.Fatal("unscoped signing configured")
	}
	signer, err := NewV2("test-only-v2", private, freeSignerScopeV2(claims))
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := signer.Sign(ctx, claims); err == nil {
		t.Fatal("cancelled request signed")
	}
	var empty V2Signer
	if _, err := empty.Sign(context.Background(), claims); err == nil {
		t.Fatal("unconfigured signer signed")
	}
}

func TestPublicV2ProfileMatchesCustomerTrustVector(t *testing.T) {
	data, err := os.ReadFile("../../../../../contracts/test-vectors/license-trust.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Keyring []json.RawMessage `json:"keyring"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	var expected ports.IssuerProfileV2
	if err := json.Unmarshal(fixture.Keyring[1], &expected); err != nil {
		t.Fatal(err)
	}
	_, private, _ := signerFixtureV2(t)
	signer, err := NewV2(expected.KeyID, private, expected.Policy)
	if err != nil {
		t.Fatal(err)
	}
	actual, err := json.Marshal(signer.Profile())
	if err != nil {
		t.Fatal(err)
	}
	var want, got any
	if err := json.Unmarshal(fixture.Keyring[1], &want); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(actual, &got); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(want, got) {
		t.Fatal("Operations profile differs from the shared Customer trust vector")
	}
}
