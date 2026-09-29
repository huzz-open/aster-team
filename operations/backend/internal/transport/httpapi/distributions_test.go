package httpapi

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"golang.org/x/crypto/bcrypt"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/domain"
)

func TestFreeDistributionHTTPRequiresIndependentPermissions(t *testing.T) {
	now := time.Now().UTC()
	hash, err := bcrypt.GenerateFromPassword([]byte("current-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatal(err)
	}
	store := &commercialHandlerStore{handlerStore: &handlerStore{operator: domain.Operator{ID: "operator_1", Email: "admin@test.invalid", Status: "active"}, passwordHash: string(hash)}, allowed: map[string]bool{application.PermissionCommercialPlanWrite: true}}
	store.session = application.CreateSessionParams{ID: "session_1", OperatorID: "operator_1", TokenHash: sha256.Sum256([]byte("session-token")), CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: now.Add(time.Hour)}
	handler := New(application.NewService(store, time.Hour), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 1 << 20})
	input := commercial.ApproveDistributionInput{OperationID: "approval_test", PlanID: "free_plan", PlanVersion: 1, ExpectedSHA256: strings.Repeat("0", 64), NotBefore: now.Truncate(time.Millisecond), Reason: "test"}
	body, _ := json.Marshal(approveDistributionInput{approvalDistributionFields: approvalDistributionFields(input), CurrentPassword: "current-password"})
	for _, tc := range []struct{ method, path, body string }{
		{"GET", "/commercial/distributions", ""},
		{"POST", "/commercial/distributions", string(body)},
		{"GET", "/commercial/distributions/dist_test", ""},
		{"POST", "/commercial/distributions/dist_test/issue", `{"key_id":"test-key","current_password":"current-password"}`},
		{"GET", "/commercial/distributions/dist_test/download", ""},
		{"GET", "/commercial/issuers", ""},
		{"GET", "/commercial/environment", ""},
	} {
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, highRiskRequest(tc.method, "/api/operations/v1"+tc.path, tc.body))
		if response.Code != http.StatusForbidden {
			t.Fatalf("%s %s: %d %s", tc.method, tc.path, response.Code, response.Body.String())
		}
	}
	if store.calls != 0 {
		t.Fatal("permission denial accessed business state")
	}
	store.allowed[application.PermissionDistributionApprove] = true
	request := highRiskRequest("POST", "/api/operations/v1/commercial/distributions", string(body))
	request.Header.Del("X-CSRF-Token")
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != 403 {
		t.Fatal("approval ignored CSRF")
	}
	var forged map[string]any
	_ = json.Unmarshal(body, &forged)
	forged["status"] = "issued"
	extra, _ := json.Marshal(forged)
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("POST", "/api/operations/v1/commercial/distributions", string(extra)))
	if response.Code != 400 {
		t.Fatalf("client supplied approval status: %d", response.Code)
	}
	store.allowed[application.PermissionLicenseIssueV2] = true
	for _, environment := range []string{"", "local", "production"} {
		environmentHandler := New(application.NewService(store, time.Hour, application.WithFulfillmentEnvironment(environment)), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{})
		response = httptest.NewRecorder()
		environmentHandler.ServeHTTP(response, highRiskRequest("GET", "/api/operations/v1/commercial/environment", ""))
		var value map[string]string
		if response.Code != http.StatusOK || json.Unmarshal(response.Body.Bytes(), &value) != nil || len(value) != 1 || value["fulfillment_environment"] != environment {
			t.Fatalf("trusted environment missing or expanded: %d %s", response.Code, response.Body.String())
		}
		response = httptest.NewRecorder()
		environmentHandler.ServeHTTP(response, httptest.NewRequest("GET", "/api/operations/v1/commercial/environment", nil))
		if response.Code != http.StatusUnauthorized {
			t.Fatal("environment disclosed without a session")
		}
	}
	// Both rights are present. Session + CSRF must still not authorize a
	// high-risk operation without a freshly verified password.
	for _, path := range []string{"/commercial/distributions", "/commercial/distributions/dist_test/issue"} {
		for _, password := range []string{"", "wrong"} {
			var payload []byte
			if strings.HasSuffix(path, "/issue") {
				payload, _ = json.Marshal(issueDistributionInput{KeyID: "unconfigured", CurrentPassword: password})
			} else {
				payload, _ = json.Marshal(approveDistributionInput{approvalDistributionFields: approvalDistributionFields(input), CurrentPassword: password})
			}
			response = httptest.NewRecorder()
			handler.ServeHTTP(response, highRiskRequest("POST", "/api/operations/v1"+path, string(payload)))
			if response.Code != http.StatusUnauthorized || !strings.Contains(response.Body.String(), "REAUTHENTICATION_FAILED") {
				t.Fatalf("%s missing/incorrect password: %d %s", path, response.Code, response.Body.String())
			}
		}
	}
	if store.calls != 0 {
		t.Fatal("reauthentication failure accessed business state")
	}
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("POST", "/api/operations/v1/commercial/distributions", string(body)))
	if response.Code != http.StatusServiceUnavailable {
		t.Fatalf("valid approval authentication did not reach service: %d %s", response.Code, response.Body.String())
	}
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, highRiskRequest("POST", "/api/operations/v1/commercial/distributions/dist_test/issue", `{"key_id":"unconfigured","current_password":"current-password"}`))
	if response.Code != 503 || !strings.Contains(response.Body.String(), "BUSINESS_STORE_UNAVAILABLE") {
		t.Fatalf("issuance must read persisted state before deciding whether a signer is needed: %d %s", response.Code, response.Body.String())
	}
}

func TestDistributionReauthenticationFieldsAreStrictAndNotPersisted(t *testing.T) {
	base := `{"operation_id":"approval_test","plan_id":"free_plan","plan_version":1,"expected_sha256":"` + strings.Repeat("0", 64) + `","not_before":"2026-09-06T00:00:00.000Z","reason":"test","current_password":"transient-secret"}`
	var in approveDistributionInput
	if err := json.Unmarshal([]byte(base), &in); err != nil {
		t.Fatal(err)
	}
	if in.CurrentPassword != "transient-secret" || in.OperationID != "approval_test" || in.PlanVersion != 1 {
		t.Fatal("transport did not decode the authentication and business fields")
	}
	business, err := json.Marshal(commercial.ApproveDistributionInput(in.approvalDistributionFields))
	if err != nil || strings.Contains(string(business), "password") || strings.Contains(string(business), "transient-secret") {
		t.Fatalf("business input contains authentication data: %v", err)
	}
	for _, invalid := range []string{
		strings.Replace(base, `,"current_password":"transient-secret"`, "", 1),
		strings.Replace(base, `"current_password"`, `"Current_Password"`, 1),
		strings.Replace(base, `"transient-secret"`, `null`, 1),
		strings.Replace(base, `"transient-secret"`, `"transient-secret","current_password":"duplicate"`, 1),
	} {
		if err := json.Unmarshal([]byte(invalid), &in); err == nil {
			t.Fatal("accepted ambiguous authentication payload")
		}
	}
}
