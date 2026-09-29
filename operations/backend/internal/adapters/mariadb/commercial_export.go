package mariadb

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"

	"aster.local/team/operations/backend/internal/domain"
)

// Each query explicitly selects report fields from a fixed table allowlist.
// All queries run in ExportOperations' one repeatable-read transaction; no
// credentials or signing configuration is selected. Nested predecessor License
// documents are removed from business snapshots before serialization.
func listCommercialExportRecords(ctx context.Context, database queryContext) ([]domain.OperationsCommercialRecord, error) {
	queries := []struct{ kind, sql string }{
		{"plan_draft", `SELECT CONCAT(draft_id, ':', revision_no), 'recorded', snapshot_json, content_sha256, '' FROM commercial_plan_draft_revisions ORDER BY 1`},
		{"plan_version", `SELECT CONCAT(plan_id, ':', version_no), 'frozen', snapshot_json, content_sha256, '' FROM commercial_plan_versions ORDER BY 1`},
		{"order", `SELECT id, status, snapshot_json, content_sha256, '' FROM commercial_orders ORDER BY 1`},
		{"payment", `SELECT id, 'confirmed', snapshot_json, content_sha256, '' FROM commercial_payment_confirmations ORDER BY 1`},
		{"free_distribution", `SELECT id, status, snapshot_json, content_sha256, COALESCE(document_sha256, '') FROM commercial_free_distributions ORDER BY 1`},
		{"paid_fulfillment", `SELECT id, status, snapshot_json, content_sha256, COALESCE(document_sha256, '') FROM commercial_paid_fulfillments ORDER BY 1`},
		{"paid_redelivery", `SELECT id, 'recorded', snapshot_json, content_sha256, document_sha256 FROM commercial_paid_redeliveries ORDER BY 1`},
		{"paid_transfer", `SELECT id, status, snapshot_json, content_sha256, COALESCE(document_sha256, '') FROM commercial_paid_transfers ORDER BY 1`},
		{"paid_lifecycle", `SELECT fulfillment_id, 'recorded', snapshot_json, content_sha256, document_sha256 FROM commercial_paid_lifecycle_sources ORDER BY 1`},
		{"catalog", `SELECT id, status, snapshot_json, content_sha256, '' FROM commercial_catalog_approvals ORDER BY 1`},
		{"publication", `SELECT id, status, snapshot_json, content_sha256, '' FROM commercial_catalog_publications ORDER BY 1`},
		{"publication_failure", `SELECT id, 'recorded', event_json, content_sha256, '' FROM commercial_publication_failures ORDER BY 1`},
	}
	items := []domain.OperationsCommercialRecord{}
	for _, query := range queries {
		rows, err := database.QueryContext(ctx, query.sql)
		if err != nil {
			return nil, fmt.Errorf("export %s: %w", query.kind, err)
		}
		for rows.Next() {
			item := domain.OperationsCommercialRecord{Kind: query.kind}
			var snapshot []byte
			if err := rows.Scan(&item.ID, &item.Status, &snapshot, &item.SourceSnapshotSHA256, &item.DocumentSHA256); err != nil {
				rows.Close()
				return nil, err
			}
			item.Snapshot, err = projectCommercialExportSnapshot(snapshot)
			if err != nil {
				rows.Close()
				return nil, fmt.Errorf("project %s: %w", query.kind, err)
			}
			items = append(items, item)
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return nil, err
		}
	}
	return items, nil
}

// Report projections must not carry predecessor License documents embedded in
// lifecycle snapshots. UseNumber preserves integer amounts and quota values.
func projectCommercialExportSnapshot(snapshot []byte) (json.RawMessage, error) {
	decoder := json.NewDecoder(bytes.NewReader(snapshot))
	decoder.UseNumber()
	var value any
	if err := decoder.Decode(&value); err != nil {
		return nil, err
	}
	if _, ok := value.(map[string]any); !ok {
		return nil, fmt.Errorf("snapshot must be an object")
	}
	redactCommercialDocuments(value)
	return json.Marshal(value)
}

func redactCommercialDocuments(value any) {
	switch item := value.(type) {
	case map[string]any:
		delete(item, "document")
		for _, nested := range item {
			redactCommercialDocuments(nested)
		}
	case []any:
		for _, nested := range item {
			redactCommercialDocuments(nested)
		}
	}
}
