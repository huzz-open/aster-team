package httpapi

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"

	"golang.org/x/crypto/bcrypt"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/domain"
)

type paymentHandlerStore struct {
	*commercialHandlerStore
	payment      commercial.PaymentRecord
	paymentCalls int
	paymentError error
}

func (s *paymentHandlerStore) GetCommercialPayment(context.Context, string) (commercial.PaymentRecord, error) {
	return s.payment, s.paymentError
}
func (s *paymentHandlerStore) ConfirmCommercialPayment(_ context.Context, _ string, _ string, in commercial.ConfirmPaymentInput, _ string) (commercial.PaymentRecord, error) {
	s.paymentCalls++
	if s.paymentError != nil {
		return commercial.PaymentRecord{}, s.paymentError
	}
	return s.payment, nil
}

func TestCommercialPaymentHTTPFullReceiptAndIndependentPermission(t *testing.T) {
	raw, err := os.ReadFile("../../../../../contracts/test-vectors/plan-definition.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var d commercial.Definition
	if err := json.Unmarshal(raw, &d); err != nil {
		t.Fatal(err)
	}
	plan, err := commercial.FreezePlan("plan", 1, d)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now().UTC().Truncate(time.Millisecond)
	snapshot, err := commercial.CreateOrderSnapshot("order", "customer", plan, 1, now)
	if err != nil {
		t.Fatal(err)
	}
	raw, _ = snapshot.Bytes()
	order := commercial.OrderRecord{Snapshot: snapshot, SHA256: commercial.ContentDigest(raw), Status: "pending_payment", CreatedAt: now}
	in := commercial.ConfirmPaymentInput{OperationID: "payment_op", ExpectedOrderSHA256: order.SHA256, PaymentReference: "receipt", ReceivedAt: now.Format("2006-01-02T15:04:05.000Z"), Notes: "test"}
	receipt, err := commercial.NewPaymentRecord("payment", in, order, "operator_1", now)
	if err != nil {
		t.Fatal(err)
	}
	hash, err := bcrypt.GenerateFromPassword([]byte("current-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatal(err)
	}
	store := &paymentHandlerStore{commercialHandlerStore: &commercialHandlerStore{handlerStore: &handlerStore{operator: domain.Operator{ID: "operator_1", Email: "payment@test.invalid", Status: "active"}, passwordHash: string(hash)}, allowed: map[string]bool{}, order: order}, payment: receipt}
	store.session = application.CreateSessionParams{ID: "session_1", OperatorID: "operator_1", TokenHash: sha256.Sum256([]byte("session-token")), CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: now.Add(time.Hour)}
	handler := New(application.NewService(store, time.Hour), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 1 << 20})
	path := "/api/operations/v1/commercial/orders/order/payment"
	body, _ := json.Marshal(confirmCommercialPaymentInput{confirmationPaymentFields: confirmationPaymentFields(in), CurrentPassword: "current-password"})
	request := func(method, path string, raw []byte) *httptest.ResponseRecorder {
		r := httptest.NewRecorder()
		handler.ServeHTTP(r, highRiskRequest(method, path, string(raw)))
		return r
	}
	if r := request("POST", path, body); r.Code != 403 || store.paymentCalls != 0 {
		t.Fatal("missing permission admitted", r.Code)
	}
	store.allowed[application.PermissionPaymentConfirm] = true
	for _, bad := range []string{
		strings.Replace(string(body), "current-password", "wrong-password", 1),
		strings.Replace(string(body), `"notes":`, `"Notes":`, 1),
		strings.Replace(string(body), `"notes":`, `"notes":"duplicate","notes":`, 1),
		`{"amount_minor":1,` + string(body[1:]), string(body) + "garbage",
	} {
		if r := request("POST", path, []byte(bad)); r.Code < 400 || store.paymentCalls != 0 {
			t.Fatal("invalid authenticated input accepted", r.Code)
		}
	}
	for _, mutate := range []func(*http.Request){func(r *http.Request) { r.Header.Del("X-CSRF-Token") }, func(r *http.Request) { r.Header.Set("Origin", "https://untrusted.invalid") }, func(r *http.Request) { r.Header.Del("Cookie") }} {
		r := highRiskRequest("POST", path, string(body))
		mutate(r)
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, r)
		if w.Code < 400 || store.paymentCalls != 0 {
			t.Fatal("mutation auth bypass", w.Code)
		}
	}
	if r := request("GET", path+"-context", nil); r.Code != 200 || strings.Contains(r.Body.String(), `"snapshot"`) {
		t.Fatal("minimal context unavailable or excessive", r.Code, r.Body.String())
	}
	if r := request("GET", "/api/operations/v1/commercial/orders/order", nil); r.Code != 403 {
		t.Fatal("payment permission granted order administration", r.Code)
	}
	if r := request("POST", path, body); r.Code != 200 || store.paymentCalls != 1 || strings.Contains(r.Body.String(), "current_password") {
		t.Fatal("valid confirmation", r.Code, r.Body.String())
	}
	if r := request("GET", path, nil); r.Code != 200 {
		t.Fatal("payment operator cannot restore receipt", r.Code)
	}
	store.paymentError = commercial.ErrPaymentIntegrity
	if r := request("GET", path, nil); r.Code < 500 {
		t.Fatal("stored damage reported as client fault", r.Code)
	}
	delete(store.allowed, application.PermissionPaymentConfirm)
	store.allowed[application.PermissionCommercialOrderRead] = true
	if r := request("POST", path, body); r.Code != 403 || store.paymentCalls != 1 {
		t.Fatal("read permission grants confirmation", r.Code)
	}
}
