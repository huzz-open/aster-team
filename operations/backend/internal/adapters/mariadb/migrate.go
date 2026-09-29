package mariadb

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"errors"
	"fmt"
	"io/fs"
	"sort"
	"strings"

	operationsschema "aster.local/team/operations/backend/schema"
)

func Migrate(ctx context.Context, db *sql.DB) error {
	var acquired int
	if err := db.QueryRowContext(ctx, "SELECT GET_LOCK('aster_operations_schema_migrations', 15)").Scan(&acquired); err != nil {
		return fmt.Errorf("acquire migration lock: %w", err)
	}
	if acquired != 1 {
		return errors.New("timed out waiting for operations migration lock")
	}
	defer db.ExecContext(context.WithoutCancel(ctx), "SELECT RELEASE_LOCK('aster_operations_schema_migrations')")

	if _, err := db.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS operations_schema_migrations (
        version VARCHAR(255) NOT NULL PRIMARY KEY,
        checksum CHAR(64) NOT NULL,
        applied_at DATETIME(6) NOT NULL
    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci`); err != nil {
		return fmt.Errorf("create migration table: %w", err)
	}
	entries, err := schemaFilePaths(operationsschema.Files)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		contents, err := fs.ReadFile(operationsschema.Files, entry)
		if err != nil {
			return err
		}
		canonicalContents := canonicalMigrationContents(contents)
		digest := sha256.Sum256(canonicalContents)
		checksum := hex.EncodeToString(digest[:])
		var existing string
		err = db.QueryRowContext(ctx, "SELECT checksum FROM operations_schema_migrations WHERE version = ?", entry).Scan(&existing)
		if err == nil {
			if existing != checksum {
				return fmt.Errorf("migration %s checksum changed after application", entry)
			}
			continue
		}
		if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		for _, statement := range splitStatements(string(canonicalContents)) {
			if _, err := db.ExecContext(ctx, statement); err != nil {
				return fmt.Errorf("apply migration %s: %w", entry, err)
			}
		}
		if _, err := db.ExecContext(ctx, "INSERT INTO operations_schema_migrations (version, checksum, applied_at) VALUES (?, ?, UTC_TIMESTAMP(6))", entry, checksum); err != nil {
			return fmt.Errorf("record migration %s: %w", entry, err)
		}
	}
	return nil
}

func schemaFilePaths(schemaFS fs.FS) ([]string, error) {
	entries := make([]string, 0)
	err := fs.WalkDir(schemaFS, ".", func(path string, entry fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if path == "." {
			return nil
		}
		if entry.IsDir() {
			if strings.Contains(path, "/") || !isTimestampChangeID(path) {
				return fmt.Errorf("invalid schema change directory %q; expected YYYYMMDDHHmm", path)
			}
			return nil
		}
		if path == "init.mariadb.sql" {
			entries = append(entries, path)
			return nil
		}
		directory, filename, found := strings.Cut(path, "/")
		if !found || !isTimestampChangeID(directory) || strings.Contains(filename, "/") ||
			filename == ".mariadb.sql" || !strings.HasSuffix(filename, ".mariadb.sql") {
			return fmt.Errorf("invalid schema change file %q", path)
		}
		entries = append(entries, path)
		return nil
	})
	if err != nil {
		return nil, err
	}
	sort.Slice(entries, func(i, j int) bool {
		if entries[i] == "init.mariadb.sql" {
			return true
		}
		if entries[j] == "init.mariadb.sql" {
			return false
		}
		return entries[i] < entries[j]
	})
	if len(entries) == 0 || entries[0] != "init.mariadb.sql" {
		return nil, errors.New("operations schema is missing init.mariadb.sql")
	}
	return entries, nil
}

func isTimestampChangeID(value string) bool {
	if len(value) != 12 {
		return false
	}
	for _, character := range value {
		if character < '0' || character > '9' {
			return false
		}
	}
	return true
}

func canonicalMigrationContents(contents []byte) []byte {
	return []byte(strings.ReplaceAll(string(contents), "\r\n", "\n"))
}

func splitStatements(contents string) []string {
	parts := strings.Split(strings.ReplaceAll(contents, "\r\n", "\n"), ";\n")
	statements := make([]string, 0, len(parts))
	for _, part := range parts {
		statement := strings.TrimSpace(part)
		if statement == "" {
			continue
		}
		statements = append(statements, statement)
	}
	return statements
}
