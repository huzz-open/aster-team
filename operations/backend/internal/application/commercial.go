package application

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"regexp"
	"slices"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/strictjson"
)

// CommercialStore is the v2 full-snapshot path. Legacy plan/price records remain
// readable until their explicit compatibility mapping is approved.
type CommercialStore interface {
	FreezePlanVersion(context.Context, string, uint32, commercial.Definition, string, string, time.Time) (commercial.PlanVersionRecord, error)
	GetCommercialPlan(context.Context, string, uint32) (commercial.PlanVersionRecord, error)
	ListCommercialPlans(context.Context, int) ([]commercial.PlanVersionRecord, error)
	CreateCommercialOrder(context.Context, string, string, string, string, uint32, uint32, time.Time, string, time.Time) (commercial.OrderRecord, error)
	GetCommercialOrder(context.Context, string) (commercial.OrderRecord, error)
	ListCommercialOrders(context.Context, CommercialOrderListQuery) (CommercialOrderPage, error)
}

type CommercialOrderListQuery struct {
	Limit   int
	Offset  int
	Keyword string
	Status  string
	Stage   string
}

type CommercialOrderPage struct {
	Items []commercial.OrderRecord `json:"items"`
	Total int                      `json:"total"`
}

type CurrentCommercialPlanStore interface {
	GetCurrentCommercialPlan(context.Context, string) (commercial.PlanVersionRecord, error)
}

const (
	PermissionCommercialPlanRead   = "commercial.plan.read"
	PermissionCommercialPlanWrite  = "commercial.plan.write"
	PermissionCommercialOrderRead  = "commercial.order.read"
	PermissionCommercialOrderWrite = "commercial.order.write"
	PermissionDistributionRead     = "commercial.distribution.read"
	PermissionDistributionApprove  = "commercial.distribution.approve"
	PermissionLicenseIssueV2       = "commercial.license.issue"
)

// Used only when creating the initial administrator. Existing accounts are not
// silently promoted by a migration; the local administration command grants them.
func BootstrapCommercialPermissions() []string {
	return []string{PermissionCommercialPlanRead, PermissionCommercialPlanWrite, PermissionCommercialOrderRead, PermissionCommercialOrderWrite, PermissionPaymentConfirm, PermissionFulfillmentRead, PermissionFulfillmentApprove, PermissionDistributionRead, PermissionDistributionApprove, PermissionLicenseIssueV2, PermissionCatalogRead, PermissionCatalogApprove, PermissionCatalogExport, PermissionPublicationRead, PermissionPublicationPrepare, PermissionPublicationAccept}
}

var commercialID = regexp.MustCompile(`^[A-Za-z0-9._:@+/-]{1,64}$`)
var commercialOperation = regexp.MustCompile(`^[A-Za-z0-9._:@+/-]{1,128}$`)

type FreezeCommercialPlanInput struct {
	OperationID     string                `json:"operation_id"`
	PlanID          string                `json:"plan_id"`
	ExpectedVersion uint32                `json:"expected_version"`
	Definition      commercial.Definition `json:"definition"`
}

func (value *FreezeCommercialPlanInput) UnmarshalJSON(data []byte) error {
	type wire FreezeCommercialPlanInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "plan_id", "expected_version", "definition"); err != nil {
		return err
	}
	*value = FreezeCommercialPlanInput(result)
	return nil
}

type CommercialOrderInput struct {
	OperationID string    `json:"operation_id"`
	CustomerID  string    `json:"customer_id"`
	PlanID      string    `json:"plan_id"`
	PlanVersion uint32    `json:"plan_version"`
	Years       uint32    `json:"years"`
	StartsAt    time.Time `json:"starts_at"`
}

func (value *CommercialOrderInput) UnmarshalJSON(data []byte) error {
	type wire CommercialOrderInput
	var result wire
	if err := strictjson.Object(data, &result, "operation_id", "customer_id", "plan_id", "plan_version", "years", "starts_at"); err != nil {
		return err
	}
	*value = CommercialOrderInput(result)
	return nil
}
func (service *Service) FreezeCommercialPlan(ctx context.Context, input FreezeCommercialPlanInput, operatorID string) (commercial.PlanVersionRecord, error) {
	if err := service.RequirePermission(ctx, operatorID, PermissionCommercialPlanWrite); err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	if !commercialOperation.MatchString(input.OperationID) || !commercialID.MatchString(operatorID) || (input.PlanID != "" && !commercialID.MatchString(input.PlanID)) || (input.PlanID == "" && input.ExpectedVersion != 0) {
		return commercial.PlanVersionRecord{}, fmt.Errorf("%w: invalid plan operation identity", ErrValidation)
	}
	if err := input.Definition.Validate(); err != nil {
		return commercial.PlanVersionRecord{}, fmt.Errorf("%w: %v", ErrValidation, err)
	}
	store, ok := service.store.(CommercialStore)
	if !ok {
		return commercial.PlanVersionRecord{}, ErrBusinessStoreUnavailable
	}
	planID := input.PlanID
	if planID == "" {
		planID = commercialObjectID("plan", operatorID, input.OperationID)
	}
	record, err := store.FreezePlanVersion(ctx, planID, input.ExpectedVersion, input.Definition, input.OperationID, operatorID, service.now().UTC().Truncate(time.Millisecond))
	return record, mapCommercialError(err)
}
func (service *Service) GetCommercialPlan(ctx context.Context, planID string, version uint32, operatorID string) (commercial.PlanVersionRecord, error) {
	if err := service.RequirePermission(ctx, operatorID, PermissionCommercialPlanRead); err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	if !commercialID.MatchString(planID) || version == 0 {
		return commercial.PlanVersionRecord{}, fmt.Errorf("%w: invalid plan version", ErrValidation)
	}
	store, ok := service.store.(CommercialStore)
	if !ok {
		return commercial.PlanVersionRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.GetCommercialPlan(ctx, planID, version)
	return record, mapCommercialError(err)
}
func (service *Service) ListCommercialPlans(ctx context.Context, limit int, operatorID string) ([]commercial.PlanVersionRecord, error) {
	if err := service.RequirePermission(ctx, operatorID, PermissionCommercialPlanRead); err != nil {
		return nil, err
	}
	store, ok := service.store.(CommercialStore)
	if !ok {
		return nil, ErrBusinessStoreUnavailable
	}
	return store.ListCommercialPlans(ctx, boundedCommercialLimit(limit))
}
func (service *Service) GetCurrentCommercialPlan(ctx context.Context, planID, operatorID string) (commercial.PlanVersionRecord, error) {
	if err := service.RequirePermission(ctx, operatorID, PermissionCommercialPlanRead); err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	if !commercialID.MatchString(planID) {
		return commercial.PlanVersionRecord{}, fmt.Errorf("%w: invalid plan identity", ErrValidation)
	}
	store, ok := service.store.(CurrentCommercialPlanStore)
	if !ok {
		return commercial.PlanVersionRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.GetCurrentCommercialPlan(ctx, planID)
	return record, mapCommercialError(err)
}
func (service *Service) CreateCommercialOrder(ctx context.Context, input CommercialOrderInput, operatorID string) (commercial.OrderRecord, error) {
	if err := service.RequirePermission(ctx, operatorID, PermissionCommercialOrderWrite); err != nil {
		return commercial.OrderRecord{}, err
	}
	if !commercialOperation.MatchString(input.OperationID) || !commercialID.MatchString(operatorID) || !commercialID.MatchString(input.CustomerID) || !commercialID.MatchString(input.PlanID) || input.PlanVersion == 0 || input.Years < 1 || input.Years > 5 || input.StartsAt.IsZero() || input.StartsAt.Nanosecond()%1_000_000 != 0 {
		return commercial.OrderRecord{}, fmt.Errorf("%w: invalid order selection or start time", ErrValidation)
	}
	store, ok := service.store.(CommercialStore)
	if !ok {
		return commercial.OrderRecord{}, ErrBusinessStoreUnavailable
	}
	// Inspect the fixed version to report invalid selections before persistence.
	plan, err := store.GetCommercialPlan(ctx, input.PlanID, input.PlanVersion)
	if err != nil {
		return commercial.OrderRecord{}, mapCommercialError(err)
	}
	frozen, err := commercial.FreezePlan(plan.Snapshot.PlanID, plan.Snapshot.Version, plan.Snapshot.Definition)
	if err != nil {
		return commercial.OrderRecord{}, err
	}
	orderID := commercialObjectID("order", operatorID, input.OperationID)
	if _, err := commercial.CreateOrderSnapshot(orderID, input.CustomerID, frozen, input.Years, input.StartsAt); err != nil {
		return commercial.OrderRecord{}, fmt.Errorf("%w: %v", ErrValidation, err)
	}
	// The transaction re-reads the immutable version and derives every amount and right itself.
	record, err := store.CreateCommercialOrder(ctx, orderID, input.OperationID, input.CustomerID, input.PlanID, input.PlanVersion, input.Years, input.StartsAt.UTC(), operatorID, service.now().UTC().Truncate(time.Millisecond))
	return record, mapCommercialError(err)
}
func (service *Service) GetCommercialOrder(ctx context.Context, orderID string, operatorID string) (commercial.OrderRecord, error) {
	if err := service.RequirePermission(ctx, operatorID, PermissionCommercialOrderRead); err != nil {
		return commercial.OrderRecord{}, err
	}
	if !commercialID.MatchString(orderID) {
		return commercial.OrderRecord{}, fmt.Errorf("%w: invalid order ID", ErrValidation)
	}
	store, ok := service.store.(CommercialStore)
	if !ok {
		return commercial.OrderRecord{}, ErrBusinessStoreUnavailable
	}
	record, err := store.GetCommercialOrder(ctx, orderID)
	return record, mapCommercialError(err)
}
func (service *Service) ListCommercialOrders(ctx context.Context, query CommercialOrderListQuery, operatorID string) (CommercialOrderPage, error) {
	if err := service.RequirePermission(ctx, operatorID, PermissionCommercialOrderRead); err != nil {
		return CommercialOrderPage{}, err
	}
	store, ok := service.store.(CommercialStore)
	if !ok {
		return CommercialOrderPage{}, ErrBusinessStoreUnavailable
	}
	query.Limit = boundedCommercialLimit(query.Limit)
	if query.Offset < 0 || query.Offset > 10_000_000 {
		return CommercialOrderPage{}, ErrValidation
	}
	query.Keyword = strings.TrimSpace(query.Keyword)
	if len(query.Keyword) > 160 {
		return CommercialOrderPage{}, ErrValidation
	}
	if query.Status != "" && !slices.Contains([]string{"pending_payment", "fulfillment_pending", "fulfilled", "cancelled", "refunded"}, query.Status) {
		return CommercialOrderPage{}, ErrValidation
	}
	if query.Stage != "" && !slices.Contains([]string{"all", "pending_payment", "pending_approval", "pending_issue", "issued"}, query.Stage) {
		return CommercialOrderPage{}, ErrValidation
	}
	return store.ListCommercialOrders(ctx, query)
}
func boundedCommercialLimit(limit int) int {
	if limit < 1 || limit > 100 {
		return 50
	}
	return limit
}
func commercialObjectID(prefix, operatorID, operationID string) string {
	sum := sha256.Sum256([]byte(operatorID + "\x00" + operationID))
	return prefix + "_" + hex.EncodeToString(sum[:24])
}
func mapCommercialError(err error) error {
	if errors.Is(err, commercial.ErrInvalidPaidFulfillment) || errors.Is(err, commercial.ErrInvalidPaidTransfer) || errors.Is(err, commercial.ErrInvalidPaidRedelivery) {
		return fmt.Errorf("%w: %w", ErrValidation, err)
	}
	if errors.Is(err, commercial.ErrInvalidPayment) {
		return fmt.Errorf("%w: %w", ErrValidation, err)
	}
	if errors.Is(err, commercial.ErrInvalidQuotationOrder) {
		return fmt.Errorf("%w: %w", ErrValidation, err)
	}
	if errors.Is(err, commercial.ErrInvalidDistribution) || errors.Is(err, commercial.ErrInvalidCatalog) || errors.Is(err, commercial.ErrInvalidPublication) || errors.Is(err, commercial.ErrPublicationNotAccepted) {
		return fmt.Errorf("%w: %w", ErrValidation, err)
	}
	if errors.Is(err, commercial.ErrNotFound) {
		return ErrNotFound
	}
	if errors.Is(err, commercial.ErrConflict) {
		return fmt.Errorf("%w: %w", ErrValidation, err)
	}
	return err
}
