package httpapi

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"io"
	"log/slog"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/domain"
)

type draftHandlerStore struct {
	*commercialHandlerStore
	draft commercial.PlanDraftRecord
}

func (s *draftHandlerStore) SavePlanDraft(_ context.Context, snap commercial.PlanDraftSnapshot, op, actor string, now time.Time) (commercial.PlanDraftRecord, error) {
	s.calls++
	if s.conflict != nil {
		return commercial.PlanDraftRecord{}, s.conflict
	}
	raw, err := snap.Bytes()
	if err != nil {
		return commercial.PlanDraftRecord{}, err
	}
	s.draft = commercial.PlanDraftRecord{Snapshot: snap, SHA256: commercial.ContentDigest(raw), OperationID: op, CreatedBy: actor, CreatedAt: now}
	return s.draft, nil
}
func (s *draftHandlerStore) GetPlanDraft(context.Context, string, uint32) (commercial.PlanDraftRecord, error) {
	s.calls++
	return s.draft, nil
}
func (s *draftHandlerStore) ListPlanDrafts(context.Context, int) ([]commercial.PlanDraftRecord, error) {
	s.calls++
	return []commercial.PlanDraftRecord{s.draft}, nil
}
func (s *draftHandlerStore) FreezePlanDraft(ctx context.Context, id string, revision uint32, digest, op, actor string, now time.Time) (commercial.PlanVersionRecord, error) {
	return s.FreezePlanVersion(ctx, s.draft.Snapshot.PlanID, s.draft.Snapshot.ExpectedVersion, s.draft.Snapshot.Definition, op, actor, now)
}

func TestPlanDraftHTTPRejectsMissingPermissionsCSRFAndClientRights(t *testing.T) {
	data, err := os.ReadFile("../../../../../contracts/test-vectors/plan-definition.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var definition commercial.Definition
	if err := json.Unmarshal(data, &definition); err != nil {
		t.Fatal(err)
	}
	now := time.Now().UTC()
	store := &draftHandlerStore{commercialHandlerStore: &commercialHandlerStore{handlerStore: &handlerStore{operator: domain.Operator{ID: "operator_1", Email: "admin@test.invalid", Status: "active"}}, allowed: map[string]bool{}}}
	store.session = application.CreateSessionParams{ID: "session_1", OperatorID: "operator_1", TokenHash: sha256.Sum256([]byte("session-token")), CSRFHash: sha256.Sum256([]byte("csrf-token")), ExpiresAt: now.Add(time.Hour)}
	handler := New(application.NewService(store, time.Hour), func(context.Context) error { return nil }, slog.New(slog.NewTextHandler(io.Discard, nil)), Config{TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, BodyMaxSize: 1 << 20})
	path := "/api/operations/v1/commercial/plan-drafts"
	save, err := json.Marshal(application.SavePlanDraftInput{OperationID: "draftop_1", Definition: definition})
	if err != nil {
		t.Fatal(err)
	}
	freeze := `{"operation_id":"freeze_1","draft_id":"draft_1","revision":1,"expected_sha256":"` + strings.Repeat("a", 64) + `"}`
	cases := []struct{ method, path, body string }{{"POST", path, string(save)}, {"POST", path + "/freeze", freeze}, {"GET", path, ""}, {"GET", path + "/draft_1", ""}}
	for _, tc := range cases {
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, highRiskRequest(tc.method, tc.path, tc.body))
		if w.Code != 403 || store.calls != 0 {
			t.Fatalf("permission bypass: %s %d %s", tc.path, w.Code, w.Body.String())
		}
	}
	store.allowed[application.PermissionCommercialPlanWrite] = true
	for _, tc := range cases[:2] {
		r := highRiskRequest(tc.method, tc.path, tc.body)
		r.Header.Del("X-CSRF-Token")
		w := httptest.NewRecorder()
		handler.ServeHTTP(w, r)
		if w.Code != 403 || store.calls != 0 {
			t.Fatal("draft mutation bypassed CSRF")
		}
	}
	w := httptest.NewRecorder()
	handler.ServeHTTP(w, highRiskRequest("POST", path, string(save)))
	if w.Code != 201 || store.draft.Snapshot.Revision != 1 {
		t.Fatalf("save: %d %s", w.Code, w.Body.String())
	}
	for _, bad := range []string{
		strings.Replace(freeze, `"revision":1`, `"revision":1,"revision":1`, 1),
		strings.Replace(freeze, `"revision":1`, `"Revision":1`, 1),
		strings.Replace(freeze, `"revision":1`, `"revision":1,"definition":{}`, 1),
		strings.Replace(string(save), `"expected_revision":0`, `"expected_revision":4294967295`, 1),
	} {
		before := store.calls
		target := path + "/freeze"
		if strings.Contains(bad, "expected_revision") {
			target = path
		}
		w = httptest.NewRecorder()
		handler.ServeHTTP(w, highRiskRequest("POST", target, bad))
		if w.Code != 400 || store.calls != before {
			t.Fatalf("invalid input touched storage: %d %s", w.Code, w.Body.String())
		}
	}
	store.conflict = commercial.ErrDraftRevisionConflict
	w = httptest.NewRecorder()
	handler.ServeHTTP(w, highRiskRequest("POST", path, string(save)))
	if w.Code != 409 || !strings.Contains(w.Body.String(), "COMMERCIAL_DRAFT_REVISION_CONFLICT") {
		t.Fatalf("conflict contract: %d %s", w.Code, w.Body.String())
	}
	store.allowed[application.PermissionCommercialPlanRead] = true
	w = httptest.NewRecorder()
	handler.ServeHTTP(w, highRiskRequest("GET", path+"/draft_1?revision=0", ""))
	if w.Code != 400 {
		t.Fatalf("invalid history revision accepted: %d", w.Code)
	}
}
