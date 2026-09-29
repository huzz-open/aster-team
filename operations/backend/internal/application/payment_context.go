package application

import (
	"context"

	"aster.local/team/operations/backend/internal/commercial"
)

// PaymentContext is the narrow order projection needed to confirm receipt.
// It does not grant access to plan/publication administration or customer lists.
type PaymentContext struct {
	OrderID     string `json:"order_id"`
	CustomerID  string `json:"customer_id"`
	OrderSHA256 string `json:"order_sha256"`
	AmountMinor int64  `json:"amount_minor"`
	Currency    string `json:"currency"`
	StartsAt    string `json:"starts_at"`
	EndsAt      string `json:"ends_at"`
	Status      string `json:"status"`
	Source      string `json:"source"`
}

func (s *Service) GetCommercialPaymentContext(ctx context.Context, orderID, actor string) (PaymentContext, error) {
	if err := s.RequirePermission(ctx, actor, PermissionPaymentConfirm); err != nil {
		return PaymentContext{}, err
	}
	if !commercialID.MatchString(orderID) {
		return PaymentContext{}, ErrValidation
	}
	store, ok := s.store.(CommercialStore)
	if !ok {
		return PaymentContext{}, ErrBusinessStoreUnavailable
	}
	order, err := store.GetCommercialOrder(ctx, orderID)
	if err != nil {
		return PaymentContext{}, mapCommercialError(err)
	}
	raw, err := order.Snapshot.Bytes()
	if err != nil || commercial.ContentDigest(raw) != order.SHA256 {
		return PaymentContext{}, commercial.ErrPaymentIntegrity
	}
	source := "manual"
	if order.Snapshot.Source != nil {
		source = order.Snapshot.Source.Environment
	}
	v := order.Snapshot
	return PaymentContext{OrderID: v.OrderID, CustomerID: v.CustomerID, OrderSHA256: order.SHA256, AmountMinor: v.AmountMinor, Currency: v.Currency, StartsAt: v.StartsAt, EndsAt: v.EndsAt, Status: order.Status, Source: source}, nil
}
