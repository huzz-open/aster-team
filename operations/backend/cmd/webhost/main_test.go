package main

import (
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestSameOriginProxyStaticAssetsAndSPAFallback(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "index.html"), []byte("spa-index"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "asset.js"), []byte("asset-body"), 0o600); err != nil {
		t.Fatal(err)
	}
	backend := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		response.Header().Set("Content-Type", "application/json")
		_, _ = response.Write([]byte(`{"path":"` + request.URL.Path + `","cookie":"` + request.Header.Get("Cookie") + `"}`))
	}))
	defer backend.Close()
	target, _ := url.Parse(backend.URL)
	server := httptest.NewServer(newHandler(root, target, slog.New(slog.NewTextHandler(io.Discard, nil))))
	defer server.Close()

	request, _ := http.NewRequest(http.MethodGet, server.URL+"/api/operations/v1/session", nil)
	request.Header.Set("Cookie", "aster_operations_session=test")
	response, err := http.DefaultClient.Do(request)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(response.Body)
	_ = response.Body.Close()
	if response.StatusCode != http.StatusOK || !strings.Contains(string(body), `"cookie":"aster_operations_session=test"`) {
		t.Fatalf("proxy response = %d %s", response.StatusCode, body)
	}

	assertBody(t, server.URL+"/asset.js", "asset-body")
	assertBody(t, server.URL+"/orders/123", "spa-index")
	if response.Header.Get("X-Frame-Options") != "DENY" {
		t.Fatal("security headers are missing")
	}
}

func TestProxyReturnsStructuredErrorWhenBackendIsUnavailable(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "index.html"), []byte("spa-index"), 0o600); err != nil {
		t.Fatal(err)
	}
	backend := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	target, _ := url.Parse(backend.URL)
	backend.Close()
	response := httptest.NewRecorder()
	newHandler(root, target, slog.New(slog.NewTextHandler(io.Discard, nil))).ServeHTTP(response, httptest.NewRequest(http.MethodGet, "/api/operations/v1/overview", nil))
	if response.Code != http.StatusBadGateway || response.Header().Get("Content-Type") != "application/json; charset=utf-8" ||
		response.Header().Get("X-Aster-Error-Number") == "" || !strings.Contains(response.Body.String(), "API_UNAVAILABLE") || !strings.Contains(response.Body.String(), `"number":`) {
		t.Fatalf("unavailable response = %d %s %s", response.Code, response.Header().Get("Content-Type"), response.Body.String())
	}
}

func assertBody(t *testing.T, address, expected string) {
	t.Helper()
	response, err := http.Get(address)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(response.Body)
	_ = response.Body.Close()
	if response.StatusCode != http.StatusOK || string(body) != expected {
		t.Fatalf("GET %s = %d %q", address, response.StatusCode, body)
	}
}
