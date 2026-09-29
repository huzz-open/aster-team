package mariadb

import (
	"context"
	"database/sql"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

func testPublicationLifecycle(t *testing.T, ctx context.Context, db *sql.DB, store *Store, definition commercial.Definition, actor string, now time.Time, suffix string) {
	definition.Code = "publication_" + suffix
	plan, err := store.FreezePlanVersion(ctx, "publication_plan_"+suffix, 0, definition, "publication_plan_"+suffix, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	request := commercial.CatalogRequest{OperationID: "publication_catalog_" + suffix, Environment: "local", Reason: "isolated publication", Plans: []commercial.CatalogSelection{{PlanID: plan.Snapshot.PlanID, Version: 1, ExpectedSHA256: plan.SHA256}}}
	revision := "catalog_" + commercial.ContentDigest([]byte("publication_" + suffix))[:48]
	preview, err := store.PreviewPublicCatalog(ctx, revision, request)
	if err != nil {
		t.Fatal(err)
	}
	_, err = store.ApprovePublicCatalog(ctx, revision, commercial.ApproveCatalogInput{Request: request, ExpectedPublicSHA256: preview.SHA256}, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	head, err := store.GetPublicationHead(ctx, "local")
	if err != nil {
		t.Fatal(err)
	}
	input := commercial.PreparePublicationInput{OperationID: "pub_" + suffix, CatalogRevision: revision, BuildSHA256: strings.Repeat("a", 64), ExpectedActiveID: head, AcceptUntil: now.Add(time.Hour).Format("2006-01-02T15:04:05.000Z"), Reason: "isolated test only"}
	id := "publication_" + suffix
	if _, err := store.PreparePublication(ctx, id, input, actor, now); !errors.Is(err, commercial.ErrPublicationNotAccepted) {
		t.Fatal("unexported catalog accepted", err)
	}
	if _, err := store.MarkCatalogExported(ctx, revision, preview.SHA256, actor, now); err != nil {
		t.Fatal(err)
	}
	record, err := store.PreparePublication(ctx, id, input, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	secondInput := input
	secondInput.OperationID += "_other"
	second, err := store.PreparePublication(ctx, id+"_other", secondInput, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	testPublicationFailurePersistence(t, ctx, db, store, record, actor, suffix)
	proof := commercial.PublicationEvidence{Environment: "local", Origin: "http://127.0.0.1:26394", BuildSHA256: input.BuildSHA256, CatalogRevision: revision, CatalogSHA256: preview.SHA256, ObservedAt: time.Now().UTC().Format("2006-01-02T15:04:05.000Z")}
	wrong := proof
	wrong.Environment = "production"
	if _, err := store.AcceptPublication(ctx, id, record.SHA256, wrong, actor); err == nil {
		t.Fatal("local evidence activated production")
	}
	// Failure in the final audit must roll back the evidence and the head together.
	auditID := operationAuditID(id, "commercial.publication_accepted")
	if _, err := db.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES(?,?,'test.injected_failure','commercial',?,'{}',?)`, auditID, actor, id, now); err != nil {
		t.Fatal(err)
	}
	if _, err := store.AcceptPublication(ctx, id, record.SHA256, proof, actor); err == nil {
		t.Fatal("audit failure ignored")
	}
	if current, err := store.GetPublicationHead(ctx, "local"); err != nil || current != head {
		t.Fatal("partial head update", current, err)
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
			_, err := store.AcceptPublication(ctx, id, record.SHA256, proof, actor)
			results <- err
		}()
	}
	wg.Wait()
	close(results)
	for err := range results {
		if err != nil {
			t.Fatal("concurrent original receipt recovery failed", err)
		}
	}
	accepted, err := store.GetPublication(ctx, id)
	if err != nil || accepted.Status != "accepted" {
		t.Fatal("acceptance missing", err)
	}
	if events, err := store.ListPublicationFailures(ctx, id, 100); err != nil || len(events) != 1 {
		t.Fatal("acceptance discarded failure history", err, len(events))
	}
	if _, err := store.AcceptPublication(ctx, second.Snapshot.ID, second.SHA256, proof, actor); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("stale candidate replaced active head", err)
	}
	retry, err := store.PreparePublication(ctx, id, input, actor, now.Add(48*time.Hour))
	if err != nil || retry.AcceptedAt != accepted.AcceptedAt {
		t.Fatal("old request did not restore original receipt", err)
	}
	input.OperationID += "_rollback"
	input.ExpectedActiveID = id
	rollback, err := store.PreparePublication(ctx, id+"_rollback", input, actor, time.Now().UTC().Truncate(time.Millisecond))
	if err != nil {
		t.Fatal(err)
	}
	proof.ObservedAt = time.Now().UTC().Format("2006-01-02T15:04:05.000Z")
	if _, err := store.AcceptPublication(ctx, rollback.Snapshot.ID, rollback.SHA256, proof, actor); err != nil {
		t.Fatal(err)
	}
	if _, err := store.AcceptPublication(ctx, id, record.SHA256, wrong, actor); err != nil {
		t.Fatal("original receipt requires new proof", err)
	}
	if current, err := store.GetPublicationHead(ctx, "local"); err != nil || current != rollback.Snapshot.ID {
		t.Fatal("retry reactivated historical publication", err)
	}

	// Hold the real environment row lock beyond the approved acceptance deadline.
	input.OperationID += "_deadline"
	input.ExpectedActiveID = rollback.Snapshot.ID
	now = time.Now().UTC().Truncate(time.Millisecond)
	input.AcceptUntil = now.Add(1200 * time.Millisecond).Format("2006-01-02T15:04:05.000Z")
	late, err := store.PreparePublication(ctx, id+"_deadline", input, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	lock, err := db.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer lock.Rollback()
	var locked string
	if err := lock.QueryRowContext(ctx, "SELECT active_id FROM commercial_publication_heads WHERE environment='local' FOR UPDATE").Scan(&locked); err != nil {
		t.Fatal(err)
	}
	proof.ObservedAt = time.Now().UTC().Format("2006-01-02T15:04:05.000Z")
	done := make(chan error, 1)
	go func() {
		_, err := store.AcceptPublication(ctx, late.Snapshot.ID, late.SHA256, proof, actor)
		done <- err
	}()
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("expired waiter accepted")
		}
	case <-time.After(4 * time.Second):
		t.Fatal("deadline did not cancel lock wait")
	}
	if err := lock.Rollback(); err != nil {
		t.Fatal(err)
	}
	unchanged, err := store.GetPublication(ctx, late.Snapshot.ID)
	if err != nil || unchanged.Status != "prepared" {
		t.Fatal("expired evidence persisted", err)
	}
	if current, err := store.GetPublicationHead(ctx, "local"); err != nil || current != rollback.Snapshot.ID {
		t.Fatal("expired evidence moved head", err)
	}
	var audits int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE id=?", operationAuditID(late.Snapshot.ID, "commercial.publication_accepted")).Scan(&audits); err != nil || audits != 0 {
		t.Fatal("expired acceptance audit persisted", err, audits)
	}
	testQuotationOrders(t, ctx, db, store, accepted, actor, suffix)
}

func testPublicationFailurePersistence(t *testing.T, ctx context.Context, db *sql.DB, store *Store, publication commercial.PublicationRecord, actor, suffix string) {
	t.Helper()
	event := commercial.PublicationFailure{ID: "pubfail_" + suffix, PublicationID: publication.Snapshot.ID, PublicationSHA256: publication.SHA256, Stage: "verification", Code: "content_unverified", OperatorID: actor, CreatedAt: time.Now().UTC().Format("2006-01-02T15:04:05.000Z")}
	auditID := operationAuditID(event.ID, "commercial.publication_attempt_failed")
	if _, err := db.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES(?,?,'test.injected_failure','commercial',?,'{}',?)`, auditID, actor, event.PublicationID, time.Now()); err != nil {
		t.Fatal(err)
	}
	if err := store.RecordPublicationFailure(ctx, event); err == nil {
		t.Fatal("failure audit rejection ignored")
	}
	if events, err := store.ListPublicationFailures(ctx, event.PublicationID, 100); err != nil || len(events) != 0 {
		t.Fatal("failure event partially committed", err)
	}
	if _, err := db.ExecContext(ctx, "DELETE FROM audit_events WHERE id=? AND action='test.injected_failure'", auditID); err != nil {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	results := make(chan error, 4)
	for range 4 {
		wg.Add(1)
		go func() { defer wg.Done(); results <- store.RecordPublicationFailure(ctx, event) }()
	}
	wg.Wait()
	close(results)
	for err := range results {
		if err != nil {
			t.Fatal(err)
		}
	}
	events, err := store.ListPublicationFailures(ctx, event.PublicationID, 100)
	if err != nil || len(events) != 1 || events[0] != event {
		t.Fatal("original failure event not recovered", err)
	}
	changed := event
	changed.Code = "target_not_configured"
	if err := store.RecordPublicationFailure(ctx, changed); !errors.Is(err, commercial.ErrConflict) {
		t.Fatal("failure event overwritten", err)
	}
	changed.ID += "_bad"
	changed.PublicationSHA256 = strings.Repeat("0", 64)
	if err := store.RecordPublicationFailure(ctx, changed); !errors.Is(err, commercial.ErrInvalidPublication) {
		t.Fatal("unbound failure accepted", err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_publication_failures SET content_sha256=? WHERE id=?", strings.Repeat("0", 64), event.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := store.ListPublicationFailures(ctx, event.PublicationID, 100); !errors.Is(err, commercial.ErrInvalidPublication) {
		t.Fatal("corrupt failure history accepted", err)
	}
	raw, err := event.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_publication_failures SET content_sha256=? WHERE id=?", commercial.ContentDigest(raw), event.ID); err != nil {
		t.Fatal(err)
	}
	current, err := store.GetPublication(ctx, event.PublicationID)
	if err != nil || current.Status != "prepared" || current.SHA256 != publication.SHA256 {
		t.Fatal("recording failure mutated publication", err)
	}
}
