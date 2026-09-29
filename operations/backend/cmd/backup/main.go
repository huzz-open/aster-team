package main

import (
	"bufio"
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/adapters/mariadb"
	"aster.local/team/operations/backend/internal/config"
	"golang.org/x/crypto/bcrypt"
)

const inventorySchema = "aster.operations-backup.inventory.v1"

var inventoryTables = []string{
	"customers", "orders", "trials", "license_records", "license_issuances",
	"release_artifacts", "deliveries", "audit_events",
}

type backupInventory struct {
	Schema                  string           `json:"schema"`
	Database                string           `json:"database"`
	CreatedAt               time.Time        `json:"created_at"`
	TableCounts             map[string]int64 `json:"table_counts"`
	DeliveryManifestSHA256s []string         `json:"delivery_manifest_sha256s"`
}

type verifiedBackup struct {
	SQLPath       string
	SQLSHA256     string
	SQLSize       int64
	InventoryPath string
	Inventory     backupInventory
}

type operatorIdentity struct {
	ID    string
	Email string
}

func main() {
	envFile := flag.String("env-file", ".env", "Operations env file")
	output := flag.String("output", "", "absolute output path for a new SQL backup")
	verifyPath := flag.String("verify", "", "verify a backup and its business inventory")
	restorePath := flag.String("restore", "", "restore a verified SQL backup")
	confirmDatabase := flag.String("confirm-database", "", "must exactly match the configured database for restore")
	operatorEmail := flag.String("operator-email", "", "active Operations operator email")
	passwordStdin := flag.Bool("password-stdin", false, "read the operator password from standard input")
	maintenanceConfirmed := flag.Bool("maintenance-confirmed", false, "confirm Operations API is stopped for backup or restore")
	flag.Parse()

	selected := 0
	for _, value := range []string{*output, *verifyPath, *restorePath} {
		if value != "" {
			selected++
		}
	}
	if selected != 1 {
		fatal(errors.New("choose exactly one of --output, --verify or --restore"))
	}
	if strings.TrimSpace(*operatorEmail) == "" || !*passwordStdin {
		fatal(errors.New("--operator-email and --password-stdin are required"))
	}
	if (*output != "" || *restorePath != "") && !*maintenanceConfirmed {
		fatal(errors.New("stop aster-operations-api and pass --maintenance-confirmed"))
	}

	cfg, err := config.Load(*envFile)
	if err != nil {
		fatal(err)
	}
	password, err := readPasswordLine(os.Stdin)
	if err != nil {
		fatal(err)
	}
	defer clearBytes(password)

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Hour)
	defer cancel()
	database, err := mariadb.Open(ctx, cfg.Database)
	if err != nil {
		fatal(err)
	}
	defer database.Close()
	operator, err := authenticateOperator(ctx, database, *operatorEmail, password)
	if err != nil {
		fatal(err)
	}

	switch {
	case *output != "":
		if err := backup(ctx, database, cfg, operator, *output); err != nil {
			fatal(err)
		}
	case *verifyPath != "":
		verified, err := verify(*verifyPath, cfg.Database.Name)
		if err != nil {
			_ = recordHistory(ctx, database, operator.ID, "verify", *verifyPath, strings.Repeat("0", 64), 0, "failed", nil)
			fatal(err)
		}
		now := time.Now().UTC()
		if err := recordHistory(ctx, database, operator.ID, "verify", verified.SQLPath, verified.SQLSHA256, verified.SQLSize, "verified", &now); err != nil {
			fatal(err)
		}
		fmt.Printf("Backup verified: %s\n", verified.SQLPath)
	case *restorePath != "":
		if *confirmDatabase != cfg.Database.Name {
			fatal(errors.New("--confirm-database must exactly match ASTER_OPERATIONS_DB_NAME"))
		}
		if err := restore(ctx, database, cfg, operator, password, *restorePath); err != nil {
			fatal(err)
		}
	}
}

func backup(ctx context.Context, database *sql.DB, cfg config.Config, operator operatorIdentity, output string) error {
	absolute, err := requireAbsoluteSQLPath(output)
	if err != nil {
		return err
	}
	inventoryPath := absolute + ".inventory.json"
	checksumPath := absolute + ".sha256"
	for _, target := range []string{absolute, inventoryPath, checksumPath} {
		if _, err := os.Stat(target); err == nil {
			return fmt.Errorf("backup target already exists; refusing to overwrite: %s", target)
		} else if !errors.Is(err, os.ErrNotExist) {
			return err
		}
	}
	if err := os.MkdirAll(filepath.Dir(absolute), 0o750); err != nil {
		return err
	}
	temporary, err := os.CreateTemp(filepath.Dir(absolute), ".aster-operations-backup-*.sql")
	if err != nil {
		return err
	}
	temporaryPath := temporary.Name()
	defer os.Remove(temporaryPath)
	defer temporary.Close()

	command := exec.CommandContext(ctx, "mariadb-dump", dumpArguments(cfg)...)
	command.Env = append(os.Environ(), "MYSQL_PWD="+cfg.Database.Password)
	command.Stdout = temporary
	command.Stderr = os.Stderr
	if err := command.Run(); err != nil {
		return fmt.Errorf("mariadb-dump failed: %w", err)
	}
	if err := temporary.Sync(); err != nil {
		return err
	}
	if err := temporary.Close(); err != nil {
		return err
	}

	inventory, err := collectInventory(ctx, database, cfg.Database.Name)
	if err != nil {
		return fmt.Errorf("collect backup inventory: %w", err)
	}
	inventoryBytes, err := json.MarshalIndent(inventory, "", "  ")
	if err != nil {
		return err
	}
	inventoryBytes = append(inventoryBytes, '\n')
	sqlDigest, err := fileSHA256(temporaryPath)
	if err != nil {
		return err
	}
	inventoryDigest := sha256Hex(inventoryBytes)
	info, err := os.Stat(temporaryPath)
	if err != nil {
		return err
	}
	checksum := fmt.Sprintf("%s  %s\n%s  %s\n", sqlDigest, filepath.Base(absolute), inventoryDigest, filepath.Base(inventoryPath))

	if err := writeAtomic(inventoryPath, inventoryBytes, 0o600); err != nil {
		return err
	}
	if err := writeAtomic(checksumPath, []byte(checksum), 0o600); err != nil {
		return err
	}
	if err := os.Chmod(temporaryPath, 0o600); err != nil {
		return err
	}
	if err := os.Rename(temporaryPath, absolute); err != nil {
		return err
	}
	if err := recordHistory(ctx, database, operator.ID, "backup", absolute, sqlDigest, info.Size(), "created", nil); err != nil {
		return fmt.Errorf("backup created but history recording failed: %w", err)
	}
	fmt.Printf("Backup created: %s\nSHA-256: %s\nInventory: %s\n", absolute, sqlDigest, inventoryPath)
	return nil
}

func restore(ctx context.Context, database *sql.DB, cfg config.Config, operator operatorIdentity, password []byte, path string) error {
	verified, err := verify(path, cfg.Database.Name)
	if err != nil {
		return err
	}
	file, err := os.Open(verified.SQLPath)
	if err != nil {
		return err
	}
	defer file.Close()
	command := exec.CommandContext(ctx, "mariadb", append(connectionArguments(cfg), cfg.Database.Name)...)
	command.Env = append(os.Environ(), "MYSQL_PWD="+cfg.Database.Password)
	command.Stdin = file
	command.Stdout = os.Stdout
	command.Stderr = os.Stderr
	if err := command.Run(); err != nil {
		return fmt.Errorf("mariadb restore failed: %w", err)
	}
	if err := database.PingContext(ctx); err != nil {
		return fmt.Errorf("connect after restore: %w", err)
	}
	restoredOperator, err := authenticateOperator(ctx, database, operator.Email, password)
	if err != nil || restoredOperator.ID != operator.ID {
		return errors.New("restored database does not contain the same active operator credentials; restore history cannot be trusted")
	}
	actual, err := collectInventory(ctx, database, cfg.Database.Name)
	if err != nil {
		return fmt.Errorf("collect restored inventory: %w", err)
	}
	if err := compareInventory(verified.Inventory, actual); err != nil {
		_ = recordHistory(ctx, database, operator.ID, "restore", verified.SQLPath, verified.SQLSHA256, verified.SQLSize, "consistency_failed", nil)
		return err
	}
	now := time.Now().UTC()
	if err := recordHistory(ctx, database, operator.ID, "restore", verified.SQLPath, verified.SQLSHA256, verified.SQLSize, "restored", &now); err != nil {
		return err
	}
	fmt.Printf("Backup restored and business inventory verified in database %s\n", cfg.Database.Name)
	return nil
}

func verify(path, expectedDatabase string) (verifiedBackup, error) {
	absolute, err := requireAbsoluteSQLPath(path)
	if err != nil {
		return verifiedBackup{}, err
	}
	inventoryPath := absolute + ".inventory.json"
	expected := map[string]string{filepath.Base(absolute): "", filepath.Base(inventoryPath): ""}
	sidecar, err := os.Open(absolute + ".sha256")
	if err != nil {
		return verifiedBackup{}, fmt.Errorf("open backup checksum: %w", err)
	}
	scanner := bufio.NewScanner(io.LimitReader(sidecar, 8192))
	for scanner.Scan() {
		fields := strings.Fields(scanner.Text())
		if len(fields) != 2 || !validSHA256(fields[0]) {
			sidecar.Close()
			return verifiedBackup{}, errors.New("backup checksum file is invalid")
		}
		if _, exists := expected[fields[1]]; !exists || expected[fields[1]] != "" {
			sidecar.Close()
			return verifiedBackup{}, errors.New("backup checksum file contains an unexpected or duplicate entry")
		}
		expected[fields[1]] = fields[0]
	}
	if err := scanner.Err(); err != nil {
		sidecar.Close()
		return verifiedBackup{}, err
	}
	if err := sidecar.Close(); err != nil {
		return verifiedBackup{}, err
	}
	for name, digest := range expected {
		if digest == "" {
			return verifiedBackup{}, fmt.Errorf("backup checksum is missing %s", name)
		}
	}
	actualSQL, err := fileSHA256(absolute)
	if err != nil {
		return verifiedBackup{}, err
	}
	if actualSQL != expected[filepath.Base(absolute)] {
		return verifiedBackup{}, errors.New("backup SQL SHA-256 verification failed")
	}
	inventoryBytes, err := os.ReadFile(inventoryPath)
	if err != nil {
		return verifiedBackup{}, err
	}
	if sha256Hex(inventoryBytes) != expected[filepath.Base(inventoryPath)] {
		return verifiedBackup{}, errors.New("backup inventory SHA-256 verification failed")
	}
	var inventory backupInventory
	if err := json.Unmarshal(inventoryBytes, &inventory); err != nil {
		return verifiedBackup{}, fmt.Errorf("decode backup inventory: %w", err)
	}
	if inventory.Schema != inventorySchema || inventory.Database != expectedDatabase {
		return verifiedBackup{}, errors.New("backup inventory schema or database does not match the configured target")
	}
	for _, table := range inventoryTables {
		if _, exists := inventory.TableCounts[table]; !exists {
			return verifiedBackup{}, fmt.Errorf("backup inventory is missing table %s", table)
		}
	}
	info, err := os.Stat(absolute)
	if err != nil {
		return verifiedBackup{}, err
	}
	return verifiedBackup{SQLPath: absolute, SQLSHA256: actualSQL, SQLSize: info.Size(), InventoryPath: inventoryPath, Inventory: inventory}, nil
}

func collectInventory(ctx context.Context, database *sql.DB, databaseName string) (backupInventory, error) {
	inventory := backupInventory{Schema: inventorySchema, Database: databaseName, CreatedAt: time.Now().UTC(), TableCounts: make(map[string]int64, len(inventoryTables))}
	for _, table := range inventoryTables {
		var count int64
		if err := database.QueryRowContext(ctx, "SELECT COUNT(*) FROM `"+table+"`").Scan(&count); err != nil {
			return backupInventory{}, err
		}
		inventory.TableCounts[table] = count
	}
	rows, err := database.QueryContext(ctx, "SELECT manifest_sha256 FROM delivery_records ORDER BY manifest_sha256")
	if err != nil {
		return backupInventory{}, err
	}
	defer rows.Close()
	for rows.Next() {
		var digest string
		if err := rows.Scan(&digest); err != nil {
			return backupInventory{}, err
		}
		inventory.DeliveryManifestSHA256s = append(inventory.DeliveryManifestSHA256s, digest)
	}
	return inventory, rows.Err()
}

func compareInventory(expected, actual backupInventory) error {
	for _, table := range inventoryTables {
		if expected.TableCounts[table] != actual.TableCounts[table] {
			return fmt.Errorf("restored inventory mismatch for %s: expected %d, got %d", table, expected.TableCounts[table], actual.TableCounts[table])
		}
	}
	expectedHashes := append([]string(nil), expected.DeliveryManifestSHA256s...)
	actualHashes := append([]string(nil), actual.DeliveryManifestSHA256s...)
	sort.Strings(expectedHashes)
	sort.Strings(actualHashes)
	if strings.Join(expectedHashes, "\n") != strings.Join(actualHashes, "\n") {
		return errors.New("restored delivery manifest hashes do not match the backup inventory")
	}
	return nil
}

func authenticateOperator(ctx context.Context, database *sql.DB, email string, password []byte) (operatorIdentity, error) {
	normalized := strings.ToLower(strings.TrimSpace(email))
	var operator operatorIdentity
	var passwordHash, status string
	err := database.QueryRowContext(ctx, "SELECT id, email, password_hash, status FROM operators WHERE normalized_email = ?", normalized).Scan(&operator.ID, &operator.Email, &passwordHash, &status)
	if err != nil || status != "active" || bcrypt.CompareHashAndPassword([]byte(passwordHash), password) != nil {
		return operatorIdentity{}, errors.New("operator authentication failed")
	}
	return operator, nil
}

func recordHistory(ctx context.Context, database *sql.DB, operatorID, kind, objectRef, digest string, size int64, status string, verifiedAt *time.Time) error {
	id, err := randomID("backup_")
	if err != nil {
		return err
	}
	_, err = database.ExecContext(ctx, `INSERT INTO backup_history
		(id, kind, object_ref, sha256, size_bytes, status, operator_id, created_at, verified_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`, id, kind, objectRef, digest, size, status, operatorID, time.Now().UTC(), verifiedAt)
	return err
}

func readPasswordLine(reader io.Reader) ([]byte, error) {
	buffered := bufio.NewReader(io.LimitReader(reader, 4097))
	password, err := buffered.ReadString('\n')
	if err != nil && !errors.Is(err, io.EOF) {
		return nil, err
	}
	password = strings.TrimRight(password, "\r\n")
	if len(password) < 12 || len(password) > 4096 {
		return nil, errors.New("operator password read from stdin must be between 12 and 4096 bytes")
	}
	return []byte(password), nil
}

func clearBytes(value []byte) {
	for index := range value {
		value[index] = 0
	}
}

func requireAbsoluteSQLPath(path string) (string, error) {
	if !filepath.IsAbs(path) {
		return "", errors.New("backup path must be absolute")
	}
	absolute := filepath.Clean(path)
	if strings.ToLower(filepath.Ext(absolute)) != ".sql" {
		return "", errors.New("backup path must end in .sql")
	}
	return absolute, nil
}

func dumpArguments(cfg config.Config) []string {
	return append(connectionArguments(cfg), "--single-transaction", "--quick", "--routines", "--triggers", "--events", "--hex-blob", "--skip-comments", "--skip-add-drop-database", cfg.Database.Name)
}

func connectionArguments(cfg config.Config) []string {
	return []string{"--host=" + cfg.Database.Host, "--port=" + strconv.Itoa(cfg.Database.Port), "--user=" + cfg.Database.User, "--default-character-set=utf8mb4", "--protocol=TCP"}
}

func writeAtomic(path string, contents []byte, mode os.FileMode) error {
	temporary, err := os.CreateTemp(filepath.Dir(path), ".aster-operations-sidecar-*")
	if err != nil {
		return err
	}
	temporaryPath := temporary.Name()
	defer os.Remove(temporaryPath)
	if err := temporary.Chmod(mode); err != nil {
		temporary.Close()
		return err
	}
	if _, err := temporary.Write(contents); err != nil {
		temporary.Close()
		return err
	}
	if err := temporary.Sync(); err != nil {
		temporary.Close()
		return err
	}
	if err := temporary.Close(); err != nil {
		return err
	}
	return os.Rename(temporaryPath, path)
}

func fileSHA256(path string) (string, error) {
	file, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer file.Close()
	hash := sha256.New()
	if _, err := io.Copy(hash, file); err != nil {
		return "", err
	}
	return hex.EncodeToString(hash.Sum(nil)), nil
}

func sha256Hex(value []byte) string {
	digest := sha256.Sum256(value)
	return hex.EncodeToString(digest[:])
}

func validSHA256(value string) bool {
	_, err := hex.DecodeString(value)
	return len(value) == 64 && err == nil
}

func randomID(prefix string) (string, error) {
	value := make([]byte, 16)
	if _, err := rand.Read(value); err != nil {
		return "", err
	}
	return prefix + hex.EncodeToString(value), nil
}

func fatal(err error) { fmt.Fprintln(os.Stderr, "operations-backup:", err); os.Exit(1) }
