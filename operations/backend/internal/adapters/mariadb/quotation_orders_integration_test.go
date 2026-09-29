package mariadb

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"sync"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/commercial"
)

func testQuotationOrders(t *testing.T, ctx context.Context, db *sql.DB, store *Store, p commercial.PublicationRecord, actor, suffix string) {
	t.Helper()
	now := time.Now().UTC().Truncate(time.Millisecond)
	customer := "quote_customer_" + suffix
	if _, err := db.ExecContext(ctx, `INSERT INTO customers(id,name,status,notes,created_at,updated_at) VALUES(?,?,'active','test-only',?,?)`, customer, "Quote test", now, now); err != nil {
		t.Fatal(err)
	}
	in := commercial.QuotationOrderInput{OperationID: "quote_" + suffix, CustomerID: customer, PublicationID: p.Snapshot.ID, CatalogRevision: p.Snapshot.Catalog.ID, PlanID: p.Snapshot.Catalog.Plans[0].PlanID, PlanVersion: 1, Years: 1, StartsAt: now.Add(24 * time.Hour).Format("2006-01-02T15:04:05.000Z")}
	id := "quote_order_" + suffix
	initialHead, err := store.GetPublicationHead(ctx, "local")
	if err != nil {
		t.Fatal(err)
	}
	for i := range 101 {
		r := p.Snapshot.Request
		r.OperationID = fmt.Sprintf("quote_noise_%s_%d", suffix, i)
		r.ExpectedActiveID = initialHead
		if _, err := store.PreparePublication(ctx, r.OperationID, r, actor, now); err != nil {
			t.Fatal(err)
		}
	}
	recent, err := store.ListPublications(ctx, 100)
	if err != nil {
		t.Fatal(err)
	}
	for _, r := range recent {
		if r.Snapshot.ID == p.Snapshot.ID {
			t.Fatal("fixture source was not outside recent window")
		}
	}
	for _, reference := range []string{p.Snapshot.ID, p.Snapshot.Catalog.ID} {
		source, err := store.GetQuotationSource(ctx, reference, "local")
		if err != nil || source.CatalogRevision != in.CatalogRevision || len(source.Plans) != 1 || source.Plans[0].PlanID != in.PlanID {
			t.Fatal("historical exact source unavailable", err)
		}
	}
	if _, err := store.GetQuotationSource(ctx, p.Snapshot.ID, "production"); !errors.Is(err, commercial.ErrPublicationNotAccepted) {
		t.Fatal("source query crossed environment", err)
	}
	for _, channel := range []string{"", "production", "invalid"} {
		if _, err := store.CreateQuotationOrder(ctx, id, in, channel, actor); !errors.Is(err, commercial.ErrPublicationNotAccepted) {
			t.Fatal("untrusted channel accepted", channel, err)
		}
	}
	bad := in
	bad.PlanVersion++
	if _, err := store.CreateQuotationOrder(ctx, id, bad, "local", actor); !errors.Is(err, commercial.ErrPublicationNotAccepted) {
		t.Fatal("spliced version accepted", err)
	}
	// Force the final audit to fail: order and source must roll back together.
	auditID := operationAuditID(in.OperationID, "commercial.quotation_order_created")
	if _, err := db.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES(?,?,'test.injected_failure','commercial',?,'{}',?)`, auditID, actor, id, now); err != nil {
		t.Fatal(err)
	}
	if _, err := store.CreateQuotationOrder(ctx, id, in, "local", actor); err == nil {
		t.Fatal("audit failure ignored")
	}
	if _, err := store.GetCommercialOrder(ctx, id); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatal("partial order persisted", err)
	}
	if _, err := db.ExecContext(ctx, "DELETE FROM audit_events WHERE id=? AND action='test.injected_failure'", auditID); err != nil {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	errorsCh := make(chan error, 6)
	for range 6 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_, err := store.CreateQuotationOrder(ctx, id, in, "local", actor)
			errorsCh <- err
		}()
	}
	wg.Wait()
	close(errorsCh)
	for err := range errorsCh {
		if err != nil {
			t.Fatal("concurrent recovery", err)
		}
	}
	order, err := store.GetCommercialOrder(ctx, id)
	if err != nil || order.Snapshot.Source == nil || order.Snapshot.Source.PublicationSHA256 != p.SHA256 || order.Snapshot.PlanSHA256 != p.Snapshot.Catalog.Request.Plans[0].ExpectedSHA256 {
		t.Fatal("incorrect fixed source", err)
	}
	page, err := store.ListCommercialOrders(ctx, application.CommercialOrderListQuery{Limit: 100})
	if err != nil {
		t.Fatal(err)
	}
	listed := page.Items
	found := false
	for _, item := range listed {
		if item.Snapshot.OrderID == id {
			found = item.SHA256 == order.SHA256
		}
	}
	if !found {
		t.Fatal("v2 missing from list")
	}
	if _, err := store.CreateQuotationOrder(ctx, id, bad, "local", actor); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("changed retry accepted", err)
	}
	starts, _ := time.Parse(time.RFC3339Nano, in.StartsAt)
	if _, err := store.CreateCommercialOrder(ctx, id, in.OperationID, customer, in.PlanID, 1, 1, starts, actor, now); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("quote downgraded through manual entry", err)
	}
	manual := in
	manual.OperationID += "_manual"
	if _, err := store.CreateCommercialOrder(ctx, id+"_manual", manual.OperationID, customer, in.PlanID, 1, 1, starts, actor, now); err != nil {
		t.Fatal(err)
	}
	if _, err := store.CreateQuotationOrder(ctx, id+"_manual", manual, "local", actor); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("manual order upgraded by retry", err)
	}
	// Create a real short-lived accepted event, then wait on a customer lock past
	// its deadline. Do not edit an immutable event to manufacture expiry.
	head, err := store.GetPublicationHead(ctx, "local")
	if err != nil {
		t.Fatal(err)
	}
	request := p.Snapshot.Request
	request.OperationID = "short_" + suffix
	request.ExpectedActiveID = head
	request.AcceptUntil = time.Now().Add(1800 * time.Millisecond).UTC().Format("2006-01-02T15:04:05.000Z")
	short, err := store.PreparePublication(ctx, "short_"+suffix, request, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	proof := *p.Evidence
	proof.ObservedAt = time.Now().UTC().Format("2006-01-02T15:04:05.000Z")
	short, err = store.AcceptPublication(ctx, short.Snapshot.ID, short.SHA256, proof, actor)
	if err != nil {
		t.Fatal(err)
	}
	shortIn := in
	shortIn.OperationID += "_short"
	shortIn.PublicationID = short.Snapshot.ID
	shortOrder, err := store.CreateQuotationOrder(ctx, id+"_short", shortIn, "local", actor)
	if err != nil {
		t.Fatal(err)
	}
	lock, err := db.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer lock.Rollback()
	if _, err := lock.ExecContext(ctx, "UPDATE customers SET notes='held quote test' WHERE id=?", customer); err != nil {
		t.Fatal(err)
	}
	late := shortIn
	late.OperationID += "_late"
	if _, err := store.CreateQuotationOrder(ctx, id+"_late", late, "local", actor); err == nil {
		t.Fatal("lock wait crossed deadline and created order")
	}
	if err := lock.Rollback(); err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetCommercialOrder(ctx, id+"_late"); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatal("expired order exists", err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE customers SET status='inactive' WHERE id=?", customer); err != nil {
		t.Fatal(err)
	}
	recovered, err := store.CreateQuotationOrder(ctx, id+"_short", shortIn, "", actor)
	if err != nil || recovered.SHA256 != shortOrder.SHA256 {
		t.Fatal("original receipt lost after expiry/customer disable/channel unset", err)
	}
	recovered, err = store.CreateQuotationOrder(ctx, id, in, "production", actor)
	if err != nil || recovered.SHA256 != order.SHA256 {
		t.Fatal("old head receipt changed", err)
	}
	var count int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE id=?", auditID).Scan(&count); err != nil || count != 1 {
		t.Fatal("duplicate audit", err, count)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_orders SET created_at=DATE_ADD(created_at,INTERVAL 1 SECOND) WHERE id=?", id); err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetCommercialOrder(ctx, id); err == nil {
		t.Fatal("source/stored timestamp mismatch accepted")
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_orders SET created_at=? WHERE id=?", order.CreatedAt, id); err != nil {
		t.Fatal(err)
	}
	testCommercialPayments(t, ctx, db, store, shortOrder, actor, suffix)
	testPaidQuotationFulfillment(t, ctx, db, store, shortOrder, actor, suffix)
}
