package httpapi

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/adapters/licensing"
	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
	"golang.org/x/crypto/bcrypt"
)

type paidHandlerStore struct {
	*paymentHandlerStore
	record             commercial.PaidFulfillmentRecord
	redelivery         commercial.PaidRedeliveryRecord
	paidCalls          int
	received           commercial.ApprovePaidFulfillmentInput
	receivedRedelivery commercial.RecordPaidRedeliveryInput
	redeliveryCalls    int
	transfer           commercial.PaidTransferRecord
	receivedTransfer   commercial.ApprovePaidTransferInput
	transferCalls      int
	lifecycle          commercial.PaidLifecycleSource
}

type paidHandlerReferences string

func (r paidHandlerReferences) Reference(string) (string, error) { return string(r), nil }

func (s *paidHandlerStore) GetPaidFulfillment(context.Context, string) (commercial.PaidFulfillmentRecord, error) {
	s.paidCalls++
	return s.record, nil
}
func (s *paidHandlerStore) GetPaidFulfillmentForOrder(context.Context, string) (commercial.PaidFulfillmentRecord, error) {
	s.paidCalls++
	return s.record, nil
}
func (s *paidHandlerStore) ApprovePaidFulfillment(_ context.Context, _ string, _ string, in commercial.ApprovePaidFulfillmentInput, _ string, _ string, _ ports.CustomerReferenceSource) (commercial.PaidFulfillmentRecord, error) {
	s.paidCalls++
	s.received = in
	return s.record, nil
}
func (s *paidHandlerStore) PreparePaidFulfillment(context.Context, string, string, string, string) (commercial.PaidFulfillmentRecord, error) {
	s.paidCalls++
	return s.record, nil
}
func (s *paidHandlerStore) CompletePaidFulfillment(context.Context, string, licenseprotocol.DocumentV2, string, string) (commercial.PaidFulfillmentRecord, error) {
	s.paidCalls++
	return s.record, nil
}
func (s *paidHandlerStore) GetPaidLifecycleSource(context.Context, string, string, string) (commercial.PaidLifecycleSource, error) {
	s.paidCalls++
	return s.lifecycle, nil
}
func (s *paidHandlerStore) GetPaidRedelivery(context.Context, string) (commercial.PaidRedeliveryRecord, error) {
	s.paidCalls++
	return s.redelivery, nil
}
func (s *paidHandlerStore) RecordPaidRedelivery(_ context.Context, _ string, _ string, input commercial.RecordPaidRedeliveryInput, _ string, _ string) (commercial.PaidRedeliveryRecord, error) {
	s.paidCalls++
	s.redeliveryCalls++
	s.receivedRedelivery = input
	return s.redelivery, nil
}
func (s *paidHandlerStore) GetPaidTransfer(_ context.Context, id string) (commercial.PaidTransferRecord, error) {
	s.paidCalls++
	if s.transfer.Snapshot.ID == "" || s.transfer.Snapshot.ID != id {
		return commercial.PaidTransferRecord{}, commercial.ErrNotFound
	}
	return s.transfer, nil
}
func (s *paidHandlerStore) GetLatestPaidTransfer(_ context.Context, fulfillmentID string) (commercial.PaidTransferRecord, error) {
	s.paidCalls++
	if s.transfer.Snapshot.FulfillmentID != fulfillmentID {
		return commercial.PaidTransferRecord{}, commercial.ErrNotFound
	}
	return s.transfer, nil
}
func (s *paidHandlerStore) ListPaidTransferChain(_ context.Context, fulfillmentID string, through uint32) ([]commercial.PaidTransferRecord, error) {
	s.paidCalls++
	if s.transfer.Snapshot.FulfillmentID != fulfillmentID || s.transfer.Snapshot.TransferSequence != through {
		return nil, commercial.ErrNotFound
	}
	return []commercial.PaidTransferRecord{s.transfer}, nil
}
func (s *paidHandlerStore) ApprovePaidTransfer(_ context.Context, id, _ string, input commercial.ApprovePaidTransferInput, _ string, actor string) (commercial.PaidTransferRecord, error) {
	s.paidCalls++
	s.transferCalls++
	s.receivedTransfer = input
	result, err := commercial.NewPaidTransfer(id, input, s.record, *s.record.Claims, s.record.DocumentSHA256, actor, time.Now().UTC().Add(time.Second))
	if err == nil {
		s.transfer = result
	}
	return result, err
}
func (s *paidHandlerStore) PreparePaidTransfer(context.Context, string, string, string, string) (commercial.PaidTransferRecord, error) {
	s.paidCalls++
	return s.transfer, nil
}
func (s *paidHandlerStore) CompletePaidTransfer(context.Context, string, licenseprotocol.DocumentV2, string, string) (commercial.PaidTransferRecord, error) {
	s.paidCalls++
	return s.transfer, nil
}

func TestPaidFulfillmentHTTPPermissionsRawInputAndVerifiedDownload(t *testing.T) {
	now := time.Now().UTC().Truncate(time.Millisecond)
	data, err := os.ReadFile("../../../../../contracts/test-vectors/plan-definition.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var definition commercial.Definition
	if err := json.Unmarshal(data, &definition); err != nil {
		t.Fatal(err)
	}
	plan, err := commercial.FreezePlan("paid_plan", 1, definition)
	if err != nil {
		t.Fatal(err)
	}
	snapshot, err := commercial.CreateOrderSnapshot("paid_order", "customer", plan, 1, now)
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := snapshot.Bytes()
	order := commercial.OrderRecord{Snapshot: snapshot, SHA256: commercial.ContentDigest(raw), Status: "pending_payment", CreatedAt: now, CreatedBy: "operator_1", OperationID: "order_op"}
	payment, err := commercial.NewPaymentRecord("paid_payment", commercial.ConfirmPaymentInput{OperationID: "payment_op", ExpectedOrderSHA256: order.SHA256, PaymentReference: "test-bank", ReceivedAt: now.Format("2006-01-02T15:04:05.000Z")}, order, "operator_1", now)
	if err != nil {
		t.Fatal(err)
	}
	data, err = os.ReadFile("../../../../../contracts/test-vectors/license-request.v2.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Request json.RawMessage `json:"request"`
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	req, err := licenseprotocol.ParseRequestV2(fixture.Request)
	if err != nil {
		t.Fatal(err)
	}
	req.ProductVersion = definition.MinimumVersion
	req.GeneratedAt = now.Add(-time.Second).Format("2006-01-02T15:04:05.000Z")
	raw, _ = json.MarshalIndent(req, "", "  ")
	input := commercial.ApprovePaidFulfillmentInput{OperationID: "approve_paid", ExpectedOrderSHA256: order.SHA256, ExpectedPaymentSHA256: payment.SHA256, LicenseRequestJSON: string(raw) + "\n", Reason: "test approval"}
	record, err := commercial.NewPaidFulfillment("paid_fulfillment", input, payment, "ref_customer", "local", "operator_1", now)
	if err != nil {
		t.Fatal(err)
	}
	hash, err := bcrypt.GenerateFromPassword([]byte("current-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatal(err)
	}
	private := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{31}, 32))
	sourceClaims, err := record.Snapshot.Claims("paid-http", now)
	if err != nil {
		t.Fatal(err)
	}
	sourceDocument, err := licenseprotocol.SignV2(sourceClaims, private)
	if err != nil {
		t.Fatal(err)
	}
	sourceDocumentRaw, _ := json.Marshal(sourceDocument)
	sequence := uint32(0)
	lifecycle := commercial.PaidLifecycleSource{Schema: commercial.PaidLifecycleSourceSchema, Kind: commercial.PaidLifecycleRenewal, SourceID: "previous_fulfillment", SourceRecordKind: "paid_fulfillment", SourceRecordID: "previous_fulfillment", SourceRecordSHA256: strings.Repeat("a", 64), CustomerID: snapshot.CustomerID, CustomerRef: sourceClaims.Source.CustomerRef, LicenseID: sourceClaims.LicenseID, Environment: "local", DocumentSHA256: commercial.ContentDigest(sourceDocumentRaw), Document: &sourceDocument, Binding: licenseprotocol.BindingV2{Mode: licenseprotocol.InstallationV2, InstallationID: req.InstallationID, MachineFingerprintSHA256: req.MachineFingerprintSHA256, TransferSequence: &sequence}, ValidFrom: snapshot.StartsAt, ValidUntil: snapshot.EndsAt}
	store := &paidHandlerStore{paymentHandlerStore: &paymentHandlerStore{commercialHandlerStore: &commercialHandlerStore{handlerStore: &handlerStore{operator: domain.Operator{ID: "operator_1", Email: "paid@test.invalid", Status: "active"}, passwordHash: string(hash)}, allowed: map[string]bool{application.PermissionCommercialPlanWrite: true}, order: order}, payment: payment}, record: record, lifecycle: lifecycle}
	store.session = application.CreateSessionParams{ID: "session_1", OperatorID: "operator_1", TokenHash: sha256.Sum256([]byte("session-token")), CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: now.Add(time.Hour)}
	public, _ := x509.MarshalPKIXPublicKey(private.Public())
	profile := ports.IssuerProfileV2{KeyID: "paid-http", PublicKeySPKI: base64.RawURLEncoding.EncodeToString(public), Policy: licenseprotocol.IssuerPolicyV2{Sources: []licenseprotocol.SourceKindV2{licenseprotocol.CommercialOrderV2}, Bindings: []licenseprotocol.BindingKindV2{licenseprotocol.InstallationV2}, Expiries: []licenseprotocol.ExpiryKindV2{licenseprotocol.FixedExpiryV2}, EntitlementCeiling: definition.Entitlements}}
	verifier, err := licensing.NewV2Verifier(profile)
	if err != nil {
		t.Fatal(err)
	}
	makeHandler := func(withVerifier bool) http.Handler {
		opts := []application.Option{application.WithFulfillmentEnvironment("local"), application.WithCustomerReferenceSource(paidHandlerReferences(sourceClaims.Source.CustomerRef))}
		if withVerifier {
			opts = append(opts, application.WithV2LicenseVerifiers(map[string]ports.LicenseVerifierV2{"paid-http": verifier}))
		}
		return New(application.NewService(store, time.Hour, opts...), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 1 << 20})
	}
	handler := makeHandler(true)
	send := func(method, path, body string) *httptest.ResponseRecorder {
		r := httptest.NewRecorder()
		handler.ServeHTTP(r, highRiskRequest(method, "/api/operations/v1"+path, body))
		return r
	}
	approvalPath := "/commercial/orders/paid_order/fulfillment"
	recordPath := "/commercial/paid-fulfillments/paid_fulfillment"
	lifecyclePath := "/commercial/paid-lifecycle-sources/renewal/previous_fulfillment"
	body, _ := json.Marshal(approvePaidInput{paidApprovalFields: paidApprovalFields(input), CurrentPassword: "current-password"})
	issueBody := `{"key_id":"paid-http","current_password":"current-password"}`
	for _, tc := range []struct{ method, path, body string }{{"GET", approvalPath, ""}, {"GET", approvalPath + "-context", ""}, {"POST", approvalPath, string(body)}, {"GET", lifecyclePath, ""}, {"GET", recordPath, ""}, {"POST", recordPath + "/issue", issueBody}, {"GET", recordPath + "/license", ""}} {
		if r := send(tc.method, tc.path, tc.body); r.Code != 403 {
			t.Fatal("unrelated plan permission admitted", tc.path, r.Code, r.Body.String())
		}
	}
	if store.paidCalls != 0 || store.calls != 0 {
		t.Fatal("permission denial touched protected business state")
	}
	store.allowed = map[string]bool{application.PermissionFulfillmentApprove: true}
	if r := send("GET", lifecyclePath, ""); r.Code != 200 || !strings.Contains(r.Body.String(), lifecycle.DocumentSHA256) {
		t.Fatal("lifecycle preview unavailable to approver", r.Code, r.Body.String())
	}
	if r := send("GET", approvalPath+"-context", ""); r.Code != 200 || !strings.Contains(r.Body.String(), payment.SHA256) {
		t.Fatal("minimal approval context required unrelated permissions", r.Code, r.Body.String())
	}
	if r := send("GET", approvalPath+"-context", ""); r.Code != 200 || !strings.Contains(r.Body.String(), definition.Name) || !strings.Contains(r.Body.String(), `"entitlements"`) {
		t.Fatal("approval context omitted the original frozen rights", r.Code, r.Body.String())
	}
	for _, password := range []string{"", "wrong"} {
		bad, _ := json.Marshal(approvePaidInput{paidApprovalFields: paidApprovalFields(input), CurrentPassword: password})
		if r := send("POST", approvalPath, string(bad)); r.Code != 401 {
			t.Fatal("missing current password admitted", r.Code)
		}
	}
	before := store.paidCalls
	missingCSRF := highRiskRequest("POST", "/api/operations/v1"+approvalPath, string(body))
	missingCSRF.Header.Del("X-CSRF-Token")
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, missingCSRF)
	if response.Code != 403 || store.paidCalls != before {
		t.Fatal("CSRF failure reached approval")
	}
	if r := send("POST", approvalPath, string(body)); r.Code != 200 || store.received.LicenseRequestJSON != input.LicenseRequestJSON {
		t.Fatal("raw installation request changed", r.Code, r.Body.String())
	}
	persisted, _ := json.Marshal(store.received)
	if strings.Contains(string(persisted), "password") {
		t.Fatal("password persisted in business input")
	}
	for _, invalid := range []string{strings.Replace(string(body), `"current_password"`, `"Current_Password"`, 1), strings.Replace(string(body), `"current_password":`, `"status":"issued","current_password":`, 1)} {
		if r := send("POST", approvalPath, invalid); r.Code != 400 {
			t.Fatal("ambiguous transport fields admitted", r.Code)
		}
	}
	duplicate := input
	duplicate.LicenseRequestJSON = strings.Replace(input.LicenseRequestJSON, `"request_id":`, `"request_id":"duplicate","request_id":`, 1)
	bad, _ := json.Marshal(approvePaidInput{paidApprovalFields: paidApprovalFields(duplicate), CurrentPassword: "current-password"})
	before = store.paidCalls
	if r := send("POST", approvalPath, string(bad)); r.Code != 400 || store.paidCalls != before {
		t.Fatal("duplicate request field collapsed before strict parser", r.Code, r.Body.String())
	}
	if r := send("POST", recordPath+"/issue", issueBody); r.Code != 403 {
		t.Fatal("approval right enabled signing", r.Code)
	}
	store.allowed = map[string]bool{application.PermissionLicenseIssueV2: true}
	if r := send("POST", recordPath+"/issue", strings.Replace(issueBody, "current-password", "wrong", 1)); r.Code != 401 {
		t.Fatal("signing skipped current password", r.Code)
	}
	if r := send("POST", recordPath+"/issue", issueBody); r.Code != 503 || !strings.Contains(r.Body.String(), "COMMERCIAL_V2_SIGNER_UNAVAILABLE") {
		t.Fatal("unissued signing fell back without private key", r.Code, r.Body.String())
	}
	if r := send("GET", recordPath+"/license", ""); r.Code != 409 {
		t.Fatal("unissued download", r.Code)
	}
	claims, err := record.Snapshot.Claims("paid-http", now)
	if err != nil {
		t.Fatal(err)
	}
	doc, err := licenseprotocol.SignV2(claims, private)
	if err != nil {
		t.Fatal(err)
	}
	documentRaw, _ := json.Marshal(doc)
	store.record.Status, store.record.Claims, store.record.Document, store.record.DocumentSHA256 = "issued", &claims, &doc, commercial.ContentDigest(documentRaw)
	for _, path := range []string{recordPath, approvalPath} {
		if r := send("GET", path, ""); r.Code != 200 {
			t.Fatal("signer could not read minimal receipt", r.Code, r.Body.String())
		}
	}
	if r := send("POST", recordPath+"/issue", issueBody); r.Code != 200 {
		t.Fatal("public-only original issued recovery failed", r.Code, r.Body.String())
	}
	redeliveryInput := commercial.RecordPaidRedeliveryInput{OperationID: "redeliver_paid", ExpectedDocumentSHA256: store.record.DocumentSHA256, Reason: "customer requested the original file again"}
	store.redelivery, err = commercial.NewPaidRedelivery("redelivery_1", redeliveryInput, store.record, "operator_1", now.Add(time.Minute))
	if err != nil {
		t.Fatal(err)
	}
	redeliveryPath := recordPath + "/redeliveries"
	redeliveryBody, _ := json.Marshal(recordPaidRedeliveryInput{paidRedeliveryFields: paidRedeliveryFields(redeliveryInput), CurrentPassword: "current-password"})
	if r := send("POST", redeliveryPath, string(redeliveryBody)); r.Code != 403 {
		t.Fatal("license issuance permission enabled redelivery", r.Code, r.Body.String())
	}
	store.allowed = map[string]bool{application.PermissionFulfillmentApprove: true}
	if r := send("POST", redeliveryPath, strings.Replace(string(redeliveryBody), "current-password", "wrong", 1)); r.Code != 401 {
		t.Fatal("redelivery skipped current password", r.Code, r.Body.String())
	}
	before = store.paidCalls
	if r := send("POST", redeliveryPath, strings.Replace(string(redeliveryBody), `"reason":`, `"document":{},"reason":`, 1)); r.Code != 400 || store.paidCalls != before {
		t.Fatal("redelivery admitted caller-owned document", r.Code, r.Body.String())
	}
	if r := send("POST", redeliveryPath, string(redeliveryBody)); r.Code != 200 || !strings.Contains(r.Body.String(), store.record.DocumentSHA256) {
		t.Fatal("redelivery was not recorded", r.Code, r.Body.String())
	}
	if store.receivedRedelivery != redeliveryInput {
		t.Fatal("redelivery input changed", store.receivedRedelivery)
	}
	persisted, _ = json.Marshal(store.receivedRedelivery)
	if strings.Contains(string(persisted), "password") {
		t.Fatal("redelivery password entered business state")
	}
	if r := send("GET", "/commercial/paid-redeliveries/redelivery_1", ""); r.Code != 200 {
		t.Fatal("verified redelivery record could not be read", r.Code, r.Body.String())
	}
	transferRequest := store.record.Snapshot.InstallationRequest
	transferRequest.RequestID = "paid_http_transfer_request"
	transferRequest.InstallationID += "_moved"
	transferRequest.MachineFingerprintSHA256 = strings.Repeat("A", 43)
	transferRequest.GeneratedAt = now.Format("2006-01-02T15:04:05.000Z")
	transferRequestRaw, _ := json.Marshal(transferRequest)
	transferInput := commercial.ApprovePaidTransferInput{OperationID: "paid_http_transfer", ExpectedCurrentDocumentSHA256: store.record.DocumentSHA256, LicenseRequestJSON: string(transferRequestRaw), Reason: "approved replacement machine"}
	transferPath := recordPath + "/transfers"
	transferBody, _ := json.Marshal(approvePaidTransferInput{paidTransferFields: paidTransferFields(transferInput), CurrentPassword: "current-password"})
	beforeTransferCalls := store.transferCalls
	if r := send("POST", transferPath, strings.Replace(string(transferBody), "current-password", "wrong", 1)); r.Code != 401 || store.transferCalls != beforeTransferCalls {
		t.Fatal("transfer skipped current password", r.Code, r.Body.String())
	}
	if r := send("POST", transferPath, strings.Replace(string(transferBody), `"reason":`, `"document":{},"reason":`, 1)); r.Code != 400 || store.transferCalls != beforeTransferCalls {
		t.Fatal("transfer admitted caller-owned document", r.Code, r.Body.String())
	}
	response = send("POST", transferPath, string(transferBody))
	if response.Code != 200 || store.receivedTransfer.LicenseRequestJSON != transferInput.LicenseRequestJSON || store.transfer.Snapshot.TransferSequence != 1 {
		t.Fatal("transfer approval changed raw request or sequence", response.Code, response.Body.String())
	}
	persisted, _ = json.Marshal(store.receivedTransfer)
	if strings.Contains(string(persisted), "password") {
		t.Fatal("transfer password entered business state")
	}
	transferRecordPath := "/commercial/paid-transfers/" + store.transfer.Snapshot.ID
	if r := send("POST", transferRecordPath+"/issue", issueBody); r.Code != 403 {
		t.Fatal("approval permission enabled transfer signing", r.Code, r.Body.String())
	}
	store.allowed = map[string]bool{application.PermissionLicenseIssueV2: true}
	if r := send("POST", transferRecordPath+"/issue", issueBody); r.Code != 503 || !strings.Contains(r.Body.String(), "COMMERCIAL_V2_SIGNER_UNAVAILABLE") {
		t.Fatal("transfer signing fell back without private key", r.Code, r.Body.String())
	}
	transferClaims, err := store.transfer.Snapshot.Claims("paid-http", now.Add(2*time.Second))
	if err != nil {
		t.Fatal(err)
	}
	transferDocument, err := licenseprotocol.SignV2(transferClaims, private)
	if err != nil {
		t.Fatal(err)
	}
	transferDocumentRaw, _ := json.Marshal(transferDocument)
	store.transfer.Status, store.transfer.Claims, store.transfer.Document, store.transfer.DocumentSHA256 = "issued", &transferClaims, &transferDocument, commercial.ContentDigest(transferDocumentRaw)
	store.allowed = map[string]bool{application.PermissionFulfillmentRead: true}
	r := send("GET", recordPath+"/license", "")
	if r.Code != 200 || !bytes.Equal(r.Body.Bytes(), documentRaw) || r.Header().Get("X-Content-SHA256") != store.record.DocumentSHA256 || r.Header().Get("Cache-Control") != "no-store" {
		t.Fatal("download bytes or integrity headers changed", r.Code, r.Body.String())
	}
	if r := send("GET", transferRecordPath, ""); r.Code != 200 {
		t.Fatal("verified transfer could not be read", r.Code, r.Body.String())
	}
	r = send("GET", transferRecordPath+"/license", "")
	if r.Code != 200 || !bytes.Equal(r.Body.Bytes(), transferDocumentRaw) || r.Header().Get("X-Content-SHA256") != store.transfer.DocumentSHA256 || r.Header().Get("Cache-Control") != "no-store" {
		t.Fatal("transfer download bytes or integrity headers changed", r.Code, r.Body.String())
	}
	handler = makeHandler(false)
	if r := send("GET", recordPath+"/license", ""); r.Code != 503 || !strings.Contains(r.Body.String(), "COMMERCIAL_V2_VERIFIER_UNAVAILABLE") {
		t.Fatal("untrusted document downloaded", r.Code, r.Body.String())
	}
	store.allowed = map[string]bool{application.PermissionFulfillmentApprove: true}
	beforeRedeliveryCalls := store.redeliveryCalls
	if r := send("POST", redeliveryPath, string(redeliveryBody)); r.Code != 503 || !strings.Contains(r.Body.String(), "COMMERCIAL_V2_VERIFIER_UNAVAILABLE") || store.redeliveryCalls != beforeRedeliveryCalls {
		t.Fatal("redelivery persisted before trusted signature verification", r.Code, r.Body.String(), store.redeliveryCalls)
	}
}
