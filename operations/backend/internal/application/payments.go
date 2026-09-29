package application

import (
	"context"
	"errors"

	"aster.local/team/operations/backend/internal/commercial"
)

const PermissionPaymentConfirm = "commercial.payment.confirm"

type CommercialPaymentStore interface {
	GetCommercialPayment(context.Context, string) (commercial.PaymentRecord, error)
	ConfirmCommercialPayment(context.Context, string, string, commercial.ConfirmPaymentInput, string) (commercial.PaymentRecord, error)
}

func (s *Service) GetCommercialPayment(ctx context.Context, orderID, actor string) (commercial.PaymentRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionCommercialOrderRead); err != nil {
		if !errors.Is(err, ErrUnauthorized) {
			return commercial.PaymentRecord{}, err
		}
		if err := s.RequirePermission(ctx, actor, PermissionPaymentConfirm); err != nil {
			return commercial.PaymentRecord{}, err
		}
	}
	if !commercialID.MatchString(orderID) {
		return commercial.PaymentRecord{}, ErrValidation
	}
	store, ok := s.store.(CommercialPaymentStore)
	if !ok {
		return commercial.PaymentRecord{}, ErrBusinessStoreUnavailable
	}
	r, err := store.GetCommercialPayment(ctx, orderID)
	return r, mapCommercialError(err)
}
func (s *Service) ConfirmCommercialPayment(ctx context.Context, orderID string, in commercial.ConfirmPaymentInput, actor string) (commercial.PaymentRecord, error) {
	if err := s.RequirePermission(ctx, actor, PermissionPaymentConfirm); err != nil {
		return commercial.PaymentRecord{}, err
	}
	if !commercialID.MatchString(orderID) || !commercialID.MatchString(actor) {
		return commercial.PaymentRecord{}, ErrValidation
	}
	if err := in.Validate(); err != nil {
		return commercial.PaymentRecord{}, mapCommercialError(err)
	}
	store, ok := s.store.(CommercialPaymentStore)
	if !ok {
		return commercial.PaymentRecord{}, ErrBusinessStoreUnavailable
	}
	r, err := store.ConfirmCommercialPayment(ctx, commercialObjectID("payment", actor, in.OperationID), orderID, in, actor)
	return r, mapCommercialError(err)
}
