package mariadb

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"strings"
	"sync"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
)

func testPlanDraftLifecycle(t *testing.T, ctx context.Context, db *sql.DB, store *Store, definition commercial.Definition, actor string, now time.Time, suffix string) {
	t.Helper()
	definition.Code = "draftcode_" + suffix
	snapshot := commercial.PlanDraftSnapshot{Schema: commercial.PlanDraftSchema, DraftID: "draft_" + suffix, Revision: 1, PlanID: "draftplan_" + suffix, Definition: definition}
	first, err := store.SavePlanDraft(ctx, snapshot, "draftsave_"+suffix, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetCommercialPlan(ctx, snapshot.PlanID, 1); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatalf("draft created sellable version: %v", err)
	}
	retry, err := store.SavePlanDraft(ctx, snapshot, "draftsave_"+suffix, actor, now.Add(time.Hour))
	if err != nil || retry.SHA256 != first.SHA256 || !retry.CreatedAt.Equal(first.CreatedAt) {
		t.Fatalf("draft retry changed: %v", err)
	}
	snapshot.Definition.Name = "second revision"
	if _, err := store.SavePlanDraft(ctx, snapshot, "draftsave_"+suffix, actor, now); !errors.Is(err, commercial.ErrConflict) || errors.Is(err, commercial.ErrDraftRevisionConflict) {
		t.Fatalf("changed replay classified absent: %v", err)
	}
	snapshot.Revision = 2
	var wg sync.WaitGroup
	results := make(chan error, 2)
	for i := range 2 {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			_, e := store.SavePlanDraft(ctx, snapshot, fmt.Sprintf("draftrace_%s_%d", suffix, i), actor, now)
			results <- e
		}(i)
	}
	wg.Wait()
	close(results)
	passed, conflicts := 0, 0
	for e := range results {
		if e == nil {
			passed++
		} else if errors.Is(e, commercial.ErrDraftRevisionConflict) {
			conflicts++
		} else {
			t.Fatalf("concurrent draft: %v", e)
		}
	}
	if passed != 1 || conflicts != 1 {
		t.Fatalf("concurrent outcomes: %d %d", passed, conflicts)
	}
	second, err := store.GetPlanDraft(ctx, snapshot.DraftID, 0)
	if err != nil || second.Snapshot.Revision != 2 {
		t.Fatalf("head: %v", err)
	}
	old, err := store.GetPlanDraft(ctx, snapshot.DraftID, 1)
	if err != nil || old.SHA256 != first.SHA256 {
		t.Fatalf("history overwritten: %v", err)
	}
	if _, err := store.FreezePlanDraft(ctx, snapshot.DraftID, 1, first.SHA256, "staledraft_"+suffix, actor, now); !errors.Is(err, commercial.ErrDraftRevisionConflict) {
		t.Fatalf("old draft frozen: %v", err)
	}
	if _, err := store.FreezePlanDraft(ctx, snapshot.DraftID, 2, strings.Repeat("0", 64), "baddraft_"+suffix, actor, now); !errors.Is(err, commercial.ErrConflict) {
		t.Fatalf("wrong digest frozen: %v", err)
	}

	freezeOperation := "draftfreeze_" + suffix
	faultID := operationAuditID(freezeOperation, "commercial.plan_draft_frozen")
	if _, err := db.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at) VALUES(?,?,'test.injected_failure','commercial',?,'{}',?)`, faultID, actor, snapshot.DraftID, now); err != nil {
		t.Fatal(err)
	}
	if _, err := store.FreezePlanDraft(ctx, snapshot.DraftID, 2, second.SHA256, freezeOperation, actor, now); err == nil {
		t.Fatal("last audit insertion did not fail")
	}
	if _, err := store.GetCommercialPlan(ctx, snapshot.PlanID, 1); !errors.Is(err, commercial.ErrNotFound) {
		t.Fatalf("partial freeze survived rollback: %v", err)
	}
	var remainingHead, remainingAudit int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM commercial_plan_heads WHERE id=?", snapshot.PlanID).Scan(&remainingHead); err != nil || remainingHead != 0 {
		t.Fatalf("plan head survived rollback: %d %v", remainingHead, err)
	}
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE id=?", operationAuditID(freezeOperation, "commercial.plan_frozen")).Scan(&remainingAudit); err != nil || remainingAudit != 0 {
		t.Fatalf("first audit survived rollback: %d %v", remainingAudit, err)
	}
	if _, err := db.ExecContext(ctx, "DELETE FROM audit_events WHERE id=? AND action='test.injected_failure'", faultID); err != nil {
		t.Fatal(err)
	}
	frozen, err := store.FreezePlanDraft(ctx, snapshot.DraftID, 2, second.SHA256, freezeOperation, actor, now)
	if err != nil || frozen.Snapshot.Definition.Name != "second revision" {
		t.Fatalf("freeze: %v", err)
	}
	// A newer draft must not stop exact response recovery of the committed freeze.
	snapshot.Revision = 3
	snapshot.ExpectedVersion = 1
	snapshot.Definition.Name = "third revision"
	third, err := store.SavePlanDraft(ctx, snapshot, "draftthird_"+suffix, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	again, err := store.FreezePlanDraft(ctx, snapshot.DraftID, 2, second.SHA256, freezeOperation, actor, now.Add(time.Hour))
	if err != nil || again.SHA256 != frozen.SHA256 || !again.CreatedAt.Equal(frozen.CreatedAt) {
		t.Fatalf("freeze retry after editing: %v", err)
	}
	if originalSave, err := store.SavePlanDraft(ctx, first.Snapshot, first.OperationID, actor, now.Add(time.Hour)); err != nil || originalSave.SHA256 != first.SHA256 {
		t.Fatalf("save response recovery after later revisions: %v", err)
	}
	// Same resulting plan digest is insufficient: the original freeze must also
	// refer to this exact draft source, not another equal-looking working copy.
	other := second.Snapshot
	other.DraftID = "otherdraft_" + suffix
	other.Revision = 1
	otherDraft, err := store.SavePlanDraft(ctx, other, "otherdraftop_"+suffix, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.FreezePlanDraft(ctx, other.DraftID, 1, otherDraft.SHA256, freezeOperation, actor, now); !errors.Is(err, commercial.ErrConflict) {
		t.Fatalf("same plan from a different source reused original freeze: %v", err)
	}
	if _, err := store.FreezePlanDraft(ctx, snapshot.DraftID, 3, third.SHA256, freezeOperation, actor, now); !errors.Is(err, commercial.ErrConflict) {
		t.Fatalf("different source replay: %v", err)
	}
	// A competing fixed plan changes only the expected plan cursor, not the draft.
	if _, err := store.FreezePlanVersion(ctx, snapshot.PlanID, 1, definition, "draftcompeting_"+suffix, actor, now); err != nil {
		t.Fatal(err)
	}
	if _, err := store.FreezePlanDraft(ctx, snapshot.DraftID, 3, third.SHA256, "draftstaleplan_"+suffix, actor, now); !errors.Is(err, commercial.ErrPlanVersionConflict) {
		t.Fatalf("stale plan base accepted: %v", err)
	}
	original, err := store.GetCommercialPlan(ctx, snapshot.PlanID, 1)
	if err != nil || original.SHA256 != frozen.SHA256 {
		t.Fatalf("fixed version rewritten: %v", err)
	}
	// Reads reject altered content even when it points to an otherwise valid draft.
	forged := third.Snapshot
	forged.DraftID = "other_" + suffix
	raw, err := forged.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_plan_draft_revisions SET snapshot_json=?,content_sha256=? WHERE draft_id=? AND revision_no=3", raw, commercial.ContentDigest(raw), snapshot.DraftID); err != nil {
		t.Fatal(err)
	}
	if _, err := store.GetPlanDraft(ctx, snapshot.DraftID, 0); err == nil {
		t.Fatal("different draft identity with recomputed digest accepted")
	}
	raw, err = third.Snapshot.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(ctx, "UPDATE commercial_plan_draft_revisions SET snapshot_json=?,content_sha256=? WHERE draft_id=? AND revision_no=3", raw, third.SHA256, snapshot.DraftID); err != nil {
		t.Fatal(err)
	}
	var auditCount int
	if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM audit_events WHERE resource_id=? AND action='commercial.plan_draft_frozen'", snapshot.DraftID).Scan(&auditCount); err != nil || auditCount != 1 {
		t.Fatalf("freeze audit duplicated: %d %v", auditCount, err)
	}
	// Saving a new revision and freezing the old one share the draft-head lock.
	// Either ordering is valid, but a successful freeze must contain the old,
	// explicitly reviewed source, and a rejected freeze must leave no plan head.
	race := first.Snapshot
	race.DraftID = "freezerace_" + suffix
	race.PlanID = "freezeraceplan_" + suffix
	race.Definition.Code = "freezeracecode_" + suffix
	raceDraft, err := store.SavePlanDraft(ctx, race, "freezeracesave_"+suffix, actor, now)
	if err != nil {
		t.Fatal(err)
	}
	changedRace := race
	changedRace.Revision = 2
	changedRace.Definition.Name = "edited during freeze"
	start := make(chan struct{})
	saved := make(chan error, 1)
	go func() {
		<-start
		_, err := store.SavePlanDraft(ctx, changedRace, "freezeraceedit_"+suffix, actor, now)
		saved <- err
	}()
	close(start)
	raceFrozen, freezeErr := store.FreezePlanDraft(ctx, race.DraftID, 1, raceDraft.SHA256, "freezeraceop_"+suffix, actor, now)
	if err := <-saved; err != nil {
		t.Fatalf("concurrent edit failed: %v", err)
	}
	if freezeErr == nil {
		if raceFrozen.Snapshot.Definition.Name != race.Definition.Name {
			t.Fatal("freeze mixed in a newer draft")
		}
	} else if errors.Is(freezeErr, commercial.ErrDraftRevisionConflict) {
		var heads int
		if err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM commercial_plan_heads WHERE id=?", race.PlanID).Scan(&heads); err != nil || heads != 0 {
			t.Fatalf("rejected racing freeze created a plan: %d %v", heads, err)
		}
	} else {
		t.Fatalf("unexpected freezing race: %v", freezeErr)
	}
}
