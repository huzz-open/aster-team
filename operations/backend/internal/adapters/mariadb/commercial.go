package mariadb

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
	mysqldriver "github.com/go-sql-driver/mysql"
)

// FreezePlanVersion appends one full version. The head is only a concurrency cursor;
// neither an existing version nor an order is rewritten when the head advances.
func (store *Store) FreezePlanVersion(ctx context.Context, planID string, expectedVersion uint32, definition commercial.Definition, operationID, operatorID string, now time.Time) (commercial.PlanVersionRecord, error) {
	if expectedVersion == math.MaxUint32 {
		return commercial.PlanVersionRecord{}, commercial.ErrConflict
	}
	frozen, err := commercial.FreezePlan(planID, expectedVersion+1, definition)
	if err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	var result commercial.PlanVersionRecord
	err = retryCommercial(ctx, func() error {
		var err error
		result, err = store.freezePlanVersionOnce(ctx, frozen, expectedVersion, operationID, operatorID, now, nil)
		return err
	})
	return result, err
}
func (store *Store) freezePlanVersionOnce(ctx context.Context, frozen commercial.FrozenPlan, expected uint32, operationID, operatorID string, now time.Time, draft *commercial.PlanDraftRecord) (commercial.PlanVersionRecord, error) {
	snapshot, err := frozen.Snapshot()
	if err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	if operationID == "" || len(operationID) > 128 || operatorID == "" {
		return commercial.PlanVersionRecord{}, errors.New("operation and operator are required")
	}
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	defer tx.Rollback()
	existing, err := scanCommercialPlan(tx.QueryRowContext(ctx, commercialPlanSelect+" WHERE operation_id=? FOR UPDATE", operationID))
	if err == nil {
		if existing.SHA256 != frozen.Digest() || existing.CreatedBy != operatorID {
			return commercial.PlanVersionRecord{}, commercial.ErrConflict
		}
		if draft != nil {
			if err := validateDraftFreezeRetry(ctx, tx, operationID, *draft); err != nil {
				return commercial.PlanVersionRecord{}, err
			}
		}
		return existing, tx.Commit()
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return commercial.PlanVersionRecord{}, err
	}
	if draft != nil {
		if err := validateDraftFreeze(ctx, tx, *draft); err != nil {
			return commercial.PlanVersionRecord{}, err
		}
	}
	var code string
	var current uint32
	err = tx.QueryRowContext(ctx, "SELECT code,current_version FROM commercial_plan_heads WHERE id=? FOR UPDATE", snapshot.PlanID).Scan(&code, &current)
	if errors.Is(err, sql.ErrNoRows) {
		if expected != 0 {
			return commercial.PlanVersionRecord{}, commercial.ErrNotFound
		}
		if _, err = tx.ExecContext(ctx, "INSERT INTO commercial_plan_heads(id,code,current_version,created_at,updated_at) VALUES(?,?,0,?,?)", snapshot.PlanID, snapshot.Definition.Code, now, now); err != nil {
			return commercial.PlanVersionRecord{}, commercialConflict(err)
		}
	} else if err != nil {
		return commercial.PlanVersionRecord{}, err
	} else if current != expected {
		return commercial.PlanVersionRecord{}, commercial.ErrPlanVersionConflict
	} else if code != snapshot.Definition.Code {
		return commercial.PlanVersionRecord{}, commercial.ErrConflict
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO commercial_plan_versions(plan_id,version_no,operation_id,snapshot_json,content_sha256,created_by,created_at) VALUES(?,?,?,?,?,?,?)`, snapshot.PlanID, snapshot.Version, operationID, frozen.Bytes(), frozen.Digest(), operatorID, now); err != nil {
		return commercial.PlanVersionRecord{}, commercialConflict(err)
	}
	result, err := tx.ExecContext(ctx, "UPDATE commercial_plan_heads SET current_version=?,updated_at=? WHERE id=? AND current_version=?", snapshot.Version, now, snapshot.PlanID, expected)
	if err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	affected, err := result.RowsAffected()
	if err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	if affected != 1 {
		return commercial.PlanVersionRecord{}, commercial.ErrConflict
	}
	if err = writeCommercialAudit(ctx, tx, operationID, "commercial.plan_frozen", snapshot.PlanID, operatorID, frozen.Digest(), now); err != nil {
		return commercial.PlanVersionRecord{}, err
	}
	if draft != nil {
		if err = writeCommercialAudit(ctx, tx, operationID, "commercial.plan_draft_frozen", draft.Snapshot.DraftID, operatorID, draft.SHA256, now); err != nil {
			return commercial.PlanVersionRecord{}, err
		}
	}
	record := commercial.PlanVersionRecord{Snapshot: snapshot, SHA256: frozen.Digest(), OperationID: operationID, CreatedBy: operatorID, CreatedAt: now}
	return record, tx.Commit()
}

const commercialPlanSelect = `SELECT plan_id,version_no,snapshot_json,content_sha256,operation_id,created_by,created_at FROM commercial_plan_versions`

type commercialScanner interface{ Scan(...any) error }

func scanCommercialPlan(row commercialScanner) (commercial.PlanVersionRecord, error) {
	var raw []byte
	var record commercial.PlanVersionRecord
	var planID string
	var version uint32
	if err := row.Scan(&planID, &version, &raw, &record.SHA256, &record.OperationID, &record.CreatedBy, &record.CreatedAt); err != nil {
		return record, err
	}
	frozen, err := commercial.ParseFrozenPlan(raw, record.SHA256)
	if err != nil {
		return record, err
	}
	record.Snapshot, err = frozen.Snapshot()
	if err == nil && (record.Snapshot.PlanID != planID || record.Snapshot.Version != version) {
		return commercial.PlanVersionRecord{}, errors.New("plan snapshot differs from stored identity")
	}
	return record, err
}
func (store *Store) GetCommercialPlan(ctx context.Context, planID string, version uint32) (commercial.PlanVersionRecord, error) {
	record, err := scanCommercialPlan(store.db.QueryRowContext(ctx, commercialPlanSelect+" WHERE plan_id=? AND version_no=?", planID, version))
	if errors.Is(err, sql.ErrNoRows) {
		err = commercial.ErrNotFound
	}
	return record, err
}
func (store *Store) GetCurrentCommercialPlan(ctx context.Context, planID string) (commercial.PlanVersionRecord, error) {
	record, err := scanCommercialPlan(store.db.QueryRowContext(ctx, commercialPlanSelect+" WHERE plan_id=? AND version_no=(SELECT current_version FROM commercial_plan_heads WHERE id=?)", planID, planID))
	if errors.Is(err, sql.ErrNoRows) {
		err = commercial.ErrNotFound
	}
	return record, err
}
func (store *Store) ListCommercialPlans(ctx context.Context, limit int) ([]commercial.PlanVersionRecord, error) {
	if limit < 1 || limit > 100 {
		limit = 50
	}
	rows, err := store.db.QueryContext(ctx, `SELECT v.plan_id,v.version_no,v.snapshot_json,v.content_sha256,v.operation_id,v.created_by,v.created_at FROM commercial_plan_heads h JOIN commercial_plan_versions v ON v.plan_id=h.id AND v.version_no=h.current_version ORDER BY h.updated_at DESC,h.id LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	records := make([]commercial.PlanVersionRecord, 0)
	for rows.Next() {
		record, err := scanCommercialPlan(rows)
		if err != nil {
			return nil, err
		}
		records = append(records, record)
	}
	return records, rows.Err()
}

func (store *Store) CreateCommercialOrder(ctx context.Context, orderID, operationID, customerID, planID string, version, years uint32, starts time.Time, operatorID string, now time.Time) (commercial.OrderRecord, error) {
	var result commercial.OrderRecord
	err := retryCommercial(ctx, func() error {
		var err error
		result, err = store.createCommercialOrderOnce(ctx, orderID, operationID, customerID, planID, version, years, starts, operatorID, now)
		return err
	})
	return result, err
}
func (store *Store) createCommercialOrderOnce(ctx context.Context, orderID, operationID, customerID, planID string, version, years uint32, starts time.Time, operatorID string, now time.Time) (commercial.OrderRecord, error) {
	if operationID == "" || len(operationID) > 128 || operatorID == "" {
		return commercial.OrderRecord{}, errors.New("operation and operator are required")
	}
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return commercial.OrderRecord{}, err
	}
	defer tx.Rollback()
	plan, err := scanCommercialPlan(tx.QueryRowContext(ctx, commercialPlanSelect+" WHERE plan_id=? AND version_no=?", planID, version))
	if err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			err = commercial.ErrNotFound
		}
		return commercial.OrderRecord{}, err
	}
	frozen, err := commercial.FreezePlan(plan.Snapshot.PlanID, plan.Snapshot.Version, plan.Snapshot.Definition)
	if err != nil {
		return commercial.OrderRecord{}, err
	}
	snapshot, err := commercial.CreateOrderSnapshot(orderID, customerID, frozen, years, starts)
	if err != nil {
		return commercial.OrderRecord{}, err
	}
	raw, err := snapshot.Bytes()
	if err != nil {
		return commercial.OrderRecord{}, err
	}
	digest := commercial.ContentDigest(raw)
	existing, err := scanCommercialOrder(tx.QueryRowContext(ctx, commercialOrderSelect+" WHERE operation_id=? FOR UPDATE", operationID))
	if err == nil {
		if existing.SHA256 != digest || existing.CreatedBy != operatorID {
			return commercial.OrderRecord{}, commercial.ErrConflict
		}
		return existing, tx.Commit()
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return commercial.OrderRecord{}, err
	}
	var activeCustomer string
	if err = tx.QueryRowContext(ctx, "SELECT id FROM customers WHERE id=? AND status<>'inactive' FOR UPDATE", customerID).Scan(&activeCustomer); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			err = commercial.ErrNotFound
		}
		return commercial.OrderRecord{}, err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO commercial_orders(id,operation_id,customer_id,plan_id,plan_version,snapshot_json,content_sha256,status,created_by,created_at,updated_at) VALUES(?,?,?,?,?,?,?,'pending_payment',?,?,?)`, orderID, operationID, customerID, planID, version, raw, digest, operatorID, now, now); err != nil {
		return commercial.OrderRecord{}, commercialConflict(err)
	}
	if err = writeCommercialAudit(ctx, tx, operationID, "commercial.order_created", orderID, operatorID, digest, now); err != nil {
		return commercial.OrderRecord{}, err
	}
	record := commercial.OrderRecord{Snapshot: snapshot, SHA256: digest, OperationID: operationID, Status: "pending_payment", CreatedBy: operatorID, CreatedAt: now}
	return record, tx.Commit()
}

const commercialOrderSelect = `SELECT id,customer_id,plan_id,plan_version,snapshot_json,content_sha256,operation_id,status,created_by,created_at FROM commercial_orders`

func scanCommercialOrder(row commercialScanner) (commercial.OrderRecord, error) {
	var raw []byte
	var record commercial.OrderRecord
	var orderID, customerID, planID string
	var version uint32
	if err := row.Scan(&orderID, &customerID, &planID, &version, &raw, &record.SHA256, &record.OperationID, &record.Status, &record.CreatedBy, &record.CreatedAt); err != nil {
		return record, err
	}
	var err error
	record.Snapshot, err = commercial.ParseOrderSnapshot(raw, record.SHA256)
	if err == nil && (record.Snapshot.OrderID != orderID || record.Snapshot.CustomerID != customerID || record.Snapshot.Plan.PlanID != planID || record.Snapshot.Plan.Version != version) {
		return commercial.OrderRecord{}, errors.New("order snapshot differs from stored identity")
	}
	if err == nil && record.Snapshot.Source != nil && record.Snapshot.Source.OrderedAt != record.CreatedAt.UTC().Format("2006-01-02T15:04:05.000Z") {
		return commercial.OrderRecord{}, commercial.ErrInvalidQuotationOrder
	}
	if err == nil && record.Snapshot.Source != nil {
		// Match the original v2 receipt regardless of the database driver's zone.
		record.CreatedAt = record.CreatedAt.UTC()
	}
	return record, err
}
func (store *Store) GetCommercialOrder(ctx context.Context, orderID string) (commercial.OrderRecord, error) {
	record, err := scanCommercialOrder(store.db.QueryRowContext(ctx, commercialOrderSelect+" WHERE id=?", orderID))
	if errors.Is(err, sql.ErrNoRows) {
		err = commercial.ErrNotFound
	}
	return record, err
}

const commercialOrderListSelect = `SELECT o.id,o.customer_id,o.plan_id,o.plan_version,o.snapshot_json,o.content_sha256,o.operation_id,o.status,o.created_by,o.created_at,c.name,COALESCE(f.id,''),COALESCE(f.status,'') FROM commercial_orders o JOIN customers c ON c.id=o.customer_id LEFT JOIN commercial_paid_fulfillments f ON f.order_id=o.id`

func commercialOrderListWhere(query application.CommercialOrderListQuery) (string, []any) {
	conditions := make([]string, 0, 3)
	arguments := make([]any, 0, 6)
	if query.Keyword != "" {
		pattern := "%" + query.Keyword + "%"
		conditions = append(conditions, "(o.id LIKE ? OR o.customer_id LIKE ? OR c.name LIKE ? OR c.legal_name LIKE ?)")
		arguments = append(arguments, pattern, pattern, pattern, pattern)
	}
	if query.Status != "" {
		conditions = append(conditions, "o.status=?")
		arguments = append(arguments, query.Status)
	}
	switch query.Stage {
	case "all":
		conditions = append(conditions, "o.status IN ('pending_payment','fulfillment_pending','fulfilled')")
	case "pending_payment":
		conditions = append(conditions, "o.status='pending_payment'")
	case "pending_approval":
		conditions = append(conditions, "o.status='fulfillment_pending' AND f.id IS NULL")
	case "pending_issue":
		conditions = append(conditions, "o.status='fulfillment_pending' AND f.status IN ('approved','prepared')")
	case "issued":
		conditions = append(conditions, "o.status='fulfilled' AND f.status='issued'")
	}
	if len(conditions) == 0 {
		return "", arguments
	}
	return " WHERE " + strings.Join(conditions, " AND "), arguments
}

func scanCommercialOrderProjection(row commercialScanner) (commercial.OrderRecord, error) {
	var raw []byte
	var record commercial.OrderRecord
	var orderID, customerID, planID string
	var version uint32
	if err := row.Scan(&orderID, &customerID, &planID, &version, &raw, &record.SHA256, &record.OperationID, &record.Status, &record.CreatedBy, &record.CreatedAt, &record.CustomerName, &record.FulfillmentID, &record.FulfillmentStatus); err != nil {
		return record, err
	}
	var err error
	record.Snapshot, err = commercial.ParseOrderSnapshot(raw, record.SHA256)
	if err == nil && (record.Snapshot.OrderID != orderID || record.Snapshot.CustomerID != customerID || record.Snapshot.Plan.PlanID != planID || record.Snapshot.Plan.Version != version) {
		return commercial.OrderRecord{}, errors.New("order snapshot differs from stored identity")
	}
	if err == nil && record.Snapshot.Source != nil && record.Snapshot.Source.OrderedAt != record.CreatedAt.UTC().Format("2006-01-02T15:04:05.000Z") {
		return commercial.OrderRecord{}, commercial.ErrInvalidQuotationOrder
	}
	if err == nil && record.Snapshot.Source != nil {
		record.CreatedAt = record.CreatedAt.UTC()
	}
	return record, err
}

func (store *Store) ListCommercialOrders(ctx context.Context, query application.CommercialOrderListQuery) (application.CommercialOrderPage, error) {
	if query.Limit < 1 || query.Limit > 100 {
		query.Limit = 50
	}
	where, arguments := commercialOrderListWhere(query)
	var total int
	if err := store.db.QueryRowContext(ctx, "SELECT COUNT(*) FROM commercial_orders o JOIN customers c ON c.id=o.customer_id LEFT JOIN commercial_paid_fulfillments f ON f.order_id=o.id"+where, arguments...).Scan(&total); err != nil {
		return application.CommercialOrderPage{}, err
	}
	pageArguments := append(append([]any{}, arguments...), query.Limit, query.Offset)
	rows, err := store.db.QueryContext(ctx, commercialOrderListSelect+where+" ORDER BY o.created_at DESC,o.id LIMIT ? OFFSET ?", pageArguments...)
	if err != nil {
		return application.CommercialOrderPage{}, err
	}
	defer rows.Close()
	records := make([]commercial.OrderRecord, 0)
	for rows.Next() {
		record, err := scanCommercialOrderProjection(rows)
		if err != nil {
			return application.CommercialOrderPage{}, err
		}
		records = append(records, record)
	}
	return application.CommercialOrderPage{Items: records, Total: total}, rows.Err()
}

func retryCommercial(ctx context.Context, run func() error) error {
	var last error
	for attempt := 0; attempt < 4; attempt++ {
		last = run()
		if last == nil || !retryableCommercialTransaction(last) {
			return last
		}
		timer := time.NewTimer(time.Duration(attempt+1) * 20 * time.Millisecond)
		select {
		case <-ctx.Done():
			timer.Stop()
			return ctx.Err()
		case <-timer.C:
		}
	}
	return last
}
func commercialConflict(err error) error {
	var driver *mysqldriver.MySQLError
	if errors.As(err, &driver) && driver.Number == 1062 {
		return fmt.Errorf("%w: duplicate operation or version", commercial.ErrConflict)
	}
	return err
}
func writeCommercialAudit(ctx context.Context, tx *sql.Tx, operationID, action, resourceID, operatorID, digest string, now time.Time) error {
	payload, err := json.Marshal(map[string]string{"operation_id": operationID, "content_sha256": digest})
	if err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES(?,?,?,?,?,?,?)`, operationAuditID(operationID, action), operatorID, action, "commercial", resourceID, payload, now)
	return err
}

// MariaDB may report ER_CHECKREAD (1020) for a serializable read/write race.
// Retrying always opens a new transaction and rechecks the expected version.
func retryableCommercialTransaction(err error) bool {
	if retryableMariaDBTransaction(err) {
		return true
	}
	var driver *mysqldriver.MySQLError
	return errors.As(err, &driver) && driver.Number == 1020
}
