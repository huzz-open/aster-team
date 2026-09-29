package application

import (
	"context"
	"errors"
	"fmt"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
)

const PermissionFulfillmentRead = "commercial.fulfillment.read"
const PermissionFulfillmentApprove = "commercial.fulfillment.approve"

type PaidFulfillmentStore interface {
	GetPaidFulfillment(context.Context, string) (commercial.PaidFulfillmentRecord, error)
	GetPaidFulfillmentForOrder(context.Context, string) (commercial.PaidFulfillmentRecord, error)
	ApprovePaidFulfillment(context.Context, string, string, commercial.ApprovePaidFulfillmentInput, string, string, ports.CustomerReferenceSource) (commercial.PaidFulfillmentRecord, error)
	PreparePaidFulfillment(context.Context, string, string, string, string) (commercial.PaidFulfillmentRecord, error)
	CompletePaidFulfillment(context.Context, string, licenseprotocol.DocumentV2, string, string) (commercial.PaidFulfillmentRecord, error)
}

type PaidLifecycleSourceStore interface {
	GetPaidLifecycleSource(context.Context, string, string, string) (commercial.PaidLifecycleSource, error)
}

type PaidRedeliveryStore interface {
	GetPaidFulfillment(context.Context, string) (commercial.PaidFulfillmentRecord, error)
	GetPaidRedelivery(context.Context, string) (commercial.PaidRedeliveryRecord, error)
	RecordPaidRedelivery(context.Context, string, string, commercial.RecordPaidRedeliveryInput, string, string) (commercial.PaidRedeliveryRecord, error)
}

type PaidTransferStore interface {
	GetPaidFulfillment(context.Context, string) (commercial.PaidFulfillmentRecord, error)
	GetPaidTransfer(context.Context, string) (commercial.PaidTransferRecord, error)
	GetLatestPaidTransfer(context.Context, string) (commercial.PaidTransferRecord, error)
	ListPaidTransferChain(context.Context, string, uint32) ([]commercial.PaidTransferRecord, error)
	ApprovePaidTransfer(context.Context, string, string, commercial.ApprovePaidTransferInput, string, string) (commercial.PaidTransferRecord, error)
	PreparePaidTransfer(context.Context, string, string, string, string) (commercial.PaidTransferRecord, error)
	CompletePaidTransfer(context.Context, string, licenseprotocol.DocumentV2, string, string) (commercial.PaidTransferRecord, error)
}

func WithFulfillmentEnvironment(environment string) Option {
	return func(s *Service) { s.fulfillmentEnvironment = environment }
}
func (s *Service) requirePaidRead(ctx context.Context, actor string) error {
	for _, permission := range []string{PermissionFulfillmentRead, PermissionFulfillmentApprove, PermissionLicenseIssueV2} {
		err := s.RequirePermission(ctx, actor, permission)
		if err == nil {
			return nil
		}
		if !errors.Is(err, ErrUnauthorized) {
			return err
		}
	}
	return ErrUnauthorized
}
func (s *Service) verifyPaidFulfillment(r commercial.PaidFulfillmentRecord) error {
	if err := r.Validate(); err != nil {
		return err
	}
	if source := r.Snapshot.Lifecycle; source != nil {
		if err := s.verifyFrozenPaidLifecycleSource(*source); err != nil {
			return err
		}
	}
	if r.Document != nil {
		return s.verifyV2Document(*r.Document)
	}
	return nil
}

// Frozen lifecycle records must remain recoverable after Operations rotates its
// customer-reference secret or changes its current fulfillment environment.
// Their immutable source identity is protected by the stored hashes and the
// original license signature, so verification must not reinterpret that source
// through mutable current configuration.
func (s *Service) verifyFrozenPaidLifecycleSource(source commercial.PaidLifecycleSource) error {
	if err := source.Validate(); err != nil {
		return err
	}
	if source.Document != nil {
		return s.verifyV2Document(*source.Document)
	}
	return commercial.ErrPaidFulfillmentIntegrity
}

// A source being admitted into a brand-new successor still has to match the
// current environment and current customer-reference derivation. Only the
// already-frozen recovery path is independent of those mutable settings.
func (s *Service) verifyPaidLifecycleSourceAdmission(source commercial.PaidLifecycleSource) error {
	if err := source.Validate(); err != nil {
		return err
	}
	if source.Environment != s.fulfillmentEnvironment {
		return commercial.ErrFulfillmentEnvironment
	}
	if s.customerRefs == nil {
		return commercial.ErrCustomerReferenceUnavailable
	}
	expectedCustomerRef, err := s.customerRefs.Reference(source.CustomerID)
	if err != nil || expectedCustomerRef != source.CustomerRef {
		return commercial.ErrPaidFulfillmentIntegrity
	}
	return s.verifyFrozenPaidLifecycleSource(source)
}
func (s *Service) GetPaidFulfillment(ctx context.Context, id, actor string) (commercial.PaidFulfillmentRecord, error) {
	if err := s.requirePaidRead(ctx, actor); err != nil {
		return commercial.PaidFulfillmentRecord{}, err
	}
	if !commercialID.MatchString(id) {
		return commercial.PaidFulfillmentRecord{}, ErrValidation
	}
	store, ok := s.store.(PaidFulfillmentStore)
	if !ok {
		return commercial.PaidFulfillmentRecord{}, ErrBusinessStoreUnavailable
	}
	r, err := store.GetPaidFulfillment(ctx, id)
	if err != nil {
		return r, mapCommercialError(err)
	}
	return r, s.verifyPaidFulfillment(r)
}
func (s *Service) GetPaidFulfillmentForOrder(ctx context.Context, orderID, actor string) (commercial.PaidFulfillmentRecord, error) {
	if err := s.requirePaidRead(ctx, actor); err != nil {
		return commercial.PaidFulfillmentRecord{}, err
	}
	if !commercialID.MatchString(orderID) {
		return commercial.PaidFulfillmentRecord{}, ErrValidation
	}
	store, ok := s.store.(PaidFulfillmentStore)
	if !ok {
		return commercial.PaidFulfillmentRecord{}, ErrBusinessStoreUnavailable
	}
	r, err := store.GetPaidFulfillmentForOrder(ctx, orderID)
	if err != nil {
		return r, mapCommercialError(err)
	}
	return r, s.verifyPaidFulfillment(r)
}

func (s *Service) GetPaidLifecycleSource(ctx context.Context, kind, sourceID, actor string) (commercial.PaidLifecycleSource, error) {
	if err := s.RequirePermission(ctx, actor, PermissionFulfillmentApprove); err != nil {
		return commercial.PaidLifecycleSource{}, err
	}
	if kind != commercial.PaidLifecycleRenewal && kind != commercial.PaidLifecycleUpgrade || !commercialID.MatchString(sourceID) {
		return commercial.PaidLifecycleSource{}, ErrValidation
	}
	store, ok := s.store.(PaidLifecycleSourceStore)
	if !ok {
		return commercial.PaidLifecycleSource{}, ErrBusinessStoreUnavailable
	}
	record, err := store.GetPaidLifecycleSource(ctx, kind, sourceID, s.fulfillmentEnvironment)
	if err != nil {
		return record, mapCommercialError(err)
	}
	return record, s.verifyPaidLifecycleSourceAdmission(record)
}
func (s *Service) ApprovePaidFulfillment(ctx context.Context, orderID string, in commercial.ApprovePaidFulfillmentInput, actor string) (commercial.PaidFulfillmentRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionFulfillmentApprove); err != nil {
		return commercial.PaidFulfillmentRecord{}, err
	}
	if !commercialID.MatchString(orderID) || !commercialID.MatchString(actor) {
		return commercial.PaidFulfillmentRecord{}, ErrValidation
	}
	if err := in.Validate(); err != nil {
		return commercial.PaidFulfillmentRecord{}, mapCommercialError(err)
	}
	store, ok := s.store.(PaidFulfillmentStore)
	if !ok {
		return commercial.PaidFulfillmentRecord{}, ErrBusinessStoreUnavailable
	}
	if in.Lifecycle != nil {
		// Preserve idempotent retries after the source has been consumed. Brand-new
		// lifecycle approvals must verify the exact predecessor before any write.
		if _, existingErr := store.GetPaidFulfillmentForOrder(ctx, orderID); existingErr != nil {
			if !errors.Is(existingErr, commercial.ErrNotFound) {
				return commercial.PaidFulfillmentRecord{}, mapCommercialError(existingErr)
			}
			lifecycleStore, ok := s.store.(PaidLifecycleSourceStore)
			if !ok {
				return commercial.PaidFulfillmentRecord{}, ErrBusinessStoreUnavailable
			}
			source, sourceErr := lifecycleStore.GetPaidLifecycleSource(ctx, in.Lifecycle.Kind, in.Lifecycle.SourceID, s.fulfillmentEnvironment)
			if sourceErr != nil {
				return commercial.PaidFulfillmentRecord{}, mapCommercialError(sourceErr)
			}
			if source.DocumentSHA256 != in.Lifecycle.ExpectedDocumentSHA256 {
				return commercial.PaidFulfillmentRecord{}, mapCommercialError(commercial.ErrConflict)
			}
			if sourceErr = s.verifyPaidLifecycleSourceAdmission(source); sourceErr != nil {
				return commercial.PaidFulfillmentRecord{}, sourceErr
			}
		}
	}
	// Resolve the customer reference only inside a new approval transaction. The
	// original operation must remain recoverable when derivation is unavailable.
	r, err := store.ApprovePaidFulfillment(ctx, commercialObjectID("fulfillment", actor, in.OperationID), orderID, in, s.fulfillmentEnvironment, actor, s.customerRefs)
	if err != nil {
		return r, mapCommercialError(err)
	}
	return r, s.verifyPaidFulfillment(r)
}
func (s *Service) IssuePaidFulfillment(ctx context.Context, id, keyID, actor string) (commercial.PaidFulfillmentRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionLicenseIssueV2); err != nil {
		return commercial.PaidFulfillmentRecord{}, err
	}
	if !commercialID.MatchString(id) || !commercialID.MatchString(actor) || !commercialOperation.MatchString(keyID) {
		return commercial.PaidFulfillmentRecord{}, ErrValidation
	}
	store, ok := s.store.(PaidFulfillmentStore)
	if !ok {
		return commercial.PaidFulfillmentRecord{}, ErrBusinessStoreUnavailable
	}
	r, err := store.GetPaidFulfillment(ctx, id)
	if err != nil {
		return r, mapCommercialError(err)
	}
	if err := s.verifyPaidFulfillment(r); err != nil {
		return r, err
	}
	if r.Claims != nil && r.Claims.KeyID != keyID {
		return r, mapCommercialError(commercial.ErrConflict)
	}
	if r.Status == "issued" {
		return r, nil
	}
	progressEnvironment := s.fulfillmentEnvironment
	if r.Snapshot.Lifecycle != nil {
		// A successor already froze its source and environment at approval. Resume
		// that exact record even if mutable Operations configuration has changed.
		progressEnvironment = r.Snapshot.Environment
	} else if progressEnvironment != r.Snapshot.Environment {
		return r, commercial.ErrFulfillmentEnvironment
	}
	signer := s.licenseSignersV2[keyID]
	if signer == nil {
		return r, ErrV2SignerUnavailable
	}
	var candidate licenseprotocol.ClaimsV2
	if r.Claims != nil {
		candidate = *r.Claims
	} else {
		candidate, err = r.Snapshot.Claims(keyID, s.now().UTC().Truncate(time.Millisecond))
		if err != nil {
			return r, fmt.Errorf("%w: %v", ErrValidation, err)
		}
	}
	if err := signer.Profile().Policy.Authorize(candidate); err != nil {
		return r, fmt.Errorf("%w: signing profile does not authorize paid fulfillment", ErrValidation)
	}
	r, err = store.PreparePaidFulfillment(ctx, id, keyID, progressEnvironment, actor)
	if err != nil {
		return r, mapCommercialError(err)
	}
	if err := s.verifyPaidFulfillment(r); err != nil {
		return r, err
	}
	if r.Status == "issued" {
		return r, nil
	}
	if r.Claims == nil || r.Claims.KeyID != keyID || r.Snapshot.Environment != progressEnvironment {
		return r, commercial.ErrPaidFulfillmentIntegrity
	}
	document, err := signer.Sign(ctx, *r.Claims)
	if err != nil {
		return r, err
	}
	if err := s.verifyV2Document(document); err != nil {
		return r, err
	}
	r, err = store.CompletePaidFulfillment(ctx, id, document, progressEnvironment, actor)
	if err != nil {
		return r, mapCommercialError(err)
	}
	return r, s.verifyPaidFulfillment(r)
}

func (s *Service) verifyPaidRedelivery(ctx context.Context, store PaidRedeliveryStore, record commercial.PaidRedeliveryRecord) error {
	fulfillment, err := store.GetPaidFulfillment(ctx, record.Snapshot.FulfillmentID)
	if err != nil {
		return mapCommercialError(err)
	}
	if err := record.ValidateFulfillment(fulfillment); err != nil {
		return err
	}
	return s.verifyPaidFulfillment(fulfillment)
}

func (s *Service) GetPaidRedelivery(ctx context.Context, id, actor string) (commercial.PaidRedeliveryRecord, error) {
	if err := s.requirePaidRead(ctx, actor); err != nil {
		return commercial.PaidRedeliveryRecord{}, err
	}
	if !commercialID.MatchString(id) {
		return commercial.PaidRedeliveryRecord{}, ErrValidation
	}
	store, ok := s.store.(PaidRedeliveryStore)
	if !ok {
		return commercial.PaidRedeliveryRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.GetPaidRedelivery(ctx, id)
	if err != nil {
		return record, mapCommercialError(err)
	}
	return record, s.verifyPaidRedelivery(ctx, store, record)
}

func (s *Service) RecordPaidRedelivery(ctx context.Context, fulfillmentID string, input commercial.RecordPaidRedeliveryInput, actor string) (commercial.PaidRedeliveryRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionFulfillmentApprove); err != nil {
		return commercial.PaidRedeliveryRecord{}, err
	}
	if !commercialID.MatchString(fulfillmentID) || !commercialID.MatchString(actor) {
		return commercial.PaidRedeliveryRecord{}, ErrValidation
	}
	if err := input.Validate(); err != nil {
		return commercial.PaidRedeliveryRecord{}, mapCommercialError(err)
	}
	store, ok := s.store.(PaidRedeliveryStore)
	if !ok {
		return commercial.PaidRedeliveryRecord{}, ErrBusinessStoreUnavailable
	}
	fulfillment, err := store.GetPaidFulfillment(ctx, fulfillmentID)
	if err != nil {
		return commercial.PaidRedeliveryRecord{}, mapCommercialError(err)
	}
	if err := s.verifyPaidFulfillment(fulfillment); err != nil {
		return commercial.PaidRedeliveryRecord{}, err
	}
	if fulfillment.Status != "issued" || fulfillment.Document == nil || fulfillment.DocumentSHA256 != input.ExpectedDocumentSHA256 {
		return commercial.PaidRedeliveryRecord{}, mapCommercialError(commercial.ErrConflict)
	}
	record, err := store.RecordPaidRedelivery(ctx, commercialObjectID("redelivery", actor, input.OperationID), fulfillmentID, input, s.fulfillmentEnvironment, actor)
	if err != nil {
		return record, mapCommercialError(err)
	}
	return record, s.verifyPaidRedelivery(ctx, store, record)
}

func (s *Service) verifyPaidTransfer(ctx context.Context, store PaidTransferStore, record commercial.PaidTransferRecord) error {
	if err := record.Validate(); err != nil {
		return err
	}
	fulfillment, err := store.GetPaidFulfillment(ctx, record.Snapshot.FulfillmentID)
	if err != nil {
		return mapCommercialError(err)
	}
	if err := record.ValidateFulfillment(fulfillment); err != nil {
		return err
	}
	if err := s.verifyPaidFulfillment(fulfillment); err != nil {
		return err
	}
	if fulfillment.Claims == nil {
		return commercial.ErrPaidFulfillmentIntegrity
	}
	chain, err := store.ListPaidTransferChain(ctx, record.Snapshot.FulfillmentID, record.Snapshot.TransferSequence)
	if err != nil || len(chain) != int(record.Snapshot.TransferSequence) || chain[len(chain)-1].SHA256 != record.SHA256 {
		return commercial.ErrPaidFulfillmentIntegrity
	}
	previousClaims, previousDocumentSHA256 := *fulfillment.Claims, fulfillment.DocumentSHA256
	for index, current := range chain {
		if current.Snapshot.TransferSequence != uint32(index+1) || current.ValidateFulfillment(fulfillment) != nil || current.ValidatePredecessor(previousClaims, previousDocumentSHA256) != nil {
			return commercial.ErrPaidFulfillmentIntegrity
		}
		if current.Document != nil {
			if err := s.verifyV2Document(*current.Document); err != nil {
				return err
			}
		}
		if index < len(chain)-1 && (current.Status != "issued" || current.Claims == nil || current.Document == nil) {
			return commercial.ErrPaidFulfillmentIntegrity
		}
		if current.Claims != nil {
			previousClaims = *current.Claims
			previousDocumentSHA256 = current.DocumentSHA256
		}
	}
	if record.Document != nil && chain[len(chain)-1].DocumentSHA256 != record.DocumentSHA256 {
		return commercial.ErrPaidFulfillmentIntegrity
	}
	return nil
}

func (s *Service) GetPaidTransfer(ctx context.Context, id, actor string) (commercial.PaidTransferRecord, error) {
	if err := s.requirePaidRead(ctx, actor); err != nil {
		return commercial.PaidTransferRecord{}, err
	}
	if !commercialID.MatchString(id) {
		return commercial.PaidTransferRecord{}, ErrValidation
	}
	store, ok := s.store.(PaidTransferStore)
	if !ok {
		return commercial.PaidTransferRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.GetPaidTransfer(ctx, id)
	if err != nil {
		return record, mapCommercialError(err)
	}
	return record, s.verifyPaidTransfer(ctx, store, record)
}

func (s *Service) GetLatestPaidTransfer(ctx context.Context, fulfillmentID, actor string) (commercial.PaidTransferRecord, error) {
	if err := s.requirePaidRead(ctx, actor); err != nil {
		return commercial.PaidTransferRecord{}, err
	}
	if !commercialID.MatchString(fulfillmentID) {
		return commercial.PaidTransferRecord{}, ErrValidation
	}
	store, ok := s.store.(PaidTransferStore)
	if !ok {
		return commercial.PaidTransferRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.GetLatestPaidTransfer(ctx, fulfillmentID)
	if err != nil {
		return record, mapCommercialError(err)
	}
	return record, s.verifyPaidTransfer(ctx, store, record)
}

func (s *Service) ApprovePaidTransfer(ctx context.Context, fulfillmentID string, input commercial.ApprovePaidTransferInput, actor string) (commercial.PaidTransferRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionFulfillmentApprove); err != nil {
		return commercial.PaidTransferRecord{}, err
	}
	if !commercialID.MatchString(fulfillmentID) || !commercialID.MatchString(actor) || input.Validate() != nil {
		return commercial.PaidTransferRecord{}, ErrValidation
	}
	store, ok := s.store.(PaidTransferStore)
	if !ok {
		return commercial.PaidTransferRecord{}, ErrBusinessStoreUnavailable
	}
	transferID := commercialObjectID("transfer", actor, input.OperationID)
	requested, err := input.Bytes()
	if err != nil {
		return commercial.PaidTransferRecord{}, mapCommercialError(err)
	}
	existing, err := store.GetPaidTransfer(ctx, transferID)
	if err == nil {
		if !existing.Matches(transferID, fulfillmentID, actor, requested) {
			return commercial.PaidTransferRecord{}, mapCommercialError(commercial.ErrConflict)
		}
		return existing, s.verifyPaidTransfer(ctx, store, existing)
	}
	if !errors.Is(err, commercial.ErrNotFound) {
		return commercial.PaidTransferRecord{}, mapCommercialError(err)
	}
	fulfillment, err := store.GetPaidFulfillment(ctx, fulfillmentID)
	if err != nil {
		return commercial.PaidTransferRecord{}, mapCommercialError(err)
	}
	if err := s.verifyPaidFulfillment(fulfillment); err != nil {
		return commercial.PaidTransferRecord{}, err
	}
	currentHash := fulfillment.DocumentSHA256
	latest, err := store.GetLatestPaidTransfer(ctx, fulfillmentID)
	if err == nil {
		if err := s.verifyPaidTransfer(ctx, store, latest); err != nil {
			return commercial.PaidTransferRecord{}, err
		}
		if latest.Status != "issued" || latest.Document == nil {
			return commercial.PaidTransferRecord{}, mapCommercialError(commercial.ErrConflict)
		}
		currentHash = latest.DocumentSHA256
	} else if !errors.Is(err, commercial.ErrNotFound) {
		return commercial.PaidTransferRecord{}, mapCommercialError(err)
	}
	if currentHash == "" || currentHash != input.ExpectedCurrentDocumentSHA256 {
		return commercial.PaidTransferRecord{}, mapCommercialError(commercial.ErrConflict)
	}
	record, err := store.ApprovePaidTransfer(ctx, transferID, fulfillmentID, input, s.fulfillmentEnvironment, actor)
	if err != nil {
		return record, mapCommercialError(err)
	}
	return record, s.verifyPaidTransfer(ctx, store, record)
}

func (s *Service) IssuePaidTransfer(ctx context.Context, id, keyID, actor string) (commercial.PaidTransferRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionLicenseIssueV2); err != nil {
		return commercial.PaidTransferRecord{}, err
	}
	if !commercialID.MatchString(id) || !commercialID.MatchString(actor) || !commercialOperation.MatchString(keyID) {
		return commercial.PaidTransferRecord{}, ErrValidation
	}
	store, ok := s.store.(PaidTransferStore)
	if !ok {
		return commercial.PaidTransferRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.GetPaidTransfer(ctx, id)
	if err != nil {
		return record, mapCommercialError(err)
	}
	if err := s.verifyPaidTransfer(ctx, store, record); err != nil {
		return record, err
	}
	if record.Claims != nil && record.Claims.KeyID != keyID {
		return record, mapCommercialError(commercial.ErrConflict)
	}
	if record.Status == "issued" {
		return record, nil
	}
	if record.Snapshot.Environment != s.fulfillmentEnvironment {
		return record, commercial.ErrFulfillmentEnvironment
	}
	signer := s.licenseSignersV2[keyID]
	if signer == nil {
		return record, ErrV2SignerUnavailable
	}
	var candidate licenseprotocol.ClaimsV2
	if record.Claims != nil {
		candidate = *record.Claims
	} else {
		candidate, err = record.Snapshot.Claims(keyID, s.now().UTC().Truncate(time.Millisecond))
		if err != nil {
			return record, fmt.Errorf("%w: %v", ErrValidation, err)
		}
	}
	if err := signer.Profile().Policy.Authorize(candidate); err != nil {
		return record, fmt.Errorf("%w: signing profile does not authorize paid transfer", ErrValidation)
	}
	record, err = store.PreparePaidTransfer(ctx, id, keyID, s.fulfillmentEnvironment, actor)
	if err != nil {
		return record, mapCommercialError(err)
	}
	if err := s.verifyPaidTransfer(ctx, store, record); err != nil {
		return record, err
	}
	if record.Status == "issued" {
		return record, nil
	}
	if record.Claims == nil || record.Claims.KeyID != keyID {
		return record, commercial.ErrPaidFulfillmentIntegrity
	}
	document, err := signer.Sign(ctx, *record.Claims)
	if err != nil {
		return record, err
	}
	if err := s.verifyV2Document(document); err != nil {
		return record, err
	}
	record, err = store.CompletePaidTransfer(ctx, id, document, s.fulfillmentEnvironment, actor)
	if err != nil {
		return record, mapCommercialError(err)
	}
	return record, s.verifyPaidTransfer(ctx, store, record)
}

type PaidFulfillmentContext struct {
	OrderID            string                  `json:"order_id"`
	CustomerID         string                  `json:"customer_id"`
	OrderSHA256        string                  `json:"order_sha256"`
	PaymentID          string                  `json:"payment_id"`
	PaymentSHA256      string                  `json:"payment_sha256"`
	Plan               commercial.PlanSnapshot `json:"plan"`
	AmountMinor        int64                   `json:"amount_minor"`
	Currency           string                  `json:"currency"`
	StartsAt           string                  `json:"starts_at"`
	EndsAt             string                  `json:"ends_at"`
	Status             string                  `json:"status"`
	Source             string                  `json:"source"`
	Environment        string                  `json:"environment"`
	MinimumVersion     string                  `json:"minimum_version"`
	CatalogVersion     uint32                  `json:"catalog_version"`
	QuotaPolicyVersion uint32                  `json:"quota_policy_version"`
}

func (s *Service) GetPaidFulfillmentContext(ctx context.Context, orderID, actor string) (PaidFulfillmentContext, error) {
	if err := s.RequirePermission(ctx, actor, PermissionFulfillmentApprove); err != nil {
		return PaidFulfillmentContext{}, err
	}
	if !commercialID.MatchString(orderID) {
		return PaidFulfillmentContext{}, ErrValidation
	}
	payments, ok := s.store.(CommercialPaymentStore)
	if !ok {
		return PaidFulfillmentContext{}, ErrBusinessStoreUnavailable
	}
	orders, ok := s.store.(CommercialStore)
	if !ok {
		return PaidFulfillmentContext{}, ErrBusinessStoreUnavailable
	}
	// Read through source-validating stores without requiring unrelated plan,
	// customer, payment or publication administration permissions.
	payment, err := payments.GetCommercialPayment(ctx, orderID)
	if err != nil {
		return PaidFulfillmentContext{}, mapCommercialError(err)
	}
	order, err := orders.GetCommercialOrder(ctx, orderID)
	if err != nil {
		return PaidFulfillmentContext{}, mapCommercialError(err)
	}
	raw, err := order.Snapshot.Bytes()
	if err != nil || payment.Validate() != nil || commercial.ContentDigest(raw) != order.SHA256 || order.SHA256 != payment.Snapshot.OrderSHA256 {
		return PaidFulfillmentContext{}, commercial.ErrPaidFulfillmentIntegrity
	}
	v := order.Snapshot
	source := "manual"
	if v.Source != nil {
		source = v.Source.Environment
	}
	return PaidFulfillmentContext{OrderID: v.OrderID, CustomerID: v.CustomerID, OrderSHA256: order.SHA256, PaymentID: payment.Snapshot.ID, PaymentSHA256: payment.SHA256, Plan: v.Plan, AmountMinor: v.AmountMinor, Currency: v.Currency, StartsAt: v.StartsAt, EndsAt: v.EndsAt, Status: order.Status, Source: source, Environment: s.fulfillmentEnvironment, MinimumVersion: v.Plan.Definition.MinimumVersion, CatalogVersion: v.Plan.Definition.Entitlements.CatalogVersion, QuotaPolicyVersion: v.Plan.Definition.QuotaPolicyVersion}, nil
}
