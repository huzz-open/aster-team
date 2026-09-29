package application

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

type failureRecordingStore struct {
	PublicationStore
	seen []commercial.PublicationFailure
	err  error
}

type committedPublicationStore struct {
	*commercialPermissionStore
	PublicationStore
	record   commercial.PublicationRecord
	accepted int
	head     string
	failures []commercial.PublicationFailure
}

func (s *committedPublicationStore) GetPublication(context.Context, string) (commercial.PublicationRecord, error) {
	return s.record, nil
}
func (s *committedPublicationStore) AcceptPublication(_ context.Context, id, _ string, evidence commercial.PublicationEvidence, actor string) (commercial.PublicationRecord, error) {
	s.accepted++
	s.record.Status, s.record.Evidence, s.record.AcceptedBy, s.record.AcceptedAt = "accepted", &evidence, actor, evidence.ObservedAt
	s.head = id
	return commercial.PublicationRecord{}, context.DeadlineExceeded
}
func (s *committedPublicationStore) RecordPublicationFailure(_ context.Context, event commercial.PublicationFailure) error {
	s.failures = append(s.failures, event)
	return event.Validate()
}

type publicationProof struct {
	evidence commercial.PublicationEvidence
	calls    int
}

func (v *publicationProof) Verify(context.Context, commercial.PublicationSnapshot) (commercial.PublicationEvidence, error) {
	v.calls++
	return v.evidence, nil
}

func TestPublicationCommittedTimeoutRemainsUnconfirmedAndRestoresOriginalReceipt(t *testing.T) {
	now := time.Now().UTC().Truncate(time.Millisecond)
	format := func(at time.Time) string { return at.Format("2006-01-02T15:04:05.000Z") }
	revision := "catalog_" + strings.Repeat("a", 48)
	request := commercial.CatalogRequest{OperationID: "catalog_test", Environment: "local", Reason: "isolated test", Plans: []commercial.CatalogSelection{}}
	plans := []commercial.PlanSnapshot{}
	preview, err := commercial.BuildPublicCatalog(revision, request, plans)
	if err != nil {
		t.Fatal(err)
	}
	snapshot := commercial.PublicationSnapshot{Schema: commercial.PublicationSchema, ID: "publication_timeout", Request: commercial.PreparePublicationInput{OperationID: "prepare_test", CatalogRevision: revision, BuildSHA256: strings.Repeat("b", 64), AcceptUntil: format(now.Add(time.Hour)), Reason: "isolated test"}, Catalog: commercial.CatalogApprovalSnapshot{Schema: commercial.CatalogApprovalSchema, ID: revision, Request: request, Plans: plans, PublicSHA256: preview.SHA256, ApprovedBy: "operator_1", ApprovedAt: format(now.Add(-time.Second))}, CreatedBy: "operator_1", CreatedAt: format(now.Add(-time.Second))}
	raw, err := snapshot.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	store := &committedPublicationStore{commercialPermissionStore: &commercialPermissionStore{fakeStore: &fakeStore{}, allowed: map[string]bool{PermissionPublicationAccept: true, PermissionPublicationRead: true}}, record: commercial.PublicationRecord{Snapshot: snapshot, SHA256: commercial.ContentDigest(raw), Status: "prepared"}}
	proof := &publicationProof{evidence: commercial.PublicationEvidence{Environment: "local", Origin: "http://127.0.0.1:26394", BuildSHA256: snapshot.Request.BuildSHA256, CatalogRevision: revision, CatalogSHA256: preview.SHA256, ObservedAt: format(now)}}
	service := NewService(store, time.Hour, WithPublicationVerifiers(map[string]PublicationVerifier{"local": proof}))
	service.now = func() time.Time { return now }
	if _, err := service.VerifyAndAcceptPublication(context.Background(), snapshot.ID, "operator_1"); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatal(err)
	}
	if len(store.failures) != 1 || store.failures[0].Code != "commit_unconfirmed" {
		t.Fatal("committed timeout misreported as definite rejection", store.failures)
	}
	original, err := service.GetPublication(context.Background(), snapshot.ID, "operator_1")
	if err != nil || original.Status != "accepted" {
		t.Fatal("committed state not readable", err)
	}
	recovered, err := service.VerifyAndAcceptPublication(context.Background(), snapshot.ID, "operator_1")
	if err != nil || recovered.AcceptedAt != original.AcceptedAt || recovered.SHA256 != original.SHA256 || proof.calls != 1 || store.accepted != 1 || store.head != snapshot.ID || len(store.failures) != 1 {
		t.Fatal("receipt recovery repeated acceptance", err)
	}
}

func (s *failureRecordingStore) RecordPublicationFailure(ctx context.Context, event commercial.PublicationFailure) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	deadline, ok := ctx.Deadline()
	if !ok || time.Until(deadline) > 5*time.Second {
		return errors.New("unbounded failure recording")
	}
	if err := event.Validate(); err != nil {
		return err
	}
	s.seen = append(s.seen, event)
	return s.err
}
func TestPublicationFailureRecordingSurvivesCallerCancellationAndReportsStorageFailure(t *testing.T) {
	s := NewService(&fakeStore{}, time.Hour)
	store := &failureRecordingStore{}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	publication := commercial.PublicationRecord{Snapshot: commercial.PublicationSnapshot{ID: "publication_test"}, SHA256: strings.Repeat("a", 64)}
	for range 2 {
		if err := s.recordPublicationFailure(ctx, store, publication, "operator_1", "verification", "content_unverified", ErrPublicationVerificationFailed); !errors.Is(err, ErrPublicationVerificationFailed) {
			t.Fatal(err)
		}
	}
	if len(store.seen) != 2 || store.seen[0].ID == store.seen[1].ID || store.seen[0].PublicationSHA256 != publication.SHA256 {
		t.Fatal("failure attempts lost or mixed")
	}
	store.err = errors.New("database unavailable")
	if err := s.recordPublicationFailure(ctx, store, publication, "operator_1", "acceptance", "commit_unconfirmed", commercial.ErrConflict); !errors.Is(err, ErrPublicationFailureRecordUnavailable) {
		t.Fatal("storage failure reported as persisted", err)
	}
}
