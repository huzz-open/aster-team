package main

import (
	"errors"
	"flag"
	"log/slog"
	"net/http"
	"net/http/httputil"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/apierrors"
)

func main() {
	address := flag.String("addr", "127.0.0.1:5173", "listen address")
	directory := flag.String("dir", "", "built SPA directory")
	apiURL := flag.String("api", "http://127.0.0.1:8080", "control API URL")
	flag.Parse()
	logger := slog.New(slog.NewTextHandler(os.Stdout, nil))
	root, err := filepath.Abs(*directory)
	if err != nil || *directory == "" {
		logger.Error("invalid static directory", "error", err)
		os.Exit(1)
	}
	if info, statErr := os.Stat(filepath.Join(root, "index.html")); statErr != nil || info.IsDir() {
		logger.Error("built frontend is missing", "directory", root)
		os.Exit(1)
	}
	target, err := url.Parse(*apiURL)
	if err != nil || target.Scheme == "" || target.Host == "" {
		logger.Error("invalid API URL", "url", *apiURL)
		os.Exit(1)
	}
	handler := newHandler(root, target, logger)
	server := &http.Server{Addr: *address, Handler: handler, ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 90 * time.Second}
	logger.Info("Aster Team web entry listening", "address", "http://"+*address, "directory", root)
	if err := server.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		logger.Error("web entry stopped", "error", err)
		os.Exit(1)
	}
}

func newHandler(root string, target *url.URL, logger *slog.Logger) http.Handler {
	proxy := httputil.NewSingleHostReverseProxy(target)
	proxy.ErrorHandler = func(w http.ResponseWriter, r *http.Request, proxyErr error) {
		number := apierrors.Write(w, http.StatusBadGateway, "API_UNAVAILABLE", "控制服务暂时不可用")
		logger.Warn("API proxy failed", "path", r.URL.Path, "code", "API_UNAVAILABLE", "number", number, "error", proxyErr)
	}
	files := http.FileServer(http.Dir(root))
	handler := securityHeaders(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/health" || r.URL.Path == "/api" || strings.HasPrefix(r.URL.Path, "/api/") || r.URL.Path == "/v1" || strings.HasPrefix(r.URL.Path, "/v1/") {
			proxy.ServeHTTP(w, r)
			return
		}
		clean := filepath.Clean(strings.TrimPrefix(r.URL.Path, "/"))
		candidate := filepath.Join(root, clean)
		if relative, relativeErr := filepath.Rel(root, candidate); relativeErr == nil && relative != ".." && !strings.HasPrefix(relative, ".."+string(filepath.Separator)) {
			if info, statErr := os.Stat(candidate); statErr == nil && !info.IsDir() {
				files.ServeHTTP(w, r)
				return
			}
		}
		http.ServeFile(w, r, filepath.Join(root, "index.html"))
	}))
	return handler
}

func securityHeaders(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("X-Frame-Options", "DENY")
		w.Header().Set("Referrer-Policy", "same-origin")
		w.Header().Set("Permissions-Policy", "camera=(), microphone=(), geolocation=()")
		next.ServeHTTP(w, r)
	})
}
