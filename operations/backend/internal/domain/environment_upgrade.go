package domain

import "time"

// Credentials are stored separately and never serialized into API responses.
type UpgradeCredentials struct {
	AdminEmail    string `json:"admin_email"`
	AdminPassword string `json:"admin_password"`
	APIKey        string `json:"api_key"`
}

type UpgradeEnvironment struct {
	ID                string    `json:"id"`
	Name              string    `json:"name"`
	InstallationID    string    `json:"installation_id"`
	AdminURL          string    `json:"admin_url"`
	MemberURL         string    `json:"member_url"`
	APIURL            string    `json:"api_url"`
	CAPEM             string    `json:"ca_pem"`
	Model             string    `json:"model"`
	CredentialVersion int       `json:"credential_version"`
	CreatedAt         time.Time `json:"created_at"`
}

type UpgradeEnvironmentInput struct {
	Name           string             `json:"name"`
	InstallationID string             `json:"installation_id"`
	AdminURL       string             `json:"admin_url"`
	MemberURL      string             `json:"member_url"`
	APIURL         string             `json:"api_url"`
	CAPEM          string             `json:"ca_pem"`
	Model          string             `json:"model"`
	Credentials    UpgradeCredentials `json:"credentials"`
}

type EnvironmentUpgrade struct {
	Summary          []UpgradeProbeSummary `json:"summary"`
	ID               string                `json:"id"`
	EnvironmentID    string                `json:"environment_id"`
	ArtifactID       string                `json:"artifact_id"`
	ArtifactSHA256   string                `json:"artifact_sha256"`
	TargetVersion    string                `json:"target_version"`
	CreatedBy        string                `json:"created_by"`
	Mode             string                `json:"mode"`
	Phase            string                `json:"phase"`
	TargetJobID      string                `json:"target_job_id"`
	UpgradeResult    string                `json:"upgrade_result"`
	ContinuityResult string                `json:"continuity_result"`
	RecoveryResult   string                `json:"recovery_result"`
	ErrorCode        string                `json:"error_code"`
	CoverageGap      bool                  `json:"coverage_gap"`
	ModelRequests    int                   `json:"model_requests"`
	CreatedAt        time.Time             `json:"created_at"`
	UpdatedAt        time.Time             `json:"updated_at"`
	BaselineUntil    time.Time             `json:"baseline_until"`
	ObserveUntil     time.Time             `json:"observe_until"`
	ProbeUntil       time.Time             `json:"probe_until"`
	BaselineFailed   bool                  `json:"baseline_failed"`
	PostFailed       bool                  `json:"post_failed"`
}

type UpgradeProbeSummary struct {
	Kind             string    `json:"kind"`
	Attempts         int       `json:"attempts"`
	Failures         int       `json:"failures"`
	LongestFailureMS int64     `json:"longest_failure_ms"`
	FailureSince     time.Time `json:"failure_since"`
	LastRecoveryAt   time.Time `json:"last_recovery_at"`
}

type UpgradeProbeSample struct {
	Sequence     int64     `json:"sequence"`
	Kind         string    `json:"kind"`
	Phase        string    `json:"phase"`
	At           time.Time `json:"at"`
	DurationMS   int64     `json:"duration_ms"`
	Status       int       `json:"status"`
	OK           bool      `json:"ok"`
	ErrorCode    string    `json:"error_code"`
	RequestID    string    `json:"request_id"`
	OutputEvents int       `json:"output_events"`
}

type EnvironmentUpgradeDetail struct {
	Task    EnvironmentUpgrade   `json:"task"`
	Samples []UpgradeProbeSample `json:"samples"`
}

type TargetMaintenance struct {
	InstallationID     string `json:"installation_id"`
	Platform           string `json:"platform"`
	Architecture       string `json:"architecture"`
	CurrentVersion     string `json:"current_version"`
	Busy               bool   `json:"busy"`
	CorrelatedUpgrades bool   `json:"correlated_upgrades"`
	Capabilities       struct {
		DatabaseDriver    string   `json:"database_driver"`
		SupportedModes    []string `json:"supported_modes"`
		UnavailableReason string   `json:"unavailable_reason"`
	} `json:"upgrade_capabilities"`
	Jobs []TargetUpgradeJob `json:"jobs"`
}

type TargetUpgradeJob struct {
	ID            string `json:"id"`
	Status        string `json:"status"`
	TargetVersion string `json:"target_version"`
}
