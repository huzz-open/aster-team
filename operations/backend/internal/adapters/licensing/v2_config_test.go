package licensing

import (
	"bytes"
	"crypto/ed25519"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"
)

func TestV2SignerRegistryKeepsPrivateMaterialAndLegacyKeysSeparate(t *testing.T) {
	claims, private, _ := signerFixtureV2(t)
	entry := configuredSignerV2{KeyID: "profile_test", PrivateKeyPKCS8: private, Policy: freeSignerScopeV2(claims)}
	raw, _ := json.Marshal([]configuredSignerV2{entry})
	signers, err := LoadV2Signers(string(raw), nil)
	if err != nil {
		t.Fatal(err)
	}
	public, _ := json.Marshal(signers[entry.KeyID].Profile())
	if bytes.Contains(public, []byte(private)) || bytes.Contains(public, []byte("private")) {
		t.Fatal("public profile leaked signing material")
	}
	alias := entry
	alias.KeyID = "alias"
	duplicated, _ := json.Marshal([]configuredSignerV2{entry, alias})
	if _, err = LoadV2Signers(string(duplicated), nil); err == nil {
		t.Fatal("same key aliased into multiple scopes")
	}
	legacy, err := New("legacy_test", private)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = LoadV2Signers(string(raw), legacy); err == nil {
		t.Fatal("v1 key reused for scoped v2 signing")
	}
	other, _ := x509.MarshalPKCS8PrivateKey(ed25519.NewKeyFromSeed(bytes.Repeat([]byte{8}, 32)))
	legacy, _ = New("profile_test", base64.RawURLEncoding.EncodeToString(other))
	if _, err = LoadV2Signers(string(raw), legacy); err == nil {
		t.Fatal("v1 key ID reused")
	}
	for _, bad := range []string{"null", "[]", strings.Replace(string(raw), "\"sources\"", "\"Sources\"", 1), strings.Replace(string(raw), "\"policy\"", "\"Policy\"", 1)} {
		if _, err = LoadV2Signers(bad, nil); err == nil {
			t.Fatal("invalid or ambiguous signing configuration accepted")
		}
	}
	empty, err := LoadV2Signers("", legacy)
	if err != nil || len(empty) != 0 {
		t.Fatal("missing configuration enabled a signer")
	}
}
