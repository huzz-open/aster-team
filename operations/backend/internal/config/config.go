package config

import (
	"bufio"
	"errors"
	"fmt"
	"net"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"
)

type Database struct {
	Driver          string
	Host            string
	Port            int
	Name            string
	User            string
	Password        string
	TLS             bool
	Charset         string
	Location        *time.Location
	ConnectTimeout  time.Duration
	ReadTimeout     time.Duration
	WriteTimeout    time.Duration
	MaxOpenConns    int
	MaxIdleConns    int
	ConnMaxLifetime time.Duration
	ConnMaxIdleTime time.Duration
}

type LicenseSigner struct {
	KeyID           string
	PrivateKeyPKCS8 string
}

type GitHubApp struct {
	Enabled                 bool
	APIBaseURL              string
	AppID                   int64
	InstallationID          int64
	PrivateKeyPEMBase64     string
	Repository              string
	WorkflowFile            string
	Environment             string
	DefaultSourceRef        string
	PollInterval            time.Duration
	RequestTimeout          time.Duration
	ArtifactDownloadTimeout time.Duration
}

type ReleaseVerification struct {
	TrustedKeysJSON  string
	MaxArtifactBytes int64
	MaxExpandedBytes int64
}

type GitHubPublisher struct {
	Enabled             bool
	APIBaseURL          string
	UploadBaseURL       string
	AppID               int64
	InstallationID      int64
	PrivateKeyPEMBase64 string
	Repository          string
	RequestTimeout      time.Duration
}

type Config struct {
	UpgradeCredentialKey        string
	Address                     string
	Database                    Database
	BootstrapEmail              string
	BootstrapPassword           string
	CookieSecure                bool
	SessionTTL                  time.Duration
	TrustedOrigins              map[string]struct{}
	AutoMigrate                 bool
	CreateDatabase              bool
	RequestBodyMaxSize          int64
	LicenseSigner               LicenseSigner
	LicenseV2SignersJSON        string
	LicenseV2VerifiersJSON      string
	ArtifactRoot                string
	PublicCatalogRoot           string
	PublicationLocalOrigin      string
	PublicationProductionOrigin string
	QuotationEnvironment        string
	FulfillmentEnvironment      string
	CustomerReferenceSecret     string
	GitHubApp                   GitHubApp
	ReleaseVerification         ReleaseVerification
	GitHubPublisher             GitHubPublisher
}

func Load(envFile string) (Config, error) {
	if envFile != "" {
		if err := loadEnvFile(envFile); err != nil && !errors.Is(err, os.ErrNotExist) {
			return Config{}, err
		}
	}
	locationName := envOr("ASTER_OPERATIONS_DB_LOCATION", "Local")
	location, err := time.LoadLocation(locationName)
	if err != nil {
		return Config{}, fmt.Errorf("invalid ASTER_OPERATIONS_DB_LOCATION: %w", err)
	}
	port, err := intEnv("ASTER_OPERATIONS_DB_PORT", 3306, 1, 65535)
	if err != nil {
		return Config{}, err
	}
	maxOpen, err := intEnv("ASTER_OPERATIONS_DB_MAX_OPEN_CONNS", 10, 1, 500)
	if err != nil {
		return Config{}, err
	}
	maxIdle, err := intEnv("ASTER_OPERATIONS_DB_MAX_IDLE_CONNS", maxOpen, 0, maxOpen)
	if err != nil {
		return Config{}, err
	}
	bodyLimit, err := intEnv("ASTER_OPERATIONS_REQUEST_BODY_MAX_BYTES", 1<<20, 1024, 16<<20)
	if err != nil {
		return Config{}, err
	}
	maxArtifactBytes, err := boundedInt64Env("ASTER_OPERATIONS_RELEASE_ARTIFACT_MAX_BYTES", 2<<30, 1<<20, 8<<30)
	if err != nil {
		return Config{}, err
	}
	maxExpandedBytes, err := boundedInt64Env("ASTER_OPERATIONS_RELEASE_EXPANDED_MAX_BYTES", 4<<30, 1<<20, 16<<30)
	if err != nil {
		return Config{}, err
	}
	githubAppID, err := int64Env("ASTER_OPERATIONS_GITHUB_APP_ID")
	if err != nil {
		return Config{}, err
	}
	githubInstallationID, err := int64Env("ASTER_OPERATIONS_GITHUB_INSTALLATION_ID")
	if err != nil {
		return Config{}, err
	}
	githubPrivateKey := strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_GITHUB_APP_PRIVATE_KEY_PEM_BASE64"))
	githubRepository := strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_GITHUB_REPOSITORY"))
	githubEnabled := boolEnv("ASTER_OPERATIONS_GITHUB_ENABLED", false)
	publisherAppID, err := int64Env("ASTER_OPERATIONS_GITHUB_PUBLISH_APP_ID")
	if err != nil {
		return Config{}, err
	}
	publisherInstallationID, err := int64Env("ASTER_OPERATIONS_GITHUB_PUBLISH_INSTALLATION_ID")
	if err != nil {
		return Config{}, err
	}
	publisherEnabled := boolEnv("ASTER_OPERATIONS_GITHUB_PUBLISH_ENABLED", false)
	cfg := Config{
		UpgradeCredentialKey: strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_UPGRADE_CREDENTIAL_KEY_BASE64")),
		Address:              envOr("ASTER_OPERATIONS_ADDR", "127.0.0.1:12090"),
		Database: Database{
			Driver:          strings.ToLower(envOr("ASTER_OPERATIONS_DB_DRIVER", "mysql")),
			Host:            envOr("ASTER_OPERATIONS_DB_HOST", "127.0.0.1"),
			Port:            port,
			Name:            strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_DB_NAME")),
			User:            strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_DB_USER")),
			Password:        os.Getenv("ASTER_OPERATIONS_DB_PASSWORD"),
			TLS:             boolEnv("ASTER_OPERATIONS_DB_TLS", false),
			Charset:         envOr("ASTER_OPERATIONS_DB_CHARSET", "utf8mb4"),
			Location:        location,
			ConnectTimeout:  durationEnv("ASTER_OPERATIONS_DB_CONNECT_TIMEOUT", 5*time.Second),
			ReadTimeout:     durationEnv("ASTER_OPERATIONS_DB_READ_TIMEOUT", 10*time.Second),
			WriteTimeout:    durationEnv("ASTER_OPERATIONS_DB_WRITE_TIMEOUT", 10*time.Second),
			MaxOpenConns:    maxOpen,
			MaxIdleConns:    maxIdle,
			ConnMaxLifetime: durationEnv("ASTER_OPERATIONS_DB_CONN_MAX_LIFETIME", 3*time.Minute),
			ConnMaxIdleTime: durationEnv("ASTER_OPERATIONS_DB_CONN_MAX_IDLE_TIME", time.Minute),
		},
		BootstrapEmail:     strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_BOOTSTRAP_ADMIN_EMAIL")),
		BootstrapPassword:  os.Getenv("ASTER_OPERATIONS_BOOTSTRAP_ADMIN_PASSWORD"),
		CookieSecure:       boolEnv("ASTER_OPERATIONS_SESSION_SECURE", false),
		SessionTTL:         durationEnv("ASTER_OPERATIONS_SESSION_TTL", 12*time.Hour),
		TrustedOrigins:     parseOrigins(envOr("ASTER_OPERATIONS_TRUSTED_ORIGINS", "http://127.0.0.1:12080,http://localhost:12080")),
		AutoMigrate:        boolEnv("ASTER_OPERATIONS_AUTO_MIGRATE", false),
		CreateDatabase:     boolEnv("ASTER_OPERATIONS_CREATE_DATABASE", false),
		RequestBodyMaxSize: int64(bodyLimit),
		LicenseSigner: LicenseSigner{
			KeyID:           strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_LICENSE_SIGNING_KEY_ID")),
			PrivateKeyPKCS8: strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_LICENSE_SIGNING_PRIVATE_KEY_PKCS8")),
		},
		LicenseV2SignersJSON:        strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_LICENSE_V2_SIGNERS_JSON")),
		LicenseV2VerifiersJSON:      strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_LICENSE_V2_VERIFIERS_JSON")),
		ArtifactRoot:                envOr("ASTER_OPERATIONS_ARTIFACT_ROOT", "./data/operations-artifacts"),
		PublicCatalogRoot:           strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_PUBLIC_CATALOG_ROOT")),
		PublicationLocalOrigin:      strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_PUBLICATION_LOCAL_ORIGIN")),
		PublicationProductionOrigin: strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_PUBLICATION_PRODUCTION_ORIGIN")),
		QuotationEnvironment:        strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_QUOTATION_ENVIRONMENT")),
		FulfillmentEnvironment:      strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_FULFILLMENT_ENVIRONMENT")),
		CustomerReferenceSecret:     strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_CUSTOMER_REF_SECRET")),
		GitHubApp: GitHubApp{
			Enabled: githubEnabled, APIBaseURL: envOr("ASTER_OPERATIONS_GITHUB_API_URL", "https://api.github.com"),
			AppID: githubAppID, InstallationID: githubInstallationID, PrivateKeyPEMBase64: githubPrivateKey,
			Repository: githubRepository, WorkflowFile: envOr("ASTER_OPERATIONS_GITHUB_WORKFLOW", "customer-release.yml"),
			Environment: envOr("ASTER_OPERATIONS_GITHUB_ENVIRONMENT", "customer-release"), DefaultSourceRef: envOr("ASTER_OPERATIONS_GITHUB_DEFAULT_REF", "main"),
			PollInterval: durationEnv("ASTER_OPERATIONS_GITHUB_POLL_INTERVAL", 15*time.Second), RequestTimeout: durationEnv("ASTER_OPERATIONS_GITHUB_REQUEST_TIMEOUT", 20*time.Second),
			ArtifactDownloadTimeout: durationEnv("ASTER_OPERATIONS_GITHUB_ARTIFACT_DOWNLOAD_TIMEOUT", 5*time.Minute),
		},
		ReleaseVerification: ReleaseVerification{
			TrustedKeysJSON:  strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_RELEASE_TRUSTED_KEYS_JSON")),
			MaxArtifactBytes: maxArtifactBytes, MaxExpandedBytes: maxExpandedBytes,
		},
		GitHubPublisher: GitHubPublisher{Enabled: publisherEnabled,
			APIBaseURL:    envOr("ASTER_OPERATIONS_GITHUB_PUBLISH_API_URL", "https://api.github.com"),
			UploadBaseURL: envOr("ASTER_OPERATIONS_GITHUB_PUBLISH_UPLOAD_URL", "https://uploads.github.com"),
			AppID:         publisherAppID, InstallationID: publisherInstallationID,
			PrivateKeyPEMBase64: strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_GITHUB_PUBLISH_APP_PRIVATE_KEY_PEM_BASE64")),
			Repository:          strings.TrimSpace(os.Getenv("ASTER_OPERATIONS_GITHUB_PUBLISH_REPOSITORY")),
			RequestTimeout:      durationEnv("ASTER_OPERATIONS_GITHUB_PUBLISH_REQUEST_TIMEOUT", 2*time.Minute),
		},
	}
	if err := cfg.Validate(); err != nil {
		return Config{}, err
	}
	return cfg, nil
}

func (cfg Config) Validate() error {
	if cfg.FulfillmentEnvironment != "" && cfg.FulfillmentEnvironment != "local" && cfg.FulfillmentEnvironment != "production" {
		return errors.New("ASTER_OPERATIONS_FULFILLMENT_ENVIRONMENT must be empty, local or production")
	}
	if cfg.QuotationEnvironment != "" && cfg.QuotationEnvironment != "local" && cfg.QuotationEnvironment != "production" {
		return errors.New("ASTER_OPERATIONS_QUOTATION_ENVIRONMENT must be empty, local or production")
	}
	if _, _, err := net.SplitHostPort(cfg.Address); err != nil {
		return fmt.Errorf("invalid ASTER_OPERATIONS_ADDR: %w", err)
	}
	if cfg.Database.Driver != "mysql" {
		return errors.New("ASTER_OPERATIONS_DB_DRIVER must be mysql")
	}
	if cfg.Database.Host == "" || cfg.Database.Name == "" || cfg.Database.User == "" || cfg.Database.Password == "" {
		return errors.New("operations database host, name, user and password are required")
	}
	if !validIdentifier(cfg.Database.Name) {
		return errors.New("ASTER_OPERATIONS_DB_NAME must contain only letters, digits and underscores")
	}
	if cfg.Database.Charset != "utf8mb4" {
		return errors.New("ASTER_OPERATIONS_DB_CHARSET must be utf8mb4")
	}
	if cfg.SessionTTL < 15*time.Minute || cfg.SessionTTL > 7*24*time.Hour {
		return errors.New("ASTER_OPERATIONS_SESSION_TTL must be between 15m and 168h")
	}
	if len(cfg.TrustedOrigins) == 0 {
		return errors.New("at least one trusted origin is required")
	}
	if cfg.LicenseSigner.KeyID != "" || cfg.LicenseSigner.PrivateKeyPKCS8 != "" {
		return errors.New("legacy license signing configuration is no longer supported; configure ASTER_OPERATIONS_LICENSE_V2_SIGNERS_JSON")
	}
	if strings.TrimSpace(cfg.ArtifactRoot) == "" {
		return errors.New("ASTER_OPERATIONS_ARTIFACT_ROOT is required")
	}
	if err := cfg.GitHubApp.Validate(); err != nil {
		return err
	}
	if cfg.GitHubApp.Enabled {
		if cfg.ReleaseVerification.TrustedKeysJSON == "" {
			return errors.New("ASTER_OPERATIONS_RELEASE_TRUSTED_KEYS_JSON is required when release orchestration is enabled")
		}
		if cfg.ReleaseVerification.MaxArtifactBytes < 1 || cfg.ReleaseVerification.MaxExpandedBytes < cfg.ReleaseVerification.MaxArtifactBytes {
			return errors.New("release artifact verification size limits are invalid")
		}
	}
	if err := cfg.GitHubPublisher.Validate(); err != nil {
		return err
	}
	return nil
}

func (cfg GitHubApp) Validate() error {
	if !cfg.Enabled {
		return nil
	}
	if cfg.AppID <= 0 || cfg.InstallationID <= 0 || cfg.PrivateKeyPEMBase64 == "" {
		return errors.New("GitHub App ID, installation ID and private key are all required when release orchestration is enabled")
	}
	parts := strings.Split(cfg.Repository, "/")
	if len(parts) != 2 || !validGitHubSlug(parts[0]) || !validGitHubSlug(parts[1]) {
		return errors.New("ASTER_OPERATIONS_GITHUB_REPOSITORY must be owner/repository")
	}
	parsed, err := url.Parse(cfg.APIBaseURL)
	if err != nil || parsed.Scheme != "https" || parsed.Host == "" || parsed.RawQuery != "" || parsed.Fragment != "" {
		return errors.New("ASTER_OPERATIONS_GITHUB_API_URL must be an HTTPS origin")
	}
	if cfg.WorkflowFile == "" || strings.ContainsAny(cfg.WorkflowFile, "\\/") || !strings.HasSuffix(cfg.WorkflowFile, ".yml") {
		return errors.New("ASTER_OPERATIONS_GITHUB_WORKFLOW must be a fixed .yml file name")
	}
	if cfg.Environment == "" || cfg.DefaultSourceRef != "main" {
		return errors.New("GitHub release environment is required and the default source ref must be main")
	}
	if cfg.PollInterval < 5*time.Second || cfg.PollInterval > 5*time.Minute {
		return errors.New("GitHub poll interval is outside the allowed range")
	}
	if cfg.RequestTimeout < 5*time.Second || cfg.RequestTimeout > time.Minute {
		return errors.New("GitHub API request timeout is outside the allowed range")
	}
	if cfg.ArtifactDownloadTimeout < 30*time.Second || cfg.ArtifactDownloadTimeout > 30*time.Minute {
		return errors.New("GitHub artifact download timeout is outside the allowed range")
	}
	return nil
}

func (cfg GitHubPublisher) Validate() error {
	if !cfg.Enabled {
		return nil
	}
	if cfg.AppID <= 0 || cfg.InstallationID <= 0 || cfg.PrivateKeyPEMBase64 == "" {
		return errors.New("dedicated GitHub publish App ID, installation ID and private key are required when publishing is enabled")
	}
	parts := strings.Split(cfg.Repository, "/")
	if len(parts) != 2 || !validGitHubSlug(parts[0]) || !validGitHubSlug(parts[1]) {
		return errors.New("ASTER_OPERATIONS_GITHUB_PUBLISH_REPOSITORY must be owner/repository")
	}
	for name, raw := range map[string]string{"API": cfg.APIBaseURL, "upload": cfg.UploadBaseURL} {
		parsed, err := url.Parse(raw)
		if err != nil || parsed.Scheme != "https" || parsed.Host == "" || parsed.Path != "" || parsed.RawQuery != "" || parsed.Fragment != "" {
			return fmt.Errorf("GitHub publish %s URL must be an HTTPS origin", name)
		}
	}
	if cfg.RequestTimeout < 10*time.Second || cfg.RequestTimeout > 10*time.Minute {
		return errors.New("GitHub publish request timeout is outside the allowed range")
	}
	return nil
}

func validGitHubSlug(value string) bool {
	if value == "" || value == "." || value == ".." {
		return false
	}
	for _, char := range value {
		if (char < 'a' || char > 'z') && (char < 'A' || char > 'Z') && (char < '0' || char > '9') && char != '-' && char != '_' && char != '.' {
			return false
		}
	}
	return true
}

func loadEnvFile(path string) error {
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	defer file.Close()
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		key, value, found := strings.Cut(line, "=")
		if !found {
			return fmt.Errorf("invalid environment line for %q", line)
		}
		key = strings.TrimSpace(strings.TrimPrefix(key, "export "))
		value = strings.TrimSpace(value)
		if len(value) >= 2 && ((value[0] == '\'' && value[len(value)-1] == '\'') || (value[0] == '"' && value[len(value)-1] == '"')) {
			value = value[1 : len(value)-1]
		}
		if _, exists := os.LookupEnv(key); !exists {
			if err := os.Setenv(key, value); err != nil {
				return err
			}
		}
	}
	return scanner.Err()
}

func parseOrigins(value string) map[string]struct{} {
	origins := make(map[string]struct{})
	for _, raw := range strings.Split(value, ",") {
		origin := strings.TrimRight(strings.TrimSpace(raw), "/")
		parsed, err := url.Parse(origin)
		if err != nil || parsed.Scheme == "" || parsed.Host == "" || parsed.Path != "" || parsed.RawQuery != "" || parsed.Fragment != "" {
			continue
		}
		origins[origin] = struct{}{}
	}
	return origins
}

func validIdentifier(value string) bool {
	if value == "" {
		return false
	}
	for _, char := range value {
		if (char < 'a' || char > 'z') && (char < 'A' || char > 'Z') && (char < '0' || char > '9') && char != '_' {
			return false
		}
	}
	return true
}

func envOr(key, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(key)); value != "" {
		return value
	}
	return fallback
}

func boolEnv(key string, fallback bool) bool {
	value := strings.TrimSpace(os.Getenv(key))
	if value == "" {
		return fallback
	}
	parsed, err := strconv.ParseBool(value)
	if err != nil {
		return fallback
	}
	return parsed
}

func durationEnv(key string, fallback time.Duration) time.Duration {
	value := strings.TrimSpace(os.Getenv(key))
	if value == "" {
		return fallback
	}
	parsed, err := time.ParseDuration(value)
	if err != nil {
		return fallback
	}
	return parsed
}

func intEnv(key string, fallback, minimum, maximum int) (int, error) {
	value := strings.TrimSpace(os.Getenv(key))
	if value == "" {
		return fallback, nil
	}
	parsed, err := strconv.Atoi(value)
	if err != nil || parsed < minimum || parsed > maximum {
		return 0, fmt.Errorf("%s must be an integer between %d and %d", key, minimum, maximum)
	}
	return parsed, nil
}

func int64Env(key string) (int64, error) {
	value := strings.TrimSpace(os.Getenv(key))
	if value == "" {
		return 0, nil
	}
	parsed, err := strconv.ParseInt(value, 10, 64)
	if err != nil || parsed <= 0 {
		return 0, fmt.Errorf("%s must be a positive integer", key)
	}
	return parsed, nil
}

func boundedInt64Env(key string, fallback, minimum, maximum int64) (int64, error) {
	value := strings.TrimSpace(os.Getenv(key))
	if value == "" {
		return fallback, nil
	}
	parsed, err := strconv.ParseInt(value, 10, 64)
	if err != nil || parsed < minimum || parsed > maximum {
		return 0, fmt.Errorf("%s must be an integer between %d and %d", key, minimum, maximum)
	}
	return parsed, nil
}
