package main

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/licenseprotocol"
)

func TestRunSignsInstallationBoundPaidLabLicense(t *testing.T) {
	directory := t.TempDir()
	profilePath := filepath.Join(directory, "installation.json")
	keyPath := filepath.Join(directory, "paid.pkcs8")
	outputPath := filepath.Join(directory, "paid-license.json")
	profile := installationProfile{
		Schema:                   "aster.installation.v1",
		InstallationID:           "installation_lab_test",
		MachineFingerprintSHA256: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
		MachineFactors: []machineFactor{
			{Kind: "dmi_product_uuid", SHA256: "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"},
			{Kind: "machine_id", SHA256: "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"},
		},
	}
	profileBytes, err := json.Marshal(profile)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(profilePath, profileBytes, 0o600); err != nil {
		t.Fatal(err)
	}
	publicKey, privateKey, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	privatePKCS8, err := x509.MarshalPKCS8PrivateKey(privateKey)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(keyPath, []byte(base64.RawURLEncoding.EncodeToString(privatePKCS8)), 0o600); err != nil {
		t.Fatal(err)
	}
	now := time.Date(2026, time.September, 6, 12, 34, 56, 789_000_000, time.UTC)
	if err = run([]string{
		"--installation-profile", profilePath,
		"--private-key", keyPath,
		"--minimum-version", "2.0.1-rc.2",
		"--output", outputPath,
	}, now); err != nil {
		t.Fatal(err)
	}

	documentBytes, err := os.ReadFile(outputPath)
	if err != nil {
		t.Fatal(err)
	}
	policy := licenseprotocol.IssuerPolicyV2{
		Sources:            []licenseprotocol.SourceKindV2{licenseprotocol.CommercialOrderV2},
		Bindings:           []licenseprotocol.BindingKindV2{licenseprotocol.InstallationV2},
		Expiries:           []licenseprotocol.ExpiryKindV2{licenseprotocol.FixedExpiryV2},
		EntitlementCeiling: paidLabEntitlements(),
	}
	publicPKIX, err := x509.MarshalPKIXPublicKey(publicKey)
	if err != nil {
		t.Fatal(err)
	}
	var keys licenseprotocol.TrustedKeysV2
	if err = keys.Insert("lab-paid-v2", base64.RawURLEncoding.EncodeToString(publicPKIX), policy); err != nil {
		t.Fatal(err)
	}
	verified, err := licenseprotocol.VerifyV2(documentBytes, &keys)
	if err != nil {
		t.Fatal(err)
	}
	claims := verified.Claims()
	if claims.LicenseID != "lab_paid_switch" || claims.PlanID != "lab_paid_20" {
		t.Fatalf("unexpected paid claims: %#v", claims)
	}
	if claims.Binding.InstallationID != profile.InstallationID || claims.Binding.MachineFingerprintSHA256 != profile.MachineFingerprintSHA256 {
		t.Fatalf("paid License is not bound to the supplied profile: %#v", claims.Binding)
	}
	memberSeats, ok := claims.Entitlements.Quota("member_seats")
	if !ok || memberSeats.Value == nil || *memberSeats.Value != 20 {
		t.Fatalf("unexpected member seat grant: %#v", memberSeats)
	}
	if claims.IssuedAt != "2026-09-06T12:34:56.789Z" || claims.Validity.Expiry.ExpiresAt != "2027-09-06T12:34:56.789Z" {
		t.Fatalf("unexpected paid validity: %#v", claims.Validity)
	}
}

func TestDecodeStrictRejectsTrailingAndUnknownProfileFields(t *testing.T) {
	valid := `{"schema":"aster.installation.v1","installation_id":"installation_lab_test","machine_fingerprint_sha256":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","machine_factors":[{"kind":"dmi_product_uuid","sha256":"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"},{"kind":"machine_id","sha256":"CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"}]}`
	for _, value := range []string{valid + `{}`, valid[:len(valid)-1] + `,"extra":true}`} {
		var profile installationProfile
		if err := decodeStrict([]byte(value), &profile); err == nil {
			t.Fatalf("decodeStrict accepted invalid JSON: %s", value)
		}
	}
}
