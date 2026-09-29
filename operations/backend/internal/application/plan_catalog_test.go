package application

import (
	"aster.local/team/operations/backend/internal/domain"
	"context"
	"errors"
	"testing"
)

func TestCreatePlanRejectsInvalidCapabilitiesBeforePersistence(t *testing.T) {
	service := &Service{}
	for _, features := range [][]string{nil, {"unimplemented"}, {"member", "member"}} {
		_, err := service.CreatePlan(context.Background(), domain.PlanInput{
			Code: "test", Name: "Test", Edition: "enterprise", Currency: "CNY",
			BillingCycle: "year", TaxMode: "inclusive", Features: features,
		}, "operator-test")
		if !errors.Is(err, ErrValidation) {
			t.Fatalf("features %v: expected validation failure, got %v", features, err)
		}
	}
}
