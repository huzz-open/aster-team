package upgradetarget

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"

	"aster.local/team/operations/backend/internal/domain"
)

func TestCredentialEnvelopeIsAuthenticatedAndBoundToEnvironment(t *testing.T) {
	s, err := NewSecrets(base64.StdEncoding.EncodeToString(make([]byte, 32)))
	if err != nil {
		t.Fatal(err)
	}
	credentials := domain.UpgradeCredentials{AdminPassword: "private-password", APIKey: "private-key"}
	sealed, err := s.Seal("env_1", credentials)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(sealed, "private") {
		t.Fatal("plaintext persisted")
	}
	actual, err := s.Open("env_1", sealed)
	if err != nil || actual != credentials {
		t.Fatal("credential roundtrip failed")
	}
	if _, err = s.Open("env_2", sealed); err == nil {
		t.Fatal("envelope moved between environments")
	}
	data, _ := base64.StdEncoding.DecodeString(sealed)
	data[len(data)-1] ^= 1
	if _, err = s.Open("env_1", base64.StdEncoding.EncodeToString(data)); err == nil {
		t.Fatal("tampered envelope accepted")
	}
}

func TestStreamRequiresModelOutputAndProtocolCompletion(t *testing.T) {
	valid := "data: {\"choices\":[{\"delta\":{\"content\":\"1\"}}]}\n\ndata: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
	for _, tc := range []struct {
		name, body string
		ok         bool
	}{
		{"complete", valid, true},
		{"truncated", strings.ReplaceAll(valid, "data: [DONE]\n\n", ""), false},
		{"heartbeat", ": ping\n\ndata: [DONE]\n\n", false},
		{"error", "data: {\"error\":{\"message\":\"private\"}}\n\n" + valid, false},
		{"missing_finish", "data: {\"choices\":[{\"delta\":{\"content\":\"1\"}}]}\n\ndata: [DONE]\n\n", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			_, ok := inspectStream(strings.NewReader(tc.body))
			if ok != tc.ok {
				t.Fatalf("result %v", ok)
			}
		})
	}
}

func TestNormalAuthenticationAndCorrelatedUpload(t *testing.T) {
	var uploads atomic.Int32
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/api/admin/auth/login":
			var input map[string]string
			json.NewDecoder(r.Body).Decode(&input)
			if input["email"] != "owner@example.com" || input["password"] != "secret" {
				t.Error("wrong credentials")
			}
			http.SetCookie(w, &http.Cookie{Name: "aster_admin_session", Value: "session", Path: "/"})
			io.WriteString(w, `{"ok":true,"password_change_required":false}`)
		case "/api/admin/maintenance":
			if _, err := r.Cookie("aster_admin_session"); err != nil {
				w.WriteHeader(401)
				return
			}
			if r.URL.Query().Get("request_id") != "upgrade_123456789" {
				t.Error("missing correlation lookup")
			}
			io.WriteString(w, `{"installation_id":"installation_1","jobs":[]}`)
		case "/api/admin/maintenance/upgrade":
			if _, err := r.Cookie("aster_admin_session"); err != nil {
				w.WriteHeader(401)
				return
			}
			if r.URL.Query().Get("request_id") != "upgrade_123456789" || r.URL.Query().Get("mode") != "maintenance" {
				t.Error("wrong upload correlation")
			}
			reader, err := r.MultipartReader()
			if err != nil {
				t.Error(err)
				return
			}
			part, err := reader.NextPart()
			if err != nil {
				t.Error(err)
				return
			}
			data, _ := io.ReadAll(part)
			if string(data) != "archive" {
				t.Error("archive changed")
			}
			uploads.Add(1)
			w.WriteHeader(202)
			io.WriteString(w, `{"id":"upgrade_123456789"}`)
		default:
			w.WriteHeader(404)
		}
	}))
	defer server.Close()
	client, err := (Factory{}).Connect(domain.UpgradeEnvironment{AdminURL: server.URL}, domain.UpgradeCredentials{AdminEmail: "owner@example.com", AdminPassword: "secret"})
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	if _, err = client.Status(context.Background(), "upgrade_123456789"); err != nil {
		t.Fatal(err)
	}
	if err = client.Upload(context.Background(), "upgrade_123456789", strings.Repeat("a", 64), strings.NewReader("archive")); err != nil {
		t.Fatal(err)
	}
	if uploads.Load() != 1 {
		t.Fatal("upload was repeated")
	}
}

func TestRedirectNeverReceivesCredentialsAndProbeDoesNotStoreBody(t *testing.T) {
	var destinationCalls atomic.Int32
	destination := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { destinationCalls.Add(1) }))
	defer destination.Close()
	source := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { http.Redirect(w, r, destination.URL, 307) }))
	defer source.Close()
	client, err := (Factory{}).Connect(domain.UpgradeEnvironment{APIURL: source.URL, Model: "model"}, domain.UpgradeCredentials{APIKey: "private-key"})
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	sample := client.Probe(context.Background(), "model")
	encoded, _ := json.Marshal(sample)
	if sample.OK || destinationCalls.Load() != 0 || strings.Contains(string(encoded), "private-key") || strings.Contains(string(encoded), destination.URL) {
		t.Fatal("redirect or secret boundary violated")
	}
}
