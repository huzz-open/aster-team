package commercial

import (
	"bytes"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"errors"
	"time"

	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/strictjson"
)

const PaidTransferSchema = "aster.paid-transfer.v1"

var ErrInvalidPaidTransfer = errors.New("invalid paid license transfer")

type ApprovePaidTransferInput struct {
	OperationID                   string `json:"operation_id"`
	ExpectedCurrentDocumentSHA256 string `json:"expected_current_document_sha256"`
	LicenseRequestJSON            string `json:"license_request_json"`
	Reason                        string `json:"reason"`
}

func (v *ApprovePaidTransferInput) UnmarshalJSON(data []byte) error {
	type wire ApprovePaidTransferInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "expected_current_document_sha256", "license_request_json", "reason"); err != nil {
		return err
	}
	*v = ApprovePaidTransferInput(result)
	return v.Validate()
}

func (v ApprovePaidTransferInput) Validate() error {
	if !identifier.MatchString(v.OperationID) || len(v.OperationID) > 128 || !catalogDigest.MatchString(v.ExpectedCurrentDocumentSHA256) || len(v.LicenseRequestJSON) > 16<<10 || !validText(v.Reason, 2000, false) {
		return ErrInvalidPaidTransfer
	}
	if _, err := licenseprotocol.ParseRequestV2([]byte(v.LicenseRequestJSON)); err != nil {
		return ErrInvalidPaidTransfer
	}
	return nil
}

func (v ApprovePaidTransferInput) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	return canonical(v)
}

type PaidTransferSnapshot struct {
	Schema                 string                    `json:"schema"`
	ID                     string                    `json:"id"`
	FulfillmentID          string                    `json:"fulfillment_id"`
	FulfillmentSHA256      string                    `json:"fulfillment_sha256"`
	CustomerID             string                    `json:"customer_id"`
	OriginalClaims         licenseprotocol.ClaimsV2  `json:"original_claims"`
	TransferLimit          uint32                    `json:"transfer_limit"`
	Request                ApprovePaidTransferInput  `json:"request"`
	InstallationRequest    licenseprotocol.RequestV2 `json:"installation_request"`
	RequestSHA256          string                    `json:"request_sha256"`
	PreviousDocumentSHA256 string                    `json:"previous_document_sha256"`
	PreviousBinding        licenseprotocol.BindingV2 `json:"previous_binding"`
	PreviousIssuedAt       string                    `json:"previous_issued_at"`
	TransferSequence       uint32                    `json:"transfer_sequence"`
	Environment            string                    `json:"environment"`
	ApprovedBy             string                    `json:"approved_by"`
	ApprovedAt             string                    `json:"approved_at"`
}

func (v *PaidTransferSnapshot) UnmarshalJSON(data []byte) error {
	type wire PaidTransferSnapshot
	var result wire
	if err := strictjson.ObjectWithin(data, &result, 1<<20, "schema", "id", "fulfillment_id", "fulfillment_sha256", "customer_id", "original_claims", "transfer_limit", "request", "installation_request", "request_sha256", "previous_document_sha256", "previous_binding", "previous_issued_at", "transfer_sequence", "environment", "approved_by", "approved_at"); err != nil {
		return err
	}
	*v = PaidTransferSnapshot(result)
	return v.Validate()
}

func (v PaidTransferSnapshot) Validate() error {
	if v.Schema != PaidTransferSchema || !identifier.MatchString(v.ID) || len(v.ID) > 64 || !identifier.MatchString(v.ApprovedBy) || len(v.ApprovedBy) > 64 ||
		!identifier.MatchString(v.FulfillmentID) || len(v.FulfillmentID) > 64 || !catalogDigest.MatchString(v.FulfillmentSHA256) ||
		!identifier.MatchString(v.CustomerID) || len(v.CustomerID) > 64 || v.OriginalClaims.Validate() != nil ||
		v.OriginalClaims.Source.Kind != licenseprotocol.CommercialOrderV2 || v.OriginalClaims.Binding.Mode != licenseprotocol.InstallationV2 ||
		v.OriginalClaims.Binding.TransferSequence == nil || *v.OriginalClaims.Binding.TransferSequence != 0 || v.TransferLimit > 10000 ||
		v.Request.Validate() != nil || v.InstallationRequest.Validate() != nil || !catalogDigest.MatchString(v.RequestSHA256) ||
		!catalogDigest.MatchString(v.PreviousDocumentSHA256) || v.Request.ExpectedCurrentDocumentSHA256 != v.PreviousDocumentSHA256 ||
		v.PreviousBinding.Mode != licenseprotocol.InstallationV2 || v.PreviousBinding.TransferSequence == nil ||
		v.TransferSequence == 0 || *v.PreviousBinding.TransferSequence+1 != v.TransferSequence ||
		v.TransferSequence > v.TransferLimit || (v.Environment != "local" && v.Environment != "production") {
		return ErrInvalidPaidTransfer
	}
	if !v.InstallationRequest.Supports(v.OriginalClaims.MinimumVersion, v.OriginalClaims.Entitlements.CatalogVersion, v.OriginalClaims.QuotaPolicyVersion) {
		return ErrInvalidPaidTransfer
	}
	if v.InstallationRequest.InstallationID == v.PreviousBinding.InstallationID && v.InstallationRequest.MachineFingerprintSHA256 == v.PreviousBinding.MachineFingerprintSHA256 {
		return ErrInvalidPaidTransfer
	}
	parsed, err := licenseprotocol.ParseRequestV2([]byte(v.Request.LicenseRequestJSON))
	if err != nil {
		return ErrInvalidPaidTransfer
	}
	expected, err := canonical(parsed)
	if err != nil {
		return err
	}
	actual, err := canonical(v.InstallationRequest)
	if err != nil || !bytes.Equal(expected, actual) || ContentDigest(actual) != v.RequestSHA256 {
		return ErrInvalidPaidTransfer
	}
	approved, e1 := parseTime(v.ApprovedAt)
	previousIssued, e2 := parseTime(v.PreviousIssuedAt)
	generated, e3 := parseTime(v.InstallationRequest.GeneratedAt)
	expires, e4 := parseTime(v.OriginalClaims.Validity.Expiry.ExpiresAt)
	if e1 != nil || e2 != nil || e3 != nil || e4 != nil || approved.Before(previousIssued) || generated.After(approved) || !approved.Before(expires) {
		return ErrInvalidPaidTransfer
	}
	return nil
}

func (v PaidTransferSnapshot) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	raw, err := canonical(v)
	if len(raw) > 1<<20 {
		return nil, ErrInvalidPaidTransfer
	}
	return raw, err
}

func (v PaidTransferSnapshot) Claims(keyID string, issuedAt time.Time) (licenseprotocol.ClaimsV2, error) {
	raw, err := v.Bytes()
	if err != nil {
		return licenseprotocol.ClaimsV2{}, err
	}
	var owned PaidTransferSnapshot
	if err := json.Unmarshal(raw, &owned); err != nil {
		return licenseprotocol.ClaimsV2{}, err
	}
	approved, _ := parseTime(owned.ApprovedAt)
	expires, _ := parseTime(owned.OriginalClaims.Validity.Expiry.ExpiresAt)
	if issuedAt.Before(approved) || !issuedAt.Before(expires) {
		return licenseprotocol.ClaimsV2{}, ErrInvalidPaidTransfer
	}
	claims := owned.OriginalClaims
	claims.KeyID = keyID
	claims.Serial = "serial_" + owned.ID
	claims.Source.RequestID = owned.InstallationRequest.RequestID
	sequence := owned.TransferSequence
	claims.Binding = licenseprotocol.BindingV2{Mode: licenseprotocol.InstallationV2, InstallationID: owned.InstallationRequest.InstallationID, MachineFingerprintSHA256: owned.InstallationRequest.MachineFingerprintSHA256, TransferSequence: &sequence}
	claims.IssuedAt = issuedAt.UTC().Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z")
	return claims, claims.Validate()
}

type PaidTransferRecord struct {
	Snapshot       PaidTransferSnapshot        `json:"snapshot"`
	SHA256         string                      `json:"sha256"`
	Status         string                      `json:"status"`
	Claims         *licenseprotocol.ClaimsV2   `json:"claims,omitempty"`
	Document       *licenseprotocol.DocumentV2 `json:"document,omitempty"`
	DocumentSHA256 string                      `json:"document_sha256,omitempty"`
}

func (v *PaidTransferRecord) UnmarshalJSON(data []byte) error {
	type wire PaidTransferRecord
	var result wire
	var tag struct {
		Status string `json:"status"`
	}
	if err := json.Unmarshal(data, &tag); err != nil {
		return err
	}
	fields := []string{"snapshot", "sha256", "status"}
	if tag.Status == "prepared" || tag.Status == "issued" {
		fields = append(fields, "claims")
	}
	if tag.Status == "issued" {
		fields = append(fields, "document", "document_sha256")
	}
	if err := strictjson.ObjectWithin(data, &result, 2<<20, fields...); err != nil {
		return err
	}
	*v = PaidTransferRecord(result)
	return v.Validate()
}

func (v PaidTransferRecord) Validate() error {
	raw, err := v.Snapshot.Bytes()
	if err != nil || ContentDigest(raw) != v.SHA256 {
		return ErrPaidFulfillmentIntegrity
	}
	switch v.Status {
	case "approved":
		if v.Claims != nil || v.Document != nil || v.DocumentSHA256 != "" {
			return ErrPaidFulfillmentIntegrity
		}
		return nil
	case "prepared", "issued":
		if v.Claims == nil {
			return ErrPaidFulfillmentIntegrity
		}
	default:
		return ErrPaidFulfillmentIntegrity
	}
	issuedAt, err := parseTime(v.Claims.IssuedAt)
	if err != nil {
		return ErrPaidFulfillmentIntegrity
	}
	expected, err := v.Snapshot.Claims(v.Claims.KeyID, issuedAt)
	if err != nil {
		return ErrPaidFulfillmentIntegrity
	}
	want, err := licenseprotocol.CanonicalClaimsV2(expected)
	if err != nil {
		return ErrPaidFulfillmentIntegrity
	}
	actual, err := licenseprotocol.CanonicalClaimsV2(*v.Claims)
	if err != nil || !bytes.Equal(want, actual) {
		return ErrPaidFulfillmentIntegrity
	}
	if v.Status == "prepared" {
		if v.Document != nil || v.DocumentSHA256 != "" {
			return ErrPaidFulfillmentIntegrity
		}
		return nil
	}
	if v.Document == nil {
		return ErrPaidFulfillmentIntegrity
	}
	signature, err := base64.RawURLEncoding.Strict().DecodeString(v.Document.Signature)
	if err != nil || len(signature) != ed25519.SignatureSize || base64.RawURLEncoding.EncodeToString(signature) != v.Document.Signature {
		return ErrPaidFulfillmentIntegrity
	}
	actual, err = licenseprotocol.CanonicalClaimsV2(v.Document.Claims)
	if err != nil || !bytes.Equal(want, actual) {
		return ErrPaidFulfillmentIntegrity
	}
	raw, err = json.Marshal(v.Document)
	if err != nil || ContentDigest(raw) != v.DocumentSHA256 {
		return ErrPaidFulfillmentIntegrity
	}
	return nil
}

func NewPaidTransfer(id string, input ApprovePaidTransferInput, fulfillment PaidFulfillmentRecord, previousClaims licenseprotocol.ClaimsV2, previousDocumentSHA256, actor string, now time.Time) (PaidTransferRecord, error) {
	if input.Validate() != nil || fulfillment.Validate() != nil || fulfillment.Status != "issued" || fulfillment.Claims == nil || fulfillment.Document == nil ||
		previousClaims.Validate() != nil || previousClaims.LicenseID != fulfillment.Claims.LicenseID || previousClaims.Source.OrderID != fulfillment.Claims.Source.OrderID ||
		previousClaims.Binding.Mode != licenseprotocol.InstallationV2 || previousClaims.Binding.TransferSequence == nil ||
		input.ExpectedCurrentDocumentSHA256 != previousDocumentSHA256 || !catalogDigest.MatchString(previousDocumentSHA256) {
		return PaidTransferRecord{}, ErrInvalidPaidTransfer
	}
	request, err := licenseprotocol.ParseRequestV2([]byte(input.LicenseRequestJSON))
	if err != nil {
		return PaidTransferRecord{}, ErrInvalidPaidTransfer
	}
	requestBytes, err := canonical(request)
	if err != nil {
		return PaidTransferRecord{}, err
	}
	snapshot := PaidTransferSnapshot{
		Schema: PaidTransferSchema, ID: id, FulfillmentID: fulfillment.Snapshot.ID, FulfillmentSHA256: fulfillment.SHA256,
		CustomerID: fulfillment.Snapshot.Payment.Snapshot.Order.CustomerID, OriginalClaims: *fulfillment.Claims,
		TransferLimit: fulfillment.Snapshot.Payment.Snapshot.Order.Plan.Definition.TransferLimit, Request: input, InstallationRequest: request,
		RequestSHA256: ContentDigest(requestBytes), PreviousDocumentSHA256: previousDocumentSHA256,
		PreviousBinding: previousClaims.Binding, PreviousIssuedAt: previousClaims.IssuedAt,
		TransferSequence: *previousClaims.Binding.TransferSequence + 1, Environment: fulfillment.Snapshot.Environment,
		ApprovedBy: actor, ApprovedAt: now.UTC().Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z"),
	}
	raw, err := snapshot.Bytes()
	if err != nil {
		return PaidTransferRecord{}, err
	}
	var owned PaidTransferSnapshot
	if err := json.Unmarshal(raw, &owned); err != nil {
		return PaidTransferRecord{}, err
	}
	return PaidTransferRecord{Snapshot: owned, SHA256: ContentDigest(raw), Status: "approved"}, nil
}

func (v PaidTransferRecord) Matches(id, fulfillmentID, actor string, input []byte) bool {
	stored, err := v.Snapshot.Request.Bytes()
	return err == nil && v.Snapshot.ID == id && v.Snapshot.FulfillmentID == fulfillmentID && v.Snapshot.ApprovedBy == actor && bytes.Equal(stored, input)
}

// ValidateFulfillment binds the compact transfer authority back to the actual
// immutable initial fulfillment fetched and locked by the store.
func (v PaidTransferRecord) ValidateFulfillment(fulfillment PaidFulfillmentRecord) error {
	if v.Validate() != nil || fulfillment.Validate() != nil || fulfillment.Status != "issued" || fulfillment.Claims == nil || fulfillment.Document == nil {
		return ErrPaidFulfillmentIntegrity
	}
	want, err := licenseprotocol.CanonicalClaimsV2(v.Snapshot.OriginalClaims)
	if err != nil {
		return ErrPaidFulfillmentIntegrity
	}
	actual, err := licenseprotocol.CanonicalClaimsV2(*fulfillment.Claims)
	order := fulfillment.Snapshot.Payment.Snapshot.Order
	if err != nil || !bytes.Equal(want, actual) || v.Snapshot.FulfillmentID != fulfillment.Snapshot.ID ||
		v.Snapshot.FulfillmentSHA256 != fulfillment.SHA256 || v.Snapshot.CustomerID != order.CustomerID ||
		v.Snapshot.TransferLimit != order.Plan.Definition.TransferLimit || v.Snapshot.Environment != fulfillment.Snapshot.Environment {
		return ErrPaidFulfillmentIntegrity
	}
	return nil
}

func (v PaidTransferRecord) ValidatePredecessor(claims licenseprotocol.ClaimsV2, documentSHA256 string) error {
	if v.Validate() != nil || claims.Validate() != nil || !catalogDigest.MatchString(documentSHA256) ||
		v.Snapshot.PreviousDocumentSHA256 != documentSHA256 || v.Snapshot.PreviousIssuedAt != claims.IssuedAt ||
		claims.LicenseID != v.Snapshot.OriginalClaims.LicenseID || claims.Source.OrderID != v.Snapshot.OriginalClaims.Source.OrderID {
		return ErrPaidFulfillmentIntegrity
	}
	want, err := canonical(v.Snapshot.PreviousBinding)
	if err != nil {
		return ErrPaidFulfillmentIntegrity
	}
	actual, err := canonical(claims.Binding)
	if err != nil || !bytes.Equal(want, actual) {
		return ErrPaidFulfillmentIntegrity
	}
	return nil
}
