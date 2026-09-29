package httpapi

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strconv"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

type downloadStore struct {
	*handlerStore
	application.BusinessStore
	releases map[string]domain.ReleaseArtifact
}

func (store *downloadStore) GetReleaseArtifact(_ context.Context, id string) (domain.ReleaseArtifact, error) {
	if artifact, ok := store.releases[id]; ok {
		return artifact, nil
	}
	return domain.ReleaseArtifact{}, application.ErrNotFound
}
func (*downloadStore) HasPermission(_ context.Context, _, permission string) (bool, error) {
	return permission == application.PermissionReleaseDownload, nil
}

type downloadObjectStore struct {
	ports.ArtifactStore
	objects map[string][]byte
}
type downloadReader struct{ *bytes.Reader }

func (*downloadReader) Close() error { return nil }
func (store downloadObjectStore) OpenObject(_ context.Context, key string, _ int64) (ports.ReadSeekCloser, int64, error) {
	contents := store.objects[key]
	return &downloadReader{bytes.NewReader(contents)}, int64(len(contents)), nil
}

func TestDownloadsReturnSelectedPlatformBytesAndFilename(t *testing.T) {
	now := time.Now().UTC()
	store := &downloadStore{handlerStore: &handlerStore{operator: domain.Operator{ID: "operator", Status: "active"}}, releases: map[string]domain.ReleaseArtifact{}}
	store.session = application.CreateSessionParams{ID: "session", OperatorID: "operator", TokenHash: sha256.Sum256([]byte("session-token")), ExpiresAt: now.Add(time.Hour)}
	objects := downloadObjectStore{objects: map[string][]byte{}}
	for _, target := range domain.ReleaseTargets() {
		contents := []byte("immutable archive for " + target.Platform)
		digest := sha256.Sum256(contents)
		store.releases[target.Platform] = domain.ReleaseArtifact{ID: target.Platform, Version: "2.0.0", Platform: target.Platform, Architecture: target.Architecture, ObjectKey: target.Platform, SizeBytes: int64(len(contents)), SHA256: hex.EncodeToString(digest[:])}
		objects.objects[target.Platform] = contents
	}
	handler := New(application.NewService(store, time.Hour, application.WithArtifactStore(objects)), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{BodyMaxSize: 4096})
	for _, target := range domain.ReleaseTargets() {
		t.Run(target.Platform, func(t *testing.T) {
			request := httptest.NewRequest(http.MethodGet, "/api/operations/v1/release-artifacts/"+target.Platform+"/download", nil)
			request.AddCookie(&http.Cookie{Name: sessionCookieName, Value: "session-token"})
			response := httptest.NewRecorder()
			handler.ServeHTTP(response, request)
			if response.Code != http.StatusOK {
				t.Fatalf("download = %d %s", response.Code, response.Body.String())
			}
			if response.Header().Get("Content-Disposition") != `attachment; filename="`+target.FileName("2.0.0")+`"` {
				t.Fatalf("wrong filename: %s", response.Header().Get("Content-Disposition"))
			}
			if !bytes.Equal(response.Body.Bytes(), objects.objects[target.Platform]) || response.Header().Get("Content-Length") != strconv.Itoa(len(objects.objects[target.Platform])) {
				t.Fatal("download bytes or length changed")
			}
			digest := sha256.Sum256(response.Body.Bytes())
			if hex.EncodeToString(digest[:]) != store.releases[target.Platform].SHA256 {
				t.Fatal("download checksum mismatch")
			}
		})
	}
}
