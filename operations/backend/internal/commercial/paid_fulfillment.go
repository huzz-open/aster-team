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

const PaidFulfillmentSchema = "aster.paid-fulfillment.v1"
const PaidFulfillmentLifecycleSchema = "aster.paid-fulfillment.v2"

var ErrInvalidPaidFulfillment = errors.New("invalid paid fulfillment approval")
var ErrPaidFulfillmentIntegrity = errors.New("stored paid fulfillment integrity failure")
var ErrFulfillmentEnvironment = errors.New("paid fulfillment environment is unavailable or differs from approval")
var ErrCustomerReferenceUnavailable = errors.New("customer reference derivation unavailable")

// The original JSON text reaches the strict parser before a browser or generic
// object decoder can collapse duplicate keys. Customer, environment, dates and
// entitlements are derived from trusted sources, never supplied in this input.
type ApprovePaidFulfillmentInput struct {
	OperationID           string                `json:"operation_id"`
	ExpectedOrderSHA256   string                `json:"expected_order_sha256"`
	ExpectedPaymentSHA256 string                `json:"expected_payment_sha256"`
	LicenseRequestJSON    string                `json:"license_request_json"`
	Reason                string                `json:"reason"`
	Lifecycle             *PaidLifecycleRequest `json:"lifecycle,omitempty"`
}

func (v *ApprovePaidFulfillmentInput) UnmarshalJSON(data []byte) error {
	type wire ApprovePaidFulfillmentInput
	var result wire
	fields := []string{"operation_id", "expected_order_sha256", "expected_payment_sha256", "license_request_json", "reason"}
	var shape map[string]json.RawMessage
	if err := json.Unmarshal(data, &shape); err == nil {
		if _, ok := shape["lifecycle"]; ok {
			fields = append(fields, "lifecycle")
		}
	}
	if err := strictjson.Object(data, &result, fields...); err != nil {
		return err
	}
	*v = ApprovePaidFulfillmentInput(result)
	return v.Validate()
}
func (v ApprovePaidFulfillmentInput) Validate() error {
	if !identifier.MatchString(v.OperationID) || !catalogDigest.MatchString(v.ExpectedOrderSHA256) || !catalogDigest.MatchString(v.ExpectedPaymentSHA256) || len(v.LicenseRequestJSON) > 16<<10 || !validText(v.Reason, 2000, false) {
		return ErrInvalidPaidFulfillment
	}
	if _, err := licenseprotocol.ParseRequestV2([]byte(v.LicenseRequestJSON)); err != nil {
		return ErrInvalidPaidFulfillment
	}
	if v.Lifecycle != nil && v.Lifecycle.Validate() != nil {
		return ErrInvalidPaidFulfillment
	}
	return nil
}
func (v ApprovePaidFulfillmentInput) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	return canonical(v)
}

// Payment already includes the complete immutable order and its original
// publication reference. Keeping that single copy avoids divergent order views.
type PaidFulfillmentSnapshot struct {
	Schema              string                      `json:"schema"`
	ID                  string                      `json:"id"`
	Payment             PaymentRecord               `json:"payment"`
	Request             ApprovePaidFulfillmentInput `json:"request"`
	InstallationRequest licenseprotocol.RequestV2   `json:"installation_request"`
	RequestSHA256       string                      `json:"request_sha256"`
	CustomerRef         string                      `json:"customer_ref"`
	Environment         string                      `json:"environment"`
	ApprovedBy          string                      `json:"approved_by"`
	ApprovedAt          string                      `json:"approved_at"`
	Lifecycle           *PaidLifecycleSource        `json:"lifecycle,omitempty"`
}

func (v *PaidFulfillmentSnapshot) UnmarshalJSON(data []byte) error {
	type wire PaidFulfillmentSnapshot
	var result wire
	fields := []string{"schema", "id", "payment", "request", "installation_request", "request_sha256", "customer_ref", "environment", "approved_by", "approved_at"}
	var tag struct {
		Schema string `json:"schema"`
	}
	if err := json.Unmarshal(data, &tag); err == nil && tag.Schema == PaidFulfillmentLifecycleSchema {
		fields = append(fields, "lifecycle")
	}
	if err := strictjson.ObjectWithin(data, &result, 3<<20, fields...); err != nil {
		return err
	}
	*v = PaidFulfillmentSnapshot(result)
	return v.Validate()
}
func (v PaidFulfillmentSnapshot) Validate() error {
	if (v.Schema != PaidFulfillmentSchema && v.Schema != PaidFulfillmentLifecycleSchema) || !identifier.MatchString(v.ID) || len(v.ID) > 64 || !identifier.MatchString(v.ApprovedBy) || len(v.ApprovedBy) > 64 || !identifier.MatchString(v.CustomerRef) || v.Payment.Validate() != nil || v.Request.Validate() != nil || v.InstallationRequest.Validate() != nil {
		return ErrInvalidPaidFulfillment
	}
	if (v.Schema == PaidFulfillmentSchema) != (v.Lifecycle == nil) || (v.Request.Lifecycle == nil) != (v.Lifecycle == nil) {
		return ErrInvalidPaidFulfillment
	}
	if v.Environment != "local" && v.Environment != "production" {
		return ErrInvalidPaidFulfillment
	}
	order := v.Payment.Snapshot.Order
	if v.Request.ExpectedPaymentSHA256 != v.Payment.SHA256 || v.Request.ExpectedOrderSHA256 != v.Payment.Snapshot.OrderSHA256 || (order.Source != nil && order.Source.Environment != v.Environment) {
		return ErrInvalidPaidFulfillment
	}
	definition := order.Plan.Definition
	if !v.InstallationRequest.Supports(definition.MinimumVersion, definition.Entitlements.CatalogVersion, definition.QuotaPolicyVersion) {
		return ErrInvalidPaidFulfillment
	}
	parsed, err := licenseprotocol.ParseRequestV2([]byte(v.Request.LicenseRequestJSON))
	if err != nil {
		return ErrInvalidPaidFulfillment
	}
	expected, err := canonical(parsed)
	if err != nil {
		return err
	}
	actual, err := canonical(v.InstallationRequest)
	if err != nil || !bytes.Equal(expected, actual) || ContentDigest(actual) != v.RequestSHA256 {
		return ErrInvalidPaidFulfillment
	}
	approved, e1 := parseTime(v.ApprovedAt)
	confirmed, e2 := parseTime(v.Payment.Snapshot.ConfirmedAt)
	generated, e3 := parseTime(v.InstallationRequest.GeneratedAt)
	end, e4 := parseTime(order.EndsAt)
	if e1 != nil || e2 != nil || e3 != nil || e4 != nil || approved.Before(confirmed) || generated.After(approved) || !approved.Before(end) {
		return ErrInvalidPaidFulfillment
	}
	if v.Lifecycle != nil {
		if v.Lifecycle.ValidateSuccessor(order, v.InstallationRequest, approved) != nil || v.Request.Lifecycle.Kind != v.Lifecycle.Kind || v.Request.Lifecycle.SourceID != v.Lifecycle.SourceID || v.Request.Lifecycle.ExpectedDocumentSHA256 != v.Lifecycle.DocumentSHA256 {
			return ErrInvalidPaidFulfillment
		}
	}
	return nil
}
func (v PaidFulfillmentSnapshot) Bytes() ([]byte, error) {
	if err := v.Validate(); err != nil {
		return nil, err
	}
	raw, err := canonical(v)
	if len(raw) > 3<<20 {
		return nil, ErrInvalidPaidFulfillment
	}
	return raw, err
}

// The store must also lock and compare the actual original payment/order/source,
// check an active customer and enforce one initial approval per order. This
// constructor only validates/fixes their already resolved, immutable values.
func NewPaidFulfillment(id string, in ApprovePaidFulfillmentInput, payment PaymentRecord, customerRef, environment, actor string, now time.Time) (PaidFulfillmentRecord, error) {
	return NewPaidFulfillmentWithLifecycle(id, in, payment, customerRef, environment, actor, now, nil)
}

func NewPaidFulfillmentWithLifecycle(id string, in ApprovePaidFulfillmentInput, payment PaymentRecord, customerRef, environment, actor string, now time.Time, lifecycle *PaidLifecycleSource) (PaidFulfillmentRecord, error) {
	if err := in.Validate(); err != nil {
		return PaidFulfillmentRecord{}, err
	}
	request, err := licenseprotocol.ParseRequestV2([]byte(in.LicenseRequestJSON))
	if err != nil {
		return PaidFulfillmentRecord{}, ErrInvalidPaidFulfillment
	}
	requestBytes, err := canonical(request)
	if err != nil {
		return PaidFulfillmentRecord{}, err
	}
	schema := PaidFulfillmentSchema
	if lifecycle != nil {
		if lifecycle.CustomerRef != customerRef || lifecycle.Environment != environment {
			return PaidFulfillmentRecord{}, ErrInvalidPaidFulfillment
		}
		schema = PaidFulfillmentLifecycleSchema
	}
	snapshot := PaidFulfillmentSnapshot{Schema: schema, ID: id, Payment: payment, Request: in, InstallationRequest: request, RequestSHA256: ContentDigest(requestBytes), CustomerRef: customerRef, Environment: environment, ApprovedBy: actor, ApprovedAt: now.UTC().Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z"), Lifecycle: lifecycle}
	raw, err := snapshot.Bytes()
	if err != nil {
		return PaidFulfillmentRecord{}, err
	}
	// Own all nested entitlement pointers/slices and payment data.
	var owned PaidFulfillmentSnapshot
	if err := json.Unmarshal(raw, &owned); err != nil {
		return PaidFulfillmentRecord{}, err
	}
	return PaidFulfillmentRecord{Snapshot: owned, SHA256: ContentDigest(raw), Status: "approved"}, nil
}

func (v PaidFulfillmentSnapshot) Claims(keyID string, issuedAt time.Time) (licenseprotocol.ClaimsV2, error) {
	raw, err := v.Bytes()
	if err != nil {
		return licenseprotocol.ClaimsV2{}, err
	}
	var owned PaidFulfillmentSnapshot
	if err := json.Unmarshal(raw, &owned); err != nil {
		return licenseprotocol.ClaimsV2{}, err
	}
	approved, _ := parseTime(v.ApprovedAt)
	if issuedAt.Before(approved) {
		return licenseprotocol.ClaimsV2{}, ErrInvalidPaidFulfillment
	}
	order := owned.Payment.Snapshot.Order
	plan := order.Plan
	transfer := uint32(0)
	claims := licenseprotocol.ClaimsV2{
		Schema: licenseprotocol.SchemaV2, KeyID: keyID, LicenseID: "license_" + v.ID, Serial: "serial_" + v.ID,
		Product: plan.Definition.Product, Edition: plan.Definition.Edition, PlanID: plan.PlanID, PlanVersion: plan.Version,
		Source:       licenseprotocol.SourceV2{Kind: licenseprotocol.CommercialOrderV2, OrderID: order.OrderID, CustomerRef: v.CustomerRef, RequestID: v.InstallationRequest.RequestID},
		Entitlements: plan.Definition.Entitlements, QuotaPolicyVersion: plan.Definition.QuotaPolicyVersion,
		Binding:  licenseprotocol.BindingV2{Mode: licenseprotocol.InstallationV2, InstallationID: v.InstallationRequest.InstallationID, MachineFingerprintSHA256: v.InstallationRequest.MachineFingerprintSHA256, TransferSequence: &transfer},
		Validity: licenseprotocol.ValidityV2{NotBefore: order.StartsAt, Expiry: licenseprotocol.ExpiryV2{Mode: licenseprotocol.FixedExpiryV2, ExpiresAt: order.EndsAt}},
		IssuedAt: issuedAt.UTC().Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z"), MinimumVersion: plan.Definition.MinimumVersion,
	}
	return claims, claims.Validate()
}

type PaidFulfillmentRecord struct {
	Snapshot       PaidFulfillmentSnapshot     `json:"snapshot"`
	SHA256         string                      `json:"sha256"`
	Status         string                      `json:"status"`
	Claims         *licenseprotocol.ClaimsV2   `json:"claims,omitempty"`
	Document       *licenseprotocol.DocumentV2 `json:"document,omitempty"`
	DocumentSHA256 string                      `json:"document_sha256,omitempty"`
}

func (v *PaidFulfillmentRecord) UnmarshalJSON(data []byte) error {
	type wire PaidFulfillmentRecord
	var result wire
	// State determines the complete field set; null, missing and unexpected
	// issuance fields cannot silently decode into an approved record.
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
	if err := strictjson.ObjectWithin(data, &result, 4<<20, fields...); err != nil {
		return err
	}
	*v = PaidFulfillmentRecord(result)
	return v.Validate()
}
func (v PaidFulfillmentRecord) Validate() error {
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
	issued, err := parseTime(v.Claims.IssuedAt)
	if err != nil {
		return ErrPaidFulfillmentIntegrity
	}
	expected, err := v.Snapshot.Claims(v.Claims.KeyID, issued)
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
	// Application verification against the configured scoped public key is
	// still mandatory before persistence, recovery and download.
	return nil
}

// Self-consistent hashes cannot substitute for checking actual stored sources.
// The caller must fetch/validate the original payment and order in the same tx.
func (v PaidFulfillmentRecord) ValidatePaymentSource(payment PaymentRecord) error {
	if v.Validate() != nil || payment.Validate() != nil {
		return ErrPaidFulfillmentIntegrity
	}
	expected, err := v.Snapshot.Payment.Snapshot.Bytes()
	if err != nil {
		return ErrPaidFulfillmentIntegrity
	}
	actual, err := payment.Snapshot.Bytes()
	if err != nil || payment.SHA256 != v.Snapshot.Payment.SHA256 || !bytes.Equal(expected, actual) {
		return ErrPaidFulfillmentIntegrity
	}
	return nil
}
