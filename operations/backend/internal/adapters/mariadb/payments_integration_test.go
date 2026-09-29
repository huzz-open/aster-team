package mariadb

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"sync"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

func testCommercialPayments(t *testing.T, ctx context.Context, db *sql.DB, store *Store, order commercial.OrderRecord, actor, suffix string) {
	t.Helper()
	now := time.Now().UTC().Truncate(time.Millisecond)
	in := commercial.ConfirmPaymentInput{OperationID: "pay_" + suffix, ExpectedOrderSHA256: order.SHA256, PaymentReference: "test-bank-" + suffix, ReceivedAt: now.Format("2006-01-02T15:04:05.000Z"), Notes: "isolated full payment"}
	id := "payment_" + suffix
	orderID := order.Snapshot.OrderID
	auditID := operationAuditID(in.OperationID, "commercial.payment_confirmed")
	if _, err := db.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES(?,?,'test.injected_failure','commercial',?,'{}',?)`, auditID, actor, id, now); err != nil {
		t.Fatal(err)
	}
	if _, err := store.ConfirmCommercialPayment(ctx, id, orderID, in, actor); err == nil {
		t.Fatal("audit failure ignored")
	}
	if _, err := store.GetCommercialPayment(ctx, orderID); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatal("partial payment persisted", err)
	}
	if current, err := store.GetCommercialOrder(ctx, orderID); err != nil || current.Status != "pending_payment" {
		t.Fatal("partial order transition", err)
	}
	if _, err := db.ExecContext(ctx, "DELETE FROM audit_events WHERE id=? AND action='test.injected_failure'", auditID); err != nil {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	results := make(chan error, 6)
	for range 6 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, err := store.ConfirmCommercialPayment(ctx, id, orderID, in, actor)
			results <- err
		}()
	}
	wg.Wait()
	close(results)
	for err := range results {
		if err != nil {
			t.Fatal("same operation recovery", err)
		}
	}
	receipt, err := store.GetCommercialPayment(ctx, orderID)
	if err != nil {
		t.Fatal(err)
	}
	originalRaw, _ := receipt.Snapshot.Bytes()
	if receipt.Snapshot.OrderSHA256 != order.SHA256 || receipt.Snapshot.Order.AmountMinor != order.Snapshot.AmountMinor {
		t.Fatal("receipt lost sale")
	}
	for _, mutate := range []func(*commercial.ConfirmPaymentInput){
		func(v *commercial.ConfirmPaymentInput) { v.OperationID += "_second" },
		func(v *commercial.ConfirmPaymentInput) { v.ExpectedOrderSHA256 = receipt.SHA256 },
		func(v *commercial.ConfirmPaymentInput) { v.PaymentReference += "_changed" },
		func(v *commercial.ConfirmPaymentInput) { v.Notes += "_changed" },
		func(v *commercial.ConfirmPaymentInput) {
			v.ReceivedAt = now.Add(-time.Second).Format("2006-01-02T15:04:05.000Z")
		},
	} {
		bad := in
		mutate(&bad)
		if _, err := store.ConfirmCommercialPayment(ctx, id, orderID, bad, actor); !errors.Is(err, commercial.ErrConflict) {
			t.Fatal("changed input recovered", err)
		}
	}
	if _, err := store.ConfirmCommercialPayment(ctx, id, orderID, in, actor+"_other"); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("actor changed", err)
	}
	// A later lifecycle state cannot erase the original confirmed receipt.
	if _, err := db.ExecContext(ctx, "UPDATE commercial_orders SET status='fulfilled' WHERE id=?", orderID); err != nil {
		t.Fatal(err)
	}
	if got, err := store.ConfirmCommercialPayment(ctx, id, orderID, in, actor); err != nil || got.SHA256 != receipt.SHA256 {
		t.Fatal("historical receipt changed", err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_orders SET status='fulfillment_pending' WHERE id=?", orderID); err != nil {
		t.Fatal(err)
	}
	// PAY-R01: every embedded digest remains valid, but this is not the real order.
	forged := receipt
	start, _ := time.Parse(time.RFC3339Nano, order.Snapshot.StartsAt)
	plan, err := commercial.FreezePlan(order.Snapshot.Plan.PlanID, order.Snapshot.Plan.Version, order.Snapshot.Plan.Definition)
	if err != nil {
		t.Fatal(err)
	}
	forgedOrder, err := commercial.CreateOrderSnapshot(orderID, order.Snapshot.CustomerID, plan, order.Snapshot.Years, start.AddDate(0, 0, 1))
	if err != nil {
		t.Fatal(err)
	}
	forgedOrder.Schema = order.Snapshot.Schema
	forgedOrder.Source = order.Snapshot.Source
	forged.Snapshot.Order = forgedOrder
	orderRaw, _ := forgedOrder.Bytes()
	forged.Snapshot.OrderSHA256 = commercial.ContentDigest(orderRaw)
	forged.Snapshot.Request.ExpectedOrderSHA256 = forged.Snapshot.OrderSHA256
	forgedRaw, err := forged.Snapshot.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	forged.SHA256 = commercial.ContentDigest(forgedRaw)
	if err := forged.Validate(); err != nil {
		t.Fatal("test must use intrinsically valid record", err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_payment_confirmations SET snapshot_json=?,content_sha256=?,order_sha256=? WHERE id=?", forgedRaw, forged.SHA256, forged.Snapshot.OrderSHA256, id); err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetCommercialPayment(ctx, orderID); !errors.Is(err, commercial.ErrPaymentIntegrity) {
		t.Fatal("forged self-consistent receipt read", err)
	}
	if _, err := store.ConfirmCommercialPayment(ctx, id, orderID, forged.Snapshot.Request, actor); !errors.Is(err, commercial.ErrPaymentIntegrity) {
		t.Fatal("forged receipt restored", err)
	}
	if _, err := store.ConfirmCommercialPayment(ctx, id, orderID, in, actor); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("original expected digest lost", err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_payment_confirmations SET snapshot_json=?,content_sha256=?,order_sha256=? WHERE id=?", originalRaw, receipt.SHA256, order.SHA256, id); err != nil {
		t.Fatal(err)
	}
	var count int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM commercial_payment_confirmations WHERE order_id=?", orderID).Scan(&count); err != nil || count != 1 {
		t.Fatal("duplicate payment", err, count)
	}
	// Different simultaneous operations can produce only one full receipt.
	manualID := "pay_race_order_" + suffix
	raceCustomer := "pay_customer_" + suffix
	if _, err := db.ExecContext(ctx, `INSERT INTO customers(id,name,status,notes,created_at,updated_at) VALUES(?,?,'active','payment race',?,?)`, raceCustomer, "Payment race", now, now); err != nil {
		t.Fatal(err)
	}
	manual, err := store.CreateCommercialOrder(ctx, manualID, manualID, raceCustomer, order.Snapshot.Plan.PlanID, order.Snapshot.Plan.Version, 1, start, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	results = make(chan error, 4)
	for i := range 4 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			v := in
			v.OperationID = fmt.Sprintf("pay_race_%s_%d", suffix, i)
			v.ExpectedOrderSHA256 = manual.SHA256
			_, err := store.ConfirmCommercialPayment(ctx, v.OperationID, manualID, v, actor)
			results <- err
		}()
	}
	wg.Wait()
	close(results)
	success := 0
	for err := range results {
		if err == nil {
			success++
		} else if !errors.Is(err, commercial.ErrConflict) {
			t.Fatal(err)
		}
	}
	if success != 1 {
		t.Fatal("multiple initial receipts", success)
	}
}
