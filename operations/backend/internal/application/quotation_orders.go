package application

import (
	"context"

	"aster.local/team/operations/backend/internal/commercial"
)

type QuotationOrderStore interface {
	CreateQuotationOrder(context.Context, string, commercial.QuotationOrderInput, string, string) (commercial.OrderRecord, error)
}
type QuotationSourceStore interface {
	GetQuotationSource(context.Context, string, string) (commercial.QuotationSource, error)
}

func (s *Service) GetQuotationSource(ctx context.Context, reference, actor string) (commercial.QuotationSource, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCommercialOrderWrite); err != nil {
		return commercial.QuotationSource{}, err
	}
	if !commercialID.MatchString(reference) {
		return commercial.QuotationSource{}, ErrValidation
	}
	store, ok := s.store.(QuotationSourceStore)
	if !ok {
		return commercial.QuotationSource{}, ErrBusinessStoreUnavailable
	}
	source, err := store.GetQuotationSource(ctx, reference, s.quotationEnvironment)
	return source, mapCommercialError(err)
}

// Empty disables new quotation orders; existing receipts remain recoverable.
// The caller's HTTP body cannot select or override this trusted channel.
func WithQuotationEnvironment(environment string) Option {
	return func(s *Service) { s.quotationEnvironment = environment }
}

func (s *Service) CreateQuotationOrder(ctx context.Context, input commercial.QuotationOrderInput, actor string) (commercial.OrderRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCommercialOrderWrite); err != nil {
		return commercial.OrderRecord{}, err
	}
	if !commercialID.MatchString(actor) {
		return commercial.OrderRecord{}, ErrValidation
	}
	if err := input.Validate(); err != nil {
		return commercial.OrderRecord{}, mapCommercialError(err)
	}
	store, ok := s.store.(QuotationOrderStore)
	if !ok {
		return commercial.OrderRecord{}, ErrBusinessStoreUnavailable
	}
	// Never perform a mutable source preflight here before original recovery.
	r, err := store.CreateQuotationOrder(ctx, commercialObjectID("order", actor, input.OperationID), input, s.quotationEnvironment, actor)
	return r, mapCommercialError(err)
}
