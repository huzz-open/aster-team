package mariadb

import (
	"crypto/sha256"
	"io/fs"
	"reflect"
	"strings"
	"testing"
	"testing/fstest"

	"aster.local/team/operations/backend/internal/application"
	operationsschema "aster.local/team/operations/backend/schema"
)

func TestSplitStatements(t *testing.T) {
	input := "CREATE TABLE first_table (id INT);\r\n\r\nCREATE TABLE second_table (id INT);\r\n"
	want := []string{"CREATE TABLE first_table (id INT)", "CREATE TABLE second_table (id INT)"}
	if got := splitStatements(input); !reflect.DeepEqual(got, want) {
		t.Fatalf("splitStatements() = %#v, want %#v", got, want)
	}
}

func TestSchemaFilePathsUsesFlatInitAndTimestampChangeSets(t *testing.T) {
	schemaFS := fstest.MapFS{
		"init.mariadb.sql":                         &fstest.MapFile{Data: []byte("CREATE TABLE initial_table (id INT);\n")},
		"202608281435/add_audit_index.mariadb.sql": &fstest.MapFile{Data: []byte("CREATE INDEX audit_idx ON audit_events (created_at);\n")},
		"202608281435/add_order_note.mariadb.sql":  &fstest.MapFile{Data: []byte("ALTER TABLE orders ADD note TEXT;\n")},
	}
	want := []string{
		"init.mariadb.sql",
		"202608281435/add_audit_index.mariadb.sql",
		"202608281435/add_order_note.mariadb.sql",
	}
	got, err := schemaFilePaths(schemaFS)
	if err != nil {
		t.Fatalf("schemaFilePaths() error = %v", err)
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("schemaFilePaths() = %#v, want %#v", got, want)
	}
}

func TestSchemaFilePathsRejectsUndatedChanges(t *testing.T) {
	schemaFS := fstest.MapFS{
		"init.mariadb.sql":      &fstest.MapFile{Data: []byte("SELECT 1;\n")},
		"add_table.mariadb.sql": &fstest.MapFile{Data: []byte("SELECT 2;\n")},
	}
	if _, err := schemaFilePaths(schemaFS); err == nil {
		t.Fatal("schemaFilePaths() accepted an undated change")
	}
}

func TestCanonicalMigrationContentsIgnoresCheckoutLineEndings(t *testing.T) {
	lf := []byte("CREATE TABLE first_table (id INT);\n\nINSERT INTO first_table VALUES (1);\n")
	crlf := []byte("CREATE TABLE first_table (id INT);\r\n\r\nINSERT INTO first_table VALUES (1);\r\n")
	if got, want := sha256.Sum256(canonicalMigrationContents(crlf)), sha256.Sum256(canonicalMigrationContents(lf)); got != want {
		t.Fatalf("canonical migration checksum differs for LF and CRLF checkouts: %x != %x", got, want)
	}
}

func TestReleaseFreeLicenseMigrationUsesRestartableCompatibleConstraints(t *testing.T) {
	contents, err := fs.ReadFile(operationsschema.Files, "202609131200/release_free_license.mariadb.sql")
	if err != nil {
		t.Fatal(err)
	}
	schema := string(contents)
	if strings.Contains(schema, "ADD CONSTRAINT IF NOT EXISTS") {
		t.Fatal("MariaDB does not support IF NOT EXISTS between ADD CONSTRAINT and FOREIGN KEY/CHECK")
	}
	fragments := []string{
		"ADD COLUMN IF NOT EXISTS free_distribution_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin",
		"DROP CONSTRAINT IF EXISTS fk_release_tasks_free_distribution",
		"MODIFY COLUMN free_distribution_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin",
		"ADD CONSTRAINT fk_release_tasks_free_distribution",
		"FOREIGN KEY (free_distribution_id) REFERENCES commercial_free_distributions(id)",
	}
	position := -1
	for _, fragment := range fragments {
		next := strings.Index(schema, fragment)
		if next <= position {
			t.Fatalf("release free-license migration is missing or misorders %q", fragment)
		}
		position = next
	}
}

func TestInitSchemaOwnsReleaseCenterHierarchyAndPermissionBoundary(t *testing.T) {
	contents, err := fs.ReadFile(operationsschema.Files, "init.mariadb.sql")
	if err != nil {
		t.Fatal(err)
	}
	schema := string(contents)
	for _, table := range []string{"operator_permissions", "release_tasks", "release_runs", "release_run_jobs", "release_run_steps", "release_task_artifacts", "release_publish_requests", "release_publish_approvals"} {
		if !strings.Contains(schema, "CREATE TABLE "+table+" (") {
			t.Errorf("init schema is missing %s", table)
		}
	}
	permissionSchema := schema
	paths, err := schemaFilePaths(operationsschema.Files)
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range paths {
		content, err := fs.ReadFile(operationsschema.Files, path)
		if err != nil {
			t.Fatal(err)
		}
		permissionSchema += string(content)
	}
	for _, permission := range application.BootstrapReleasePermissions() {
		if !strings.Contains(permissionSchema, "'"+permission+"'") {
			t.Errorf("schema migrations are missing permission %s", permission)
		}
	}
	if !strings.Contains(schema, "github_digest_sha256 CHAR(64)") {
		t.Fatal("release task artifacts must keep the GitHub ZIP digest separate from the customer archive digest")
	}
	if strings.Contains(schema, "github_token") || strings.Contains(schema, "signing_private_key") || strings.Contains(schema, "full_log") {
		t.Fatal("release center schema must not persist GitHub tokens, signing private keys, or full logs")
	}
}
