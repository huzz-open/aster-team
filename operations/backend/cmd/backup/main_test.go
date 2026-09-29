package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/config"
)

func TestBackupChecksumRoundTripAndTamperDetection(t *testing.T) {
	directory := t.TempDir()
	path := filepath.Join(directory, "backup.sql")
	if err := os.WriteFile(path, []byte("SELECT 1;\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	inventory := backupInventory{
		Schema: inventorySchema, Database: "aster_team_dev", CreatedAt: time.Now().UTC(),
		TableCounts: map[string]int64{},
	}
	for _, table := range inventoryTables {
		inventory.TableCounts[table] = 0
	}
	inventoryBytes, err := json.Marshal(inventory)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path+".inventory.json", inventoryBytes, 0o600); err != nil {
		t.Fatal(err)
	}
	digest, err := fileSHA256(path)
	if err != nil {
		t.Fatal(err)
	}
	checksum := digest + "  backup.sql\n" + sha256Hex(inventoryBytes) + "  backup.sql.inventory.json\n"
	if err := os.WriteFile(path+".sha256", []byte(checksum), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := verify(path, "aster_team_dev"); err != nil {
		t.Fatalf("expected valid backup: %v", err)
	}
	if err := os.WriteFile(path, []byte("SELECT 2;\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := verify(path, "aster_team_dev"); err == nil {
		t.Fatal("expected tampered backup to fail verification")
	}
}

func TestVerifyRejectsWrongDatabaseAndTamperedInventory(t *testing.T) {
	directory := t.TempDir()
	path := filepath.Join(directory, "backup.sql")
	if err := os.WriteFile(path, []byte("SELECT 1;\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	inventory := backupInventory{Schema: inventorySchema, Database: "source_db", TableCounts: map[string]int64{}}
	for _, table := range inventoryTables {
		inventory.TableCounts[table] = 0
	}
	inventoryBytes, _ := json.Marshal(inventory)
	if err := os.WriteFile(path+".inventory.json", inventoryBytes, 0o600); err != nil {
		t.Fatal(err)
	}
	sqlDigest, _ := fileSHA256(path)
	checksum := sqlDigest + "  backup.sql\n" + sha256Hex(inventoryBytes) + "  backup.sql.inventory.json\n"
	if err := os.WriteFile(path+".sha256", []byte(checksum), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := verify(path, "other_db"); err == nil {
		t.Fatal("expected database mismatch to fail")
	}
	if err := os.WriteFile(path+".inventory.json", append(inventoryBytes, ' '), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := verify(path, "source_db"); err == nil {
		t.Fatal("expected tampered inventory to fail")
	}
}

func TestCompareInventory(t *testing.T) {
	expected := backupInventory{TableCounts: map[string]int64{}, DeliveryManifestSHA256s: []string{"b", "a"}}
	actual := backupInventory{TableCounts: map[string]int64{}, DeliveryManifestSHA256s: []string{"a", "b"}}
	for _, table := range inventoryTables {
		expected.TableCounts[table] = 2
		actual.TableCounts[table] = 2
	}
	if err := compareInventory(expected, actual); err != nil {
		t.Fatal(err)
	}
	actual.TableCounts["orders"] = 3
	if err := compareInventory(expected, actual); err == nil {
		t.Fatal("expected count mismatch")
	}
}

func TestReadPasswordLine(t *testing.T) {
	password, err := readPasswordLine(strings.NewReader("a-secure-password\nignored"))
	if err != nil || string(password) != "a-secure-password" {
		t.Fatalf("unexpected password result %q, %v", password, err)
	}
	if _, err := readPasswordLine(strings.NewReader("short\n")); err == nil {
		t.Fatal("expected short password rejection")
	}
}

func TestDumpArgumentsSelectOnlyConfiguredDatabase(t *testing.T) {
	cfg := config.Config{Database: config.Database{Host: "127.0.0.1", Port: 3306, User: "operations", Name: "aster_team_dev"}}
	arguments := dumpArguments(cfg)
	if !reflect.DeepEqual(arguments[len(arguments)-2:], []string{"--skip-add-drop-database", "aster_team_dev"}) {
		t.Fatalf("unexpected database arguments: %v", arguments)
	}
	for _, argument := range arguments {
		if argument == "--databases" || argument == "--all-databases" {
			t.Fatalf("database-creating dump option must not be used: %s", argument)
		}
	}
}

func TestRequireAbsoluteSQLPath(t *testing.T) {
	if _, err := requireAbsoluteSQLPath("backup.sql"); err == nil {
		t.Fatal("expected relative path to be rejected")
	}
	if _, err := requireAbsoluteSQLPath(filepath.Join(t.TempDir(), "backup.txt")); err == nil {
		t.Fatal("expected non-SQL path to be rejected")
	}
}
