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
	"aster.local/team/operations/backend/internal/domain"
	"golang.org/x/crypto/bcrypt"
)

func TestPublicationHTTPKeepsEvidenceAndTargetOutOfCallerControl(t *testing.T) {
	hash, err := bcrypt.GenerateFromPassword([]byte("test-current-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatal(err)
	}
	store := &commercialHandlerStore{handlerStore: &handlerStore{operator: domain.Operator{ID: "operator_1", Email: "admin@test.invalid", Status: "active"}, passwordHash: string(hash)}, allowed: map[string]bool{}}
	store.session = application.CreateSessionParams{ID: "session_1", OperatorID: "operator_1", TokenHash: sha256.Sum256([]byte("session-token")), CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: time.Now().Add(time.Hour)}
	handler := New(application.NewService(store, time.Hour), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 1 << 20})
	base := "/api/operations/v1/commercial/publications"
	prepare := `{"operation_id":"test","catalog_revision":"catalog_` + strings.Repeat("a", 48) + `","build_sha256":"` + strings.Repeat("b", 64) + `","expected_active_id":"","accept_until":"2027-01-01T00:00:00.000Z","reason":"isolated","current_password":"test-current-password"}`
	accept := `{"current_password":"test-current-password"}`
	cases := []struct{ method, path, body string }{{"GET", base, ""}, {"GET", base + "/publication_test", ""}, {"GET", base + "/publication_test/failures", ""}, {"GET", "/api/operations/v1/commercial/publication-heads/local", ""}, {"POST", base, prepare}, {"POST", base + "/publication_test/accept", accept}}
	for _, tc := range cases {
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, highRiskRequest(tc.method, tc.path, tc.body))
		if w.Code != 403 || store.calls != 0 {
			t.Fatal("publication permission bypass", tc.path, w.Code)
		}
	}
	for _, permission := range []string{application.PermissionPublicationRead, application.PermissionPublicationPrepare, application.PermissionPublicationAccept} {
		store.allowed[permission] = true
	}
	for _, tc := range cases {
		if tc.method != "POST" {
			continue
		}
		for _, password := range []string{"", "wrong"} {
			w := httptest.NewRecorder()
			handler.ServeHTTP(w, highRiskRequest(tc.method, tc.path, strings.ReplaceAll(tc.body, "test-current-password", password)))
			if w.Code != 401 || store.calls != 0 {
				t.Fatal("publication reauthentication bypass", w.Code)
			}
		}
		r := highRiskRequest(tc.method, tc.path, tc.body)
		r.Header.Del("X-CSRF-Token")
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, r)
		if w.Code != 403 {
			t.Fatal("publication CSRF bypass", w.Code)
		}
		for _, extra := range []string{`"origin":"https://attacker.invalid"`, `"environment":"production"`, `"evidence":{}`, `"status":"accepted"`, `"current_password":"duplicate"`} {
			w := httptest.NewRecorder()
			handler.ServeHTTP(w, highRiskRequest(tc.method, tc.path, strings.TrimSuffix(tc.body, "}")+","+extra+"}"))
			if w.Code != 400 {
				t.Fatal("caller-controlled verification accepted", extra, w.Code)
			}
		}
	}
	var decoded preparePublicationRequest
	if err := json.Unmarshal([]byte(prepare), &decoded); err != nil {
		t.Fatal(err)
	}
	raw, err := json.Marshal(decoded.preparePublicationFields)
	if err != nil || strings.Contains(string(raw), "password") {
		t.Fatal("authentication persisted with business data", err)
	}
}
