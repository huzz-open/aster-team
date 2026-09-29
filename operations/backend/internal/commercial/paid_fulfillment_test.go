package commercial

import (
	"crypto/ed25519"
	"encoding/json"
	"os"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/licenseprotocol"
)

func paidFulfillmentFixture(t *testing.T) (PaidFulfillmentRecord, PaymentRecord, time.Time) {
	t.Helper()
	p := acceptFixture(publicationFixture(t))
	now, _ := parseTime("2026-09-06T00:00:03.000Z")
	in := QuotationOrderInput{OperationID: "paid_order_op", CustomerID: "customer", PublicationID: p.Snapshot.ID, CatalogRevision: p.Snapshot.Catalog.ID, PlanID: "plan_annual", PlanVersion: 1, Years: 1, StartsAt: "2026-09-07T00:00:00.000Z"}
	order, err := CreateQuotationOrderSnapshot("paid_order", in, p, "local", now)
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := order.Bytes()
	orderRecord := OrderRecord{Snapshot: order, SHA256: ContentDigest(raw), Status: "pending_payment", CreatedAt: now, CreatedBy: "actor", OperationID: in.OperationID}
	payment, err := NewPaymentRecord("paid_payment", ConfirmPaymentInput{OperationID: "paid_payment_op", ExpectedOrderSHA256: orderRecord.SHA256, PaymentReference: "test_receipt", ReceivedAt: now.Format("2006-01-02T15:04:05.000Z"), Notes: "test only"}, orderRecord, "actor", now)
	if err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile("../../../../contracts/test-vectors/license-request.v2.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Request json.RawMessage `json:"request"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	request, err := licenseprotocol.ParseRequestV2(fixture.Request)
	if err != nil {
		t.Fatal(err)
	}
	request.ProductVersion = order.Plan.Definition.MinimumVersion
	raw, _ = json.Marshal(request)
	approval := ApprovePaidFulfillmentInput{OperationID: "paid_approval_op", ExpectedOrderSHA256: orderRecord.SHA256, ExpectedPaymentSHA256: payment.SHA256, LicenseRequestJSON: string(raw), Reason: "initial paid delivery fixture"}
	record, err := NewPaidFulfillment("paid_fulfillment", approval, payment, "customer_ref_original", "local", "actor", now)
	if err != nil {
		t.Fatal(err)
	}
	return record, payment, now
}

func clonePaid(t *testing.T, record PaidFulfillmentRecord) PaidFulfillmentRecord {
	t.Helper()
	raw, err := json.Marshal(record)
	if err != nil {
		t.Fatal(err)
	}
	var copy PaidFulfillmentRecord
	if err := json.Unmarshal(raw, &copy); err != nil {
		t.Fatal(err)
	}
	return copy
}

func TestPaidFulfillmentFreezesOriginalSourceRequestAndClaims(t *testing.T) {
	record, payment, now := paidFulfillmentFixture(t)
	if err := record.ValidatePaymentSource(payment); err != nil {
		t.Fatal(err)
	}
	claims, err := record.Snapshot.Claims("paid_test_key", now)
	if err != nil {
		t.Fatal(err)
	}
	order := payment.Snapshot.Order
	if claims.Source.Kind != licenseprotocol.CommercialOrderV2 || claims.Source.OrderID != order.OrderID || claims.Source.CustomerRef != "customer_ref_original" || claims.Source.RequestID != record.Snapshot.InstallationRequest.RequestID || claims.Binding.Mode != licenseprotocol.InstallationV2 || *claims.Binding.TransferSequence != 0 || claims.Validity.NotBefore != order.StartsAt || claims.Validity.Expiry.ExpiresAt != order.EndsAt {
		t.Fatal("claims lost original source or contract", claims)
	}
	if claims.Binding.InstallationID != record.Snapshot.InstallationRequest.InstallationID || claims.Binding.MachineFingerprintSHA256 != record.Snapshot.InstallationRequest.MachineFingerprintSHA256 {
		t.Fatal("binding changed")
	}
	want, _ := canonical(order.Plan.Definition.Entitlements)
	got, _ := canonical(claims.Entitlements)
	if string(want) != string(got) {
		t.Fatal("rights changed")
	}
	record.Status, record.Claims = "prepared", &claims
	if err := record.Validate(); err != nil {
		t.Fatal(err)
	}
	// Preparation after the end is a new issuance and must fail. Recovery of an
	// already frozen record has no wall-clock argument and retains old dates.
	end, _ := parseTime(order.EndsAt)
	if _, err := record.Snapshot.Claims("paid_test_key", end); err == nil {
		t.Fatal("new expired issuance allowed")
	}
	if _, err := record.Snapshot.Claims("paid_test_key", now.Add(-time.Millisecond)); err == nil {
		t.Fatal("backdated preparation allowed")
	}
	document, err := licenseprotocol.SignV2(claims, ed25519.NewKeyFromSeed(make([]byte, 32)))
	if err != nil {
		t.Fatal(err)
	}
	record.Status, record.Document = "issued", &document
	raw, _ := json.Marshal(document)
	record.DocumentSHA256 = ContentDigest(raw)
	if err := record.Validate(); err != nil {
		t.Fatal(err)
	}
	copy := clonePaid(t, record)
	if copy.DocumentSHA256 != record.DocumentSHA256 || copy.Claims.Validity != record.Claims.Validity {
		t.Fatal("recovery changed issued document")
	}
	for _, invalidSignature := range []string{"", "!" + strings.Repeat("A", 85), strings.Repeat("A", 85) + "B", document.Signature + "\n"} {
		bad := clonePaid(t, record)
		bad.Document.Signature = invalidSignature
		raw, _ := json.Marshal(bad.Document)
		bad.DocumentSHA256 = ContentDigest(raw)
		if bad.Validate() == nil {
			t.Fatal("malformed signature remained a valid issued record")
		}
		raw, _ = json.Marshal(bad)
		var decoded PaidFulfillmentRecord
		if json.Unmarshal(raw, &decoded) == nil {
			t.Fatal("malformed signature accepted through direct decode")
		}
	}
	// Neither a mutable source object nor returned claims may alter a fixed
	// approval's entitlement pointers, features or machine factors.
	before, _ := record.Snapshot.Bytes()
	payment.Snapshot.Order.Plan.Definition.Entitlements.Features[0] = "changed"
	newClaims, err := record.Snapshot.Claims("paid_test_key", now)
	if err != nil {
		t.Fatal(err)
	}
	newClaims.Entitlements.Features[0] = "changed"
	for _, q := range newClaims.Entitlements.Quotas {
		if q.Limit.Value != nil {
			*q.Limit.Value = 999
		}
	}
	after, err := record.Snapshot.Bytes()
	if err != nil || string(before) != string(after) {
		t.Fatal("aliased immutable approval", err)
	}
}

func TestPaidFulfillmentRejectsWrongAuthorityAndRawRequestAmbiguity(t *testing.T) {
	record, payment, now := paidFulfillmentFixture(t)
	for name, mutate := range map[string]func(*ApprovePaidFulfillmentInput){
		"wrong order":   func(v *ApprovePaidFulfillmentInput) { v.ExpectedOrderSHA256 = strings.Repeat("a", 64) },
		"wrong payment": func(v *ApprovePaidFulfillmentInput) { v.ExpectedPaymentSHA256 = strings.Repeat("b", 64) },
		"duplicate raw ID": func(v *ApprovePaidFulfillmentInput) {
			v.LicenseRequestJSON = strings.Replace(v.LicenseRequestJSON, `"request_id":`, `"request_id":"different","request_id":`, 1)
		},
		"duplicate raw nested": func(v *ApprovePaidFulfillmentInput) {
			v.LicenseRequestJSON = strings.Replace(v.LicenseRequestJSON, `"kind":`, `"kind":"machine_id","kind":`, 1)
		},
		"trailing raw": func(v *ApprovePaidFulfillmentInput) { v.LicenseRequestJSON += "{}" },
		"legacy schema": func(v *ApprovePaidFulfillmentInput) {
			v.LicenseRequestJSON = strings.Replace(v.LicenseRequestJSON, licenseprotocol.RequestSchemaV2, "aster.license-request.v1", 1)
		},
		"too old": func(v *ApprovePaidFulfillmentInput) {
			var r licenseprotocol.RequestV2
			_ = json.Unmarshal([]byte(v.LicenseRequestJSON), &r)
			r.ProductVersion = "0.0.0"
			b, _ := json.Marshal(r)
			v.LicenseRequestJSON = string(b)
		},
		"blank reason": func(v *ApprovePaidFulfillmentInput) { v.Reason = " " },
	} {
		t.Run(name, func(t *testing.T) {
			input := record.Snapshot.Request
			mutate(&input)
			if _, err := NewPaidFulfillment("paid", input, payment, "customer_ref_original", "local", "actor", now); err == nil {
				t.Fatal("invalid approval accepted")
			}
		})
	}
	for _, environment := range []string{"production", "", "invented"} {
		if _, err := NewPaidFulfillment("paid", record.Snapshot.Request, payment, "customer_ref_original", environment, "actor", now); err == nil {
			t.Fatal("local quotation became another environment")
		}
	}
	end, _ := parseTime(payment.Snapshot.Order.EndsAt)
	for _, when := range []time.Time{end, now.Add(-time.Second)} {
		if _, err := NewPaidFulfillment("paid", record.Snapshot.Request, payment, "customer_ref_original", "local", "actor", when); err == nil {
			t.Fatal("invalid approval time admitted")
		}
	}
	encoded, _ := json.Marshal(record.Snapshot.Request)
	for _, bad := range []string{
		`{"environment":"production",` + string(encoded[1:]),
		`{"customer_ref":"chosen",` + string(encoded[1:]),
		`{"entitlements":{},` + string(encoded[1:]),
		strings.Replace(string(encoded), `"reason":`, `"Reason":`, 1),
		strings.Replace(string(encoded), `"reason":`, `"reason":null,"reason":`, 1),
	} {
		var value ApprovePaidFulfillmentInput
		if json.Unmarshal([]byte(bad), &value) == nil {
			t.Fatal("client authority admitted")
		}
	}
	bad := clonePaid(t, record)
	bad.Snapshot.InstallationRequest.InstallationID = "different_installation"
	if bad.Snapshot.Validate() == nil {
		t.Fatal("same request ID changed installation")
	}
	bad = clonePaid(t, record)
	bad.Snapshot.RequestSHA256 = strings.Repeat("c", 64)
	if bad.Snapshot.Validate() == nil {
		t.Fatal("request digest mismatch accepted")
	}
	// An internally valid but different receipt is not the actual source.
	other := payment
	other.Snapshot.Request.PaymentReference = "another_receipt"
	raw, err := other.Snapshot.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	other.SHA256 = ContentDigest(raw)
	if other.Validate() != nil || record.ValidatePaymentSource(other) == nil {
		t.Fatal("self-hashed replacement source accepted")
	}
}

func TestPaidFulfillmentStatesRequireExactFrozenClaims(t *testing.T) {
	record, _, now := paidFulfillmentFixture(t)
	claims, _ := record.Snapshot.Claims("paid_test_key", now)
	record.Status, record.Claims = "prepared", &claims
	for name, mutate := range map[string]func(*PaidFulfillmentRecord){
		"rights":      func(v *PaidFulfillmentRecord) { v.Claims.Entitlements.Features[0] = "changed" },
		"customer":    func(v *PaidFulfillmentRecord) { v.Claims.Source.CustomerRef = "new_customer_ref" },
		"binding":     func(v *PaidFulfillmentRecord) { v.Claims.Binding.InstallationID = "new_installation" },
		"serial":      func(v *PaidFulfillmentRecord) { v.Claims.Serial += "changed" },
		"term":        func(v *PaidFulfillmentRecord) { v.Claims.Validity.Expiry.ExpiresAt = "2030-09-06T00:00:00.000Z" },
		"unprepared":  func(v *PaidFulfillmentRecord) { v.Status = "approved" },
		"undelivered": func(v *PaidFulfillmentRecord) { v.Status = "issued" },
	} {
		t.Run(name, func(t *testing.T) {
			bad := clonePaid(t, record)
			mutate(&bad)
			if bad.Validate() == nil {
				t.Fatal("changed frozen record accepted")
			}
		})
	}
	approved, _, _ := paidFulfillmentFixture(t)
	raw, _ := json.Marshal(approved)
	for _, bad := range []string{
		`{"claims":null,` + string(raw[1:]),
		`{"document":null,` + string(raw[1:]),
		strings.Replace(string(raw), `"status":"approved"`, `"status":"prepared"`, 1),
		strings.Replace(string(raw), `"status":`, `"Status":`, 1),
		string(raw) + "{}",
	} {
		var value PaidFulfillmentRecord
		if json.Unmarshal([]byte(bad), &value) == nil {
			t.Fatal("invalid wire state accepted")
		}
	}
}

func TestPaidFulfillmentLifecycleFreezesContinuityAndDates(t *testing.T) {
	predecessor, _, now := paidFulfillmentFixture(t)
	claims, err := predecessor.Snapshot.Claims("paid_test_key", now.Add(time.Second))
	if err != nil {
		t.Fatal(err)
	}
	document, err := licenseprotocol.SignV2(claims, ed25519.NewKeyFromSeed(make([]byte, 32)))
	if err != nil {
		t.Fatal(err)
	}
	documentRaw, _ := json.Marshal(document)
	predecessor.Status, predecessor.Claims, predecessor.Document, predecessor.DocumentSHA256 = "issued", &claims, &document, ContentDigest(documentRaw)

	publication := acceptFixture(publicationFixture(t))
	starts, _ := parseTime(claims.Validity.Expiry.ExpiresAt)
	input := QuotationOrderInput{OperationID: "renewal_order_op", CustomerID: predecessor.Snapshot.Payment.Snapshot.Order.CustomerID, PublicationID: publication.Snapshot.ID, CatalogRevision: publication.Snapshot.Catalog.ID, PlanID: "plan_annual", PlanVersion: 1, Years: 1, StartsAt: starts.Format("2006-01-02T15:04:05.000Z")}
	order, err := CreateQuotationOrderSnapshot("renewal_order", input, publication, "local", now)
	if err != nil {
		t.Fatal(err)
	}
	orderRaw, _ := order.Bytes()
	orderRecord := OrderRecord{Snapshot: order, SHA256: ContentDigest(orderRaw), Status: "pending_payment", CreatedAt: now, CreatedBy: "actor", OperationID: input.OperationID}
	payment, err := NewPaymentRecord("renewal_payment", ConfirmPaymentInput{OperationID: "renewal_payment_op", ExpectedOrderSHA256: orderRecord.SHA256, PaymentReference: "renewal_receipt", ReceivedAt: now.Format("2006-01-02T15:04:05.000Z"), Notes: "renewal"}, orderRecord, "actor", now)
	if err != nil {
		t.Fatal(err)
	}
	request := predecessor.Snapshot.InstallationRequest
	request.RequestID = "request_renewal_fixture"
	requestRaw, _ := json.Marshal(request)
	lifecycleRequest := PaidLifecycleRequest{Kind: PaidLifecycleRenewal, SourceID: predecessor.Snapshot.ID, ExpectedDocumentSHA256: predecessor.DocumentSHA256}
	approval := ApprovePaidFulfillmentInput{OperationID: "renewal_approval", ExpectedOrderSHA256: orderRecord.SHA256, ExpectedPaymentSHA256: payment.SHA256, LicenseRequestJSON: string(requestRaw), Reason: "renew current installation", Lifecycle: &lifecycleRequest}
	lifecycle := PaidLifecycleSource{Schema: PaidLifecycleSourceSchema, Kind: PaidLifecycleRenewal, SourceID: predecessor.Snapshot.ID, SourceRecordKind: "paid_fulfillment", SourceRecordID: predecessor.Snapshot.ID, SourceRecordSHA256: predecessor.SHA256, CustomerID: order.CustomerID, CustomerRef: claims.Source.CustomerRef, LicenseID: claims.LicenseID, Environment: "local", DocumentSHA256: predecessor.DocumentSHA256, Document: &document, Binding: claims.Binding, ValidFrom: claims.Validity.NotBefore, ValidUntil: claims.Validity.Expiry.ExpiresAt}
	record, err := NewPaidFulfillmentWithLifecycle("renewal_fulfillment", approval, payment, predecessor.Snapshot.CustomerRef, "local", "actor", now.Add(2*time.Second), &lifecycle)
	if err != nil {
		t.Fatal(err)
	}
	if record.Snapshot.Schema != PaidFulfillmentLifecycleSchema || record.Snapshot.Lifecycle == nil || record.Snapshot.Lifecycle.DocumentSHA256 != predecessor.DocumentSHA256 {
		t.Fatal("successor did not freeze its current source")
	}
	for name, approvalTime := range map[string]time.Time{
		"equal predecessor millisecond": now.Add(time.Second),
		"clock moved backwards":         now,
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := NewPaidFulfillmentWithLifecycle("invalid_successor_time", approval, payment, predecessor.Snapshot.CustomerRef, "local", "actor", approvalTime, &lifecycle); err == nil {
				t.Fatal("successor approval did not advance predecessor issued_at")
			}
		})
	}
	raw, _ := json.Marshal(record)
	var decoded PaidFulfillmentRecord
	if err := json.Unmarshal(raw, &decoded); err != nil {
		t.Fatal(err)
	}

	bad := lifecycle
	bad.Binding.InstallationID = "different_installation"
	if _, err := NewPaidFulfillmentWithLifecycle("bad_binding", approval, payment, predecessor.Snapshot.CustomerRef, "local", "actor", now.Add(2*time.Second), &bad); err == nil {
		t.Fatal("different successor installation accepted")
	}
	bad = lifecycle
	bad.ValidUntil = starts.Add(time.Hour).Format("2006-01-02T15:04:05.000Z")
	if _, err := NewPaidFulfillmentWithLifecycle("bad_boundary", approval, payment, predecessor.Snapshot.CustomerRef, "local", "actor", now.Add(2*time.Second), &bad); err == nil {
		t.Fatal("renewal gap accepted")
	}
	badApproval := approval
	badLifecycleRequest := lifecycleRequest
	badLifecycleRequest.ExpectedDocumentSHA256 = strings.Repeat("f", 64)
	badApproval.Lifecycle = &badLifecycleRequest
	if _, err := NewPaidFulfillmentWithLifecycle("bad_digest", badApproval, payment, predecessor.Snapshot.CustomerRef, "local", "actor", now.Add(2*time.Second), &lifecycle); err == nil {
		t.Fatal("stale source digest accepted")
	}
}

func TestLegacyTrialConversionIsNotAPaidLifecycleSource(t *testing.T) {
	request := PaidLifecycleRequest{Kind: "trial_conversion", SourceID: "old_trial", ExpectedDocumentSHA256: strings.Repeat("a", 64)}
	if request.Validate() == nil {
		t.Fatal("legacy trial request accepted")
	}
	record, _, _ := paidFulfillmentFixture(t)
	source := PaidLifecycleSource{Schema: PaidLifecycleSourceSchema, Kind: "trial_conversion", Document: record.Document}
	if source.Validate() == nil {
		t.Fatal("legacy trial source accepted")
	}
	if json.Unmarshal([]byte(`{"legacy_document_base64":"e30"}`), &source) == nil {
		t.Fatal("legacy document field accepted")
	}
}
