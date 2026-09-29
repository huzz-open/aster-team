package httpapi

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"io"
	"log/slog"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/domain"
	"golang.org/x/crypto/bcrypt"
)

func TestPublicCatalogHTTPPermissionsReauthenticationAndStrictInputs(t *testing.T) {
	hash, err := bcrypt.GenerateFromPassword([]byte("test-current-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatal(err)
	}
	store := &commercialHandlerStore{handlerStore: &handlerStore{operator: domain.Operator{ID: "operator_1", Email: "admin@test.invalid", Status: "active"}, passwordHash: string(hash)}, allowed: map[string]bool{}}
	store.session = application.CreateSessionParams{ID: "session_1", OperatorID: "operator_1", TokenHash: sha256.Sum256([]byte("session-token")), CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: time.Now().Add(time.Hour)}
	handler := New(application.NewService(store, time.Hour), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 1 << 20})
	base := "/api/operations/v1/commercial/catalogs"
	id := "catalog_" + strings.Repeat("a", 48)
	request := commercial.CatalogRequest{OperationID: "preview_1", Environment: "local", Reason: "fixture", Plans: []commercial.CatalogSelection{}}
	preview, _ := json.Marshal(request)
	input := approveCatalogRequest{approveCatalogFields: approveCatalogFields(commercial.ApproveCatalogInput{Request: request, ExpectedPublicSHA256: strings.Repeat("a", 64)}), CurrentPassword: "test-current-password"}
	approval, _ := json.Marshal(input)
	export := `{"current_password":"test-current-password"}`
	cases := []struct{ method, path, body string }{{"POST", base + "/preview", string(preview)}, {"POST", base, string(approval)}, {"GET", base, ""}, {"GET", base + "/" + id, ""}, {"POST", base + "/" + id + "/export", export}, {"GET", base + "/" + id + "/download", ""}}
	for _, tc := range cases {
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, highRiskRequest(tc.method, tc.path, tc.body))
		if w.Code != 403 || store.calls != 0 {
			t.Fatalf("permission bypass: %s %d %s", tc.path, w.Code, w.Body.String())
		}
	}
	for _, p := range []string{application.PermissionCatalogRead, application.PermissionCatalogApprove, application.PermissionCatalogExport} {
		store.allowed[p] = true
	}
	for _, tc := range []struct{ path, body string }{{base, string(approval)}, {base + "/" + id + "/export", export}} {
		for _, password := range []string{"", "wrong"} {
			body := strings.ReplaceAll(tc.body, "test-current-password", password)
			w := httptest.NewRecorder()
			handler.ServeHTTP(w, highRiskRequest("POST", tc.path, body))
			if w.Code != 401 || store.calls != 0 {
				t.Fatalf("reauthentication bypass: %d %s", w.Code, w.Body.String())
			}
		}
	}
	for _, tc := range cases {
		if tc.method != "POST" {
			continue
		}
		r := highRiskRequest(tc.method, tc.path, tc.body)
		r.Header.Del("X-CSRF-Token")
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, r)
		if w.Code != 403 || store.calls != 0 {
			t.Fatal("CSRF bypass")
		}
	}
	for _, extra := range []string{`"path":"../escape"`, `"environment":"production"`, `"public":{}`, `"current_password":"duplicate"`} {
		body := strings.TrimSuffix(export, "}") + "," + extra + "}"
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, highRiskRequest("POST", base+"/"+id+"/export", body))
		if w.Code != 400 {
			t.Fatalf("export accepted caller data: %d %s", w.Code, w.Body.String())
		}
	}
	for _, body := range []string{strings.Replace(string(approval), `"current_password"`, `"Current_Password"`, 1), strings.Replace(string(approval), `"environment":"local"`, `"environment":"local","environment":"production"`, 1), strings.Replace(string(approval), `"plans":[]`, `"plans":null`, 1)} {
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, highRiskRequest("POST", base, body))
		if w.Code != 400 {
			t.Fatalf("ambiguous approval: %d", w.Code)
		}
	}
	var decoded approveCatalogRequest
	if err := json.Unmarshal(approval, &decoded); err != nil {
		t.Fatal(err)
	}
	business, err := json.Marshal(commercial.ApproveCatalogInput(decoded.approveCatalogFields))
	if err != nil || strings.Contains(string(business), "password") {
		t.Fatal("authentication leaked into business payload", err)
	}
	for _, tc := range cases {
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, highRiskRequest(tc.method, tc.path, tc.body))
		if w.Code != 503 {
			t.Fatalf("valid auth did not reach unavailable test store: %s %d %s", tc.path, w.Code, w.Body.String())
		}
	}
}
