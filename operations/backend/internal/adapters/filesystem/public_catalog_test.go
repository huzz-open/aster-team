package filesystem

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"syscall"
	"testing"

	"aster.local/team/operations/backend/internal/commercial"
)

func requireSymlink(t *testing.T, oldname, newname string) {
	t.Helper()
	if err := os.Symlink(oldname, newname); err != nil {
		if runtime.GOOS == "windows" && errors.Is(err, syscall.Errno(1314)) {
			t.Skipf("symlink fixture requires Windows symbolic-link privilege: %v", err)
		}
		t.Fatal(err)
	}
}

func publicCatalogRecord(t *testing.T) commercial.CatalogApprovalRecord {
	t.Helper()
	raw, err := os.ReadFile("../../../../../contracts/test-vectors/plan-definition.v1.json")
	if err != nil {
		t.Fatal(err)
	}
	var definition commercial.Definition
	if err := json.Unmarshal(raw, &definition); err != nil {
		t.Fatal(err)
	}
	frozen, err := commercial.FreezePlan("plan_1", 1, definition)
	if err != nil {
		t.Fatal(err)
	}
	plan, err := frozen.Snapshot()
	if err != nil {
		t.Fatal(err)
	}
	request := commercial.CatalogRequest{OperationID: "catalog_export_test", Environment: "local", Reason: "INTERNAL_REASON", Plans: []commercial.CatalogSelection{{PlanID: "plan_1", Version: 1, ExpectedSHA256: frozen.Digest()}}}
	id := "catalog_" + strings.Repeat("a", 48)
	preview, err := commercial.BuildPublicCatalog(id, request, []commercial.PlanSnapshot{plan})
	if err != nil {
		t.Fatal(err)
	}
	snapshot := commercial.CatalogApprovalSnapshot{Schema: commercial.CatalogApprovalSchema, ID: id, Request: request, Plans: []commercial.PlanSnapshot{plan}, PublicSHA256: preview.SHA256, ApprovedBy: "operator_1", ApprovedAt: "2026-09-06T00:00:00.000Z"}
	raw, err = snapshot.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	return commercial.CatalogApprovalRecord{Snapshot: snapshot, SHA256: commercial.ContentDigest(raw), Public: preview, Status: "approved"}
}
func TestPublicCatalogExportIsImmutableConcurrentAndRecoversMissingFiles(t *testing.T) {
	directory := t.TempDir()
	exporter, err := NewPublicCatalogExporter(directory)
	if err != nil {
		t.Fatal(err)
	}
	defer exporter.Close()
	r := publicCatalogRecord(t)
	ctx := context.Background()
	var wg sync.WaitGroup
	results := make(chan error, 8)
	for range 8 {
		wg.Add(1)
		go func() { defer wg.Done(); results <- exporter.Export(ctx, r) }()
	}
	wg.Wait()
	close(results)
	for err := range results {
		if err != nil {
			t.Fatal(err)
		}
	}
	expected, err := r.PublicBytes()
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(directory, "local", r.Snapshot.ID, "plans.json")
	raw, err := exporter.Read(ctx, r)
	if err != nil || !bytes.Equal(raw, expected) || bytes.Contains(raw, []byte("INTERNAL_REASON")) {
		t.Fatal("wrong export", err)
	}
	// CATALOG-R02: preserve the revision directory and only remove its file.
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	if _, err := exporter.Read(ctx, r); err == nil {
		t.Fatal("missing file accepted")
	}
	if err := exporter.Export(ctx, r); err != nil {
		t.Fatal("missing file not repaired", err)
	}
	raw, err = exporter.Read(ctx, r)
	if err != nil || !bytes.Equal(raw, expected) {
		t.Fatal("recovery changed bytes", err)
	}
	if err := os.WriteFile(path, []byte("unexpected bytes"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := exporter.Export(ctx, r); err == nil {
		t.Fatal("unknown existing bytes overwritten")
	}
	raw, err = os.ReadFile(path)
	if err != nil || string(raw) != "unexpected bytes" {
		t.Fatal("conflicting file changed", err)
	}
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(filepath.Dir(path)); err != nil {
		t.Fatal(err)
	}
	if err := exporter.Export(ctx, r); err != nil {
		t.Fatal("whole missing revision not recovered", err)
	}
}
func TestPublicCatalogExportRejectsEscapeLinksAndFalseRecord(t *testing.T) {
	r := publicCatalogRecord(t)
	directory := t.TempDir()
	outside := t.TempDir()
	exporter, err := NewPublicCatalogExporter(directory)
	if err != nil {
		t.Fatal(err)
	}
	defer exporter.Close()
	ctx := context.Background()
	requireSymlink(t, outside, filepath.Join(directory, "local"))
	if err := exporter.Export(ctx, r); err == nil {
		t.Fatal("environment link followed")
	}
	if err := os.Remove(filepath.Join(directory, "local")); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(filepath.Join(directory, "local"), 0o700); err != nil {
		t.Fatal(err)
	}
	requireSymlink(t, outside, filepath.Join(directory, "local", r.Snapshot.ID))
	if err := exporter.Export(ctx, r); err == nil {
		t.Fatal("revision link followed")
	}
	entries, err := os.ReadDir(outside)
	if err != nil || len(entries) != 0 {
		t.Fatal("wrote outside root", err)
	}
	if err := os.Remove(filepath.Join(directory, "local", r.Snapshot.ID)); err != nil {
		t.Fatal(err)
	}
	if err := exporter.Export(ctx, r); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(directory, "local", r.Snapshot.ID, "plans.json")
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	expected, _ := r.PublicBytes()
	target := filepath.Join(outside, "same-public.json")
	if err := os.WriteFile(target, expected, 0o600); err != nil {
		t.Fatal(err)
	}
	requireSymlink(t, target, path)
	if _, err := exporter.Read(ctx, r); err == nil {
		t.Fatal("file link followed")
	}
	if err := exporter.Export(ctx, r); err == nil {
		t.Fatal("file link replaced")
	}
	r.Snapshot.ID = "../escape"
	if err := exporter.Export(ctx, r); err == nil {
		t.Fatal("caller path accepted")
	}
}
