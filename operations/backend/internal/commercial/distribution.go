package commercial

import (
	"bytes"
	"encoding/json"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/strictjson"
)

const DistributionSchema = "aster.free-distribution.v1"

var ErrInvalidDistribution = errors.New("invalid free distribution approval")

type ApproveDistributionInput struct {
	OperationID    string    `json:"operation_id"`
	PlanID         string    `json:"plan_id"`
	PlanVersion    uint32    `json:"plan_version"`
	ExpectedSHA256 string    `json:"expected_sha256"`
	NotBefore      time.Time `json:"not_before"`
	Reason         string    `json:"reason"`
}

func (v *ApproveDistributionInput) UnmarshalJSON(data []byte) error {
	type wire ApproveDistributionInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "plan_id", "plan_version", "expected_sha256", "not_before", "reason"); err != nil {
		return err
	}
	*v = ApproveDistributionInput(result)
	return nil
}
func (v ApproveDistributionInput) Validate() error {
	if !identifier.MatchString(v.OperationID) || !identifier.MatchString(v.PlanID) || len(v.PlanID) > 64 || v.PlanVersion == 0 || len(v.ExpectedSHA256) != 64 || v.NotBefore.IsZero() || v.NotBefore.Nanosecond()%1_000_000 != 0 || !validText(v.Reason, 2000, false) {
		return errors.New("invalid free distribution approval")
	}
	return nil
}

// An approved source is separate from both a saved plan and a public website
// publication. No customer or installation is invented for a distribution.
type DistributionSnapshot struct {
	Schema     string       `json:"schema"`
	ID         string       `json:"id"`
	Plan       PlanSnapshot `json:"plan"`
	PlanSHA256 string       `json:"plan_sha256"`
	NotBefore  string       `json:"not_before"`
	Reason     string       `json:"reason"`
	ApprovedBy string       `json:"approved_by"`
	ApprovedAt string       `json:"approved_at"`
}

func (v *DistributionSnapshot) UnmarshalJSON(data []byte) error {
	type wire DistributionSnapshot
	var result wire
	if err := strictjson.Object(data, &result, "schema", "id", "plan", "plan_sha256", "not_before", "reason", "approved_by", "approved_at"); err != nil {
		return err
	}
	*v = DistributionSnapshot(result)
	return v.Validate()
}
func (v DistributionSnapshot) Validate() error {
	if v.Schema != DistributionSchema || !identifier.MatchString(v.ID) || len(v.ID) > 64 || !identifier.MatchString(v.ApprovedBy) || len(v.ApprovedBy) > 64 || !validText(v.Reason, 2000, false) {
		return errors.New("invalid distribution approval identity")
	}
	plan, err := FreezePlan(v.Plan.PlanID, v.Plan.Version, v.Plan.Definition)
	if err != nil {
		return err
	}
	if v.Plan.Schema != PlanSchema || plan.Digest() != v.PlanSHA256 || v.Plan.Definition.Offer.Kind != "free" {
		return errors.New("distribution must reference a fixed free plan")
	}
	start, err := parseTime(v.NotBefore)
	if err != nil {
		return err
	}
	approved, err := parseTime(v.ApprovedAt)
	if err != nil {
		return err
	}
	expiry := v.Plan.Definition.Offer.Expiry
	if expiry.Mode == licenseprotocol.FixedExpiryV2 {
		end, err := parseTime(expiry.ExpiresAt)
		if err != nil || !start.Before(end) || !approved.Before(end) {
			return errors.New("distribution approval is outside the free term")
		}
	}
	return nil
}
func (v DistributionSnapshot) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	return json.Marshal(v)
}

func (v DistributionSnapshot) Claims(keyID string, issuedAt time.Time) (licenseprotocol.ClaimsV2, error) {
	if err := v.Validate(); err != nil {
		return licenseprotocol.ClaimsV2{}, err
	}
	// Clone through the validated fixed snapshot so callers cannot mutate it by
	// retaining a quota pointer or feature slice from returned claims.
	raw, err := json.Marshal(v.Plan)
	if err != nil {
		return licenseprotocol.ClaimsV2{}, err
	}
	var plan PlanSnapshot
	if err = json.Unmarshal(raw, &plan); err != nil {
		return licenseprotocol.ClaimsV2{}, err
	}
	claims := licenseprotocol.ClaimsV2{
		Schema: licenseprotocol.SchemaV2, KeyID: keyID, LicenseID: "license_" + v.ID, Serial: "serial_" + v.ID,
		Product: plan.Definition.Product, Edition: plan.Definition.Edition, PlanID: plan.PlanID, PlanVersion: plan.Version,
		Source:       licenseprotocol.SourceV2{Kind: licenseprotocol.FreeDistributionV2, DistributionID: v.ID},
		Entitlements: plan.Definition.Entitlements, QuotaPolicyVersion: plan.Definition.QuotaPolicyVersion,
		Binding:  licenseprotocol.BindingV2{Mode: licenseprotocol.UnboundV2},
		Validity: licenseprotocol.ValidityV2{NotBefore: v.NotBefore, Expiry: *plan.Definition.Offer.Expiry},
		IssuedAt: issuedAt.UTC().Format("2006-01-02T15:04:05.000Z"), MinimumVersion: plan.Definition.MinimumVersion,
	}
	return claims, claims.Validate()
}

type DistributionRecord struct {
	Snapshot       DistributionSnapshot        `json:"snapshot"`
	SHA256         string                      `json:"sha256"`
	OperationID    string                      `json:"operation_id"`
	Status         string                      `json:"status"`
	Claims         *licenseprotocol.ClaimsV2   `json:"claims,omitempty"`
	Document       *licenseprotocol.DocumentV2 `json:"document,omitempty"`
	DocumentSHA256 string                      `json:"document_sha256,omitempty"`
}

func (v DistributionRecord) Validate() error {
	raw, err := v.Snapshot.Bytes()
	if err != nil {
		return err
	}
	if ContentDigest(raw) != v.SHA256 || !identifier.MatchString(v.OperationID) {
		return errors.New("distribution identity or digest mismatch")
	}
	switch v.Status {
	case "approved":
		if v.Claims != nil || v.Document != nil || v.DocumentSHA256 != "" {
			return errors.New("unprepared approval contains issuance")
		}
		return nil
	case "prepared", "issued":
		if v.Claims == nil {
			return errors.New("prepared distribution has no claims")
		}
	default:
		return errors.New("unknown distribution state")
	}
	issued, err := parseTime(v.Claims.IssuedAt)
	if err != nil {
		return err
	}
	approved, err := parseTime(v.Snapshot.ApprovedAt)
	if err != nil || issued.Before(approved) {
		return errors.New("issuance precedes approval")
	}
	expected, err := v.Snapshot.Claims(v.Claims.KeyID, issued)
	if err != nil {
		return err
	}
	want, err := licenseprotocol.CanonicalClaimsV2(expected)
	if err != nil {
		return err
	}
	got, err := licenseprotocol.CanonicalClaimsV2(*v.Claims)
	if err != nil || !bytes.Equal(want, got) {
		return errors.New("claims differ from approved distribution")
	}
	if v.Status == "prepared" {
		if v.Document != nil || v.DocumentSHA256 != "" {
			return errors.New("uncommitted distribution contains document")
		}
		return nil
	}
	if v.Document == nil {
		return errors.New("issued distribution has no document")
	}
	got, err = licenseprotocol.CanonicalClaimsV2(v.Document.Claims)
	if err != nil || !bytes.Equal(want, got) {
		return errors.New("document differs from prepared claims")
	}
	raw, err = json.Marshal(v.Document)
	if err != nil {
		return err
	}
	if ContentDigest(raw) != v.DocumentSHA256 {
		return errors.New("document digest mismatch")
	}
	// Signature authenticity is checked against the trusted signer by the
	// application before persistence and before any download.
	return nil
}
