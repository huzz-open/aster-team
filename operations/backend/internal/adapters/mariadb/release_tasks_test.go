package mariadb

import (
	"context"
	"database/sql"
	"io/fs"
	"os"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
	operationsschema "aster.local/team/operations/backend/schema"
	mysqldriver "github.com/go-sql-driver/mysql"
)

func TestReleasePackageMigrationPreservesIndependentResults(t *testing.T) {
	// Use an explicitly configured MariaDB database. Session-local
	// temporary tables shadow its release tables, leaving all real records intact.
	dsn := os.Getenv("ASTER_OPERATIONS_TEST_DB_DSN")
	if dsn == "" {
		t.Skip("set ASTER_OPERATIONS_TEST_DB_DSN to run the MariaDB release summary regression")
	}
	cfg, err := mysqldriver.ParseDSN(dsn)
	if err != nil {
		t.Fatal("invalid ASTER_OPERATIONS_TEST_DB_DSN")
	}
	cfg.ParseTime = true
	connector, err := mysqldriver.NewConnector(cfg)
	if err != nil {
		t.Fatal(err)
	}
	db := sql.OpenDB(connector)
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	t.Cleanup(func() { db.Close() })
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	fixtureConn, err := db.Conn(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer fixtureConn.Close()
	exec := func(query string, args ...any) {
		t.Helper()
		if _, err := fixtureConn.ExecContext(ctx, query, args...); err != nil {
			t.Fatal(err)
		}
	}
	schema, err := fs.ReadFile(operationsschema.Files, "init.mariadb.sql")
	if err != nil {
		t.Fatal(err)
	}
	for _, statement := range splitStatements(string(schema)) {
		for _, table := range []string{"release_tasks", "release_runs", "release_task_artifacts", "release_artifacts", "release_run_jobs", "release_run_steps", "audit_events"} {
			if !strings.HasPrefix(statement, "CREATE TABLE "+table+" (") {
				continue
			}
			// MariaDB temporary tables cannot have foreign keys. Keep the real
			// columns, indexes and check constraints for the query under test.
			lines := []string{}
			for _, line := range strings.Split(statement, "\n") {
				if !strings.Contains(line, "FOREIGN KEY") {
					lines = append(lines, line)
				}
			}
			exec(strings.Replace(strings.ReplaceAll(strings.Join(lines, "\n"), ",\n)", "\n)"), "CREATE TABLE", "CREATE TEMPORARY TABLE", 1))
		}
	}
	now := time.Date(2026, 9, 6, 9, 0, 0, 0, time.UTC)
	for _, task := range []struct{ id, status string }{
		{"linux_verified", "completed"}, {"linux_failed", "failed"}, {"linux_missing", "verifying"},
	} {
		exec(`INSERT INTO release_tasks
			(id, version, platform, architecture, mode, github_repository, workflow_file, source_ref,
			 source_commit_sha, phase, status, created_by, created_at, updated_at)
			VALUES (?, '2.0.1-rc.1', 'linux', 'amd64', 'verification', 'test/repository', 'customer-release.yml',
			 'main', ?, ?, ?, 'test_operator', ?, ?)`, task.id, strings.Repeat("a", 40), task.status, task.status, now, now)
		exec(`INSERT INTO release_runs
			(id, release_task_id, attempt, workflow_name, head_branch, head_sha, status, conclusion, html_url, created_at)
			VALUES (?, ?, 1, 'customer-release.yml', 'main', ?, 'completed', 'success', 'https://example.test/run', ?)`,
			task.id+"_run", task.id, strings.Repeat("a", 40), now)
	}
	for index, artifact := range []struct {
		id, task, name, status string
		releaseID, errorCode   any
	}{
		{"linux_package", "linux_verified", "customer-linux-amd64-2.0.1-rc.1", "verified", "release_linux", nil},
		{"windows_package", "linux_verified", "customer-windows-amd64-2.0.1-rc.1", "pending", nil, nil},
		{"other_version", "linux_verified", "customer-linux-amd64-2.0.1", "pending", nil, nil},
		{"other_architecture", "linux_verified", "customer-linux-arm64-2.0.1-rc.1", "pending", nil, nil},
		{"cli_artifact", "linux_verified", "release-asterctl-windows-x64", "pending", nil, nil},
		{"failed_linux_package", "linux_failed", "customer-linux-amd64-2.0.1-rc.1", "failed", nil, "RELEASE_PACKAGE_POLICY_FAILED"},
		{"pending_windows_package", "linux_failed", "customer-windows-amd64-2.0.1-rc.1", "pending", nil, nil},
		{"only_windows_package", "linux_missing", "customer-windows-amd64-2.0.1-rc.1", "pending", nil, nil},
	} {
		exec(`INSERT INTO release_task_artifacts
			(id, release_task_id, release_run_id, github_artifact_id, name, file_name, size_bytes,
			 verification_status, runtime_linkage, release_artifact_id, verification_error_code, created_at)
			VALUES (?, ?, ?, ?, ?, 'package.zip', 1, ?, 'musl-static', ?, ?, ?)`, artifact.id, artifact.task,
			artifact.task+"_run", index+1, artifact.name, artifact.status, artifact.releaseID, artifact.errorCode,
			now.Add(time.Duration(index)*time.Minute))
	}
	// Populate the actual verification evidence required for historical verified rows.
	exec(`UPDATE release_task_artifacts SET sha256=?, release_manifest_sha256=?, signature_key_id='test-key', verified_at=? WHERE verification_status='verified'`, strings.Repeat("b", 64), strings.Repeat("c", 64), now)
	migration, err := fs.ReadFile(operationsschema.Files, "202609062050/release_packages.mariadb.sql")
	if err != nil {
		t.Fatal(err)
	}
	for _, statement := range splitStatements(string(migration)) {
		exec(statement)
	}
	// Retrying an interrupted migration remains safe and never verifies packages.
	for _, statement := range splitStatements(string(migration)) {
		exec(statement)
	}
	// Return the fixture session to the single-connection pool for the store.
	// Pinning all writes above prevents reconnects from reaching real tables.
	if err := fixtureConn.Close(); err != nil {
		t.Fatal(err)
	}
	items, err := NewStore(db).ListReleaseTasks(ctx, 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(items) != 3 {
		t.Fatalf("got %d tasks, want 3 (including the task without a Linux package)", len(items))
	}
	byID := make(map[string]domain.ReleaseTask, len(items))
	for _, item := range items {
		byID[item.ID] = item
	}
	verified := byID["linux_verified"]
	if len(verified.Packages) != 2 {
		t.Fatalf("want two platform packages, got %#v", verified.Packages)
	}
	linux, windows := verified.Packages[0], verified.Packages[1]
	if linux.Platform != "linux" || linux.VerificationStatus != "verified" || linux.ReleaseArtifactID == nil || *linux.ReleaseArtifactID != "release_linux" || linux.SHA256 == nil || *linux.SHA256 != strings.Repeat("b", 64) {
		t.Fatalf("Linux verification evidence changed: %#v", linux)
	}
	if windows.Platform != "windows" || windows.VerificationStatus != "pending" || windows.ReleaseArtifactID != nil || windows.SHA256 != nil {
		t.Fatalf("historical Windows package was changed: %#v", windows)
	}
	if windows.FileName != "aster-team-2.0.1-rc.1-windows-amd64.tar.gz" {
		t.Fatalf("wrong Windows filename: %s", windows.FileName)
	}
	failed := byID["linux_failed"].Packages
	if len(failed) != 2 || failed[0].VerificationStatus != "failed" || failed[0].VerificationErrorCode == nil || *failed[0].VerificationErrorCode != "RELEASE_PACKAGE_POLICY_FAILED" || failed[1].VerificationStatus != "pending" {
		t.Fatalf("independent failure evidence changed: %#v", failed)
	}
	missing := byID["linux_missing"].Packages
	if len(missing) != 1 || missing[0].Platform != "windows" || missing[0].ReleaseArtifactID != nil {
		t.Fatalf("missing Linux package was fabricated: %#v", missing)
	}
	store := NewStore(db)
	task := domain.ReleaseTask{ID: "new_task", Version: "2.0.0", Mode: "verification", FreeDistributionID: "dist_free_1", FreeLicenseSHA256: strings.Repeat("b", 64), GitHubRepository: "test/repository", WorkflowFile: "customer-release.yml", SourceRef: "main", SourceCommitSHA: strings.Repeat("a", 40), Phase: "dispatch", Status: "dispatching", CreatedBy: "test_operator", CreatedAt: now, UpdatedAt: now}
	if err := store.CreateReleaseTask(ctx, task, "test_operator"); err != nil {
		t.Fatal(err)
	}
	conclusion := "success"
	runID := int64(500)
	run := domain.ReleaseRun{ID: "new_run", ReleaseTaskID: task.ID, Attempt: 1, GitHubRunID: &runID, WorkflowName: task.WorkflowFile, HeadBranch: "main", HeadSHA: task.SourceCommitSHA, Status: "completed", Conclusion: &conclusion, HTMLURL: "https://example.test/500", CreatedAt: now, CompletedAt: &now}
	packages := []domain.ReleaseTaskArtifact{}
	for index, target := range domain.ReleaseTargets() {
		packages = append(packages, domain.ReleaseTaskArtifact{ID: "new_" + target.Platform, ReleaseTaskID: task.ID, ReleaseRunID: run.ID, GitHubArtifactID: int64(500 + index), Name: target.ArtifactName(task.Version), FileName: target.FileName(task.Version), Platform: target.Platform, Architecture: target.Architecture, VerificationStatus: "queued", CreatedAt: now})
	}
	if err := store.ApplyReleaseSnapshot(ctx, task.ID, ports.ReleaseSnapshot{Run: run, Artifacts: packages}, "verifying", "artifact_verification", nil, now); err != nil {
		t.Fatal(err)
	}
	linuxRelease := domain.ReleaseArtifact{ID: "new_release_linux", Version: task.Version, Platform: "linux", Architecture: "amd64", ObjectKey: "objects/linux", SHA256: strings.Repeat("d", 64), ReleaseManifestSHA256: strings.Repeat("e", 64), SizeBytes: 123, SignatureRef: "release-key:test-key", SourceCommitSHA: &task.SourceCommitSHA, GitHubRunID: &runID, RuntimeLinkage: "musl-static", CreatedAt: now}
	if err := store.CompleteReleaseTaskArtifact(ctx, task.ID, "new_linux", linuxRelease, "test-key", now); err != nil {
		t.Fatal(err)
	}
	if err := store.FinishReleaseTaskVerification(ctx, task.ID, nil, now); err != nil {
		t.Fatal(err)
	}
	detail, err := store.GetReleaseTaskDetail(ctx, task.ID)
	if err != nil {
		t.Fatal(err)
	}
	if detail.Task.Status != "verifying" || detail.Task.FreeDistributionID != task.FreeDistributionID || detail.Task.FreeLicenseSHA256 != task.FreeLicenseSHA256 || len(detail.Task.Packages) != 2 || detail.Task.Packages[0].ReleaseArtifactID == nil {
		t.Fatalf("Linux did not become independently downloadable: %#v", detail)
	}
	if err := store.FailReleaseTaskArtifact(ctx, task.ID, "new_windows", "RELEASE_SIGNATURE_INVALID", now); err != nil {
		t.Fatal(err)
	}
	if err := store.FinishReleaseTaskVerification(ctx, task.ID, nil, now); err != nil {
		t.Fatal(err)
	}
	detail, err = store.GetReleaseTaskDetail(ctx, task.ID)
	if err != nil {
		t.Fatal(err)
	}
	if detail.Task.Status != "failed" || detail.Task.Packages[0].VerificationStatus != "verified" || detail.Task.Packages[1].VerificationStatus != "failed" {
		t.Fatalf("package failure was not independent: %#v", detail)
	}
	if err := store.PrepareReleaseTaskArtifactVerification(ctx, task.ID, "new_windows", "test_operator", now.Add(time.Second)); err != nil {
		t.Fatal(err)
	}
	windowsRelease := linuxRelease
	windowsRelease.ID = "new_release_windows"
	windowsRelease.Platform = "windows"
	windowsRelease.RuntimeLinkage = "msvc"
	windowsRelease.SHA256 = strings.Repeat("f", 64)
	windowsRelease.ObjectKey = "objects/windows"
	if err := store.CompleteReleaseTaskArtifact(ctx, task.ID, "new_windows", windowsRelease, "test-key", now); err != nil {
		t.Fatal(err)
	}
	if err := store.FinishReleaseTaskVerification(ctx, task.ID, nil, now); err != nil {
		t.Fatal(err)
	}
	detail, err = store.GetReleaseTaskDetail(ctx, task.ID)
	if err != nil {
		t.Fatal(err)
	}
	if detail.Task.Status != "completed" || *detail.Task.Packages[0].ReleaseArtifactID != linuxRelease.ID || *detail.Task.Packages[1].ReleaseArtifactID != windowsRelease.ID {
		t.Fatalf("wrong immutable package associations: %#v", detail)
	}

}
