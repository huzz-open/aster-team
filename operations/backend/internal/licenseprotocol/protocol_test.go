package licenseprotocol

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type crossLanguageFixture struct {
	PublicKeySPKI string          `json:"public_key_spki"`
	Document      json.RawMessage `json:"document"`
}

func validRequestJSON(t *testing.T) []byte {
	t.Helper()
	value := Request{
		Schema:                   "aster.license-request.v1",
		RequestID:                "request_test_001",
		Product:                  "aster-team",
		ProductVersion:           "1.0.5",
		Platform:                 "linux",
		Architecture:             "amd64",
		InstallationID:           "installation_test_001",
		MachineFingerprintSHA256: strings.Repeat("A", 43),
		MachineFactors: []MachineFactor{
			{Kind: "dmi_product_uuid", SHA256: strings.Repeat("B", 43)},
			{Kind: "machine_id", SHA256: strings.Repeat("C", 43)},
		},
		GeneratedAt: "2026-08-26T12:00:00.000Z",
	}
	data, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	return data
}

func signedLicenseJSON(t *testing.T) ([]byte, string) {
	t.Helper()
	publicKey, privateKey, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	claims := License{
		Schema: "aster.license.v1", KeyID: "license-test-01", LicenseID: "license_test_001", Serial: "AT-TEST-001",
		RequestID: "request_test_001", CustomerRef: "customer_test_001", Product: "aster-team",
		Edition: "enterprise", Features: []string{"proxy", "runner"},
		Limits:         Limits{MemberSeats: 10, SeatOverLimitGraceDays: 7},
		MinimumVersion: "1.0.5", InstallationID: "installation_test_001",
		MachineFingerprintSHA256: strings.Repeat("A", 43), TransferSequence: 0,
		IssuedAt: "2026-08-26T12:00:00.000Z", NotBefore: "2026-08-26T12:00:00.000Z",
		ExpiresAt: "2027-08-26T12:00:00.000Z",
	}
	signed, err := Sign(claims, privateKey)
	if err != nil {
		t.Fatal(err)
	}
	data, err := json.Marshal(signed)
	if err != nil {
		t.Fatal(err)
	}
	spki, err := x509.MarshalPKIXPublicKey(publicKey)
	if err != nil {
		t.Fatal(err)
	}
	return data, base64.RawURLEncoding.EncodeToString(spki)
}

func TestParseRequestRejectsUnknownNestedField(t *testing.T) {
	data := strings.Replace(string(validRequestJSON(t)), `"sha256":"`+strings.Repeat("B", 43)+`"`, `"sha256":"`+strings.Repeat("B", 43)+`","source":"dmi"`, 1)
	if _, err := ParseRequest([]byte(data)); err == nil {
		t.Fatal("expected nested field rejection")
	}
}

func TestParseRequestRejectsTrailingJSON(t *testing.T) {
	if _, err := ParseRequest(append(validRequestJSON(t), []byte(` {}`)...)); err == nil {
		t.Fatal("expected trailing JSON rejection")
	}
}

func TestParseRequestAcceptsSupportedPlatforms(t *testing.T) {
	for _, platform := range []string{"linux", "windows", "macos"} {
		var request Request
		if err := json.Unmarshal(validRequestJSON(t), &request); err != nil {
			t.Fatal(err)
		}
		request.Platform = platform
		data, err := json.Marshal(request)
		if err != nil {
			t.Fatal(err)
		}
		if parsed, err := ParseRequest(data); err != nil || parsed.Platform != platform {
			t.Fatalf("platform %s was rejected: %v", platform, err)
		}
	}
}

func TestVerifyRejectsUnknownLimitField(t *testing.T) {
	data, publicKey := signedLicenseJSON(t)
	modified := strings.Replace(string(data), `"seat_over_limit_grace_days":7`, `"seat_over_limit_grace_days":7,"burst":1`, 1)
	if _, err := Verify([]byte(modified), map[string]string{"license-test-01": publicKey}); err == nil {
		t.Fatal("expected nested field rejection")
	}
}

func TestSignAndVerify(t *testing.T) {
	data, publicKey := signedLicenseJSON(t)
	license, err := Verify(data, map[string]string{"license-test-01": publicKey})
	if err != nil {
		t.Fatal(err)
	}
	if license.LicenseID != "license_test_001" {
		t.Fatalf("unexpected license %q", license.LicenseID)
	}
}

func TestVerifySelectsPublicKeyByKeyID(t *testing.T) {
	data, publicKey := signedLicenseJSON(t)
	if _, err := Verify(data, map[string]string{"another-key": publicKey}); err == nil {
		t.Fatal("expected unknown key ID rejection")
	}
}

func TestGoSignerMatchesRustCrossLanguageFixture(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("..", "..", "..", "..", "contracts", "test-vectors", "license.v1.json"))
	if err != nil {
		t.Fatal(err)
	}
	var fixture crossLanguageFixture
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	var document License
	if err := json.Unmarshal(fixture.Document, &document); err != nil {
		t.Fatal(err)
	}
	seed := make([]byte, ed25519.SeedSize)
	for index := range seed {
		seed[index] = byte(index + 1)
	}
	privateKey := ed25519.NewKeyFromSeed(seed)
	spki, err := x509.MarshalPKIXPublicKey(privateKey.Public())
	if err != nil {
		t.Fatal(err)
	}
	if actual := base64.RawURLEncoding.EncodeToString(spki); actual != fixture.PublicKeySPKI {
		t.Fatalf("public key = %q, want fixture key", actual)
	}
	wantSignature := document.Signature
	document.Signature = ""
	signed, err := Sign(document, privateKey)
	if err != nil {
		t.Fatal(err)
	}
	if signed.Signature != wantSignature {
		t.Fatalf("signature = %q, want shared fixture signature", signed.Signature)
	}
	if _, err := Verify(fixture.Document, map[string]string{signed.KeyID: fixture.PublicKeySPKI}); err != nil {
		t.Fatalf("Go verify shared fixture: %v", err)
	}
}
