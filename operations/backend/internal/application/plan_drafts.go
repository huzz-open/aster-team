package application

import (
	"context"
	"fmt"
	"math"
	"regexp"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/strictjson"
)

var draftDigest = regexp.MustCompile(`^[a-f0-9]{64}$`)

type PlanDraftStore interface {
	SavePlanDraft(context.Context, commercial.PlanDraftSnapshot, string, string, time.Time) (commercial.PlanDraftRecord, error)
	GetPlanDraft(context.Context, string, uint32) (commercial.PlanDraftRecord, error)
	ListPlanDrafts(context.Context, int) ([]commercial.PlanDraftRecord, error)
	FreezePlanDraft(context.Context, string, uint32, string, string, string, time.Time) (commercial.PlanVersionRecord, error)
}

type SavePlanDraftInput struct {
	OperationID      string                `json:"operation_id"`
	DraftID          string                `json:"draft_id"`
	ExpectedRevision uint32                `json:"expected_revision"`
	PlanID           string                `json:"plan_id"`
	ExpectedVersion  uint32                `json:"expected_version"`
	Definition       commercial.Definition `json:"definition"`
}

func (v *SavePlanDraftInput) UnmarshalJSON(data []byte) error {
	type wire SavePlanDraftInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "draft_id", "expected_revision", "plan_id", "expected_version", "definition"); err != nil {
		return err
	}
	*v = SavePlanDraftInput(result)
	return nil
}

type FreezePlanDraftInput struct {
	OperationID    string `json:"operation_id"`
	DraftID        string `json:"draft_id"`
	Revision       uint32 `json:"revision"`
	ExpectedSHA256 string `json:"expected_sha256"`
}

func (v *FreezePlanDraftInput) UnmarshalJSON(data []byte) error {
	type wire FreezePlanDraftInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "draft_id", "revision", "expected_sha256"); err != nil {
		return err
	}
	*v = FreezePlanDraftInput(result)
	return nil
}

func (s *Service) SavePlanDraft(ctx context.Context, in SavePlanDraftInput, actor string) (commercial.PlanDraftRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCommercialPlanWrite); err != nil {
		return commercial.PlanDraftRecord{}, err
	}
	if !commercialID.MatchString(actor) || !commercialOperation.MatchString(in.OperationID) || in.ExpectedRevision == math.MaxUint32 || in.ExpectedVersion == math.MaxUint32 || (in.DraftID == "" && in.ExpectedRevision != 0) || (in.DraftID != "" && (!commercialID.MatchString(in.DraftID) || in.ExpectedRevision == 0)) || (in.PlanID == "" && (in.ExpectedVersion != 0 || in.DraftID != "")) || (in.PlanID != "" && !commercialID.MatchString(in.PlanID)) {
		return commercial.PlanDraftRecord{}, fmt.Errorf("%w: invalid draft identity or revision", ErrValidation)
	}
	if in.DraftID == "" {
		in.DraftID = commercialObjectID("draft", actor, in.OperationID)
	}
	if in.PlanID == "" {
		in.PlanID = commercialObjectID("plan", actor, in.OperationID)
	}
	snapshot := commercial.PlanDraftSnapshot{Schema: commercial.PlanDraftSchema, DraftID: in.DraftID, Revision: in.ExpectedRevision + 1, PlanID: in.PlanID, ExpectedVersion: in.ExpectedVersion, Definition: in.Definition}
	if _, err := snapshot.Bytes(); err != nil {
		return commercial.PlanDraftRecord{}, fmt.Errorf("%w: %v", ErrValidation, err)
	}
	store, ok := s.store.(PlanDraftStore)
	if !ok {
		return commercial.PlanDraftRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.SavePlanDraft(ctx, snapshot, in.OperationID, actor, s.now().UTC().Truncate(time.Millisecond))
	return record, mapCommercialError(err)
}

func (s *Service) GetPlanDraft(ctx context.Context, id string, revision uint32, actor string) (commercial.PlanDraftRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCommercialPlanRead); err != nil {
		return commercial.PlanDraftRecord{}, err
	}
	if !commercialID.MatchString(id) {
		return commercial.PlanDraftRecord{}, fmt.Errorf("%w: invalid draft identity", ErrValidation)
	}
	store, ok := s.store.(PlanDraftStore)
	if !ok {
		return commercial.PlanDraftRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.GetPlanDraft(ctx, id, revision)
	return record, mapCommercialError(err)
}

func (s *Service) ListPlanDrafts(ctx context.Context, limit int, actor string) ([]commercial.PlanDraftRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCommercialPlanRead); err != nil {
		return nil, err
	}
	store, ok := s.store.(PlanDraftStore)
	if !ok {
		return nil, ErrBusinessStoreUnavailable
	}
	return store.ListPlanDrafts(ctx, boundedCommercialLimit(limit))
}

func (s *Service) FreezePlanDraft(ctx context.Context, in FreezePlanDraftInput, actor string) (commercial.PlanVersionRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCommercialPlanWrite); err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	if !commercialID.MatchString(actor) || !commercialID.MatchString(in.DraftID) || !commercialOperation.MatchString(in.OperationID) || in.Revision == 0 || !draftDigest.MatchString(in.ExpectedSHA256) {
		return commercial.PlanVersionRecord{}, fmt.Errorf("%w: invalid draft freeze selection", ErrValidation)
	}
	store, ok := s.store.(PlanDraftStore)
	if !ok {
		return commercial.PlanVersionRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.FreezePlanDraft(ctx, in.DraftID, in.Revision, in.ExpectedSHA256, in.OperationID, actor, s.now().UTC().Truncate(time.Millisecond))
	return record, mapCommercialError(err)
}
