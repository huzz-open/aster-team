package mariadb

import (
	"reflect"
	"testing"

	"aster.local/team/operations/backend/internal/application"
)

func TestCommercialOrderListWhereBuildsWorkflowFilters(t *testing.T) {
	tests := []struct {
		name      string
		query     application.CommercialOrderListQuery
		wantWhere string
		wantArgs  []any
	}{
		{name: "unfiltered", wantArgs: []any{}},
		{
			name:      "search and status",
			query:     application.CommercialOrderListQuery{Keyword: "Acme", Status: "fulfilled"},
			wantWhere: " WHERE (o.id LIKE ? OR o.customer_id LIKE ? OR c.name LIKE ? OR c.legal_name LIKE ?) AND o.status=?",
			wantArgs:  []any{"%Acme%", "%Acme%", "%Acme%", "%Acme%", "fulfilled"},
		},
		{
			name:      "waiting for request",
			query:     application.CommercialOrderListQuery{Stage: "pending_approval"},
			wantWhere: " WHERE o.status='fulfillment_pending' AND f.id IS NULL",
			wantArgs:  []any{},
		},
		{
			name:      "waiting for issue",
			query:     application.CommercialOrderListQuery{Stage: "pending_issue"},
			wantWhere: " WHERE o.status='fulfillment_pending' AND f.status IN ('approved','prepared')",
			wantArgs:  []any{},
		},
		{
			name:      "issued",
			query:     application.CommercialOrderListQuery{Stage: "issued"},
			wantWhere: " WHERE o.status='fulfilled' AND f.status='issued'",
			wantArgs:  []any{},
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			where, arguments := commercialOrderListWhere(test.query)
			if where != test.wantWhere || !reflect.DeepEqual(arguments, test.wantArgs) {
				t.Fatalf("where=%q arguments=%#v", where, arguments)
			}
		})
	}
}
