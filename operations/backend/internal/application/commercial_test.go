package application

import (
	"context"
	"errors"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

type commercialPermissionStore struct {
	*fakeStore
	allowed map[string]bool
	err     error
	checked []string
}

func (store *commercialPermissionStore) HasPermission(_ context.Context, operator, permission string) (bool, error) {
	store.checked = append(store.checked, operator+":"+permission)
	return store.allowed[permission], store.err
}

func TestCommercialServiceRequiresExactPermissionsBeforeBusinessStorage(t *testing.T) {
	cases := []struct {
		name, permission string
		run              func(*Service) error
	}{
		{"preview_catalog", PermissionCatalogRead, func(s *Service) error {
			_, e := s.PreviewPublicCatalog(context.Background(), commercial.CatalogRequest{}, "operator_1")
			return e
		}},
		{"approve_catalog", PermissionCatalogApprove, func(s *Service) error {
			_, e := s.ApprovePublicCatalog(context.Background(), commercial.ApproveCatalogInput{}, "operator_1")
			return e
		}},
		{"get_catalog", PermissionCatalogRead, func(s *Service) error {
			_, e := s.GetCatalogApproval(context.Background(), "invalid", "operator_1")
			return e
		}},
		{"list_catalog", PermissionCatalogRead, func(s *Service) error {
			_, e := s.ListCatalogApprovals(context.Background(), 10, "operator_1")
			return e
		}},
		{"export_catalog", PermissionCatalogExport, func(s *Service) error {
			_, e := s.ExportPublicCatalog(context.Background(), "invalid", "operator_1")
			return e
		}},
		{"download_catalog", PermissionCatalogRead, func(s *Service) error {
			_, _, e := s.DownloadPublicCatalog(context.Background(), "invalid", "operator_1")
			return e
		}},
		{"save_draft", PermissionCommercialPlanWrite, func(s *Service) error {
			_, e := s.SavePlanDraft(context.Background(), SavePlanDraftInput{}, "operator_1")
			return e
		}},
		{"freeze_draft", PermissionCommercialPlanWrite, func(s *Service) error {
			_, e := s.FreezePlanDraft(context.Background(), FreezePlanDraftInput{}, "operator_1")
			return e
		}},
		{"get_draft", PermissionCommercialPlanRead, func(s *Service) error {
			_, e := s.GetPlanDraft(context.Background(), "draft_1", 0, "operator_1")
			return e
		}},
		{"list_drafts", PermissionCommercialPlanRead, func(s *Service) error {
			_, e := s.ListPlanDrafts(context.Background(), 10, "operator_1")
			return e
		}},
		{"freeze", PermissionCommercialPlanWrite, func(s *Service) error {
			_, e := s.FreezeCommercialPlan(context.Background(), FreezeCommercialPlanInput{}, "operator_1")
			return e
		}},
		{"create_order", PermissionCommercialOrderWrite, func(s *Service) error {
			_, e := s.CreateCommercialOrder(context.Background(), CommercialOrderInput{}, "operator_1")
			return e
		}},
		{"get_plan", PermissionCommercialPlanRead, func(s *Service) error {
			_, e := s.GetCommercialPlan(context.Background(), "plan_1", 1, "operator_1")
			return e
		}},
		{"get_current_plan", PermissionCommercialPlanRead, func(s *Service) error {
			_, e := s.GetCurrentCommercialPlan(context.Background(), "plan_1", "operator_1")
			return e
		}},
		{"list_plans", PermissionCommercialPlanRead, func(s *Service) error {
			_, e := s.ListCommercialPlans(context.Background(), 10, "operator_1")
			return e
		}},
		{"get_order", PermissionCommercialOrderRead, func(s *Service) error {
			_, e := s.GetCommercialOrder(context.Background(), "order_1", "operator_1")
			return e
		}},
		{"list_orders", PermissionCommercialOrderRead, func(s *Service) error {
			_, e := s.ListCommercialOrders(context.Background(), CommercialOrderListQuery{Limit: 10}, "operator_1")
			return e
		}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			// No commercial store exists: permission denial must win over parsing or
			// storage availability, including direct non-HTTP callers.
			store := &commercialPermissionStore{fakeStore: &fakeStore{}, allowed: map[string]bool{PermissionReleaseBuild: true}}
			service := NewService(store, time.Hour)
			if err := tc.run(service); !errors.Is(err, ErrUnauthorized) {
				t.Fatalf("denial = %v", err)
			}
			if len(store.checked) != 1 || store.checked[0] != "operator_1:"+tc.permission {
				t.Fatalf("wrong permission: %v", store.checked)
			}
			failure := errors.New("permission database unavailable")
			store.err = failure
			if err := tc.run(service); !errors.Is(err, failure) {
				t.Fatalf("lookup failure was ignored: %v", err)
			}
			store.err = nil
			store.allowed[tc.permission] = true
			if err := tc.run(service); errors.Is(err, ErrUnauthorized) {
				t.Fatalf("exact permission rejected: %v", err)
			}
		})
	}
}
