package commercial

import (
	"bytes"
	"encoding/json"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/strictjson"
)

const PaidLifecycleSourceSchema = "aster.paid-lifecycle-source.v1"

const (
	PaidLifecycleRenewal = "renewal"
	PaidLifecycleUpgrade = "upgrade"
)

var ErrInvalidPaidLifecycle = errors.New("invalid paid fulfillment lifecycle source")

// PaidLifecycleRequest is only an optimistic-lock request. The transaction
// resolves and freezes all source identity, binding and validity fields.
type PaidLifecycleRequest struct {
	Kind                   string `json:"kind"`
	SourceID               string `json:"source_id"`
	ExpectedDocumentSHA256 string `json:"expected_document_sha256"`
}

func (v *PaidLifecycleRequest) UnmarshalJSON(data []byte) error {
	type wire PaidLifecycleRequest
	var result wire
	if err := strictjson.Object(data, &result, "kind", "source_id", "expected_document_sha256"); err != nil {
		return err
	}
	*v = PaidLifecycleRequest(result)
	return v.Validate()
}

func (v PaidLifecycleRequest) Validate() error {
	if v.Kind != PaidLifecycleRenewal && v.Kind != PaidLifecycleUpgrade {
		return ErrInvalidPaidLifecycle
	}
	if !identifier.MatchString(v.SourceID) || len(v.SourceID) > 64 || !catalogDigest.MatchString(v.ExpectedDocumentSHA256) {
		return ErrInvalidPaidLifecycle
	}
	return nil
}

// PaidLifecycleSource is the immutable link from an existing v2 paid license to a new paid order. It intentionally stores only the binding
// and validity needed to prove continuity; pricing and new rights stay in the
// immutable paid order.
type PaidLifecycleSource struct {
	Schema             string                      `json:"schema"`
	Kind               string                      `json:"kind"`
	SourceID           string                      `json:"source_id"`
	SourceRecordKind   string                      `json:"source_record_kind"`
	SourceRecordID     string                      `json:"source_record_id"`
	SourceRecordSHA256 string                      `json:"source_record_sha256"`
	CustomerID         string                      `json:"customer_id"`
	CustomerRef        string                      `json:"customer_ref"`
	LicenseID          string                      `json:"license_id"`
	Environment        string                      `json:"environment"`
	DocumentSHA256     string                      `json:"document_sha256"`
	Document           *licenseprotocol.DocumentV2 `json:"document,omitempty"`
	Binding            licenseprotocol.BindingV2   `json:"binding"`
	ValidFrom          string                      `json:"valid_from"`
	ValidUntil         string                      `json:"valid_until"`
}

func (v *PaidLifecycleSource) UnmarshalJSON(data []byte) error {
	type wire PaidLifecycleSource
	var result wire
	fields := []string{"schema", "kind", "source_id", "source_record_kind", "source_record_id", "source_record_sha256", "customer_id", "customer_ref", "license_id", "environment", "document_sha256", "binding", "valid_from", "valid_until"}
	fields = append(fields, "document")
	if err := strictjson.Object(data, &result, fields...); err != nil {
		return err
	}
	*v = PaidLifecycleSource(result)
	return v.Validate()
}

func (v PaidLifecycleSource) Validate() error {
	if v.Schema != PaidLifecycleSourceSchema || (v.Kind != PaidLifecycleRenewal && v.Kind != PaidLifecycleUpgrade) {
		return ErrInvalidPaidLifecycle
	}
	for _, id := range []string{v.SourceID, v.SourceRecordID, v.CustomerID, v.CustomerRef, v.LicenseID} {
		if !identifier.MatchString(id) || len(id) > 128 {
			return ErrInvalidPaidLifecycle
		}
	}
	if !catalogDigest.MatchString(v.SourceRecordSHA256) || !catalogDigest.MatchString(v.DocumentSHA256) || v.Binding.Validate() != nil || v.Binding.Mode != licenseprotocol.InstallationV2 {
		return ErrInvalidPaidLifecycle
	}
	if v.Environment != "local" && v.Environment != "production" {
		return ErrInvalidPaidLifecycle
	}
	if v.SourceRecordKind != "paid_fulfillment" && v.SourceRecordKind != "paid_transfer" {
		return ErrInvalidPaidLifecycle
	}
	if v.Document == nil {
		return ErrInvalidPaidLifecycle
	}
	from, e1 := parseTime(v.ValidFrom)
	until, e2 := parseTime(v.ValidUntil)
	if e1 != nil || e2 != nil || !until.After(from) {
		return ErrInvalidPaidLifecycle
	}
	if v.Document != nil {
		claims := v.Document.Claims
		raw, err := json.Marshal(v.Document)
		if err != nil || claims.Validate() != nil || ContentDigest(raw) != v.DocumentSHA256 || claims.LicenseID != v.LicenseID || claims.Source.CustomerRef != v.CustomerRef || claims.Binding.Mode != licenseprotocol.InstallationV2 || claims.Validity.NotBefore != v.ValidFrom || claims.Validity.Expiry.Mode != licenseprotocol.FixedExpiryV2 || claims.Validity.Expiry.ExpiresAt != v.ValidUntil {
			return ErrInvalidPaidLifecycle
		}
		want, err := canonical(claims.Binding)
		if err != nil {
			return ErrInvalidPaidLifecycle
		}
		actual, err := canonical(v.Binding)
		if err != nil || !bytes.Equal(want, actual) {
			return ErrInvalidPaidLifecycle
		}
	}

	return nil
}

func (v PaidLifecycleSource) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	return canonical(v)
}

func (v PaidLifecycleSource) ValidateSuccessor(order OrderSnapshot, request licenseprotocol.RequestV2, approvedAt time.Time) error {
	if v.Validate() != nil || order.Validate() != nil || request.Validate() != nil || order.CustomerID != v.CustomerID ||
		request.InstallationID != v.Binding.InstallationID || request.MachineFingerprintSHA256 != v.Binding.MachineFingerprintSHA256 {
		return ErrInvalidPaidLifecycle
	}
	issuedAt, _ := parseTime(v.Document.Claims.IssuedAt)
	// Customer accepts a different license identity only when issued_at moves
	// strictly forward. Reject approval while the Operations clock is equal to
	// or behind the predecessor so an unusable successor can never be frozen.
	if !approvedAt.After(issuedAt) {
		return ErrInvalidPaidLifecycle
	}
	starts, _ := parseTime(order.StartsAt)
	from, _ := parseTime(v.ValidFrom)
	until, _ := parseTime(v.ValidUntil)
	switch v.Kind {
	case PaidLifecycleRenewal:
		if !starts.Equal(until) {
			return ErrInvalidPaidLifecycle
		}
	case PaidLifecycleUpgrade:
		if starts.Before(from) || !starts.Before(until) || !approvedAt.Before(until) {
			return ErrInvalidPaidLifecycle
		}

	}
	return nil
}
