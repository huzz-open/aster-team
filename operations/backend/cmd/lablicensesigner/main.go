// Command lablicensesigner exists only for disposable Customer package acceptance.
// It is never included in a Customer or Operations release artifact.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"regexp"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/adapters/licensing"
	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/productcatalog"
)

const exactTimeLayout = "2006-01-02T15:04:05.000Z"

var digestPattern = regexp.MustCompile(`^[A-Za-z0-9_-]{43}$`)

type machineFactor struct {
	Kind   string `json:"kind"`
	SHA256 string `json:"sha256"`
}

type installationProfile struct {
	Schema                   string          `json:"schema"`
	InstallationID           string          `json:"installation_id"`
	MachineFingerprintSHA256 string          `json:"machine_fingerprint_sha256"`
	MachineFactors           []machineFactor `json:"machine_factors"`
}

func main() {
	if err := run(os.Args[1:], time.Now().UTC()); err != nil {
		fmt.Fprintln(os.Stderr, "lab paid-license signing failed:", err)
		os.Exit(1)
	}
}

func run(arguments []string, now time.Time) error {
	flags := flag.NewFlagSet("lablicensesigner", flag.ContinueOnError)
	flags.SetOutput(os.Stderr)
	profilePath := flags.String("installation-profile", "", "installed Aster profile JSON")
	privateKeyPath := flags.String("private-key", "", "ephemeral lab PKCS#8 key")
	minimumVersion := flags.String("minimum-version", "", "test package version")
	outputPath := flags.String("output", "", "new paid License file")
	if err := flags.Parse(arguments); err != nil {
		return err
	}
	if flags.NArg() != 0 || *profilePath == "" || *privateKeyPath == "" || *minimumVersion == "" || *outputPath == "" {
		return errors.New("installation-profile, private-key, minimum-version and output are required")
	}

	profileBytes, err := os.ReadFile(*profilePath)
	if err != nil {
		return fmt.Errorf("read installation profile: %w", err)
	}
	var profile installationProfile
	if err := decodeStrict(profileBytes, &profile); err != nil {
		return fmt.Errorf("decode installation profile: %w", err)
	}
	if err := validateProfile(profile); err != nil {
		return err
	}
	privateKey, err := os.ReadFile(*privateKeyPath)
	if err != nil {
		return fmt.Errorf("read private key: %w", err)
	}

	entitlements := paidLabEntitlements()
	policy := licenseprotocol.IssuerPolicyV2{
		Sources:            []licenseprotocol.SourceKindV2{licenseprotocol.CommercialOrderV2},
		Bindings:           []licenseprotocol.BindingKindV2{licenseprotocol.InstallationV2},
		Expiries:           []licenseprotocol.ExpiryKindV2{licenseprotocol.FixedExpiryV2},
		EntitlementCeiling: entitlements,
	}
	signer, err := licensing.NewV2("lab-paid-v2", strings.TrimSpace(string(privateKey)), policy)
	if err != nil {
		return fmt.Errorf("configure signer: %w", err)
	}
	issued := now.UTC().Truncate(time.Millisecond)
	fixtureIssuedAt := time.Date(2026, time.September, 1, 0, 0, 0, 0, time.UTC)
	if !issued.After(fixtureIssuedAt) {
		return errors.New("lab clock must be later than the bundled free fixture issue time")
	}
	transferSequence := uint32(0)
	claims := licenseprotocol.ClaimsV2{
		Schema:      licenseprotocol.SchemaV2,
		KeyID:       "lab-paid-v2",
		LicenseID:   "lab_paid_switch",
		Serial:      "LAB-PAID-SWITCH",
		Product:     productcatalog.Product,
		Edition:     "lab-paid",
		PlanID:      "lab_paid_20",
		PlanVersion: 1,
		Source: licenseprotocol.SourceV2{
			Kind:        licenseprotocol.CommercialOrderV2,
			OrderID:     "lab_order",
			CustomerRef: "lab_customer",
			RequestID:   "lab_request",
		},
		Entitlements:       entitlements,
		QuotaPolicyVersion: licenseprotocol.QuotaPolicyVersionV2,
		Binding: licenseprotocol.BindingV2{
			Mode:                     licenseprotocol.InstallationV2,
			InstallationID:           profile.InstallationID,
			MachineFingerprintSHA256: profile.MachineFingerprintSHA256,
			TransferSequence:         &transferSequence,
		},
		Validity: licenseprotocol.ValidityV2{
			NotBefore: issued.Format(exactTimeLayout),
			Expiry: licenseprotocol.ExpiryV2{
				Mode:      licenseprotocol.FixedExpiryV2,
				ExpiresAt: issued.AddDate(1, 0, 0).Format(exactTimeLayout),
			},
		},
		IssuedAt:       issued.Format(exactTimeLayout),
		MinimumVersion: *minimumVersion,
	}
	document, err := signer.Sign(context.Background(), claims)
	if err != nil {
		return fmt.Errorf("sign paid License: %w", err)
	}
	encoded, err := json.MarshalIndent(document, "", "  ")
	if err != nil {
		return fmt.Errorf("encode paid License: %w", err)
	}
	encoded = append(encoded, '\n')
	file, err := os.OpenFile(*outputPath, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return fmt.Errorf("create output: %w", err)
	}
	if _, err = file.Write(encoded); err == nil {
		err = file.Sync()
	}
	closeErr := file.Close()
	if err != nil {
		_ = os.Remove(*outputPath)
		return fmt.Errorf("write output: %w", err)
	}
	if closeErr != nil {
		_ = os.Remove(*outputPath)
		return fmt.Errorf("close output: %w", closeErr)
	}
	return nil
}

func paidLabEntitlements() productcatalog.Entitlements {
	return productcatalog.Entitlements{
		CatalogVersion: productcatalog.Version,
		Features: []productcatalog.CapabilityID{
			productcatalog.CapabilityGateway,
			productcatalog.CapabilityMember,
			productcatalog.CapabilityRunner,
		},
		Quotas: []productcatalog.QuotaGrant{
			{ID: productcatalog.QuotaMemberSeats, Limit: productcatalog.Limited(20)},
			{ID: productcatalog.QuotaRunners, Limit: productcatalog.Unlimited()},
			{ID: productcatalog.QuotaUpstreamAccounts, Limit: productcatalog.Unlimited()},
			{ID: productcatalog.QuotaApiKeysPerMember, Limit: productcatalog.Unlimited()},
		},
	}
}

func decodeStrict(data []byte, target any) error {
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(target); err != nil {
		return err
	}
	var trailing any
	if err := decoder.Decode(&trailing); !errors.Is(err, io.EOF) {
		if err == nil {
			return errors.New("trailing JSON")
		}
		return err
	}
	return nil
}

func validateProfile(profile installationProfile) error {
	if profile.Schema != "aster.installation.v1" || profile.InstallationID == "" || !digestPattern.MatchString(profile.MachineFingerprintSHA256) || len(profile.MachineFactors) != 2 {
		return errors.New("installation profile is invalid")
	}
	kinds := map[string]bool{}
	for _, factor := range profile.MachineFactors {
		if !matchesMachineFactor(factor.Kind) || kinds[factor.Kind] || !digestPattern.MatchString(factor.SHA256) {
			return errors.New("installation profile is invalid")
		}
		kinds[factor.Kind] = true
	}
	if !kinds["dmi_product_uuid"] || !kinds["machine_id"] {
		return errors.New("installation profile is invalid")
	}
	return nil
}

func matchesMachineFactor(value string) bool {
	return value == "dmi_product_uuid" || value == "machine_id"
}
