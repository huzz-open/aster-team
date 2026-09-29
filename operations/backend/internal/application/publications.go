package application

import (
	"context"
	"errors"
	"fmt"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

const PermissionPublicationRead = "commercial.publication.read"
const PermissionPublicationPrepare = "commercial.publication.prepare"
const PermissionPublicationAccept = "commercial.publication.accept"

var ErrPublicationVerifierUnavailable = errors.New("website verification target is not configured")
var ErrPublicationVerificationFailed = errors.New("website publication could not be verified")
var ErrPublicationFailureRecordUnavailable = errors.New("publication attempt could not be recorded")

type PublicationVerifier interface {
	Verify(context.Context, commercial.PublicationSnapshot) (commercial.PublicationEvidence, error)
}
type PublicationStore interface {
	PreparePublication(context.Context, string, commercial.PreparePublicationInput, string, time.Time) (commercial.PublicationRecord, error)
	GetPublication(context.Context, string) (commercial.PublicationRecord, error)
	ListPublications(context.Context, int) ([]commercial.PublicationRecord, error)
	GetPublicationHead(context.Context, string) (string, error)
	AcceptPublication(context.Context, string, string, commercial.PublicationEvidence, string) (commercial.PublicationRecord, error)
	RecordPublicationFailure(context.Context, commercial.PublicationFailure) error
	ListPublicationFailures(context.Context, string, int) ([]commercial.PublicationFailure, error)
}

func WithPublicationVerifiers(verifiers map[string]PublicationVerifier) Option {
	return func(s *Service) {
		s.publicationVerifiers = make(map[string]PublicationVerifier, len(verifiers))
		for environment, verifier := range verifiers {
			s.publicationVerifiers[environment] = verifier
		}
	}
}
func (s *Service) PreparePublication(ctx context.Context, in commercial.PreparePublicationInput, actor string) (commercial.PublicationRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionPublicationPrepare); err != nil {
		return commercial.PublicationRecord{}, err
	}
	if err := in.Validate(); err != nil {
		return commercial.PublicationRecord{}, mapCommercialError(err)
	}
	if !commercialID.MatchString(actor) {
		return commercial.PublicationRecord{}, ErrValidation
	}
	store, ok := s.store.(PublicationStore)
	if !ok {
		return commercial.PublicationRecord{}, ErrBusinessStoreUnavailable
	}
	r, err := store.PreparePublication(ctx, commercialObjectID("publication", actor, in.OperationID), in, actor, s.now().UTC().Truncate(time.Millisecond))
	if err != nil {
		return commercial.PublicationRecord{}, mapCommercialError(err)
	}
	return r, r.Validate()
}
func (s *Service) GetPublication(ctx context.Context, id, actor string) (commercial.PublicationRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionPublicationRead); err != nil {
		return commercial.PublicationRecord{}, err
	}
	if !commercialID.MatchString(id) {
		return commercial.PublicationRecord{}, ErrValidation
	}
	store, ok := s.store.(PublicationStore)
	if !ok {
		return commercial.PublicationRecord{}, ErrBusinessStoreUnavailable
	}
	r, err := store.GetPublication(ctx, id)
	if err != nil {
		return commercial.PublicationRecord{}, mapCommercialError(err)
	}
	return r, r.Validate()
}
func (s *Service) ListPublications(ctx context.Context, limit int, actor string) ([]commercial.PublicationRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionPublicationRead); err != nil {
		return nil, err
	}
	store, ok := s.store.(PublicationStore)
	if !ok {
		return nil, ErrBusinessStoreUnavailable
	}
	records, err := store.ListPublications(ctx, boundedCommercialLimit(limit))
	if err != nil {
		return nil, err
	}
	for _, r := range records {
		if err := r.Validate(); err != nil {
			return nil, err
		}
	}
	return records, nil
}
func (s *Service) GetPublicationHead(ctx context.Context, environment, actor string) (string, error) {
	if err := s.RequirePermission(ctx, actor, PermissionPublicationRead); err != nil {
		return "", err
	}
	if environment != "local" && environment != "production" {
		return "", ErrValidation
	}
	store, ok := s.store.(PublicationStore)
	if !ok {
		return "", ErrBusinessStoreUnavailable
	}
	id, err := store.GetPublicationHead(ctx, environment)
	return id, mapCommercialError(err)
}
func (s *Service) VerifyAndAcceptPublication(ctx context.Context, id, actor string) (commercial.PublicationRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionPublicationAccept); err != nil {
		return commercial.PublicationRecord{}, err
	}
	if !commercialID.MatchString(id) || !commercialID.MatchString(actor) {
		return commercial.PublicationRecord{}, ErrValidation
	}
	store, ok := s.store.(PublicationStore)
	if !ok {
		return commercial.PublicationRecord{}, ErrBusinessStoreUnavailable
	}
	r, err := store.GetPublication(ctx, id)
	if err != nil {
		return commercial.PublicationRecord{}, mapCommercialError(err)
	}
	if err = r.Validate(); err != nil {
		return commercial.PublicationRecord{}, err
	}
	if r.Status == "accepted" {
		return r, nil
	}
	verifier := s.publicationVerifiers[r.Snapshot.Catalog.Request.Environment]
	if verifier == nil {
		return commercial.PublicationRecord{}, s.recordPublicationFailure(ctx, store, r, actor, "verification", "target_not_configured", ErrPublicationVerifierUnavailable)
	}
	evidence, err := verifier.Verify(ctx, r.Snapshot)
	if err != nil {
		return commercial.PublicationRecord{}, s.recordPublicationFailure(ctx, store, r, actor, "verification", "content_unverified", ErrPublicationVerificationFailed)
	}
	// A typed adapter is still checked against the immutable source before the
	// transactional store can move this environment's active head.
	now := s.now().UTC().Truncate(time.Millisecond)
	candidate := r
	candidate.Status, candidate.Evidence, candidate.AcceptedBy, candidate.AcceptedAt = "accepted", &evidence, actor, now.Format("2006-01-02T15:04:05.000Z")
	if err = candidate.Validate(); err != nil {
		return commercial.PublicationRecord{}, s.recordPublicationFailure(ctx, store, r, actor, "verification", "invalid_evidence", ErrPublicationVerificationFailed)
	}
	accepted, err := store.AcceptPublication(ctx, id, r.SHA256, evidence, actor)
	if err != nil {
		code := "commit_unconfirmed"
		if errors.Is(err, commercial.ErrConflict) {
			code = "head_conflict"
		}
		if errors.Is(err, commercial.ErrPublicationNotAccepted) {
			code = "deadline_expired"
		}
		return commercial.PublicationRecord{}, s.recordPublicationFailure(ctx, store, r, actor, "acceptance", code, mapCommercialError(err))
	}
	return accepted, accepted.Validate()
}

func (s *Service) recordPublicationFailure(ctx context.Context, store PublicationStore, publication commercial.PublicationRecord, actor, stage, code string, cause error) error {
	token, err := randomToken()
	if err != nil {
		return ErrPublicationFailureRecordUnavailable
	}
	event := commercial.PublicationFailure{ID: commercialObjectID("pubfail", actor, token), PublicationID: publication.Snapshot.ID, PublicationSHA256: publication.SHA256, Stage: stage, Code: code, OperatorID: actor, CreatedAt: s.now().UTC().Truncate(time.Millisecond).Format("2006-01-02T15:04:05.000Z")}
	// The browser closing does not discard a completed attempt's failure. Bound
	// the independent write; a storage outage must not be reported as persisted.
	recordCtx, cancel := context.WithTimeout(context.WithoutCancel(ctx), 5*time.Second)
	defer cancel()
	if err := store.RecordPublicationFailure(recordCtx, event); err != nil {
		return fmt.Errorf("%w: %v", ErrPublicationFailureRecordUnavailable, err)
	}
	return cause
}

func (s *Service) ListPublicationFailures(ctx context.Context, id, actor string) ([]commercial.PublicationFailure, error) {
	if err := s.RequirePermission(ctx, actor, PermissionPublicationRead); err != nil {
		return nil, err
	}
	if !commercialID.MatchString(id) {
		return nil, ErrValidation
	}
	store, ok := s.store.(PublicationStore)
	if !ok {
		return nil, ErrBusinessStoreUnavailable
	}
	items, err := store.ListPublicationFailures(ctx, id, 100)
	if err != nil {
		return nil, mapCommercialError(err)
	}
	for _, event := range items {
		if event.PublicationID != id {
			return nil, commercial.ErrInvalidPublication
		}
		if err := event.Validate(); err != nil {
			return nil, err
		}
	}
	return items, nil
}
