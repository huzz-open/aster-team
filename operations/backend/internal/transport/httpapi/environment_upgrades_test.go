package httpapi

import (
	"context"
	"crypto/sha256"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/domain"
)

func TestEnvironmentMutationRequiresNewPermissionAndCSRF(t *testing.T) {
	store := &handlerStore{operator: domain.Operator{ID: "operator", Status: "active"}, releaseRead: true}
	store.session = application.CreateSessionParams{ID: "session", OperatorID: "operator", TokenHash: sha256.Sum256([]byte("session-token")), CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: time.Now().Add(time.Hour)}
	handler := New(application.NewService(store, time.Hour), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{BodyMaxSize: 4096, TrustedOrigins: map[string]struct{}{"https://operations.test": {}}})
	for _, path := range []string{"/upgrade-environments", "/environment-upgrades", "/upgrade-environments/env_1/inspect"} {
		for _, csrf := range []string{"", "csrf-token"} {
			r := httptest.NewRequest(http.MethodPost, "/api/operations/v1"+path, strings.NewReader(`{}`))
			r.Header.Set("Origin", "https://operations.test")
			r.Header.Set("Content-Type", "application/json")
			r.Header.Set("X-CSRF-Token", csrf)
			r.AddCookie(&http.Cookie{Name: sessionCookieName, Value: "session-token"})
			w := httptest.NewRecorder()
			handler.ServeHTTP(w, r)
			if w.Code != http.StatusForbidden {
				t.Fatalf("%s accepted existing build permission or missing CSRF: %d %s", path, w.Code, w.Body.String())
			}
		}
	}
	r := httptest.NewRequest(http.MethodGet, "/api/operations/v1/environment-upgrades", nil)
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, r)
	if w.Code != 401 {
		t.Fatal("anonymous read allowed")
	}
}
