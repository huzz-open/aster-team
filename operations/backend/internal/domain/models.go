package domain

import (
	"encoding/json"
	"time"
)

type Operator struct {
	ID                     string    `json:"id"`
	Email                  string    `json:"email"`
	DisplayName            string    `json:"display_name"`
	Status                 string    `json:"status"`
	PasswordChangeRequired bool      `json:"password_change_required"`
	CreatedAt              time.Time `json:"created_at"`
}

type AuthenticatedOperator struct {
	Operator
	SessionID string
	CSRFHash  [32]byte
	ExpiresAt time.Time
}

type SessionTokens struct {
	SessionToken string
	CSRFToken    string
	ExpiresAt    time.Time
	Operator     Operator
}

type Customer struct {
	ID            string    `json:"id"`
	Name          string    `json:"name"`
	LegalName     string    `json:"legal_name"`
	Status        string    `json:"status"`
	ContactName   string    `json:"contact_name"`
	ContactEmail  string    `json:"contact_email"`
	ContactPhone  string    `json:"contact_phone"`
	ContactWeChat string    `json:"contact_wechat"`
	Notes         string    `json:"notes"`
	CreatedAt     time.Time `json:"created_at"`
	UpdatedAt     time.Time `json:"updated_at"`
}

type CustomerInput struct {
	Name          string `json:"name"`
	LegalName     string `json:"legal_name"`
	Status        string `json:"status"`
	ContactName   string `json:"contact_name"`
	ContactEmail  string `json:"contact_email"`
	ContactPhone  string `json:"contact_phone"`
	ContactWeChat string `json:"contact_wechat"`
	Notes         string `json:"notes"`
}

type Contact struct {
	ID         string    `json:"id"`
	CustomerID string    `json:"customer_id"`
	Name       string    `json:"name"`
	Email      string    `json:"email"`
	Phone      string    `json:"phone"`
	WeChat     string    `json:"wechat"`
	RoleTitle  string    `json:"role_title"`
	IsPrimary  bool      `json:"is_primary"`
	CreatedAt  time.Time `json:"created_at"`
	UpdatedAt  time.Time `json:"updated_at"`
}

type ContactInput struct {
	Name      string `json:"name"`
	Email     string `json:"email"`
	Phone     string `json:"phone"`
	WeChat    string `json:"wechat"`
	RoleTitle string `json:"role_title"`
	IsPrimary bool   `json:"is_primary"`
}

type BillingProfile struct {
	ID            string    `json:"id"`
	CustomerID    string    `json:"customer_id"`
	InvoiceTitle  string    `json:"invoice_title"`
	TaxIdentifier string    `json:"tax_identifier"`
	BillingEmail  string    `json:"billing_email"`
	Address       string    `json:"address"`
	CreatedAt     time.Time `json:"created_at"`
	UpdatedAt     time.Time `json:"updated_at"`
}

type BillingProfileInput struct {
	InvoiceTitle  string `json:"invoice_title"`
	TaxIdentifier string `json:"tax_identifier"`
	BillingEmail  string `json:"billing_email"`
	Address       string `json:"address"`
}

type CustomerProfile struct {
	Customer Customer        `json:"customer"`
	Contacts []Contact       `json:"contacts"`
	Billing  *BillingProfile `json:"billing_profile"`
}

type Overview struct {
	CustomersTotal          int64 `json:"customers_total"`
	CustomersActive         int64 `json:"customers_active"`
	CustomersLeads          int64 `json:"customers_leads"`
	PlansTotal              int64 `json:"plans_total"`
	OrdersPending           int64 `json:"orders_pending"`
	FreeDistributionsIssued int64 `json:"free_distributions_issued"`
	PaidFulfillmentsPending int64 `json:"paid_fulfillments_pending"`
	PaidLicensesIssued      int64 `json:"paid_licenses_issued"`
	ReleaseArtifactsTotal   int64 `json:"release_artifacts_total"`
	BackupsFailed           int64 `json:"backups_failed"`
}

type Plan struct {
	ID                        string    `json:"id"`
	ProductCode               string    `json:"product_code"`
	Code                      string    `json:"code"`
	Name                      string    `json:"name"`
	Edition                   string    `json:"edition"`
	Status                    string    `json:"status"`
	Features                  []string  `json:"features"`
	TransferLimit             int       `json:"transfer_limit"`
	MemberSeatsLimit          int       `json:"member_seats_limit"`
	SeatOverLimitGraceDays    int       `json:"seat_over_limit_grace_days"`
	MinimumVersion            string    `json:"minimum_version"`
	PriceVersionID            string    `json:"price_version_id"`
	Currency                  string    `json:"currency"`
	BillingCycle              string    `json:"billing_cycle"`
	BaseAmountMinor           int64     `json:"base_amount_minor"`
	IncludedMemberSeats       int       `json:"included_member_seats"`
	AdditionalMemberSeatMinor int64     `json:"additional_member_seat_minor"`
	TaxMode                   string    `json:"tax_mode"`
	CreatedAt                 time.Time `json:"created_at"`
	UpdatedAt                 time.Time `json:"updated_at"`
}

type PlanInput struct {
	Code                      string   `json:"code"`
	Name                      string   `json:"name"`
	Edition                   string   `json:"edition"`
	Features                  []string `json:"features"`
	TransferLimit             int      `json:"transfer_limit"`
	MemberSeatsLimit          int      `json:"member_seats_limit"`
	SeatOverLimitGraceDays    int      `json:"seat_over_limit_grace_days"`
	MinimumVersion            string   `json:"minimum_version"`
	Currency                  string   `json:"currency"`
	BillingCycle              string   `json:"billing_cycle"`
	BaseAmountMinor           int64    `json:"base_amount_minor"`
	IncludedMemberSeats       int      `json:"included_member_seats"`
	AdditionalMemberSeatMinor int64    `json:"additional_member_seat_minor"`
	TaxMode                   string   `json:"tax_mode"`
}

type PriceVersionInput struct {
	Currency                  string `json:"currency"`
	BillingCycle              string `json:"billing_cycle"`
	BaseAmountMinor           int64  `json:"base_amount_minor"`
	IncludedMemberSeats       int    `json:"included_member_seats"`
	AdditionalMemberSeatMinor int64  `json:"additional_member_seat_minor"`
	TaxMode                   string `json:"tax_mode"`
}

type Order struct {
	ID             string     `json:"id"`
	CustomerID     string     `json:"customer_id"`
	CustomerName   string     `json:"customer_name"`
	PlanID         string     `json:"plan_id"`
	PlanName       string     `json:"plan_name"`
	PriceVersionID string     `json:"price_version_id"`
	ContractRef    string     `json:"contract_ref"`
	Status         string     `json:"status"`
	AmountMinor    int64      `json:"amount_minor"`
	Currency       string     `json:"currency"`
	StartsAt       time.Time  `json:"starts_at"`
	EndsAt         time.Time  `json:"ends_at"`
	Notes          string     `json:"notes"`
	CreatedAt      time.Time  `json:"created_at"`
	UpdatedAt      time.Time  `json:"updated_at"`
	PaidAt         *time.Time `json:"paid_at"`
}

type OrderInput struct {
	CustomerID  string    `json:"customer_id"`
	PlanID      string    `json:"plan_id"`
	ContractRef string    `json:"contract_ref"`
	StartsAt    time.Time `json:"starts_at"`
	EndsAt      time.Time `json:"ends_at"`
	Notes       string    `json:"notes"`
}

type OfflinePaymentInput struct {
	OperationID      string `json:"operation_id"`
	PaymentReference string `json:"payment_reference"`
	Notes            string `json:"notes"`
	CurrentPassword  string `json:"current_password"`
}

type RefundNote struct {
	ID          string    `json:"id"`
	OrderID     string    `json:"order_id"`
	AmountMinor int64     `json:"amount_minor"`
	Reason      string    `json:"reason"`
	OperatorID  string    `json:"operator_id"`
	CreatedAt   time.Time `json:"created_at"`
}

type RefundNoteInput struct {
	AmountMinor     int64  `json:"amount_minor"`
	Reason          string `json:"reason"`
	CurrentPassword string `json:"current_password"`
}

type FulfillmentInput struct {
	OperationID string `json:"operation_id"`
	LicenseID   string `json:"license_id"`
}

type Trial struct {
	ID             string    `json:"id"`
	CustomerID     string    `json:"customer_id"`
	CustomerName   string    `json:"customer_name"`
	PlanID         string    `json:"plan_id"`
	PlanName       string    `json:"plan_name"`
	Status         string    `json:"status"`
	StartsAt       time.Time `json:"starts_at"`
	EndsAt         time.Time `json:"ends_at"`
	MemberSeats    int       `json:"member_seats"`
	TransferLimit  int       `json:"transfer_limit"`
	ApprovalReason string    `json:"approval_reason"`
	CreatedAt      time.Time `json:"created_at"`
	UpdatedAt      time.Time `json:"updated_at"`
}

type TrialInput struct {
	CustomerID     string    `json:"customer_id"`
	PlanID         string    `json:"plan_id"`
	StartsAt       time.Time `json:"starts_at"`
	EndsAt         time.Time `json:"ends_at"`
	MemberSeats    int       `json:"member_seats"`
	TransferLimit  int       `json:"transfer_limit"`
	ApprovalReason string    `json:"approval_reason"`
}

type TrialExtensionInput struct {
	EndsAt          time.Time `json:"ends_at"`
	Reason          string    `json:"reason"`
	CurrentPassword string    `json:"current_password"`
}

type RiskNote struct {
	ID         string    `json:"id"`
	TrialID    string    `json:"trial_id"`
	RiskLevel  string    `json:"risk_level"`
	Note       string    `json:"note"`
	OperatorID string    `json:"operator_id"`
	CreatedAt  time.Time `json:"created_at"`
}

type RiskNoteInput struct {
	RiskLevel string `json:"risk_level"`
	Note      string `json:"note"`
}

type BackupRecord struct {
	ID         string     `json:"id"`
	Kind       string     `json:"kind"`
	ObjectRef  string     `json:"object_ref"`
	SHA256     string     `json:"sha256"`
	SizeBytes  int64      `json:"size_bytes"`
	Status     string     `json:"status"`
	OperatorID string     `json:"operator_id"`
	CreatedAt  time.Time  `json:"created_at"`
	VerifiedAt *time.Time `json:"verified_at"`
}

// OperationsExport is a business report, not a database restore or signing input.
type OperationsExport struct {
	SchemaVersion     string                       `json:"schema_version"`
	ExportedAt        time.Time                    `json:"exported_at"`
	Customers         []Customer                   `json:"customers"`
	Contacts          []Contact                    `json:"contacts"`
	Billing           []BillingProfile             `json:"billing_profiles"`
	CommercialRecords []OperationsCommercialRecord `json:"commercial_records"`
	ReleaseArtifacts  []ReleaseArtifact            `json:"release_artifacts"`
	AuditEvents       []AuditEvent                 `json:"audit_events"`
	Backups           []BackupRecord               `json:"backup_history"`
}

// A record projects a business snapshot and its current workflow state.
// SourceSnapshotSHA256 identifies the original stored snapshot, not the projection.
// It intentionally excludes signed License documents and all key material.
type OperationsCommercialRecord struct {
	Kind                 string          `json:"kind"`
	ID                   string          `json:"id"`
	Status               string          `json:"status"`
	Snapshot             json.RawMessage `json:"snapshot"`
	SourceSnapshotSHA256 string          `json:"source_snapshot_sha256"`
	DocumentSHA256       string          `json:"document_sha256,omitempty"`
}

type ProductRecord struct {
	ID        string    `json:"id"`
	Code      string    `json:"code"`
	Name      string    `json:"name"`
	Status    string    `json:"status"`
	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
}
type PriceVersionRecord struct {
	ID                        string    `json:"id"`
	PlanID                    string    `json:"plan_id"`
	VersionNo                 int       `json:"version_no"`
	Currency                  string    `json:"currency"`
	BillingCycle              string    `json:"billing_cycle"`
	BaseAmountMinor           int64     `json:"base_amount_minor"`
	IncludedMemberSeats       int       `json:"included_member_seats"`
	AdditionalMemberSeatMinor int64     `json:"additional_member_seat_minor"`
	TaxMode                   string    `json:"tax_mode"`
	PublishedAt               time.Time `json:"published_at"`
	CreatedAt                 time.Time `json:"created_at"`
}
type OrderItemRecord struct {
	ID              string          `json:"id"`
	OrderID         string          `json:"order_id"`
	Description     string          `json:"description"`
	Quantity        int             `json:"quantity"`
	UnitAmountMinor int64           `json:"unit_amount_minor"`
	PriceSnapshot   json.RawMessage `json:"price_snapshot"`
	CreatedAt       time.Time       `json:"created_at"`
}
type OfflinePaymentConfirmation struct {
	ID               string    `json:"id"`
	OperationID      string    `json:"operation_id"`
	OrderID          string    `json:"order_id"`
	PaymentReference string    `json:"payment_reference"`
	AmountMinor      int64     `json:"amount_minor"`
	Currency         string    `json:"currency"`
	ConfirmedBy      string    `json:"confirmed_by"`
	ReviewedBy       string    `json:"reviewed_by"`
	ConfirmedAt      time.Time `json:"confirmed_at"`
	Notes            string    `json:"notes"`
}
type LocalLicenseRecord struct {
	LicenseID     string          `json:"license_id"`
	CustomerID    string          `json:"customer_id"`
	SourceType    string          `json:"source_type"`
	SourceID      string          `json:"source_id"`
	CustomerRef   string          `json:"customer_ref"`
	Policy        json.RawMessage `json:"policy"`
	Status        string          `json:"status"`
	TransferLimit int             `json:"transfer_limit"`
	TransferCount int             `json:"transfer_count"`
	CreatedAt     time.Time       `json:"created_at"`
	UpdatedAt     time.Time       `json:"updated_at"`
}

type LocalLicenseIssuance struct {
	ID                       string          `json:"id"`
	LicenseID                string          `json:"license_id"`
	RequestID                string          `json:"request_id"`
	InstallationID           string          `json:"installation_id"`
	MachineFingerprintSHA256 string          `json:"machine_fingerprint_sha256"`
	TransferSequence         int             `json:"transfer_sequence"`
	Document                 json.RawMessage `json:"document"`
	SHA256                   string          `json:"sha256"`
	IssuedBy                 string          `json:"issued_by"`
	IssuedAt                 time.Time       `json:"issued_at"`
	ExpiresAt                time.Time       `json:"expires_at"`
}

type AuditEvent struct {
	ID           string          `json:"id"`
	OperatorID   string          `json:"operator_id"`
	Action       string          `json:"action"`
	ResourceType string          `json:"resource_type"`
	ResourceID   string          `json:"resource_id"`
	Payload      json.RawMessage `json:"payload"`
	CreatedAt    time.Time       `json:"created_at"`
}

type ReleaseArtifact struct {
	ID                    string    `json:"id"`
	Version               string    `json:"version"`
	Platform              string    `json:"platform"`
	Architecture          string    `json:"architecture"`
	ObjectKey             string    `json:"object_key"`
	SHA256                string    `json:"sha256"`
	ReleaseManifestSHA256 string    `json:"release_manifest_sha256"`
	SizeBytes             int64     `json:"size_bytes"`
	SignatureRef          string    `json:"signature_ref"`
	SourceCommitSHA       *string   `json:"source_commit_sha"`
	GitHubRunID           *int64    `json:"github_run_id"`
	RuntimeLinkage        string    `json:"runtime_linkage"`
	CreatedAt             time.Time `json:"created_at"`
}

type ReleaseArtifactInput struct {
	InboxFilename         string `json:"inbox_filename"`
	Version               string `json:"version"`
	Platform              string `json:"platform"`
	Architecture          string `json:"architecture"`
	ExpectedSHA256        string `json:"expected_sha256"`
	ReleaseManifestSHA256 string `json:"release_manifest_sha256"`
	SignatureRef          string `json:"signature_ref"`
}

type ReleaseTask struct {
	Packages           []ReleaseTaskArtifact `json:"packages"`
	ID                 string                `json:"id"`
	Version            string                `json:"version"`
	Mode               string                `json:"mode"`
	FreeDistributionID string                `json:"free_distribution_id"`
	FreeLicenseSHA256  string                `json:"free_license_sha256"`
	GitHubRepository   string                `json:"github_repository"`
	WorkflowFile       string                `json:"workflow_file"`
	SourceRef          string                `json:"source_ref"`
	SourceCommitSHA    string                `json:"source_commit_sha"`
	Phase              string                `json:"phase"`
	Status             string                `json:"status"`
	ErrorCode          *string               `json:"error_code"`
	RetryOfTaskID      *string               `json:"retry_of_task_id"`
	CreatedBy          string                `json:"created_by"`
	CreatedAt          time.Time             `json:"created_at"`
	UpdatedAt          time.Time             `json:"updated_at"`
	StartedAt          *time.Time            `json:"started_at"`
	CompletedAt        *time.Time            `json:"completed_at"`
	GitHubRunURL       string                `json:"github_run_url"`
	GitHubConclusion   *string               `json:"github_conclusion"`
}

type ReleaseRun struct {
	ID              string       `json:"id"`
	ReleaseTaskID   string       `json:"release_task_id"`
	Attempt         int          `json:"attempt"`
	GitHubRunID     *int64       `json:"github_run_id"`
	GitHubRunNumber *int64       `json:"github_run_number"`
	WorkflowName    string       `json:"workflow_name"`
	HeadBranch      string       `json:"head_branch"`
	HeadSHA         string       `json:"head_sha"`
	Status          string       `json:"status"`
	Conclusion      *string      `json:"conclusion"`
	HTMLURL         string       `json:"html_url"`
	StartedAt       *time.Time   `json:"started_at"`
	CompletedAt     *time.Time   `json:"completed_at"`
	SyncedAt        *time.Time   `json:"synced_at"`
	CreatedAt       time.Time    `json:"created_at"`
	Jobs            []ReleaseJob `json:"jobs"`
}

type ReleaseJob struct {
	ID           string        `json:"id"`
	ReleaseRunID string        `json:"release_run_id"`
	GitHubJobID  int64         `json:"github_job_id"`
	SequenceNo   int           `json:"sequence_no"`
	Name         string        `json:"name"`
	RunnerName   string        `json:"runner_name"`
	Status       string        `json:"status"`
	Conclusion   *string       `json:"conclusion"`
	HTMLURL      string        `json:"html_url"`
	StartedAt    *time.Time    `json:"started_at"`
	CompletedAt  *time.Time    `json:"completed_at"`
	Steps        []ReleaseStep `json:"steps"`
}

type ReleaseStep struct {
	ID               string     `json:"id"`
	ReleaseRunJobID  string     `json:"release_run_job_id"`
	GitHubStepNumber int        `json:"github_step_number"`
	SequenceNo       int        `json:"sequence_no"`
	Name             string     `json:"name"`
	Status           string     `json:"status"`
	Conclusion       *string    `json:"conclusion"`
	FailureSummary   string     `json:"failure_summary"`
	LogRef           string     `json:"log_ref"`
	StartedAt        *time.Time `json:"started_at"`
	CompletedAt      *time.Time `json:"completed_at"`
}

type ReleaseTaskArtifact struct {
	Platform              string     `json:"platform"`
	Architecture          string     `json:"architecture"`
	ID                    string     `json:"id"`
	ReleaseTaskID         string     `json:"release_task_id"`
	ReleaseRunID          string     `json:"release_run_id"`
	GitHubArtifactID      int64      `json:"github_artifact_id"`
	Name                  string     `json:"name"`
	FileName              string     `json:"file_name"`
	SizeBytes             int64      `json:"size_bytes"`
	ExpiresAt             *time.Time `json:"expires_at"`
	DownloadRef           string     `json:"download_ref"`
	GitHubDigestSHA256    *string    `json:"github_digest_sha256"`
	SHA256                *string    `json:"sha256"`
	ReleaseManifestSHA256 *string    `json:"release_manifest_sha256"`
	SignatureKeyID        string     `json:"signature_key_id"`
	RuntimeLinkage        string     `json:"runtime_linkage"`
	VerificationStatus    string     `json:"verification_status"`
	VerificationErrorCode *string    `json:"verification_error_code"`
	ReleaseArtifactID     *string    `json:"release_artifact_id"`
	VerifiedAt            *time.Time `json:"verified_at"`
	CreatedAt             time.Time  `json:"created_at"`
}

type ReleaseTaskDetail struct {
	Task      ReleaseTask           `json:"task"`
	Runs      []ReleaseRun          `json:"runs"`
	Artifacts []ReleaseTaskArtifact `json:"artifacts"`
}

type ReleaseTaskInput struct {
	Version            string `json:"version"`
	SourceRef          string `json:"source_ref"`
	FreeDistributionID string `json:"free_distribution_id"`
}

type ReleaseCapabilities struct {
	Targets              []ReleaseTarget `json:"targets"`
	Configured           bool            `json:"configured"`
	Repository           string          `json:"repository"`
	WorkflowFile         string          `json:"workflow_file"`
	Environment          string          `json:"environment"`
	DefaultSourceRef     string          `json:"default_source_ref"`
	PublishingConfigured bool            `json:"publishing_configured"`
}

type ReleasePublishRequest struct {
	ID                string     `json:"id"`
	ReleaseArtifactID string     `json:"release_artifact_id"`
	Version           string     `json:"version"`
	SourceCommitSHA   string     `json:"source_commit_sha"`
	TagName           string     `json:"tag_name"`
	Status            string     `json:"status"`
	ErrorCode         *string    `json:"error_code"`
	RequestedBy       string     `json:"requested_by"`
	ApprovedBy        *string    `json:"approved_by"`
	ExecutedBy        *string    `json:"executed_by"`
	ApprovalComment   string     `json:"approval_comment"`
	GitHubReleaseID   *int64     `json:"github_release_id"`
	HTMLURL           string     `json:"html_url"`
	CreatedAt         time.Time  `json:"created_at"`
	UpdatedAt         time.Time  `json:"updated_at"`
	ApprovedAt        *time.Time `json:"approved_at"`
	PublishedAt       *time.Time `json:"published_at"`
}

type ReleasePublishApprovalInput struct {
	Decision string `json:"decision"`
	Comment  string `json:"comment"`
}

type DeliveryRecord struct {
	ID                string     `json:"id"`
	CustomerID        string     `json:"customer_id"`
	OrderID           *string    `json:"order_id"`
	TrialID           *string    `json:"trial_id"`
	LicenseID         string     `json:"license_id"`
	ReleaseArtifactID string     `json:"release_artifact_id"`
	Channel           string     `json:"channel"`
	Recipient         string     `json:"recipient"`
	ReceiptObjectKey  string     `json:"receipt_object_key"`
	ReceiptSHA256     string     `json:"receipt_sha256"`
	DeliveredAt       *time.Time `json:"delivered_at"`
	CreatedAt         time.Time  `json:"created_at"`
}

type DeliveryInput struct {
	CustomerID        string  `json:"customer_id"`
	OrderID           *string `json:"order_id"`
	TrialID           *string `json:"trial_id"`
	LicenseID         string  `json:"license_id"`
	ReleaseArtifactID string  `json:"release_artifact_id"`
	Channel           string  `json:"channel"`
	Recipient         string  `json:"recipient"`
}
