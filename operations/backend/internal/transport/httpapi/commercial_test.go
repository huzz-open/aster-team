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
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/domain"
)

type commercialHandlerStore struct {
	*handlerStore
	allowed              map[string]bool
	calls                int
	plan                 commercial.PlanVersionRecord
	order                commercial.OrderRecord
	conflict             error
	quotationEnvironment string
	orderListQuery       application.CommercialOrderListQuery
}

func (s *commercialHandlerStore) CreateQuotationOrder(_ context.Context, _ string, _ commercial.QuotationOrderInput, environment, _ string) (commercial.OrderRecord, error) {
	s.calls++
	s.quotationEnvironment = environment
	return s.order, nil
}
func (s *commercialHandlerStore) GetQuotationSource(_ context.Context, reference, environment string) (commercial.QuotationSource, error) {
	s.calls++
	return commercial.QuotationSource{PublicationID: reference, Environment: environment, Plans: []commercial.PlanSnapshot{}}, nil
}

func TestQuotationOrderHTTPRejectsCallerAuthorityAndRequiresMutationAuth(t *testing.T) {
	now := time.Now().UTC()
	store := &commercialHandlerStore{handlerStore: &handlerStore{operator: domain.Operator{ID: "operator_1", Email: "quote@test.invalid", Status: "active"}}, allowed: map[string]bool{}}
	store.session = application.CreateSessionParams{ID: "session_1", OperatorID: "operator_1", TokenHash: sha256.Sum256([]byte("session-token")), CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: now.Add(time.Hour)}
	handler := New(application.NewService(store, time.Hour, application.WithQuotationEnvironment("production")), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 1 << 20})
	body := `{"operation_id":"quote","customer_id":"customer","publication_id":"publication","catalog_revision":"catalog_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","plan_id":"plan","plan_version":1,"years":1,"starts_at":"2026-09-07T00:00:00.000Z"}`
	path := "/api/operations/v1/commercial/quotation-orders"
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("POST", path, body))
	if response.Code != 403 || store.calls != 0 {
		t.Fatal("missing order permission admitted", response.Code)
	}
	store.allowed[application.PermissionCommercialOrderWrite] = true
	for _, mutate := range []func(*http.Request){
		func(r *http.Request) { r.Header.Del("X-CSRF-Token") },
		func(r *http.Request) { r.Header.Set("Origin", "https://untrusted.invalid") },
		func(r *http.Request) { r.Header.Del("Cookie") },
	} {
		r := highRiskRequest("POST", path, body)
		mutate(r)
		response = httptest.NewRecorder()
		handler.ServeHTTP(response, r)
		if response.Code < 400 || store.calls != 0 {
			t.Fatal("mutation auth bypassed", response.Code)
		}
	}
	for _, field := range []string{"environment", "amount_minor", "entitlements", "source", "accepted_at"} {
		var value map[string]any
		_ = json.Unmarshal([]byte(body), &value)
		value[field] = "forged"
		raw, _ := json.Marshal(value)
		response = httptest.NewRecorder()
		handler.ServeHTTP(response, highRiskRequest("POST", path, string(raw)))
		if response.Code != 400 || store.calls != 0 {
			t.Fatal("caller authority admitted", field, response.Code)
		}
	}
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("POST", path, body))
	if response.Code != 201 || store.calls != 1 || store.quotationEnvironment != "production" {
		t.Fatal("trusted configured channel not passed", response.Code, store.calls, store.quotationEnvironment)
	}
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("GET", "/api/operations/v1/commercial/quotation-sources/publication", ""))
	if response.Code != 200 || store.calls != 2 {
		t.Fatal("order permission cannot read minimal source", response.Code)
	}
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("GET", "/api/operations/v1/commercial/publications/publication", ""))
	if response.Code != 403 || store.calls != 2 {
		t.Fatal("order permission leaked publication management read", response.Code)
	}
}

func (s *commercialHandlerStore) HasPermission(_ context.Context, _ string, p string) (bool, error) {
	return s.allowed[p], nil
}
func (s *commercialHandlerStore) FreezePlanVersion(_ context.Context, id string, expected uint32, d commercial.Definition, op, actor string, now time.Time) (commercial.PlanVersionRecord, error) {
	s.calls++
	if s.conflict != nil {
		return commercial.PlanVersionRecord{}, s.conflict
	}
	f, e := commercial.FreezePlan(id, expected+1, d)
	if e != nil {
		return commercial.PlanVersionRecord{}, e
	}
	v, e := f.Snapshot()
	s.plan = commercial.PlanVersionRecord{Snapshot: v, SHA256: f.Digest(), OperationID: op, CreatedBy: actor, CreatedAt: now}
	return s.plan, e
}
func (s *commercialHandlerStore) GetCommercialPlan(context.Context, string, uint32) (commercial.PlanVersionRecord, error) {
	s.calls++
	return s.plan, nil
}
func (s *commercialHandlerStore) GetCurrentCommercialPlan(context.Context, string) (commercial.PlanVersionRecord, error) {
	s.calls++
	return s.plan, nil
}
func (s *commercialHandlerStore) ListCommercialPlans(context.Context, int) ([]commercial.PlanVersionRecord, error) {
	s.calls++
	return []commercial.PlanVersionRecord{s.plan}, nil
}
func (s *commercialHandlerStore) CreateCommercialOrder(_ context.Context, id, op, customer, plan string, version, years uint32, start time.Time, actor string, now time.Time) (commercial.OrderRecord, error) {
	s.calls++
	f, e := commercial.FreezePlan(s.plan.Snapshot.PlanID, s.plan.Snapshot.Version, s.plan.Snapshot.Definition)
	if e != nil {
		return commercial.OrderRecord{}, e
	}
	v, e := commercial.CreateOrderSnapshot(id, customer, f, years, start)
	if e != nil {
		return commercial.OrderRecord{}, e
	}
	raw, e := v.Bytes()
	s.order = commercial.OrderRecord{Snapshot: v, SHA256: commercial.ContentDigest(raw), OperationID: op, Status: "pending_payment", CreatedBy: actor, CreatedAt: now}
	return s.order, e
}
func (s *commercialHandlerStore) GetCommercialOrder(context.Context, string) (commercial.OrderRecord, error) {
	s.calls++
	return s.order, nil
}
func (s *commercialHandlerStore) ListCommercialOrders(_ context.Context, query application.CommercialOrderListQuery) (application.CommercialOrderPage, error) {
	s.calls++
	s.orderListQuery = query
	return application.CommercialOrderPage{Items: []commercial.OrderRecord{s.order}, Total: 1}, nil
}

func TestCommercialHTTPPermissionsAndDerivedOrder(t *testing.T) {
	raw, err := os.ReadFile("../../../../../contracts/test-vectors/plan-definition.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var definition commercial.Definition
	if err = json.Unmarshal(raw, &definition); err != nil {
		t.Fatal(err)
	}
	now := time.Now().UTC()
	store := &commercialHandlerStore{handlerStore: &handlerStore{operator: domain.Operator{ID: "operator_1", Email: "admin@test.invalid", Status: "active"}}, allowed: map[string]bool{}}
	store.session = application.CreateSessionParams{ID: "session_1", OperatorID: "operator_1", TokenHash: sha256.Sum256([]byte("session-token")), CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: now.Add(time.Hour)}
	handler := New(application.NewService(store, time.Hour), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 1 << 20})
	planBody, _ := json.Marshal(application.FreezeCommercialPlanInput{OperationID: "freeze_1", Definition: definition})
	orderBody := `{"operation_id":"order_1","customer_id":"customer_1","plan_id":"plan_1","plan_version":1,"years":3,"starts_at":"2026-09-06T00:00:00.000Z"}`
	base := "/api/operations/v1/commercial"
	cases := []struct{ method, path, body string }{
		{"POST", base + "/plans/versions", string(planBody)},
		{"POST", base + "/orders", orderBody},
		{"GET", base + "/plans", ""}, {"GET", base + "/plans/plan_1/versions/1", ""},
		{"GET", base + "/plans/plan_1", ""},
		{"GET", base + "/orders", ""}, {"GET", base + "/orders/order_1", ""},
	}
	for _, tc := range cases {
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, highRiskRequest(tc.method, tc.path, tc.body))
		if response.Code != http.StatusForbidden {
			t.Fatalf("%s %s status=%d: %s", tc.method, tc.path, response.Code, response.Body.String())
		}
	}
	if store.calls != 0 {
		t.Fatalf("denied requests touched business storage %d times", store.calls)
	}
	store.allowed[application.PermissionCommercialPlanWrite] = true
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("POST", base+"/plans/versions", string(planBody)))
	if response.Code != 201 {
		t.Fatalf("freeze: %d %s", response.Code, response.Body.String())
	}
	// Plan write does not grant order write, or even plan read.
	for _, tc := range cases[1:] {
		response = httptest.NewRecorder()
		handler.ServeHTTP(response, highRiskRequest(tc.method, tc.path, tc.body))
		if response.Code != 403 {
			t.Fatalf("permission leaked to %s: %d", tc.path, response.Code)
		}
	}
	store.allowed[application.PermissionCommercialOrderWrite] = true
	var orderInput map[string]any
	_ = json.Unmarshal([]byte(orderBody), &orderInput)
	orderInput["plan_id"] = store.plan.Snapshot.PlanID
	body, _ := json.Marshal(orderInput)
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("POST", base+"/orders", string(body)))
	if response.Code != 201 {
		t.Fatalf("create: %d %s", response.Code, response.Body.String())
	}
	if store.order.Snapshot.AmountMinor != 1529745 || store.order.Snapshot.PlanSHA256 != store.plan.SHA256 || store.order.Status != "pending_payment" {
		t.Fatalf("incorrect order: %#v", store.order)
	}
	orderInput["amount_minor"] = 1
	body, _ = json.Marshal(orderInput)
	before := store.calls
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("POST", base+"/orders", string(body)))
	if response.Code != 400 || store.calls != before {
		t.Fatalf("client amount was accepted: %d", response.Code)
	}
	request := highRiskRequest("POST", base+"/plans/versions", string(planBody))
	request.Header.Del("X-CSRF-Token")
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != 403 || store.calls != before {
		t.Fatal("write permission bypassed CSRF")
	}
	store.allowed[application.PermissionCommercialPlanRead] = true
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("GET", base+"/plans/"+store.plan.Snapshot.PlanID, ""))
	var current commercial.PlanVersionRecord
	if response.Code != 200 || json.Unmarshal(response.Body.Bytes(), &current) != nil || current.SHA256 != store.plan.SHA256 {
		t.Fatalf("current snapshot: %d %s", response.Code, response.Body.String())
	}
	store.conflict = commercial.ErrConflict
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("POST", base+"/plans/versions", string(planBody)))
	var failure struct {
		Error struct {
			Code   string `json:"code"`
			Number int    `json:"number"`
		} `json:"error"`
	}
	if response.Code != 409 || json.Unmarshal(response.Body.Bytes(), &failure) != nil || failure.Error.Code != "COMMERCIAL_SNAPSHOT_CONFLICT" || failure.Error.Number != 67702 {
		t.Fatalf("conflict not distinguishable: %d %s", response.Code, response.Body.String())
	}
	store.conflict = commercial.ErrPlanVersionConflict
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("POST", base+"/plans/versions", string(planBody)))
	if response.Code != 409 || json.Unmarshal(response.Body.Bytes(), &failure) != nil || failure.Error.Code != "COMMERCIAL_PLAN_VERSION_CONFLICT" || failure.Error.Number != 67703 {
		t.Fatalf("confirmed version conflict not distinguishable: %d %s", response.Code, response.Body.String())
	}
	store.allowed[application.PermissionCommercialOrderRead] = true
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("GET", base+"/orders?limit=25&offset=50&keyword=%20Acme%20&status=fulfilled&stage=issued", ""))
	if response.Code != http.StatusOK {
		t.Fatalf("list orders: %d %s", response.Code, response.Body.String())
	}
	if store.orderListQuery != (application.CommercialOrderListQuery{Limit: 25, Offset: 50, Keyword: "Acme", Status: "fulfilled", Stage: "issued"}) {
		t.Fatalf("unexpected order list query: %#v", store.orderListQuery)
	}
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("GET", base+"/orders?offset=-1", ""))
	if response.Code != http.StatusBadRequest {
		t.Fatalf("negative offset accepted: %d %s", response.Code, response.Body.String())
	}
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("GET", base+"/orders?stage=unknown", ""))
	if response.Code != http.StatusBadRequest {
		t.Fatalf("unknown order stage accepted: %d %s", response.Code, response.Body.String())
	}
}
