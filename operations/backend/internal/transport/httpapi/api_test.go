package httpapi

import (
	"bytes"
	"context"
	"crypto/sha256"
	"errors"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
	"golang.org/x/crypto/bcrypt"
)

type handlerStore struct {
	operator      domain.Operator
	passwordHash  string
	session       application.CreateSessionParams
	customers     []domain.Customer
	releaseRead   bool
	releaseTasks  []domain.ReleaseTask
	releaseDetail domain.ReleaseTaskDetail
	licenseRecord ports.LicenseRecord
	issuances     []ports.LicenseIssuance
}

func (store *handlerStore) OperatorCount(context.Context) (int64, error) { return 1, nil }
func (*handlerStore) CreateOperator(context.Context, application.CreateOperatorParams) (domain.Operator, error) {
	return domain.Operator{}, errors.New("unexpected CreateOperator call")
}
func (store *handlerStore) FindOperatorForLogin(_ context.Context, email string) (domain.Operator, string, error) {
	if email != store.operator.Email {
		return domain.Operator{}, "", errors.New("not found")
	}
	return store.operator, store.passwordHash, nil
}
func (store *handlerStore) CreateSession(_ context.Context, params application.CreateSessionParams) error {
	store.session = params
	return nil
}
func (store *handlerStore) AuthenticateSession(_ context.Context, tokenHash [32]byte, now time.Time) (domain.AuthenticatedOperator, error) {
	if tokenHash != store.session.TokenHash || !store.session.ExpiresAt.After(now) {
		return domain.AuthenticatedOperator{}, errors.New("not found")
	}
	return domain.AuthenticatedOperator{Operator: store.operator, SessionID: store.session.ID, CSRFHash: store.session.CSRFHash, ExpiresAt: store.session.ExpiresAt}, nil
}
func (store *handlerStore) DeleteSession(_ context.Context, tokenHash [32]byte) error {
	if tokenHash == store.session.TokenHash {
		store.session = application.CreateSessionParams{}
	}
	return nil
}
func (*handlerStore) DeleteExpiredSessions(context.Context, time.Time) error { return nil }
func (store *handlerStore) CreateCustomer(_ context.Context, customer domain.Customer, _ string) error {
	store.customers = append(store.customers, customer)
	return nil
}
func (store *handlerStore) ListCustomers(_ context.Context, _ string, limit int) ([]domain.Customer, error) {
	if limit > len(store.customers) {
		limit = len(store.customers)
	}
	return store.customers[:limit], nil
}
func (store *handlerStore) Overview(context.Context) (domain.Overview, error) {
	return domain.Overview{CustomersTotal: int64(len(store.customers))}, nil
}
func (store *handlerStore) HasPermission(_ context.Context, _ string, permission string) (bool, error) {
	return store.releaseRead && (permission == application.PermissionReleaseRead || permission == application.PermissionReleaseBuild), nil
}
func (store *handlerStore) ListReleaseTasks(_ context.Context, limit int) ([]domain.ReleaseTask, error) {
	if limit > len(store.releaseTasks) {
		limit = len(store.releaseTasks)
	}
	return append([]domain.ReleaseTask(nil), store.releaseTasks[:limit]...), nil
}
func (store *handlerStore) GetReleaseTaskDetail(_ context.Context, taskID string) (domain.ReleaseTaskDetail, error) {
	if store.releaseDetail.Task.ID != taskID {
		return domain.ReleaseTaskDetail{}, application.ErrNotFound
	}
	return store.releaseDetail, nil
}
func (store *handlerStore) CreateReleaseTask(_ context.Context, task domain.ReleaseTask, _ string) error {
	store.releaseTasks = append([]domain.ReleaseTask{task}, store.releaseTasks...)
	store.releaseDetail = domain.ReleaseTaskDetail{Task: task, Runs: []domain.ReleaseRun{}, Artifacts: []domain.ReleaseTaskArtifact{}}
	return nil
}
func (store *handlerStore) RecordReleaseDispatch(_ context.Context, taskID string, run domain.ReleaseRun) error {
	if store.releaseDetail.Task.ID != taskID {
		return application.ErrNotFound
	}
	store.releaseDetail.Runs = []domain.ReleaseRun{run}
	return nil
}
func (*handlerStore) MarkReleaseTaskFailed(context.Context, string, string) error { return nil }
func (store *handlerStore) ApplyReleaseSnapshot(_ context.Context, taskID string, snapshot ports.ReleaseSnapshot, status, phase string, errorCode *string, now time.Time) error {
	store.releaseDetail.Task.Status, store.releaseDetail.Task.Phase, store.releaseDetail.Task.ErrorCode = status, phase, errorCode
	store.releaseDetail.Task.UpdatedAt = now
	store.releaseDetail.Runs = []domain.ReleaseRun{snapshot.Run}
	store.releaseDetail.Artifacts = snapshot.Artifacts
	return nil
}
func (*handlerStore) CompleteReleaseTaskArtifact(context.Context, string, string, domain.ReleaseArtifact, string, time.Time) error {
	return nil
}
func (*handlerStore) FinishReleaseTaskVerification(context.Context, string, *string, time.Time) error {
	return nil
}
func (*handlerStore) FailReleaseTaskArtifact(context.Context, string, string, string, time.Time) error {
	return nil
}
func (store *handlerStore) PrepareReleaseTaskArtifactVerification(_ context.Context, taskID, artifactID, _ string, now time.Time) error {
	if store.releaseDetail.Task.ID != taskID {
		return application.ErrNotFound
	}
	store.releaseDetail.Task.Status = "verifying"
	store.releaseDetail.Task.Phase = "artifact_verification"
	store.releaseDetail.Task.ErrorCode = nil
	store.releaseDetail.Task.UpdatedAt = now
	for index := range store.releaseDetail.Artifacts {
		if store.releaseDetail.Artifacts[index].ID == artifactID {
			store.releaseDetail.Artifacts[index].VerificationStatus = "pending"
			store.releaseDetail.Artifacts[index].VerificationErrorCode = nil
		}
	}
	return nil
}
func (*handlerStore) ListPendingReleaseTaskIDs(context.Context, int) ([]string, error) {
	return nil, nil
}
func (store *handlerStore) ListLicenseRecords(context.Context, int) ([]ports.LicenseRecord, error) {
	return []ports.LicenseRecord{store.licenseRecord}, nil
}
func (store *handlerStore) LicenseForIssuance(_ context.Context, licenseID string) (ports.LicenseRecord, error) {
	if store.licenseRecord.LicenseID != licenseID {
		return ports.LicenseRecord{}, application.ErrNotFound
	}
	return store.licenseRecord, nil
}
func (store *handlerStore) SaveLicenseIssuance(_ context.Context, issuance ports.LicenseIssuance, _ string) error {
	store.issuances = append([]ports.LicenseIssuance{issuance}, store.issuances...)
	return nil
}
func (store *handlerStore) ListLicenseIssuances(context.Context, string) ([]ports.LicenseIssuance, error) {
	return append([]ports.LicenseIssuance(nil), store.issuances...), nil
}

func TestLegacyBusinessRoutesAreNotAvailable(t *testing.T) {
	now := time.Now().UTC()
	store := &handlerStore{operator: domain.Operator{ID: "operator_legacy", Email: "operator@example.test", Status: "active", CreatedAt: now}}
	store.session = application.CreateSessionParams{ID: "session_legacy", OperatorID: store.operator.ID,
		TokenHash: sha256.Sum256([]byte("session-token")), CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: now.Add(time.Hour)}
	handler := New(application.NewService(store, time.Hour), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{
		TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 64 * 1024,
	})
	for _, path := range []string{
		"/licenses", "/licenses/license_legacy/issuances", "/plans", "/plans/plan_legacy/prices",
		"/orders", "/orders/order_legacy/confirm-offline-payment", "/orders/order_legacy/fulfill", "/orders/order_legacy/refund-notes",
		"/trials", "/trials/trial_legacy/fulfill", "/trials/trial_legacy/extend", "/trials/trial_legacy/risk-notes",
		"/deliveries", "/deliveries/delivery_legacy/receipt",
	} {
		for _, method := range []string{http.MethodGet, http.MethodPost} {
			request := highRiskRequest(method, "/api/operations/v1"+path, `{}`)
			response := httptest.NewRecorder()
			handler.ServeHTTP(response, request)
			if response.Code != http.StatusNotFound || len(store.issuances) != 0 {
				t.Fatalf("removed route: %s %s returned %d with %d issuances", method, path, response.Code, len(store.issuances))
			}
		}
	}
}

func TestSessionAndCustomerMutationSecurity(t *testing.T) {
	hash, err := bcrypt.GenerateFromPassword([]byte("long-test-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatal(err)
	}
	store := &handlerStore{
		operator:     domain.Operator{ID: "operator_1", Email: "admin@example.com", DisplayName: "Operator", Status: "active", CreatedAt: time.Now().UTC()},
		passwordHash: string(hash),
	}
	service := application.NewService(store, time.Hour)
	handler := New(service, func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{
		TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 4096,
	})

	request := httptest.NewRequest(http.MethodGet, "/api/operations/v1/overview", nil)
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusUnauthorized {
		t.Fatalf("unauthenticated overview status = %d, want %d", response.Code, http.StatusUnauthorized)
	}
	if response.Header().Get("X-Aster-Error-Number") == "" || !strings.Contains(response.Body.String(), `"number":`) {
		t.Fatalf("unauthenticated overview does not expose a numeric support code: headers=%v body=%s", response.Header(), response.Body.String())
	}

	request = httptest.NewRequest(http.MethodPost, "/api/operations/v1/session", bytes.NewBufferString(`{"email":"admin@example.com","password":"long-test-password"}`))
	request.Header.Set("Content-Type", "text/plain")
	request.Header.Set("Origin", "http://127.0.0.1:12080")
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusBadRequest {
		t.Fatalf("wrong content type status = %d, want %d", response.Code, http.StatusBadRequest)
	}

	request = httptest.NewRequest(http.MethodPost, "/api/operations/v1/session", bytes.NewBufferString(`{"email":"admin@example.com","password":"long-test-password"}`))
	request.Header.Set("Content-Type", "application/json")
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusForbidden {
		t.Fatalf("login without trusted Origin status = %d, want %d", response.Code, http.StatusForbidden)
	}

	request = httptest.NewRequest(http.MethodPost, "/api/operations/v1/session", bytes.NewBufferString(`{"email":"admin@example.com","password":"long-test-password","extra":true}`))
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", "http://127.0.0.1:12080")
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusBadRequest {
		t.Fatalf("login with unknown JSON field status = %d, want %d", response.Code, http.StatusBadRequest)
	}

	request = httptest.NewRequest(http.MethodPost, "/api/operations/v1/session", bytes.NewBufferString(`{"email":"admin@example.com","password":"long-test-password"}`))
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", "http://127.0.0.1:12080")
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusOK {
		t.Fatalf("login status = %d, want %d: %s", response.Code, http.StatusOK, response.Body.String())
	}
	var sessionCookie, csrfCookie *http.Cookie
	for _, cookie := range response.Result().Cookies() {
		switch cookie.Name {
		case sessionCookieName:
			sessionCookie = cookie
		case csrfCookieName:
			csrfCookie = cookie
		}
	}
	if sessionCookie == nil || csrfCookie == nil || !sessionCookie.HttpOnly || csrfCookie.HttpOnly {
		t.Fatal("login did not return the expected session and CSRF cookies")
	}
	if store.session.TokenHash != sha256.Sum256([]byte(sessionCookie.Value)) {
		t.Fatal("session cookie was not persisted as a hash")
	}

	request = newCustomerRequest(sessionCookie, csrfCookie, "")
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusForbidden {
		t.Fatalf("customer mutation without CSRF header status = %d, want %d", response.Code, http.StatusForbidden)
	}

	request = newCustomerRequest(sessionCookie, csrfCookie, csrfCookie.Value)
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusCreated {
		t.Fatalf("customer mutation status = %d, want %d: %s", response.Code, http.StatusCreated, response.Body.String())
	}
	if len(store.customers) != 1 {
		t.Fatalf("created customers = %d, want 1", len(store.customers))
	}
}

func TestOperationsExportRequiresReauthentication(t *testing.T) {
	hash, err := bcrypt.GenerateFromPassword([]byte("current-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatal(err)
	}
	store := &handlerStore{operator: domain.Operator{ID: "operator_1", Email: "admin@example.com", Status: "active", CreatedAt: time.Now().UTC()}, passwordHash: string(hash)}
	store.session = application.CreateSessionParams{ID: "session_1", OperatorID: store.operator.ID, TokenHash: sha256.Sum256([]byte("session-token")), CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: time.Now().Add(time.Hour)}
	service := application.NewService(store, time.Hour)
	handler := New(service, func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 4096})

	request := highRiskRequest(http.MethodPost, "/api/operations/v1/exports", `{"current_password":"wrong"}`)
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusUnauthorized {
		t.Fatalf("export with wrong password status=%d, want 401", response.Code)
	}
}

func TestReleaseCenterReadRequiresPermissionAndReturnsPersistedHierarchy(t *testing.T) {
	now := time.Now().UTC()
	task := domain.ReleaseTask{ID: "release_task_1", Version: "2.0.0", Packages: []domain.ReleaseTaskArtifact{}, Mode: "verification",
		GitHubRepository: "huzz-max/aster-team", WorkflowFile: "customer-release.yml", SourceRef: "main",
		SourceCommitSHA: strings.Repeat("a", 40), Phase: "build", Status: "in_progress", CreatedBy: "operator_1", CreatedAt: now, UpdatedAt: now}
	store := &handlerStore{operator: domain.Operator{ID: "operator_1", Email: "admin@example.com", Status: "active", CreatedAt: now},
		releaseTasks: []domain.ReleaseTask{task}, releaseDetail: domain.ReleaseTaskDetail{Task: task, Runs: []domain.ReleaseRun{}, Artifacts: []domain.ReleaseTaskArtifact{}}}
	store.session = application.CreateSessionParams{ID: "session_1", OperatorID: store.operator.ID, TokenHash: sha256.Sum256([]byte("session-token")),
		CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: now.Add(time.Hour)}
	service := application.NewService(store, time.Hour)
	handler := New(service, func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{BodyMaxSize: 4096})

	request := httptest.NewRequest(http.MethodGet, "/api/operations/v1/release-tasks", nil)
	request.AddCookie(&http.Cookie{Name: sessionCookieName, Value: "session-token"})
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusForbidden || !strings.Contains(response.Body.String(), `"number":67001`) {
		t.Fatalf("release list without permission = %d %s, want fixed 67001", response.Code, response.Body.String())
	}

	store.releaseRead = true
	request = httptest.NewRequest(http.MethodGet, "/api/operations/v1/release-tasks/release_task_1", nil)
	request.AddCookie(&http.Cookie{Name: sessionCookieName, Value: "session-token"})
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusOK || !strings.Contains(response.Body.String(), `"id":"release_task_1"`) || !strings.Contains(response.Body.String(), `"runs":[]`) {
		t.Fatalf("release detail = %d %s", response.Code, response.Body.String())
	}
}

func TestReleaseArtifactReverificationIsQueuedOnExistingTask(t *testing.T) {
	now := time.Now().UTC()
	conclusion := "success"
	digest := strings.Repeat("d", 64)
	code := "RELEASE_PACKAGE_POLICY_FAILED"
	task := domain.ReleaseTask{ID: "release_task_1", Version: "2.0.0", Status: "failed", Phase: "failed", ErrorCode: &code, CreatedAt: now, UpdatedAt: now}
	store := &handlerStore{
		operator:    domain.Operator{ID: "operator_1", Email: "admin@example.com", Status: "active", CreatedAt: now},
		releaseRead: true,
		releaseDetail: domain.ReleaseTaskDetail{Task: task, Runs: []domain.ReleaseRun{{Conclusion: &conclusion}}, Artifacts: []domain.ReleaseTaskArtifact{{
			ID: "artifact_1", Name: "customer-linux-amd64-2.0.0", Platform: "linux", Architecture: "amd64", GitHubArtifactID: 1, GitHubDigestSHA256: &digest,
			VerificationStatus: "failed", VerificationErrorCode: &code,
		}}},
	}
	store.session = application.CreateSessionParams{ID: "session_1", OperatorID: store.operator.ID, TokenHash: sha256.Sum256([]byte("session-token")),
		CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: now.Add(time.Hour)}
	service := application.NewService(store, time.Hour, application.WithReleaseArtifactVerifier(handlerReleaseVerifier{}))
	handler := New(service, func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{
		TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 4096,
	})

	request := highRiskRequest(http.MethodPost, "/api/operations/v1/release-tasks/release_task_1/artifacts/artifact_1/reverify", "")
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != http.StatusAccepted || !strings.Contains(response.Body.String(), `"status":"verifying"`) {
		t.Fatalf("reverify response = %d %s", response.Code, response.Body.String())
	}
}

type handlerReleaseVerifier struct{}

func (handlerReleaseVerifier) Verify(context.Context, domain.ReleaseTask, domain.ReleaseTaskArtifact) (ports.VerifiedReleaseArtifact, error) {
	return ports.VerifiedReleaseArtifact{}, errors.New("not called by the queueing request")
}

func highRiskRequest(method, target, body string) *http.Request {
	request := httptest.NewRequest(method, target, bytes.NewBufferString(body))
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", "http://127.0.0.1:12080")
	request.Header.Set("X-CSRF-Token", "csrf-token")
	request.AddCookie(&http.Cookie{Name: sessionCookieName, Value: "session-token"})
	request.AddCookie(&http.Cookie{Name: csrfCookieName, Value: "csrf-token"})
	return request
}

func newCustomerRequest(sessionCookie, csrfCookie *http.Cookie, csrfHeader string) *http.Request {
	request := httptest.NewRequest(http.MethodPost, "/api/operations/v1/customers", bytes.NewBufferString(`{"name":"Example Customer","status":"lead"}`))
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", "http://127.0.0.1:12080")
	if csrfHeader != "" {
		request.Header.Set("X-CSRF-Token", csrfHeader)
	}
	request.AddCookie(sessionCookie)
	request.AddCookie(csrfCookie)
	return request
}
