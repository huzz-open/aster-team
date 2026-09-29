package config

import (
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestLoadUsesEnvFileWithoutOverridingProcessEnvironment(t *testing.T) {
	t.Setenv("ASTER_OPERATIONS_DB_NAME", "process_database")
	unsetForTest(t, "ASTER_OPERATIONS_DB_USER")
	unsetForTest(t, "ASTER_OPERATIONS_DB_PASSWORD")
	unsetForTest(t, "ASTER_OPERATIONS_TRUSTED_ORIGINS")
	unsetForTest(t, "ASTER_OPERATIONS_LICENSE_SIGNING_PRIVATE_KEY_PKCS8")
	unsetForTest(t, "ASTER_OPERATIONS_LICENSE_SIGNING_KEY_ID")
	unsetForTest(t, "ASTER_OPERATIONS_CUSTOMER_REF_SECRET")
	unsetForTest(t, "ASTER_OPERATIONS_GITHUB_ENABLED")

	path := filepath.Join(t.TempDir(), "operations.env")
	contents := []byte("ASTER_OPERATIONS_DB_NAME=file_database\n" +
		"ASTER_OPERATIONS_DB_USER=operations_user\n" +
		"ASTER_OPERATIONS_DB_PASSWORD='local secret'\n" +
		"ASTER_OPERATIONS_TRUSTED_ORIGINS=http://127.0.0.1:12080/\n" +
		"ASTER_OPERATIONS_CUSTOMER_REF_SECRET=test-customer-secret\n")
	if err := os.WriteFile(path, contents, 0o600); err != nil {
		t.Fatal(err)
	}

	cfg, err := Load(path)
	if err != nil {
		t.Fatalf("Load() error = %v", err)
	}
	if cfg.Database.Name != "process_database" {
		t.Fatalf("database name = %q, want process environment value", cfg.Database.Name)
	}
	if cfg.Database.Driver != "mysql" {
		t.Fatalf("database driver = %q, want mysql", cfg.Database.Driver)
	}
	if cfg.Database.User != "operations_user" || cfg.Database.Password != "local secret" {
		t.Fatal("database credentials were not loaded from the env file")
	}
	if _, ok := cfg.TrustedOrigins["http://127.0.0.1:12080"]; !ok {
		t.Fatal("trusted origin was not normalized")
	}
}

func TestGitHubAppConfigurationRequiresCompleteHTTPSAppIdentity(t *testing.T) {
	valid := GitHubApp{Enabled: true, APIBaseURL: "https://api.github.com", AppID: 1, InstallationID: 2,
		PrivateKeyPEMBase64: "base64", Repository: "huzz-max/aster-team", WorkflowFile: "customer-release.yml",
		Environment: "customer-release", DefaultSourceRef: "main", PollInterval: 15 * time.Second, RequestTimeout: 20 * time.Second,
		ArtifactDownloadTimeout: 5 * time.Minute}
	if err := valid.Validate(); err != nil {
		t.Fatalf("valid GitHub App config: %v", err)
	}
	invalid := valid
	invalid.APIBaseURL = "http://api.github.com"
	if err := invalid.Validate(); err == nil {
		t.Fatal("GitHub App config accepted an HTTP API origin")
	}
	invalid = valid
	invalid.AppID = 0
	if err := invalid.Validate(); err == nil {
		t.Fatal("GitHub App config accepted a missing App ID")
	}
	invalid = valid
	invalid.ArtifactDownloadTimeout = 20 * time.Second
	if err := invalid.Validate(); err == nil || !strings.Contains(err.Error(), "artifact download timeout") {
		t.Fatalf("GitHub App config accepted a short artifact download timeout: %v", err)
	}
}

func TestConfigRequiresIndependentReleasePublicKeyringWhenGitHubIsEnabled(t *testing.T) {
	location := time.Local
	cfg := Config{Address: "127.0.0.1:12090", Database: Database{Driver: "mysql", Host: "127.0.0.1", Name: "operations",
		User: "operations", Password: "secret", Charset: "utf8mb4", Location: location}, SessionTTL: time.Hour,
		TrustedOrigins:          map[string]struct{}{(&url.URL{Scheme: "https", Host: "operations.example.test"}).String(): {}},
		ArtifactRoot:            "artifacts",
		CustomerReferenceSecret: "customer-secret", GitHubApp: GitHubApp{Enabled: true, APIBaseURL: "https://api.github.com", AppID: 1,
			InstallationID: 2, PrivateKeyPEMBase64: "base64", Repository: "huzz-max/aster-team", WorkflowFile: "customer-release.yml",
			Environment: "customer-release", DefaultSourceRef: "main", PollInterval: 15 * time.Second, RequestTimeout: 20 * time.Second,
			ArtifactDownloadTimeout: 5 * time.Minute},
		ReleaseVerification: ReleaseVerification{MaxArtifactBytes: 2 << 30, MaxExpandedBytes: 4 << 30}}
	if err := cfg.Validate(); err == nil || !strings.Contains(err.Error(), "ASTER_OPERATIONS_RELEASE_TRUSTED_KEYS_JSON") {
		t.Fatalf("Config.Validate() error = %v", err)
	}
}

func unsetForTest(t *testing.T, key string) {
	t.Helper()
	value, exists := os.LookupEnv(key)
	if err := os.Unsetenv(key); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if exists {
			_ = os.Setenv(key, value)
		} else {
			_ = os.Unsetenv(key)
		}
	})
}

func TestLoadRejectsInvalidDatabaseConfiguration(t *testing.T) {
	t.Setenv("ASTER_OPERATIONS_DB_NAME", "invalid-name")
	t.Setenv("ASTER_OPERATIONS_DB_USER", "operations_user")
	t.Setenv("ASTER_OPERATIONS_DB_PASSWORD", "secret")

	if _, err := Load(""); err == nil {
		t.Fatal("Load() succeeded with an unsafe database identifier")
	}
}

func TestLoadRejectsUnsupportedDatabaseDriver(t *testing.T) {
	t.Setenv("ASTER_OPERATIONS_DB_DRIVER", "sqlite")
	t.Setenv("ASTER_OPERATIONS_DB_NAME", "operations")
	t.Setenv("ASTER_OPERATIONS_DB_USER", "operations_user")
	t.Setenv("ASTER_OPERATIONS_DB_PASSWORD", "secret")

	if _, err := Load(""); err == nil {
		t.Fatal("Load() succeeded with an unsupported database driver")
	}
}

func TestReadOnlyLicenseConfigurationDoesNotInventMissingSecrets(t *testing.T) {
	cfg := Config{Address: "127.0.0.1:12090", Database: Database{Driver: "mysql", Host: "127.0.0.1", Name: "operations", User: "operations", Password: "secret", Charset: "utf8mb4"}, SessionTTL: time.Hour, TrustedOrigins: map[string]struct{}{"http://127.0.0.1:12080": {}}, ArtifactRoot: "artifacts"}
	if err := cfg.Validate(); err != nil {
		t.Fatal("public-key-only startup rejected", err)
	}
	if cfg.LicenseSigner.KeyID != "" || cfg.LicenseSigner.PrivateKeyPKCS8 != "" || cfg.CustomerReferenceSecret != "" {
		t.Fatal("missing secret replaced")
	}
	for _, signer := range []LicenseSigner{{KeyID: "partial"}, {PrivateKeyPKCS8: "partial"}, {KeyID: "legacy", PrivateKeyPKCS8: "complete"}} {
		cfg.LicenseSigner = signer
		if err := cfg.Validate(); err == nil {
			t.Fatal("legacy signing identity accepted")
		}
	}
	cfg.LicenseSigner = LicenseSigner{}
	for _, environment := range []string{"", "local", "production"} {
		cfg.FulfillmentEnvironment = environment
		if err := cfg.Validate(); err != nil {
			t.Fatal(err)
		}
	}
	cfg.FulfillmentEnvironment = "test"
	if err := cfg.Validate(); err == nil {
		t.Fatal("unknown fulfillment environment accepted")
	}
}
