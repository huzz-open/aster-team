package application

import (
	"context"
	"errors"
	"fmt"
	"sort"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
)

var ErrV2SignerUnavailable = errors.New("v2 signer unavailable; configure a scoped v2 key")

type DistributionStore interface {
	ApproveFreeDistribution(context.Context, string, commercial.ApproveDistributionInput, string, time.Time) (commercial.DistributionRecord, error)
	PrepareFreeDistribution(context.Context, string, string, string, time.Time) (commercial.DistributionRecord, error)
	CompleteFreeDistribution(context.Context, string, licenseprotocol.DocumentV2, string, time.Time) (commercial.DistributionRecord, error)
	GetFreeDistribution(context.Context, string) (commercial.DistributionRecord, error)
	ListFreeDistributions(context.Context, int) ([]commercial.DistributionRecord, error)
}

func WithV2LicenseSigners(signers map[string]ports.LicenseSignerV2) Option {
	return func(s *Service) {
		s.licenseSignersV2 = make(map[string]ports.LicenseSignerV2, len(signers))
		s.licenseVerifiersV2 = make(map[string]ports.LicenseVerifierV2, len(signers))
		for id, signer := range signers {
			s.licenseSignersV2[id] = signer
			s.licenseVerifiersV2[id] = signer
		}
	}
}
func (s *Service) ListV2IssuerProfiles(ctx context.Context, actor string) ([]ports.IssuerProfileV2, error) {
	if err := s.RequirePermission(ctx, actor, PermissionLicenseIssueV2); err != nil {
		return nil, err
	}
	profiles := []ports.IssuerProfileV2{}
	for _, signer := range s.licenseSignersV2 {
		if signer != nil {
			profiles = append(profiles, signer.Profile())
		}
	}
	sort.Slice(profiles, func(i, j int) bool { return profiles[i].KeyID < profiles[j].KeyID })
	return profiles, nil
}

func (s *Service) GetFulfillmentEnvironment(ctx context.Context, actor string) (string, error) {
	if err := s.RequirePermission(ctx, actor, PermissionLicenseIssueV2); err != nil {
		return "", err
	}
	return s.fulfillmentEnvironment, nil
}
func (s *Service) ApproveFreeDistribution(ctx context.Context, in commercial.ApproveDistributionInput, actor string) (commercial.DistributionRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionDistributionApprove); err != nil {
		return commercial.DistributionRecord{}, err
	}
	if !commercialID.MatchString(actor) {
		return commercial.DistributionRecord{}, ErrValidation
	}
	if err := in.Validate(); err != nil {
		return commercial.DistributionRecord{}, fmt.Errorf("%w: %v", ErrValidation, err)
	}
	store, ok := s.store.(DistributionStore)
	if !ok {
		return commercial.DistributionRecord{}, ErrBusinessStoreUnavailable
	}
	// The fixed plan, approval and audit are read/written in one transaction.
	r, err := store.ApproveFreeDistribution(ctx, commercialObjectID("dist", actor, in.OperationID), in, actor, s.now().UTC().Truncate(time.Millisecond))
	if err != nil {
		return r, mapCommercialError(err)
	}
	return r, s.verifyDistribution(r)
}
func (s *Service) verifyDistribution(r commercial.DistributionRecord) error {
	if err := r.Validate(); err != nil {
		return err
	}
	if r.Document == nil {
		return nil
	}
	return s.verifyV2Document(*r.Document)
}
func (s *Service) GetFreeDistribution(ctx context.Context, id, actor string) (commercial.DistributionRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionDistributionRead); err != nil {
		return commercial.DistributionRecord{}, err
	}
	if !commercialID.MatchString(id) {
		return commercial.DistributionRecord{}, ErrValidation
	}
	store, ok := s.store.(DistributionStore)
	if !ok {
		return commercial.DistributionRecord{}, ErrBusinessStoreUnavailable
	}
	r, err := store.GetFreeDistribution(ctx, id)
	if err != nil {
		return r, mapCommercialError(err)
	}
	return r, s.verifyDistribution(r)
}
func (s *Service) ListFreeDistributions(ctx context.Context, limit int, actor string) ([]commercial.DistributionRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionDistributionRead); err != nil {
		return nil, err
	}
	store, ok := s.store.(DistributionStore)
	if !ok {
		return nil, ErrBusinessStoreUnavailable
	}
	records, err := store.ListFreeDistributions(ctx, boundedCommercialLimit(limit))
	if err != nil {
		return nil, err
	}
	for _, r := range records {
		if err = s.verifyDistribution(r); err != nil {
			return nil, err
		}
	}
	return records, nil
}
func (s *Service) IssueFreeDistribution(ctx context.Context, id, keyID, actor string) (commercial.DistributionRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionLicenseIssueV2); err != nil {
		return commercial.DistributionRecord{}, err
	}
	if !commercialID.MatchString(id) || !commercialOperation.MatchString(keyID) {
		return commercial.DistributionRecord{}, ErrValidation
	}
	store, ok := s.store.(DistributionStore)
	if !ok {
		return commercial.DistributionRecord{}, ErrBusinessStoreUnavailable
	}
	r, err := store.GetFreeDistribution(ctx, id)
	if err != nil {
		return r, mapCommercialError(err)
	}
	if err = s.verifyDistribution(r); err != nil {
		return r, err
	}
	if r.Claims != nil && r.Claims.KeyID != keyID {
		return commercial.DistributionRecord{}, mapCommercialError(commercial.ErrConflict)
	}
	if r.Status == "issued" {
		return r, nil
	}
	signer := s.licenseSignersV2[keyID]
	if signer == nil {
		return commercial.DistributionRecord{}, ErrV2SignerUnavailable
	}
	now := s.now().UTC().Truncate(time.Millisecond)
	var candidate licenseprotocol.ClaimsV2
	if r.Claims != nil {
		candidate = *r.Claims
	} else {
		candidate, err = r.Snapshot.Claims(keyID, now)
		if err != nil {
			return r, fmt.Errorf("%w: %v", ErrValidation, err)
		}
	}
	if err = signer.Profile().Policy.Authorize(candidate); err != nil {
		return r, fmt.Errorf("%w: signing profile does not authorize this distribution", ErrValidation)
	}
	r, err = store.PrepareFreeDistribution(ctx, id, keyID, actor, now)
	if err != nil {
		return r, mapCommercialError(err)
	}
	if err = s.verifyDistribution(r); err != nil {
		return r, err
	}
	if r.Status == "issued" {
		return r, nil
	}
	if r.Claims == nil {
		return r, errors.New("distribution has no prepared claims")
	}
	document, err := signer.Sign(ctx, *r.Claims)
	if err != nil {
		return r, err
	}
	if err = signer.Verify(document); err != nil {
		return r, err
	}
	r, err = store.CompleteFreeDistribution(ctx, id, document, actor, s.now().UTC().Truncate(time.Millisecond))
	if err != nil {
		return r, mapCommercialError(err)
	}
	return r, s.verifyDistribution(r)
}
