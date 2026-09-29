package githubactions

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"errors"
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

type publishMemoryStore struct{ contents []byte }

func (*publishMemoryStore) ImportInbox(context.Context, string, string) (ports.StoredArtifact, error) {
	return ports.StoredArtifact{}, errors.New("unexpected ImportInbox")
}
func (*publishMemoryStore) PutObject(context.Context, io.Reader, string, int64) (ports.StoredArtifact, error) {
	return ports.StoredArtifact{}, errors.New("unexpected PutObject")
}
func (*publishMemoryStore) PutCachedObject(context.Context, io.Reader, string, int64) (ports.StoredArtifact, error) {
	return ports.StoredArtifact{}, errors.New("unexpected PutCachedObject")
}
func (*publishMemoryStore) PutReceipt(context.Context, []byte) (ports.StoredArtifact, error) {
	return ports.StoredArtifact{}, errors.New("unexpected PutReceipt")
}
func (store *publishMemoryStore) OpenObject(context.Context, string, int64) (ports.ReadSeekCloser, int64, error) {
	return &publisherTestReader{Reader: bytes.NewReader(store.contents)}, int64(len(store.contents)), nil
}
func (*publishMemoryStore) OpenCachedObject(context.Context, string, int64) (ports.ReadSeekCloser, int64, error) {
	return nil, 0, errors.New("unexpected OpenCachedObject")
}
func (*publishMemoryStore) ReadObject(context.Context, string, int64) ([]byte, error) {
	return nil, errors.New("unexpected ReadObject")
}

type publisherTestReader struct{ *bytes.Reader }

func (*publisherTestReader) Close() error { return nil }

func TestPublisherCreatesProtectedTagAndPublishesExactAssetsIdempotently(t *testing.T) {
	privateKey, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	privatePEM := pem.EncodeToMemory(&pem.Block{Type: "RSA PRIVATE KEY", Bytes: x509MarshalPKCS1(privateKey)})
	archive := []byte("independently verified release archive")
	archiveDigest := sha256.Sum256(archive)
	archiveSHA := hex.EncodeToString(archiveDigest[:])
	commitSHA := strings.Repeat("a", 40)
	tagCreated, releaseCreated, published := false, false, false
	assets := map[string]githubReleaseAsset{}
	uploads := 0
	server := httptest.NewTLSServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		response.Header().Set("Content-Type", "application/json")
		switch {
		case request.URL.Path == "/app/installations/2/access_tokens":
			var body struct {
				Permissions map[string]string `json:"permissions"`
			}
			_ = json.NewDecoder(request.Body).Decode(&body)
			if body.Permissions["contents"] != "write" || len(body.Permissions) != 1 {
				t.Fatalf("unexpected publish App permissions: %#v", body.Permissions)
			}
			_ = json.NewEncoder(response).Encode(map[string]any{"token": "installation-token", "expires_at": time.Now().UTC().Add(time.Hour)})
		case request.URL.Path == "/repos/huzz-max/aster-team/git/ref/tags/v2.0.0" && request.Method == http.MethodGet:
			if !tagCreated {
				response.WriteHeader(http.StatusNotFound)
				return
			}
			_ = json.NewEncoder(response).Encode(map[string]any{"ref": "refs/tags/v2.0.0", "object": map[string]string{"type": "commit", "sha": commitSHA}})
		case request.URL.Path == "/repos/huzz-max/aster-team/git/refs" && request.Method == http.MethodPost:
			tagCreated = true
			_ = json.NewEncoder(response).Encode(map[string]any{"ref": "refs/tags/v2.0.0", "object": map[string]string{"type": "commit", "sha": commitSHA}})
		case request.URL.Path == "/repos/huzz-max/aster-team/releases/tags/v2.0.0":
			if !releaseCreated {
				response.WriteHeader(http.StatusNotFound)
				return
			}
			_ = json.NewEncoder(response).Encode(map[string]any{"id": 77, "tag_name": "v2.0.0", "draft": !published, "html_url": "https://github.test/releases/v2.0.0"})
		case request.URL.Path == "/repos/huzz-max/aster-team/releases" && request.Method == http.MethodPost:
			releaseCreated = true
			_ = json.NewEncoder(response).Encode(map[string]any{"id": 77, "tag_name": "v2.0.0", "draft": true, "html_url": "https://github.test/releases/v2.0.0"})
		case request.URL.Path == "/repos/huzz-max/aster-team/releases/77/assets" && request.Method == http.MethodGet:
			items := make([]githubReleaseAsset, 0, len(assets))
			for _, asset := range assets {
				items = append(items, asset)
			}
			_ = json.NewEncoder(response).Encode(items)
		case request.URL.Path == "/repos/huzz-max/aster-team/releases/77/assets" && request.Method == http.MethodPost:
			contents, _ := io.ReadAll(request.Body)
			digest := sha256.Sum256(contents)
			name := request.URL.Query().Get("name")
			uploads++
			asset := githubReleaseAsset{ID: int64(100 + uploads), Name: name, Size: int64(len(contents)), Digest: "sha256:" + hex.EncodeToString(digest[:])}
			assets[name] = asset
			_ = json.NewEncoder(response).Encode(asset)
		case request.URL.Path == "/repos/huzz-max/aster-team/releases/77" && request.Method == http.MethodPatch:
			published = true
			_ = json.NewEncoder(response).Encode(map[string]any{"id": 77, "tag_name": "v2.0.0", "draft": false, "html_url": "https://github.test/releases/v2.0.0"})
		default:
			t.Fatalf("unexpected GitHub request: %s %s", request.Method, request.URL.String())
		}
	}))
	defer server.Close()
	publisher, err := NewPublisher(config.GitHubPublisher{Enabled: true, APIBaseURL: server.URL, UploadBaseURL: server.URL,
		AppID: 1, InstallationID: 2, PrivateKeyPEMBase64: base64.StdEncoding.EncodeToString(privatePEM),
		Repository: "huzz-max/aster-team", RequestTimeout: time.Minute}, &publishMemoryStore{contents: archive}, 1024)
	if err != nil {
		t.Fatal(err)
	}
	publisher.client.httpClient = server.Client()
	release := domain.ReleaseArtifact{ID: "release_1", Version: "2.0.0", Platform: "linux", Architecture: "amd64",
		ObjectKey: "objects/aa/archive", SHA256: archiveSHA, SizeBytes: int64(len(archive)), SourceCommitSHA: &commitSHA}
	for attempt := 0; attempt < 2; attempt++ {
		result, err := publisher.Publish(context.Background(), release)
		if err != nil || result.GitHubReleaseID != 77 || result.TagName != "v2.0.0" {
			t.Fatalf("Publish() = %#v, %v", result, err)
		}
	}
	if uploads != 2 || !published {
		t.Fatalf("uploads = %d, published = %v", uploads, published)
	}
}

func x509MarshalPKCS1(key *rsa.PrivateKey) []byte {
	return x509.MarshalPKCS1PrivateKey(key)
}
