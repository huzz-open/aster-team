package githubactions

import (
	"context"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/config"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

func TestGitHubAppPreflightDispatchAndSnapshot(t *testing.T) {
	fixedNow := time.Date(2026, 8, 28, 12, 0, 0, 0, time.UTC)
	commitSHA := strings.Repeat("a", 40)
	tokenRequests := 0
	server := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		if request.Header.Get("X-GitHub-Api-Version") != apiVersion {
			t.Errorf("API version = %q", request.Header.Get("X-GitHub-Api-Version"))
		}
		if request.URL.Path == "/app/installations/2/access_tokens" {
			tokenRequests++
			if parts := strings.Split(strings.TrimPrefix(request.Header.Get("Authorization"), "Bearer "), "."); len(parts) != 3 {
				t.Errorf("installation token request did not use a signed app JWT")
			}
			writeTestJSON(response, map[string]any{"token": "ghs_installation_token_value", "expires_at": fixedNow.Add(time.Hour)})
			return
		}
		if request.Header.Get("Authorization") != "Bearer ghs_installation_token_value" {
			t.Errorf("repository request authorization was not the installation token")
		}
		switch request.URL.Path {
		case "/repos/huzz-max/aster-team/commits/main":
			writeTestJSON(response, map[string]any{"sha": commitSHA})
		case "/repos/huzz-max/aster-team/contents/package.json":
			if request.URL.Query().Get("ref") != commitSHA {
				t.Errorf("package ref = %q", request.URL.Query().Get("ref"))
			}
			writeTestJSON(response, map[string]any{"encoding": "base64", "content": base64.StdEncoding.EncodeToString([]byte(`{"version":"2.0.0"}`))})
		case "/repos/huzz-max/aster-team/actions/workflows/customer-release.yml":
			writeTestJSON(response, map[string]any{"state": "active"})
		case "/repos/huzz-max/aster-team/environments/customer-release/secrets/ASTER_RELEASE_SIGNING_SEED_BASE64",
			"/repos/huzz-max/aster-team/environments/customer-release/variables/ASTER_LICENSE_TRUSTED_KEYS_JSON",
			"/repos/huzz-max/aster-team/environments/customer-release/variables/ASTER_RELEASE_TRUSTED_KEYS_JSON",
			"/repos/huzz-max/aster-team/environments/customer-release/variables/ASTER_RELEASE_SIGNING_KEY_ID",
			"/repos/huzz-max/aster-team/environments/customer-release/variables/ASTER_CUSTOMER_FREE_LICENSE_BASE64",
			"/repos/huzz-max/aster-team/environments/customer-release/variables/ASTER_CUSTOMER_FREE_LICENSE_SHA256":
			writeTestJSON(response, map[string]any{})
		case "/repos/huzz-max/aster-team/actions/workflows/customer-release.yml/dispatches":
			if request.Method != http.MethodPost {
				t.Errorf("dispatch method = %s", request.Method)
			}
			var body struct {
				Ref    string            `json:"ref"`
				Inputs map[string]string `json:"inputs"`
			}
			_ = json.NewDecoder(request.Body).Decode(&body)
			freeLicense := []byte(`{"signed":"free"}`)
			freeLicenseDigest := fmt.Sprintf("%x", sha256.Sum256(freeLicense))
			if body.Ref != "main" || body.Inputs["release_task_id"] != "release_task_1" || body.Inputs["version"] != "2.0.0" || body.Inputs["source_commit_sha"] != commitSHA ||
				body.Inputs["free_distribution_id"] != "dist_free_1" || body.Inputs["free_license_sha256"] != freeLicenseDigest || body.Inputs["free_license_base64"] != base64.StdEncoding.EncodeToString(freeLicense) {
				t.Errorf("unexpected dispatch body: %#v", body)
			}
			writeTestJSON(response, map[string]any{"workflow_run_id": 321, "run_url": "https://api.github.test/runs/321", "html_url": "https://github.test/runs/321"})
		case "/repos/huzz-max/aster-team/actions/runs/321":
			writeTestJSON(response, map[string]any{"id": 321, "run_number": 44, "run_attempt": 1, "name": "Release Rust-native customer package",
				"display_title": "Customer 2.0.0 · release_task_1 · " + commitSHA, "head_branch": "main", "head_sha": commitSHA,
				"status": "completed", "conclusion": "failure", "html_url": "https://github.test/runs/321",
				"run_started_at": fixedNow, "created_at": fixedNow, "updated_at": fixedNow.Add(time.Minute)})
		case "/repos/huzz-max/aster-team/actions/runs/321/jobs":
			writeTestJSON(response, map[string]any{"jobs": []any{map[string]any{"id": 654, "name": "Verify customer install (ubuntu-24.04)",
				"runner_name": "GitHub Actions 1", "status": "completed", "conclusion": "failure", "html_url": "https://github.test/jobs/654",
				"started_at": fixedNow, "completed_at": fixedNow.Add(time.Minute), "steps": []any{map[string]any{"name": "Exercise customer install",
					"status": "completed", "conclusion": "failure", "number": 4, "started_at": fixedNow, "completed_at": fixedNow.Add(time.Minute)}}}}})
		case "/repos/huzz-max/aster-team/actions/jobs/654/logs":
			_, _ = response.Write([]byte("install failed\nAuthorization: Bearer top-secret\npassword=hunter2\nghp_abcdefghijklmnopqrstuv\n"))
		case "/repos/huzz-max/aster-team/actions/runs/321/artifacts":
			writeTestJSON(response, map[string]any{"artifacts": []any{map[string]any{"id": 987, "name": "customer-install-diagnostics-321",
				"size_in_bytes": 42, "expired": false, "created_at": fixedNow, "expires_at": fixedNow.Add(7 * 24 * time.Hour),
				"digest": "sha256:" + strings.Repeat("b", 64), "archive_download_url": "https://api.github.test/artifacts/987/zip"}, map[string]any{"id": 988, "name": "customer-linux-amd64-2.0.0"}, map[string]any{"id": 989, "name": "customer-windows-amd64-2.0.0"}}})
		default:
			http.Error(response, fmt.Sprintf("unexpected %s %s", request.Method, request.URL.String()), http.StatusNotFound)
		}
	}))
	defer server.Close()

	client := newTestClient(t, server.URL, fixedNow)
	preflight, err := client.Preflight(context.Background(), "2.0.0", "main")
	if err != nil || preflight.CommitSHA != commitSHA {
		t.Fatalf("Preflight() = %#v, %v", preflight, err)
	}
	freeLicense := []byte(`{"signed":"free"}`)
	freeLicenseDigest := fmt.Sprintf("%x", sha256.Sum256(freeLicense))
	dispatch, err := client.Dispatch(context.Background(), ports.ReleaseDispatchInput{TaskID: "release_task_1", Version: "2.0.0", SourceRef: "main", SourceCommitSHA: commitSHA,
		FreeDistributionID: "dist_free_1", FreeLicenseSHA256: freeLicenseDigest, FreeLicenseDocument: freeLicense})
	if err != nil || dispatch.RunID != 321 || dispatch.RunURL != "https://github.test/runs/321" {
		t.Fatalf("Dispatch() = %#v, %v", dispatch, err)
	}
	snapshot, err := client.Snapshot(context.Background(), domain.ReleaseTask{ID: "release_task_1", Version: "2.0.0", SourceRef: "main", SourceCommitSHA: commitSHA}, 321)
	if err != nil {
		t.Fatal(err)
	}
	if len(snapshot.Run.Jobs) != 1 || len(snapshot.Run.Jobs[0].Steps) != 1 || len(snapshot.Artifacts) != 3 {
		t.Fatalf("unexpected snapshot hierarchy: %#v", snapshot)
	}
	summary := snapshot.Run.Jobs[0].Steps[0].FailureSummary
	if !strings.Contains(summary, "install failed") || strings.Contains(summary, "top-secret") || strings.Contains(summary, "hunter2") || strings.Contains(summary, "ghp_") {
		t.Fatalf("failure summary was not safely redacted: %q", summary)
	}
	if snapshot.Artifacts[0].GitHubDigestSHA256 == nil || *snapshot.Artifacts[0].GitHubDigestSHA256 != strings.Repeat("b", 64) || snapshot.Artifacts[0].SHA256 != nil {
		t.Fatalf("artifact digest was not retained: %#v", snapshot.Artifacts[0])
	}
	if snapshot.Artifacts[0].Platform != "" || snapshot.Artifacts[1].Platform != "linux" || snapshot.Artifacts[2].Platform != "windows" || snapshot.Artifacts[2].FileName != "aster-team-2.0.0-windows-amd64.tar.gz" {
		t.Fatalf("wrong package targets: %#v", snapshot.Artifacts)
	}
	if tokenRequests != 1 {
		t.Fatalf("installation token requests = %d, want cached single request", tokenRequests)
	}
}

func TestPreflightRejectsVersionBeforeCallingGitHub(t *testing.T) {
	client := newTestClient(t, "https://api.github.invalid", time.Now().UTC())
	for _, version := range []string{"latest", "v2.0.0", "01.2.3", "1.0.0-alpha.01"} {
		if _, err := client.Preflight(context.Background(), version, "main"); err == nil {
			t.Fatalf("Preflight() accepted non-SemVer version %q", version)
		}
	}
}

func TestDownloadArtifactUsesIndependentTimeout(t *testing.T) {
	fixedNow := time.Date(2026, 8, 29, 0, 0, 0, 0, time.UTC)
	server := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		switch request.URL.Path {
		case "/app/installations/2/access_tokens":
			writeTestJSON(response, map[string]any{"token": "ghs_installation_token_value", "expires_at": fixedNow.Add(time.Hour)})
		case "/repos/huzz-max/aster-team/actions/artifacts/987/zip":
			_, _ = response.Write([]byte("first"))
			response.(http.Flusher).Flush()
			time.Sleep(75 * time.Millisecond)
			_, _ = response.Write([]byte("second"))
		default:
			http.NotFound(response, request)
		}
	}))
	defer server.Close()

	privateKey, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	pemBytes := pem.EncodeToMemory(&pem.Block{Type: "RSA PRIVATE KEY", Bytes: x509.MarshalPKCS1PrivateKey(privateKey)})
	client, err := New(config.GitHubApp{Enabled: true, APIBaseURL: server.URL, AppID: 1, InstallationID: 2,
		PrivateKeyPEMBase64: base64.StdEncoding.EncodeToString(pemBytes), Repository: "huzz-max/aster-team",
		RequestTimeout: 25 * time.Millisecond, ArtifactDownloadTimeout: time.Second})
	if err != nil {
		t.Fatal(err)
	}
	client.now = func() time.Time { return fixedNow }

	body, err := client.DownloadArtifact(context.Background(), 987)
	if err != nil {
		t.Fatal(err)
	}
	defer body.Close()
	contents, err := io.ReadAll(body)
	if err != nil {
		t.Fatalf("artifact body was constrained by the API timeout: %v", err)
	}
	if string(contents) != "firstsecond" {
		t.Fatalf("artifact contents = %q", contents)
	}
}

func newTestClient(t *testing.T, apiURL string, now time.Time) *Client {
	t.Helper()
	privateKey, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	pemBytes := pem.EncodeToMemory(&pem.Block{Type: "RSA PRIVATE KEY", Bytes: x509.MarshalPKCS1PrivateKey(privateKey)})
	client, err := New(config.GitHubApp{Enabled: true, APIBaseURL: apiURL, AppID: 1, InstallationID: 2,
		PrivateKeyPEMBase64: base64.StdEncoding.EncodeToString(pemBytes), Repository: "huzz-max/aster-team",
		WorkflowFile: "customer-release.yml", Environment: "customer-release", DefaultSourceRef: "main", RequestTimeout: 5 * time.Second,
		ArtifactDownloadTimeout: 5 * time.Minute})
	if err != nil {
		t.Fatal(err)
	}
	client.now = func() time.Time { return now }
	return client
}

func writeTestJSON(response http.ResponseWriter, body any) {
	response.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(response).Encode(body)
}
