package licensing

import (
	"aster.local/team/operations/backend/internal/ports"
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestRetainedV2PublicKeysVerifyWithoutPrivateCapabilities(t *testing.T) {
	claims, private, _ := signerFixtureV2(t)
	signer, err := NewV2("retained-test", private, freeSignerScopeV2(claims))
	if err != nil {
		t.Fatal(err)
	}
	claims.KeyID = "retained-test"
	doc, err := signer.Sign(context.Background(), claims)
	if err != nil {
		t.Fatal(err)
	}
	profile := signer.Profile()
	raw, _ := json.Marshal([]ports.IssuerProfileV2{profile})
	registry, err := LoadV2Verifiers(string(raw), nil, nil)
	if err != nil {
		t.Fatal(err)
	}
	verifier := registry[profile.KeyID]
	if _, canSign := verifier.(ports.LicenseSignerV2); canSign {
		t.Fatal("verification registry retained signing capability")
	}
	if err := verifier.Verify(doc); err != nil {
		t.Fatal(err)
	}
	bad := doc
	bad.Claims.LicenseID += "_forged"
	if verifier.Verify(bad) == nil {
		t.Fatal("public verifier accepted altered signed claims")
	}
	copied := verifier.Profile()
	copied.Policy.Sources = nil
	if verifier.Verify(doc) != nil {
		t.Fatal("caller changed retained scope")
	}
	if _, err := LoadV2Verifiers(string(raw), map[string]ports.LicenseSignerV2{profile.KeyID: signer}, nil); err != nil {
		t.Fatal("identical public/private profiles conflict", err)
	}
	conflicting := profile
	conflicting.Policy = profile.Policy.Clone()
	conflicting.Policy.Expiries = nil
	conflictRaw, _ := json.Marshal([]ports.IssuerProfileV2{conflicting})
	if _, err := LoadV2Verifiers(string(conflictRaw), map[string]ports.LicenseSignerV2{profile.KeyID: signer}, nil); err == nil {
		t.Fatal("conflicting trust policy accepted")
	}
	alias := profile
	alias.KeyID = "retained-alias"
	aliases, _ := json.Marshal([]ports.IssuerProfileV2{profile, alias})
	duplicates, _ := json.Marshal([]ports.IssuerProfileV2{profile, profile})
	for _, invalid := range []string{"null", "[]", string(aliases), string(duplicates), strings.Replace(string(raw), `"key_id"`, `"Key_ID"`, 1), strings.Replace(string(raw), `"policy"`, `"policy":null,"policy"`, 1)} {
		if _, err := LoadV2Verifiers(invalid, nil, nil); err == nil {
			t.Fatal("ambiguous verification registry accepted")
		}
	}
	legacy, err := New("legacy-retained", private)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := LoadV2Verifiers(string(raw), nil, legacy); err == nil {
		t.Fatal("legacy key reused for v2 verification")
	}
}
