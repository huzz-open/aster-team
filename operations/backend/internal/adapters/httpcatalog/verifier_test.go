package httpcatalog

import (
	"context"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

func runtimeFixture(t *testing.T) (commercial.PublicationSnapshot, map[string][]byte, []byte) {
	t.Helper()
	id := "catalog_" + strings.Repeat("a", 48)
	request := commercial.CatalogRequest{OperationID: "test_catalog", Environment: "local", Reason: "test only", Plans: []commercial.CatalogSelection{}}
	preview, err := commercial.BuildPublicCatalog(id, request, []commercial.PlanSnapshot{})
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now().Add(-time.Second).UTC().Truncate(time.Millisecond)
	approval := commercial.CatalogApprovalSnapshot{Schema: commercial.CatalogApprovalSchema, ID: id, Request: request, Plans: []commercial.PlanSnapshot{}, PublicSHA256: preview.SHA256, ApprovedBy: "operator", ApprovedAt: now.Format("2006-01-02T15:04:05.000Z")}
	raw, _ := approval.Bytes()
	record := commercial.CatalogApprovalRecord{Snapshot: approval, SHA256: commercial.ContentDigest(raw), Public: preview, Status: "exported"}
	catalog, err := record.PublicBytes()
	if err != nil {
		t.Fatal(err)
	}
	identity := catalogIdentity{Revision: id, SHA256: preview.SHA256, Environment: "local", Path: "/catalog/" + id + "/plans.json"}
	catalogState, _ := json.Marshal(catalogManifest{Schema: "aster.website-catalog.v1", State: "configured", Catalog: identity})
	files := map[string][]byte{"/index.html": []byte(`<html><head></head><body><script type="module" src="/assets/app.js"></script></body></html>`), "/assets/app.js": []byte("document.body.dataset.release='test'"), "/assets/app.css": []byte("body{color:black}"), "/catalog-manifest.json": catalogState, identity.Path: catalog}
	manifest := runtimeManifest{Schema: "aster.website-release.v1", Catalog: identity, Files: []runtimeFile{}}
	for path, data := range files {
		manifest.Files = append(manifest.Files, runtimeFile{Path: path, SHA256: commercial.ContentDigest(data), SizeBytes: int64(len(data))})
	}
	release, _ := json.Marshal(manifest)
	snapshot := commercial.PublicationSnapshot{Schema: commercial.PublicationSchema, ID: "publication_test", Catalog: approval, CreatedBy: "operator", CreatedAt: approval.ApprovedAt,
		Request: commercial.PreparePublicationInput{OperationID: "test", CatalogRevision: id, BuildSHA256: commercial.ContentDigest(release), AcceptUntil: now.Add(time.Hour).Format("2006-01-02T15:04:05.000Z"), Reason: "test only"}}
	return snapshot, files, release
}

func TestVerifierReadsActualRuntimeAndRejectsMixedFiles(t *testing.T) {
	for _, failure := range []string{"none", "old script", "old stylesheet", "old homepage", "catalog", "missing file", "manifest changed", "redirect"} {
		t.Run(failure, func(t *testing.T) {
			snapshot, files, release := runtimeFixture(t)
			externalCalls := 0
			external := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { externalCalls++; w.WriteHeader(http.StatusOK) }))
			defer external.Close()
			manifestReads := 0
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.Method != http.MethodGet {
					t.Error("verifier attempted mutation", r.Method)
					w.WriteHeader(405)
					return
				}
				if r.Header.Get("Cache-Control") == "" || r.URL.Query().Get("aster_publication_check") == "" {
					t.Error("unversioned cached read")
				}
				if r.URL.Path == "/website-release.json" {
					manifestReads++
					if failure == "manifest changed" && manifestReads > 1 {
						_, _ = w.Write(append(release, ' '))
						return
					}
					_, _ = w.Write(release)
					return
				}
				path := r.URL.Path
				if path == "/" {
					path = "/index.html"
				}
				if failure == "redirect" && path == "/index.html" {
					http.Redirect(w, r, external.URL, 302)
					return
				}
				if failure == "missing file" && path == "/assets/app.js" {
					w.WriteHeader(404)
					return
				}
				data, exists := files[path]
				if !exists {
					w.WriteHeader(404)
					return
				}
				if (failure == "old script" && path == "/assets/app.js") || (failure == "old stylesheet" && path == "/assets/app.css") || (failure == "old homepage" && path == "/index.html") || (failure == "catalog" && strings.HasSuffix(path, "/plans.json")) {
					data = []byte("old runtime")
				}
				_, _ = w.Write(data)
			}))
			defer server.Close()
			verifier, err := New("local", server.URL)
			if err != nil {
				t.Fatal(err)
			}
			evidence, err := verifier.Verify(context.Background(), snapshot)
			if failure != "none" {
				if err == nil {
					t.Fatal("mixed or unverified target accepted", failure)
				}
			} else if err != nil || evidence.CatalogSHA256 != snapshot.Catalog.PublicSHA256 || evidence.Origin != server.URL || manifestReads != 2 {
				t.Fatal("runtime verification failed", evidence, err)
			}
			if externalCalls != 0 {
				t.Fatal("followed redirect outside trusted origin")
			}
		})
	}
}

func TestVerifierPinsDNSAndKeepsEnvironmentBoundary(t *testing.T) {
	for _, origin := range []string{"https://localhost.", "https://test.localhost", "https://127.0.0.1", "https://10.0.0.1", "https://[::1]", "http://public.example", "https://example.com/path", "https://user:password@example.com"} {
		if _, err := New("production", origin); err == nil {
			t.Fatal("invalid production target accepted", origin)
		}
	}
	v, err := New("production", "https://release.example.invalid")
	if err != nil {
		t.Fatal(err)
	}
	if v.client.Transport.(*http.Transport).Proxy != nil {
		t.Fatal("uncontrolled proxy enabled")
	}
	resolved := []net.IPAddr{{IP: net.ParseIP("93.184.216.34")}}
	lookups, dials := 0, 0
	v.lookup = func(context.Context, string) ([]net.IPAddr, error) { lookups++; return resolved, nil }
	v.dial = func(_ context.Context, _ string, address string) (net.Conn, error) {
		dials++
		if address != "93.184.216.34:443" {
			t.Fatal("dial resolved the name again", address)
		}
		return nil, errors.New("isolated dial")
	}
	_, _ = v.dialContext(context.Background(), "tcp", "release.example.invalid:443")
	if lookups != 1 || dials != 1 {
		t.Fatal("did not use pinned public address")
	}
	for _, ip := range []string{"127.0.0.1", "10.0.0.1", "169.254.169.254", "::1", "fc00::1"} {
		resolved = []net.IPAddr{{IP: net.ParseIP(ip)}}
		if _, err := v.dialContext(context.Background(), "tcp", "release.example.invalid:443"); err == nil {
			t.Fatal("rebound target accepted", ip)
		}
		if dials != 1 {
			t.Fatal("private address was dialed", ip)
		}
	}
	resolved = []net.IPAddr{{IP: net.ParseIP("93.184.216.34")}, {IP: net.ParseIP("127.0.0.1")}}
	_, _ = v.dialContext(context.Background(), "tcp", "release.example.invalid:443")
	if dials != 1 {
		t.Fatal("mixed DNS answer dialed")
	}
	if _, err := v.dialContext(context.Background(), "tcp", "another.example.invalid:443"); err == nil {
		t.Fatal("unexpected hostname accepted")
	}
}
