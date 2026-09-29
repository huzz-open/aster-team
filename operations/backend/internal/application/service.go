package application

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/mail"
	"regexp"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
	"aster.local/team/operations/backend/internal/productcatalog"
	"golang.org/x/crypto/bcrypt"
)

var (
	ErrInvalidCredentials          = errors.New("invalid credentials")
	ErrUnauthorized                = errors.New("unauthorized")
	ErrValidation                  = errors.New("validation failed")
	ErrNotFound                    = errors.New("not found")
	ErrBusinessStoreUnavailable    = errors.New("operations business store is unavailable")
	ErrReleaseNotConfigured        = errors.New("release orchestrator is not configured")
	ErrReleasePreflight            = errors.New("release preflight failed")
	ErrReleaseDuplicate            = errors.New("an active release task already exists")
	ErrReleaseDispatch             = errors.New("release dispatch failed")
	ErrReleaseSync                 = errors.New("release synchronization failed")
	ErrReleasePublishNotConfigured = errors.New("release publisher is not configured")
	ErrReleasePublishApproval      = errors.New("release publish approval failed")
	ErrReleasePublish              = errors.New("release publish failed")
	ErrReleasePublishDuplicate     = errors.New("an active publish request already exists")
)

var releaseCommitPattern = regexp.MustCompile(`^[0-9a-f]{40}$`)

const (
	PermissionReleaseRead           = "release.read"
	PermissionReleaseBuild          = "release.build"
	PermissionReleaseDownload       = "release.download"
	PermissionReleasePublishRequest = "release.publish.request"
	PermissionReleasePublishApprove = "release.publish.approve"
	PermissionReleasePublishExecute = "release.publish.execute"
)

func BootstrapReleasePermissions() []string {
	return []string{
		PermissionReleaseRead,
		PermissionReleaseBuild,
		PermissionReleaseDownload,
		PermissionReleasePublishRequest,
		PermissionReleasePublishApprove,
		PermissionReleasePublishExecute,
		PermissionEnvironmentWrite,
		PermissionEnvironmentUpgrade,
	}
}

type Store interface {
	OperatorCount(context.Context) (int64, error)
	CreateOperator(context.Context, CreateOperatorParams) (domain.Operator, error)
	FindOperatorForLogin(context.Context, string) (domain.Operator, string, error)
	CreateSession(context.Context, CreateSessionParams) error
	AuthenticateSession(context.Context, [32]byte, time.Time) (domain.AuthenticatedOperator, error)
	DeleteSession(context.Context, [32]byte) error
	DeleteExpiredSessions(context.Context, time.Time) error
	CreateCustomer(context.Context, domain.Customer, string) error
	ListCustomers(context.Context, string, int) ([]domain.Customer, error)
	Overview(context.Context) (domain.Overview, error)
}

type BusinessStore interface {
	CreatePlan(context.Context, domain.Plan, string) error
	PublishPlanPrice(context.Context, domain.PriceVersionRecord, string) (domain.PriceVersionRecord, error)
	ListPlans(context.Context, int) ([]domain.Plan, error)
	CreateOrder(context.Context, domain.Order, string) error
	ListOrders(context.Context, int) ([]domain.Order, error)
	ConfirmOfflinePayment(context.Context, string, domain.OfflinePaymentInput, string, time.Time) (domain.Order, error)
	CreateTrial(context.Context, domain.Trial, string) error
	ListTrials(context.Context, int) ([]domain.Trial, error)
	ListAuditEvents(context.Context, int) ([]domain.AuditEvent, error)
	FulfillmentCustomer(context.Context, string, string) (string, error)
	PrepareSourceFulfillment(context.Context, string, string, domain.FulfillmentInput, string, string, time.Time) (ports.LicenseRecord, error)
	CreateReleaseArtifact(context.Context, domain.ReleaseArtifact, string) error
	ListReleaseArtifacts(context.Context, int) ([]domain.ReleaseArtifact, error)
	GetReleaseArtifact(context.Context, string) (domain.ReleaseArtifact, error)
	ValidateDelivery(context.Context, domain.DeliveryInput) error
	CreateDelivery(context.Context, domain.DeliveryRecord, string) error
	ListDeliveries(context.Context, int) ([]domain.DeliveryRecord, error)
	GetDelivery(context.Context, string) (domain.DeliveryRecord, error)
}

type LicenseStore interface {
	ListLicenseRecords(context.Context, int) ([]ports.LicenseRecord, error)
	LicenseForIssuance(context.Context, string) (ports.LicenseRecord, error)
	SaveLicenseIssuance(context.Context, ports.LicenseIssuance, string) error
	ListLicenseIssuances(context.Context, string) ([]ports.LicenseIssuance, error)
}

type PasswordStore interface {
	UpdateOperatorPassword(context.Context, string, string, string, time.Time) error
}

type SupportStore interface {
	UpdateCustomer(context.Context, domain.Customer, string) error
	GetCustomerProfile(context.Context, string) (domain.CustomerProfile, error)
	CreateContact(context.Context, domain.Contact, string) error
	UpsertBillingProfile(context.Context, domain.BillingProfile, string) error
	CreateRefundNote(context.Context, domain.RefundNote, string) error
	ListRefundNotes(context.Context, string) ([]domain.RefundNote, error)
	ExtendTrial(context.Context, string, time.Time, string, string, time.Time) (domain.Trial, error)
	CreateRiskNote(context.Context, domain.RiskNote, string) error
	ListRiskNotes(context.Context, string) ([]domain.RiskNote, error)
	ListBackupHistory(context.Context, int) ([]domain.BackupRecord, error)
	ExportOperations(context.Context, string, time.Time) (domain.OperationsExport, error)
}

type PermissionStore interface {
	HasPermission(context.Context, string, string) (bool, error)
}

type ReleaseTaskStore interface {
	ListReleaseTasks(context.Context, int) ([]domain.ReleaseTask, error)
	GetReleaseTaskDetail(context.Context, string) (domain.ReleaseTaskDetail, error)
	CreateReleaseTask(context.Context, domain.ReleaseTask, string) error
	RecordReleaseDispatch(context.Context, string, domain.ReleaseRun) error
	MarkReleaseTaskFailed(context.Context, string, string) error
	ApplyReleaseSnapshot(context.Context, string, ports.ReleaseSnapshot, string, string, *string, time.Time) error
	CompleteReleaseTaskArtifact(context.Context, string, string, domain.ReleaseArtifact, string, time.Time) error
	FailReleaseTaskArtifact(context.Context, string, string, string, time.Time) error
	PrepareReleaseTaskArtifactVerification(context.Context, string, string, string, time.Time) error
	FinishReleaseTaskVerification(context.Context, string, *string, time.Time) error
	ListPendingReleaseTaskIDs(context.Context, int) ([]string, error)
}

type ReleasePublishStore interface {
	ListReleasePublishRequests(context.Context, int) ([]domain.ReleasePublishRequest, error)
	GetReleasePublishRequest(context.Context, string) (domain.ReleasePublishRequest, error)
	CreateReleasePublishRequest(context.Context, domain.ReleasePublishRequest) error
	DecideReleasePublishRequest(context.Context, string, string, string, string, time.Time) error
	BeginReleasePublish(context.Context, string, string, time.Time) (domain.ReleasePublishRequest, error)
	CompleteReleasePublish(context.Context, string, ports.ReleasePublishResult, time.Time) error
	FailReleasePublish(context.Context, string, string, time.Time) error
	ListPublishingReleaseRequestIDs(context.Context, int) ([]string, error)
}

type CreateOperatorParams struct {
	ID                     string
	Email                  string
	NormalizedEmail        string
	DisplayName            string
	PasswordHash           string
	PasswordChangeRequired bool
	CreatedAt              time.Time
}

type CreateSessionParams struct {
	ID         string
	OperatorID string
	TokenHash  [32]byte
	CSRFHash   [32]byte
	ExpiresAt  time.Time
	CreatedAt  time.Time
}

type Service struct {
	environmentUpgrades    EnvironmentUpgradeStore
	upgradeSecrets         ports.UpgradeSecretStore
	upgradeTargets         ports.UpgradeTargetFactory
	store                  Store
	now                    func() time.Time
	sessionTTL             time.Duration
	licenseSignersV2       map[string]ports.LicenseSignerV2
	licenseVerifiersV2     map[string]ports.LicenseVerifierV2
	licenses               LicenseStore
	business               BusinessStore
	artifacts              ports.ArtifactStore
	catalogExporter        PublicCatalogExporter
	publicationVerifiers   map[string]PublicationVerifier
	quotationEnvironment   string
	fulfillmentEnvironment string
	passwords              PasswordStore
	support                SupportStore
	customerRefs           ports.CustomerReferenceSource
	permissions            PermissionStore
	releaseTasks           ReleaseTaskStore
	releases               ports.ReleaseOrchestrator
	releaseVerifier        ports.ReleaseArtifactVerifier
	releasePublisher       ports.ReleasePublisher
	releasePublishes       ReleasePublishStore
}

type Option func(*Service)

func WithArtifactStore(store ports.ArtifactStore) Option {
	return func(service *Service) { service.artifacts = store }
}

func WithCustomerReferenceSource(source ports.CustomerReferenceSource) Option {
	return func(service *Service) { service.customerRefs = source }
}

func WithReleaseOrchestrator(orchestrator ports.ReleaseOrchestrator) Option {
	return func(service *Service) { service.releases = orchestrator }
}

func WithReleaseArtifactVerifier(verifier ports.ReleaseArtifactVerifier) Option {
	return func(service *Service) { service.releaseVerifier = verifier }
}

func WithReleasePublisher(publisher ports.ReleasePublisher) Option {
	return func(service *Service) { service.releasePublisher = publisher }
}

func NewService(store Store, sessionTTL time.Duration, options ...Option) *Service {
	service := &Service{store: store, now: time.Now, sessionTTL: sessionTTL}
	service.business, _ = store.(BusinessStore)
	service.passwords, _ = store.(PasswordStore)
	service.support, _ = store.(SupportStore)
	service.licenses, _ = store.(LicenseStore)
	service.permissions, _ = store.(PermissionStore)
	service.releaseTasks, _ = store.(ReleaseTaskStore)
	service.releasePublishes, _ = store.(ReleasePublishStore)
	for _, option := range options {
		option(service)
	}
	return service
}

func (service *Service) RequirePermission(ctx context.Context, operatorID, permission string) error {
	if service.permissions == nil {
		return ErrBusinessStoreUnavailable
	}
	allowed, err := service.permissions.HasPermission(ctx, operatorID, permission)
	if err != nil {
		return err
	}
	if !allowed {
		return ErrUnauthorized
	}
	return nil
}

func (service *Service) ReleaseCapabilities() domain.ReleaseCapabilities {
	if service.releases == nil {
		return domain.ReleaseCapabilities{Configured: false, PublishingConfigured: service.releasePublisher != nil && service.releasePublisher.Configured(), Targets: domain.ReleaseTargets()}
	}
	capabilities := service.releases.Capabilities()
	capabilities.PublishingConfigured = service.releasePublisher != nil && service.releasePublisher.Configured()
	return capabilities
}

func (service *Service) BootstrapAdmin(ctx context.Context, email, password string) error {
	count, err := service.store.OperatorCount(ctx)
	if err != nil {
		return err
	}
	if count > 0 {
		return nil
	}
	email = normalizeEmail(email)
	if email == "" || password == "" {
		return errors.New("bootstrap administrator credentials are required for an empty OperationsStore")
	}
	if err := validateEmail(email); err != nil {
		return err
	}
	if err := validatePassword(password); err != nil {
		return err
	}
	hash, err := bcrypt.GenerateFromPassword([]byte(password), 12)
	if err != nil {
		return fmt.Errorf("hash bootstrap password: %w", err)
	}
	now := service.now().UTC()
	_, err = service.store.CreateOperator(ctx, CreateOperatorParams{
		ID:                     newID("op", now),
		Email:                  email,
		NormalizedEmail:        email,
		DisplayName:            "Aster Team Operator",
		PasswordHash:           string(hash),
		PasswordChangeRequired: true,
		CreatedAt:              now,
	})
	return err
}

func (service *Service) Login(ctx context.Context, email, password string) (domain.SessionTokens, error) {
	email = normalizeEmail(email)
	if email == "" || password == "" {
		return domain.SessionTokens{}, ErrInvalidCredentials
	}
	operator, passwordHash, err := service.store.FindOperatorForLogin(ctx, email)
	if err != nil || operator.Status != "active" || bcrypt.CompareHashAndPassword([]byte(passwordHash), []byte(password)) != nil {
		return domain.SessionTokens{}, ErrInvalidCredentials
	}
	sessionToken, err := randomToken()
	if err != nil {
		return domain.SessionTokens{}, err
	}
	csrfToken, err := randomToken()
	if err != nil {
		return domain.SessionTokens{}, err
	}
	now := service.now().UTC()
	expiresAt := now.Add(service.sessionTTL)
	if err := service.store.CreateSession(ctx, CreateSessionParams{
		ID:         newID("sess", now),
		OperatorID: operator.ID,
		TokenHash:  sha256.Sum256([]byte(sessionToken)),
		CSRFHash:   sha256.Sum256([]byte(csrfToken)),
		ExpiresAt:  expiresAt,
		CreatedAt:  now,
	}); err != nil {
		return domain.SessionTokens{}, err
	}
	return domain.SessionTokens{SessionToken: sessionToken, CSRFToken: csrfToken, ExpiresAt: expiresAt, Operator: operator}, nil
}

func (service *Service) Authenticate(ctx context.Context, sessionToken string) (domain.AuthenticatedOperator, error) {
	if sessionToken == "" {
		return domain.AuthenticatedOperator{}, ErrUnauthorized
	}
	operator, err := service.store.AuthenticateSession(ctx, sha256.Sum256([]byte(sessionToken)), service.now().UTC())
	if err != nil || operator.Status != "active" {
		return domain.AuthenticatedOperator{}, ErrUnauthorized
	}
	return operator, nil
}

func (service *Service) ValidateCSRF(operator domain.AuthenticatedOperator, cookieToken, headerToken string) bool {
	if cookieToken == "" || headerToken == "" || subtle.ConstantTimeCompare([]byte(cookieToken), []byte(headerToken)) != 1 {
		return false
	}
	hash := sha256.Sum256([]byte(headerToken))
	return subtle.ConstantTimeCompare(hash[:], operator.CSRFHash[:]) == 1
}

func (service *Service) Logout(ctx context.Context, sessionToken string) error {
	if sessionToken == "" {
		return nil
	}
	return service.store.DeleteSession(ctx, sha256.Sum256([]byte(sessionToken)))
}

func (service *Service) ChangePassword(ctx context.Context, operator domain.AuthenticatedOperator, currentPassword, newPassword string) error {
	if service.passwords == nil {
		return ErrUnauthorized
	}
	_, passwordHash, err := service.store.FindOperatorForLogin(ctx, normalizeEmail(operator.Email))
	if err != nil || bcrypt.CompareHashAndPassword([]byte(passwordHash), []byte(currentPassword)) != nil {
		return ErrInvalidCredentials
	}
	if err := validatePassword(newPassword); err != nil {
		return fmt.Errorf("%w: %v", ErrValidation, err)
	}
	if currentPassword == newPassword {
		return fmt.Errorf("%w: new password must differ from current password", ErrValidation)
	}
	hash, err := bcrypt.GenerateFromPassword([]byte(newPassword), 12)
	if err != nil {
		return err
	}
	return service.passwords.UpdateOperatorPassword(ctx, operator.ID, operator.SessionID, string(hash), service.now().UTC())
}

func (service *Service) Reauthenticate(ctx context.Context, operator domain.AuthenticatedOperator, currentPassword string) error {
	if currentPassword == "" {
		return ErrInvalidCredentials
	}
	current, passwordHash, err := service.store.FindOperatorForLogin(ctx, normalizeEmail(operator.Email))
	if err != nil || current.ID != operator.ID || current.Status != "active" || bcrypt.CompareHashAndPassword([]byte(passwordHash), []byte(currentPassword)) != nil {
		return ErrInvalidCredentials
	}
	return nil
}

func (service *Service) CreateCustomer(ctx context.Context, input domain.CustomerInput, operatorID string) (domain.Customer, error) {
	input = normalizeCustomer(input)
	if err := validateCustomer(input); err != nil {
		return domain.Customer{}, err
	}
	now := service.now().UTC()
	customer := domain.Customer{
		ID:            newID("cust", now),
		Name:          input.Name,
		LegalName:     input.LegalName,
		Status:        input.Status,
		ContactName:   input.ContactName,
		ContactEmail:  input.ContactEmail,
		ContactPhone:  input.ContactPhone,
		ContactWeChat: input.ContactWeChat,
		Notes:         input.Notes,
		CreatedAt:     now,
		UpdatedAt:     now,
	}
	if err := service.store.CreateCustomer(ctx, customer, operatorID); err != nil {
		return domain.Customer{}, err
	}
	return customer, nil
}

func (service *Service) ListCustomers(ctx context.Context, after string, limit int) ([]domain.Customer, string, error) {
	if limit <= 0 || limit > 100 {
		limit = 50
	}
	customers, err := service.store.ListCustomers(ctx, after, limit+1)
	if err != nil {
		return nil, "", err
	}
	next := ""
	if len(customers) > limit {
		next = customers[limit-1].ID
		customers = customers[:limit]
	}
	return customers, next, nil
}

func (service *Service) UpdateCustomer(ctx context.Context, customerID string, input domain.CustomerInput, operatorID string) (domain.Customer, error) {
	input = normalizeCustomer(input)
	if !identifierPattern(customerID) {
		return domain.Customer{}, fmt.Errorf("%w: customer id is invalid", ErrValidation)
	}
	if err := validateCustomer(input); err != nil {
		return domain.Customer{}, err
	}
	if service.support == nil {
		return domain.Customer{}, ErrBusinessStoreUnavailable
	}
	profile, err := service.support.GetCustomerProfile(ctx, customerID)
	if err != nil {
		return domain.Customer{}, err
	}
	updated := profile.Customer
	updated.Name, updated.LegalName, updated.Status = input.Name, input.LegalName, input.Status
	updated.ContactName, updated.ContactEmail = input.ContactName, input.ContactEmail
	updated.ContactPhone, updated.ContactWeChat, updated.Notes = input.ContactPhone, input.ContactWeChat, input.Notes
	updated.UpdatedAt = service.now().UTC()
	if err := service.support.UpdateCustomer(ctx, updated, operatorID); err != nil {
		return domain.Customer{}, err
	}
	return updated, nil
}

func (service *Service) GetCustomerProfile(ctx context.Context, customerID string) (domain.CustomerProfile, error) {
	if !identifierPattern(customerID) {
		return domain.CustomerProfile{}, fmt.Errorf("%w: customer id is invalid", ErrValidation)
	}
	if service.support == nil {
		return domain.CustomerProfile{}, ErrBusinessStoreUnavailable
	}
	return service.support.GetCustomerProfile(ctx, customerID)
}

func (service *Service) CreateContact(ctx context.Context, customerID string, input domain.ContactInput, operatorID string) (domain.Contact, error) {
	input.Name = strings.TrimSpace(input.Name)
	input.Email = normalizeEmail(input.Email)
	input.Phone, input.WeChat, input.RoleTitle = strings.TrimSpace(input.Phone), strings.TrimSpace(input.WeChat), strings.TrimSpace(input.RoleTitle)
	if !identifierPattern(customerID) || input.Name == "" || len([]rune(input.Name)) > 120 || len([]rune(input.Phone)) > 64 || len([]rune(input.WeChat)) > 120 || len([]rune(input.RoleTitle)) > 120 {
		return domain.Contact{}, fmt.Errorf("%w: contact fields are invalid", ErrValidation)
	}
	if input.Email != "" {
		if err := validateEmail(input.Email); err != nil {
			return domain.Contact{}, err
		}
	}
	if service.support == nil {
		return domain.Contact{}, ErrBusinessStoreUnavailable
	}
	now := service.now().UTC()
	contact := domain.Contact{ID: newID("contact", now), CustomerID: customerID, Name: input.Name, Email: input.Email, Phone: input.Phone, WeChat: input.WeChat, RoleTitle: input.RoleTitle, IsPrimary: input.IsPrimary, CreatedAt: now, UpdatedAt: now}
	if err := service.support.CreateContact(ctx, contact, operatorID); err != nil {
		return domain.Contact{}, err
	}
	return contact, nil
}

func (service *Service) UpsertBillingProfile(ctx context.Context, customerID string, input domain.BillingProfileInput, operatorID string) (domain.BillingProfile, error) {
	input.InvoiceTitle, input.TaxIdentifier = strings.TrimSpace(input.InvoiceTitle), strings.TrimSpace(input.TaxIdentifier)
	input.BillingEmail, input.Address = normalizeEmail(input.BillingEmail), strings.TrimSpace(input.Address)
	if !identifierPattern(customerID) || input.InvoiceTitle == "" || len([]rune(input.InvoiceTitle)) > 200 || len([]rune(input.TaxIdentifier)) > 80 || len([]rune(input.Address)) > 4000 {
		return domain.BillingProfile{}, fmt.Errorf("%w: billing profile fields are invalid", ErrValidation)
	}
	if input.BillingEmail != "" {
		if err := validateEmail(input.BillingEmail); err != nil {
			return domain.BillingProfile{}, err
		}
	}
	if service.support == nil {
		return domain.BillingProfile{}, ErrBusinessStoreUnavailable
	}
	now := service.now().UTC()
	profile := domain.BillingProfile{ID: newID("billing", now), CustomerID: customerID, InvoiceTitle: input.InvoiceTitle, TaxIdentifier: input.TaxIdentifier, BillingEmail: input.BillingEmail, Address: input.Address, CreatedAt: now, UpdatedAt: now}
	prior, err := service.support.GetCustomerProfile(ctx, customerID)
	if err != nil {
		return domain.BillingProfile{}, err
	}
	if prior.Billing != nil {
		profile.ID, profile.CreatedAt = prior.Billing.ID, prior.Billing.CreatedAt
	}
	if err := service.support.UpsertBillingProfile(ctx, profile, operatorID); err != nil {
		return domain.BillingProfile{}, err
	}
	return profile, nil
}

func (service *Service) Overview(ctx context.Context) (domain.Overview, error) {
	return service.store.Overview(ctx)
}

func (service *Service) CreatePlan(ctx context.Context, input domain.PlanInput, operatorID string) (domain.Plan, error) {
	input.Code = strings.ToLower(strings.TrimSpace(input.Code))
	input.Name = strings.TrimSpace(input.Name)
	input.Edition = strings.TrimSpace(input.Edition)
	input.Currency = strings.ToUpper(strings.TrimSpace(input.Currency))
	input.BillingCycle = strings.ToLower(strings.TrimSpace(input.BillingCycle))
	input.TaxMode = strings.ToLower(strings.TrimSpace(input.TaxMode))
	if !identifierPattern(input.Code) || input.Name == "" || input.Edition == "" || len(input.Currency) != 3 ||
		input.TransferLimit < 0 || input.MemberSeatsLimit < 0 ||
		input.SeatOverLimitGraceDays < 0 || input.SeatOverLimitGraceDays > 90 || input.BaseAmountMinor < 0 ||
		input.IncludedMemberSeats < 0 || input.AdditionalMemberSeatMinor < 0 {
		return domain.Plan{}, fmt.Errorf("%w: plan fields are invalid", ErrValidation)
	}
	if input.BillingCycle != "month" && input.BillingCycle != "year" && input.BillingCycle != "one_time" {
		return domain.Plan{}, fmt.Errorf("%w: billing cycle is invalid", ErrValidation)
	}
	if input.TaxMode != "inclusive" && input.TaxMode != "exclusive" && input.TaxMode != "none" {
		return domain.Plan{}, fmt.Errorf("%w: tax mode is invalid", ErrValidation)
	}
	features, err := productcatalog.ResolveFeatures(input.Features)
	if err != nil {
		return domain.Plan{}, fmt.Errorf("%w: %s", ErrValidation, err)
	}
	input.Features = features
	now := service.now().UTC()
	plan := domain.Plan{
		ID: newID("plan", now), ProductCode: "aster-team", Code: input.Code, Name: input.Name, Edition: input.Edition,
		Status: "active", Features: input.Features, TransferLimit: input.TransferLimit,
		MemberSeatsLimit:       input.MemberSeatsLimit,
		SeatOverLimitGraceDays: input.SeatOverLimitGraceDays, MinimumVersion: input.MinimumVersion,
		PriceVersionID: newID("price", now), Currency: input.Currency, BillingCycle: input.BillingCycle,
		BaseAmountMinor: input.BaseAmountMinor, IncludedMemberSeats: input.IncludedMemberSeats,
		AdditionalMemberSeatMinor: input.AdditionalMemberSeatMinor, TaxMode: input.TaxMode, CreatedAt: now, UpdatedAt: now,
	}
	if service.business == nil {
		return domain.Plan{}, ErrBusinessStoreUnavailable
	}
	if err := service.business.CreatePlan(ctx, plan, operatorID); err != nil {
		return domain.Plan{}, err
	}
	return plan, nil
}

func (service *Service) ListPlans(ctx context.Context, limit int) ([]domain.Plan, error) {
	if limit < 1 || limit > 100 {
		limit = 50
	}
	if service.business == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	return service.business.ListPlans(ctx, limit)
}

func (service *Service) PublishPlanPrice(ctx context.Context, planID string, input domain.PriceVersionInput, operatorID string) (domain.PriceVersionRecord, error) {
	input.Currency = strings.ToUpper(strings.TrimSpace(input.Currency))
	input.BillingCycle = strings.ToLower(strings.TrimSpace(input.BillingCycle))
	input.TaxMode = strings.ToLower(strings.TrimSpace(input.TaxMode))
	if !identifierPattern(planID) || len(input.Currency) != 3 || input.BaseAmountMinor < 0 || input.IncludedMemberSeats < 0 || input.AdditionalMemberSeatMinor < 0 || (input.BillingCycle != "month" && input.BillingCycle != "year" && input.BillingCycle != "one_time") || (input.TaxMode != "inclusive" && input.TaxMode != "exclusive" && input.TaxMode != "none") {
		return domain.PriceVersionRecord{}, fmt.Errorf("%w: price version fields are invalid", ErrValidation)
	}
	if service.business == nil {
		return domain.PriceVersionRecord{}, ErrBusinessStoreUnavailable
	}
	now := service.now().UTC()
	price := domain.PriceVersionRecord{ID: newID("price", now), PlanID: planID, Currency: input.Currency, BillingCycle: input.BillingCycle, BaseAmountMinor: input.BaseAmountMinor, IncludedMemberSeats: input.IncludedMemberSeats, AdditionalMemberSeatMinor: input.AdditionalMemberSeatMinor, TaxMode: input.TaxMode, PublishedAt: now, CreatedAt: now}
	return service.business.PublishPlanPrice(ctx, price, operatorID)
}

func (service *Service) CreateOrder(ctx context.Context, input domain.OrderInput, operatorID string) (domain.Order, error) {
	input.CustomerID = strings.TrimSpace(input.CustomerID)
	input.PlanID = strings.TrimSpace(input.PlanID)
	input.ContractRef = strings.TrimSpace(input.ContractRef)
	input.Notes = strings.TrimSpace(input.Notes)
	if input.CustomerID == "" || input.PlanID == "" || input.ContractRef == "" || len(input.ContractRef) > 128 || !input.EndsAt.After(input.StartsAt) {
		return domain.Order{}, fmt.Errorf("%w: order fields are invalid", ErrValidation)
	}
	now := service.now().UTC()
	order := domain.Order{ID: newID("order", now), CustomerID: input.CustomerID, PlanID: input.PlanID, ContractRef: input.ContractRef,
		Status: "pending_payment", StartsAt: input.StartsAt.UTC(), EndsAt: input.EndsAt.UTC(), Notes: input.Notes, CreatedAt: now, UpdatedAt: now}
	if service.business == nil {
		return domain.Order{}, ErrBusinessStoreUnavailable
	}
	if err := service.business.CreateOrder(ctx, order, operatorID); err != nil {
		return domain.Order{}, err
	}
	items, err := service.business.ListOrders(ctx, 100)
	if err != nil {
		return domain.Order{}, err
	}
	for _, item := range items {
		if item.ID == order.ID {
			return item, nil
		}
	}
	return domain.Order{}, errors.New("created order could not be read")
}

func (service *Service) ListOrders(ctx context.Context, limit int) ([]domain.Order, error) {
	if limit < 1 || limit > 100 {
		limit = 50
	}
	if service.business == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	return service.business.ListOrders(ctx, limit)
}

func (service *Service) ConfirmOfflinePayment(ctx context.Context, orderID string, input domain.OfflinePaymentInput, operatorID string) (domain.Order, error) {
	input.OperationID = strings.TrimSpace(input.OperationID)
	input.PaymentReference = strings.TrimSpace(input.PaymentReference)
	input.Notes = strings.TrimSpace(input.Notes)
	if !identifierPattern(input.OperationID) || input.PaymentReference == "" || len(input.PaymentReference) > 160 {
		return domain.Order{}, fmt.Errorf("%w: payment confirmation is invalid", ErrValidation)
	}
	if service.business == nil {
		return domain.Order{}, ErrBusinessStoreUnavailable
	}
	return service.business.ConfirmOfflinePayment(ctx, orderID, input, operatorID, service.now().UTC())
}

func (service *Service) CreateRefundNote(ctx context.Context, orderID string, input domain.RefundNoteInput, operatorID string) (domain.RefundNote, error) {
	input.Reason = strings.TrimSpace(input.Reason)
	if !identifierPattern(orderID) || input.AmountMinor <= 0 || input.Reason == "" || len([]rune(input.Reason)) > 4000 {
		return domain.RefundNote{}, fmt.Errorf("%w: refund note fields are invalid", ErrValidation)
	}
	if service.support == nil {
		return domain.RefundNote{}, ErrBusinessStoreUnavailable
	}
	now := service.now().UTC()
	note := domain.RefundNote{ID: newID("refund", now), OrderID: orderID, AmountMinor: input.AmountMinor, Reason: input.Reason, OperatorID: operatorID, CreatedAt: now}
	if err := service.support.CreateRefundNote(ctx, note, operatorID); err != nil {
		return domain.RefundNote{}, err
	}
	return note, nil
}

func (service *Service) ListRefundNotes(ctx context.Context, orderID string) ([]domain.RefundNote, error) {
	if !identifierPattern(orderID) || service.support == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	return service.support.ListRefundNotes(ctx, orderID)
}

func (service *Service) CreateTrial(ctx context.Context, input domain.TrialInput, operatorID string) (domain.Trial, error) {
	input.CustomerID = strings.TrimSpace(input.CustomerID)
	input.PlanID = strings.TrimSpace(input.PlanID)
	input.ApprovalReason = strings.TrimSpace(input.ApprovalReason)
	if input.CustomerID == "" || input.PlanID == "" || input.ApprovalReason == "" || input.MemberSeats < 0 || input.TransferLimit < 0 || !input.EndsAt.After(input.StartsAt) || input.EndsAt.Sub(input.StartsAt) > 30*24*time.Hour {
		return domain.Trial{}, fmt.Errorf("%w: trial fields are invalid", ErrValidation)
	}
	now := service.now().UTC()
	trial := domain.Trial{ID: newID("trial", now), CustomerID: input.CustomerID, PlanID: input.PlanID, Status: "approved",
		StartsAt: input.StartsAt.UTC(), EndsAt: input.EndsAt.UTC(), MemberSeats: input.MemberSeats,
		TransferLimit: input.TransferLimit, ApprovalReason: input.ApprovalReason, CreatedAt: now, UpdatedAt: now}
	if service.business == nil {
		return domain.Trial{}, ErrBusinessStoreUnavailable
	}
	if err := service.business.CreateTrial(ctx, trial, operatorID); err != nil {
		return domain.Trial{}, err
	}
	return trial, nil
}

func (service *Service) ListTrials(ctx context.Context, limit int) ([]domain.Trial, error) {
	if limit < 1 || limit > 100 {
		limit = 50
	}
	if service.business == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	return service.business.ListTrials(ctx, limit)
}

func (service *Service) ExtendTrial(ctx context.Context, trialID string, input domain.TrialExtensionInput, operatorID string) (domain.Trial, error) {
	input.Reason = strings.TrimSpace(input.Reason)
	if !identifierPattern(trialID) || input.EndsAt.IsZero() || input.Reason == "" || len([]rune(input.Reason)) > 1000 {
		return domain.Trial{}, fmt.Errorf("%w: trial extension is invalid", ErrValidation)
	}
	if service.support == nil {
		return domain.Trial{}, ErrBusinessStoreUnavailable
	}
	now := service.now().UTC()
	return service.support.ExtendTrial(ctx, trialID, input.EndsAt.UTC(), input.Reason, operatorID, now)
}

func (service *Service) CreateRiskNote(ctx context.Context, trialID string, input domain.RiskNoteInput, operatorID string) (domain.RiskNote, error) {
	input.RiskLevel, input.Note = strings.ToLower(strings.TrimSpace(input.RiskLevel)), strings.TrimSpace(input.Note)
	if !identifierPattern(trialID) || (input.RiskLevel != "low" && input.RiskLevel != "medium" && input.RiskLevel != "high") || input.Note == "" || len([]rune(input.Note)) > 4000 {
		return domain.RiskNote{}, fmt.Errorf("%w: risk note fields are invalid", ErrValidation)
	}
	if service.support == nil {
		return domain.RiskNote{}, ErrBusinessStoreUnavailable
	}
	now := service.now().UTC()
	note := domain.RiskNote{ID: newID("risk", now), TrialID: trialID, RiskLevel: input.RiskLevel, Note: input.Note, OperatorID: operatorID, CreatedAt: now}
	if err := service.support.CreateRiskNote(ctx, note, operatorID); err != nil {
		return domain.RiskNote{}, err
	}
	return note, nil
}

func (service *Service) ListRiskNotes(ctx context.Context, trialID string) ([]domain.RiskNote, error) {
	if !identifierPattern(trialID) || service.support == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	return service.support.ListRiskNotes(ctx, trialID)
}

func (service *Service) ListBackupHistory(ctx context.Context, limit int) ([]domain.BackupRecord, error) {
	if limit < 1 || limit > 100 {
		limit = 50
	}
	if service.support == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	return service.support.ListBackupHistory(ctx, limit)
}

func (service *Service) ExportOperations(ctx context.Context, operatorID string) (domain.OperationsExport, error) {
	if service.support == nil {
		return domain.OperationsExport{}, ErrBusinessStoreUnavailable
	}
	return service.support.ExportOperations(ctx, operatorID, service.now().UTC())
}

func (service *Service) FulfillOrder(ctx context.Context, orderID string, input domain.FulfillmentInput, operatorID string) (ports.LicenseRecord, error) {
	return service.fulfillSource(ctx, "order", orderID, input, operatorID)
}

func (service *Service) FulfillTrial(ctx context.Context, trialID string, input domain.FulfillmentInput, operatorID string) (ports.LicenseRecord, error) {
	return service.fulfillSource(ctx, "trial", trialID, input, operatorID)
}

func (service *Service) fulfillSource(ctx context.Context, sourceType, sourceID string, input domain.FulfillmentInput, operatorID string) (ports.LicenseRecord, error) {
	if service.business == nil {
		return ports.LicenseRecord{}, ErrBusinessStoreUnavailable
	}
	if service.customerRefs == nil {
		return ports.LicenseRecord{}, ErrBusinessStoreUnavailable
	}
	input.OperationID = strings.TrimSpace(input.OperationID)
	input.LicenseID = strings.TrimSpace(input.LicenseID)
	sourceID = strings.TrimSpace(sourceID)
	if !identifierPattern(input.OperationID) || !identifierPattern(input.LicenseID) || !identifierPattern(sourceID) {
		return ports.LicenseRecord{}, fmt.Errorf("%w: fulfillment fields are invalid", ErrValidation)
	}
	customerID, err := service.business.FulfillmentCustomer(ctx, sourceType, sourceID)
	if err != nil {
		return ports.LicenseRecord{}, err
	}
	customerRef, err := service.customerRefs.Reference(customerID)
	if err != nil {
		return ports.LicenseRecord{}, err
	}
	return service.business.PrepareSourceFulfillment(ctx, sourceType, sourceID, input, customerRef, operatorID, service.now().UTC())
}

func (service *Service) ListAuditEvents(ctx context.Context, limit int) ([]domain.AuditEvent, error) {
	if limit < 1 || limit > 200 {
		limit = 100
	}
	if service.business == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	return service.business.ListAuditEvents(ctx, limit)
}

func (service *Service) ImportReleaseArtifact(ctx context.Context, input domain.ReleaseArtifactInput, operatorID string) (domain.ReleaseArtifact, error) {
	input.InboxFilename = strings.TrimSpace(input.InboxFilename)
	input.Version = strings.TrimSpace(input.Version)
	input.Platform = strings.ToLower(strings.TrimSpace(input.Platform))
	input.Architecture = strings.ToLower(strings.TrimSpace(input.Architecture))
	input.ExpectedSHA256 = strings.ToLower(strings.TrimSpace(input.ExpectedSHA256))
	input.ReleaseManifestSHA256 = strings.ToLower(strings.TrimSpace(input.ReleaseManifestSHA256))
	input.SignatureRef = strings.TrimSpace(input.SignatureRef)
	if service.artifacts == nil || service.business == nil {
		return domain.ReleaseArtifact{}, ErrBusinessStoreUnavailable
	}
	if !domain.ValidSemanticVersion(input.Version) || input.Platform != "linux" || input.Architecture != "amd64" ||
		!sha256Pattern(input.ExpectedSHA256) || !sha256Pattern(input.ReleaseManifestSHA256) {
		return domain.ReleaseArtifact{}, fmt.Errorf("%w: release artifact fields are invalid", ErrValidation)
	}
	expectedFilename := fmt.Sprintf("aster-team-%s-linux-amd64.tar.gz", input.Version)
	if input.InboxFilename != expectedFilename {
		return domain.ReleaseArtifact{}, fmt.Errorf("%w: release artifact filename does not match the production package", ErrValidation)
	}
	stored, err := service.artifacts.ImportInbox(ctx, input.InboxFilename, input.ExpectedSHA256)
	if err != nil {
		return domain.ReleaseArtifact{}, err
	}
	now := service.now().UTC()
	release := domain.ReleaseArtifact{ID: newID("release", now), Version: input.Version, Platform: input.Platform,
		Architecture: input.Architecture, ObjectKey: stored.ObjectKey, SHA256: stored.SHA256,
		ReleaseManifestSHA256: input.ReleaseManifestSHA256, SizeBytes: stored.SizeBytes,
		SignatureRef: input.SignatureRef, RuntimeLinkage: "unverified", CreatedAt: now}
	if err := service.business.CreateReleaseArtifact(ctx, release, operatorID); err != nil {
		return domain.ReleaseArtifact{}, err
	}
	return release, nil
}

func (service *Service) ListReleaseArtifacts(ctx context.Context, limit int) ([]domain.ReleaseArtifact, error) {
	if service.business == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	if limit < 1 || limit > 100 {
		limit = 50
	}
	return service.business.ListReleaseArtifacts(ctx, limit)
}

func (service *Service) OpenReleaseArtifact(ctx context.Context, releaseID string) (domain.ReleaseArtifact, ports.ReadSeekCloser, error) {
	if service.business == nil || service.artifacts == nil {
		return domain.ReleaseArtifact{}, nil, ErrBusinessStoreUnavailable
	}
	releaseID = strings.TrimSpace(releaseID)
	if releaseID == "" {
		return domain.ReleaseArtifact{}, nil, fmt.Errorf("%w: release artifact id is required", ErrValidation)
	}
	release, err := service.business.GetReleaseArtifact(ctx, releaseID)
	if err != nil {
		return domain.ReleaseArtifact{}, nil, err
	}
	maximum := release.SizeBytes
	if maximum < 1 {
		return domain.ReleaseArtifact{}, nil, fmt.Errorf("%w: release artifact size is invalid", ErrValidation)
	}
	reader, size, err := service.artifacts.OpenObject(ctx, release.ObjectKey, maximum)
	if err != nil {
		return domain.ReleaseArtifact{}, nil, err
	}
	if size != release.SizeBytes {
		reader.Close()
		return domain.ReleaseArtifact{}, nil, errors.New("release artifact object size does not match its immutable record")
	}
	return release, reader, nil
}

func (service *Service) ListReleaseTasks(ctx context.Context, limit int) ([]domain.ReleaseTask, error) {
	if service.releaseTasks == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	if limit < 1 || limit > 100 {
		limit = 50
	}
	return service.releaseTasks.ListReleaseTasks(ctx, limit)
}

func (service *Service) GetReleaseTaskDetail(ctx context.Context, taskID string) (domain.ReleaseTaskDetail, error) {
	if service.releaseTasks == nil {
		return domain.ReleaseTaskDetail{}, ErrBusinessStoreUnavailable
	}
	taskID = strings.TrimSpace(taskID)
	if taskID == "" {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: release task id is required", ErrValidation)
	}
	return service.releaseTasks.GetReleaseTaskDetail(ctx, taskID)
}

func (service *Service) CreateReleaseTask(ctx context.Context, input domain.ReleaseTaskInput, operatorID string) (domain.ReleaseTaskDetail, error) {
	return service.createReleaseTask(ctx, input, operatorID, nil, "")
}

func (service *Service) RetryReleaseTask(ctx context.Context, taskID, operatorID string) (domain.ReleaseTaskDetail, error) {
	if service.releaseTasks == nil {
		return domain.ReleaseTaskDetail{}, ErrBusinessStoreUnavailable
	}
	original, err := service.releaseTasks.GetReleaseTaskDetail(ctx, strings.TrimSpace(taskID))
	if err != nil {
		return domain.ReleaseTaskDetail{}, err
	}
	return service.createReleaseTask(ctx, domain.ReleaseTaskInput{Version: original.Task.Version, SourceRef: original.Task.SourceRef, FreeDistributionID: original.Task.FreeDistributionID}, operatorID, &original.Task.ID, original.Task.FreeLicenseSHA256)
}

func (service *Service) ReverifyReleaseTask(ctx context.Context, taskID, artifactID, operatorID string) (domain.ReleaseTaskDetail, error) {
	if service.releaseTasks == nil || service.releaseVerifier == nil {
		return domain.ReleaseTaskDetail{}, ErrReleaseNotConfigured
	}
	detail, err := service.releaseTasks.GetReleaseTaskDetail(ctx, strings.TrimSpace(taskID))
	if err != nil {
		return domain.ReleaseTaskDetail{}, err
	}
	if (detail.Task.Status != "failed" && detail.Task.Status != "completed" && detail.Task.Status != "verifying") || len(detail.Runs) == 0 || detail.Runs[0].Conclusion == nil || *detail.Runs[0].Conclusion != "success" {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: only packages from a successful GitHub run can be verified", ErrValidation)
	}
	var candidate *domain.ReleaseTaskArtifact
	for index := range detail.Artifacts {
		if detail.Artifacts[index].ID == strings.TrimSpace(artifactID) {
			candidate = &detail.Artifacts[index]
			break
		}
	}
	if candidate == nil || !candidate.Target().Supported() || candidate.Name != candidate.Target().ArtifactName(detail.Task.Version) ||
		(candidate.VerificationStatus != "pending" && candidate.VerificationStatus != "failed" && candidate.VerificationStatus != "unavailable") || candidate.GitHubDigestSHA256 == nil {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: selected package is not available for verification", ErrValidation)
	}
	if err := service.releaseTasks.PrepareReleaseTaskArtifactVerification(ctx, detail.Task.ID, candidate.ID, operatorID, service.now().UTC()); err != nil {
		return domain.ReleaseTaskDetail{}, err
	}
	return service.releaseTasks.GetReleaseTaskDetail(ctx, detail.Task.ID)
}

func (service *Service) createReleaseTask(ctx context.Context, input domain.ReleaseTaskInput, operatorID string, retryOf *string, expectedFreeLicenseSHA256 string) (domain.ReleaseTaskDetail, error) {
	if service.releaseTasks == nil {
		return domain.ReleaseTaskDetail{}, ErrBusinessStoreUnavailable
	}
	if service.releases == nil {
		return domain.ReleaseTaskDetail{}, ErrReleaseNotConfigured
	}
	input.Version = strings.TrimSpace(input.Version)
	input.SourceRef = strings.TrimSpace(input.SourceRef)
	input.FreeDistributionID = strings.TrimSpace(input.FreeDistributionID)
	capabilities := service.releases.Capabilities()
	if input.SourceRef == "" {
		input.SourceRef = capabilities.DefaultSourceRef
	}
	if !domain.ValidSemanticVersion(input.Version) || input.SourceRef == "" || !commercialID.MatchString(input.FreeDistributionID) || strings.TrimSpace(operatorID) == "" {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: version must be SemVer 2.0.0 and source ref, issued free distribution, and operator are required", ErrValidation)
	}
	distributionStore, ok := service.store.(interface {
		GetFreeDistribution(context.Context, string) (commercial.DistributionRecord, error)
	})
	if !ok {
		return domain.ReleaseTaskDetail{}, ErrBusinessStoreUnavailable
	}
	distribution, err := distributionStore.GetFreeDistribution(ctx, input.FreeDistributionID)
	if err != nil {
		return domain.ReleaseTaskDetail{}, mapCommercialError(err)
	}
	if distribution.Status != "issued" || distribution.Document == nil || distribution.DocumentSHA256 == "" {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: free distribution must be issued before it can be bundled", ErrValidation)
	}
	if distribution.Document.Claims.Source.Kind != licenseprotocol.FreeDistributionV2 ||
		distribution.Document.Claims.Binding.Mode != licenseprotocol.UnboundV2 ||
		distribution.Document.Claims.Validity.Expiry.Mode != licenseprotocol.NoExpiryV2 {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: bundled free license must be unbound and have no expiry", ErrValidation)
	}
	if err := service.verifyDistribution(distribution); err != nil {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: free distribution verification failed: %v", ErrValidation, err)
	}
	freeLicenseDocument, err := json.Marshal(distribution.Document)
	if err != nil || len(freeLicenseDocument) == 0 || len(freeLicenseDocument) > 32<<10 {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: signed free license is too large or cannot be encoded", ErrValidation)
	}
	freeLicenseDigest := sha256.Sum256(freeLicenseDocument)
	freeLicenseSHA256 := hex.EncodeToString(freeLicenseDigest[:])
	if freeLicenseSHA256 != distribution.DocumentSHA256 {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: signed free license digest changed", ErrValidation)
	}
	if expectedFreeLicenseSHA256 != "" && freeLicenseSHA256 != expectedFreeLicenseSHA256 {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: retry cannot change the bundled free license", ErrValidation)
	}
	preflight, err := service.releases.Preflight(ctx, input.Version, input.SourceRef)
	if err != nil {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: %v", ErrReleasePreflight, err)
	}
	now := service.now().UTC()
	task := domain.ReleaseTask{ID: newID("release_task", now), Version: input.Version, Mode: "verification", FreeDistributionID: input.FreeDistributionID, FreeLicenseSHA256: freeLicenseSHA256, GitHubRepository: capabilities.Repository,
		WorkflowFile: capabilities.WorkflowFile, SourceRef: input.SourceRef, SourceCommitSHA: preflight.CommitSHA,
		Phase: "dispatch", Status: "dispatching", RetryOfTaskID: retryOf, CreatedBy: operatorID, CreatedAt: now, UpdatedAt: now}
	if err := service.releaseTasks.CreateReleaseTask(ctx, task, operatorID); err != nil {
		return domain.ReleaseTaskDetail{}, err
	}
	dispatch, err := service.releases.Dispatch(ctx, ports.ReleaseDispatchInput{TaskID: task.ID, Version: task.Version, SourceRef: task.SourceRef,
		SourceCommitSHA: task.SourceCommitSHA, FreeDistributionID: task.FreeDistributionID, FreeLicenseSHA256: task.FreeLicenseSHA256,
		FreeLicenseDocument: freeLicenseDocument})
	if err != nil {
		_ = service.releaseTasks.MarkReleaseTaskFailed(context.WithoutCancel(ctx), task.ID, "GITHUB_DISPATCH_FAILED")
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: %v", ErrReleaseDispatch, err)
	}
	attempt := 1
	run := domain.ReleaseRun{ID: fmt.Sprintf("release_run_%d", dispatch.RunID), ReleaseTaskID: task.ID, Attempt: attempt,
		GitHubRunID: &dispatch.RunID, GitHubRunNumber: dispatch.RunNumber, WorkflowName: capabilities.WorkflowFile,
		HeadBranch: task.SourceRef, HeadSHA: task.SourceCommitSHA, Status: dispatch.Status, HTMLURL: dispatch.RunURL,
		CreatedAt: dispatch.CreatedAt.UTC(), Jobs: []domain.ReleaseJob{}}
	if err := service.releaseTasks.RecordReleaseDispatch(ctx, task.ID, run); err != nil {
		return domain.ReleaseTaskDetail{}, err
	}
	// A just-created run can take a moment to become readable. The restart-safe monitor will retry it.
	_ = service.SyncReleaseTask(ctx, task.ID)
	return service.releaseTasks.GetReleaseTaskDetail(ctx, task.ID)
}

func (service *Service) SyncReleaseTask(ctx context.Context, taskID string) error {
	if service.releaseTasks == nil {
		return ErrBusinessStoreUnavailable
	}
	if service.releases == nil {
		return ErrReleaseNotConfigured
	}
	detail, err := service.releaseTasks.GetReleaseTaskDetail(ctx, strings.TrimSpace(taskID))
	if err != nil {
		return err
	}
	if detail.Task.Status == "completed" || detail.Task.Status == "failed" || detail.Task.Status == "cancelled" {
		return nil
	}
	if len(detail.Runs) == 0 || detail.Runs[0].GitHubRunID == nil {
		return fmt.Errorf("%w: release task has no GitHub run", ErrReleaseSync)
	}
	// Once GitHub metadata has been persisted and a task is in the artifact-verification phase,
	// verification no longer depends on another GitHub metadata request. This also lets a manual
	// reverification reuse the content-addressed cache after the GitHub artifact has expired.
	if detail.Task.Status == "verifying" && detail.Task.Phase == "artifact_verification" {
		return service.verifyReleaseTaskArtifact(ctx, detail.Task, ports.ReleaseSnapshot{Run: detail.Runs[0], Artifacts: detail.Artifacts})
	}
	snapshot, err := service.releases.Snapshot(ctx, detail.Task, *detail.Runs[0].GitHubRunID)
	if err != nil {
		return fmt.Errorf("%w: %v", ErrReleaseSync, err)
	}
	status, phase, errorCode := releaseTaskState(snapshot)
	if status == "verifying" {
		found := map[domain.ReleaseTarget]bool{}
		for index := range snapshot.Artifacts {
			artifact := &snapshot.Artifacts[index]
			if artifact.Target().Supported() && artifact.Name == artifact.Target().ArtifactName(detail.Task.Version) {
				found[artifact.Target()] = true
				if artifact.VerificationStatus == "pending" || artifact.VerificationStatus == "unavailable" {
					artifact.VerificationStatus = "queued"
				}
			}
		}
		if len(found) != len(domain.ReleaseTargets()) {
			if snapshot.Run.CompletedAt == nil || service.now().UTC().Sub(snapshot.Run.CompletedAt.UTC()) < 2*time.Minute {
				phase = "github_artifacts"
			} else {
				code := "RELEASE_ARTIFACT_UNAVAILABLE"
				errorCode = &code
			}
		}
	}
	if err := service.releaseTasks.ApplyReleaseSnapshot(ctx, detail.Task.ID, snapshot, status, phase, errorCode, service.now().UTC()); err != nil {
		return fmt.Errorf("%w: %v", ErrReleaseSync, err)
	}
	if status == "verifying" && phase == "artifact_verification" {
		detail, err = service.releaseTasks.GetReleaseTaskDetail(ctx, detail.Task.ID)
		if err != nil {
			return err
		}
		return service.verifyReleaseTaskArtifact(ctx, detail.Task, ports.ReleaseSnapshot{Run: detail.Runs[0], Artifacts: detail.Artifacts})
	}
	return nil
}

func (service *Service) verifyReleaseTaskArtifact(ctx context.Context, task domain.ReleaseTask, snapshot ports.ReleaseSnapshot) error {
	if service.releaseVerifier == nil {
		return fmt.Errorf("%w: release artifact verifier is not configured", ErrReleaseSync)
	}
	var syncErrors []error
	counts := map[domain.ReleaseTarget]int{}
	for _, candidate := range snapshot.Artifacts {
		if candidate.Target().Supported() {
			counts[candidate.Target()]++
		}
	}
	for _, candidate := range snapshot.Artifacts {
		if candidate.VerificationStatus != "queued" {
			continue
		}
		if counts[candidate.Target()] != 1 {
			if err := service.failReleaseArtifact(ctx, task.ID, candidate.ID, "RELEASE_PACKAGE_POLICY_FAILED"); err != nil {
				syncErrors = append(syncErrors, err)
			}
			continue
		}
		if err := service.verifyReleasePackage(ctx, task, snapshot.Run, candidate); err != nil {
			syncErrors = append(syncErrors, err)
		}
	}
	if err := service.releaseTasks.FinishReleaseTaskVerification(ctx, task.ID, task.ErrorCode, service.now().UTC()); err != nil {
		syncErrors = append(syncErrors, err)
	}
	return errors.Join(syncErrors...)
}

func (service *Service) verifyReleasePackage(ctx context.Context, task domain.ReleaseTask, run domain.ReleaseRun, candidate domain.ReleaseTaskArtifact) error {
	if !candidate.Target().Supported() || candidate.Name != candidate.Target().ArtifactName(task.Version) {
		return service.failReleaseArtifact(ctx, task.ID, candidate.ID, "RELEASE_PACKAGE_POLICY_FAILED")
	}
	verified, err := service.releaseVerifier.Verify(ctx, task, candidate)
	if err != nil {
		if errors.Is(err, ports.ErrReleaseArtifactPending) {
			return nil
		}
		var verificationError *ports.ReleaseArtifactVerificationError
		if errors.As(err, &verificationError) {
			return service.failReleaseArtifact(ctx, task.ID, candidate.ID, verificationError.Code)
		}
		return fmt.Errorf("%w: %v", ErrReleaseSync, err)
	}
	if !sha256Pattern(verified.SHA256) || !sha256Pattern(verified.ManifestSHA256) || verified.ObjectKey == "" || verified.SizeBytes < 1 || strings.TrimSpace(verified.SignatureKeyID) == "" {
		return service.failReleaseArtifact(ctx, task.ID, candidate.ID, "RELEASE_PACKAGE_POLICY_FAILED")
	}
	now := service.now().UTC()
	release := domain.ReleaseArtifact{ID: "release_" + verified.SHA256[:56], Version: task.Version, Platform: candidate.Platform, Architecture: candidate.Architecture,
		ObjectKey: verified.ObjectKey, SHA256: verified.SHA256, ReleaseManifestSHA256: verified.ManifestSHA256,
		SizeBytes: verified.SizeBytes, SignatureRef: "release-key:" + verified.SignatureKeyID, SourceCommitSHA: &task.SourceCommitSHA,
		GitHubRunID: run.GitHubRunID, RuntimeLinkage: verified.RuntimeLinkage, CreatedAt: now}
	return service.releaseTasks.CompleteReleaseTaskArtifact(ctx, task.ID, candidate.ID, release, verified.SignatureKeyID, now)
}

func (service *Service) failReleaseArtifact(ctx context.Context, taskID, artifactID, code string) error {
	if code == "" {
		code = "RELEASE_PACKAGE_POLICY_FAILED"
	}
	if err := service.releaseTasks.FailReleaseTaskArtifact(ctx, taskID, artifactID, code, service.now().UTC()); err != nil {
		return fmt.Errorf("%w: %v", ErrReleaseSync, err)
	}
	return nil
}

func (service *Service) SyncPendingReleaseTasks(ctx context.Context, limit int) []error {
	if service.releaseTasks == nil || service.releases == nil {
		return nil
	}
	ids, err := service.releaseTasks.ListPendingReleaseTaskIDs(ctx, limit)
	if err != nil {
		return []error{err}
	}
	errorsFound := make([]error, 0)
	for _, taskID := range ids {
		if ctx.Err() != nil {
			break
		}
		if err := service.SyncReleaseTask(ctx, taskID); err != nil {
			errorsFound = append(errorsFound, fmt.Errorf("sync %s: %w", taskID, err))
		}
	}
	return errorsFound
}

func (service *Service) ListReleasePublishRequests(ctx context.Context, limit int) ([]domain.ReleasePublishRequest, error) {
	if service.releasePublishes == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	if limit < 1 || limit > 100 {
		limit = 50
	}
	return service.releasePublishes.ListReleasePublishRequests(ctx, limit)
}

func (service *Service) RequestReleasePublish(ctx context.Context, releaseArtifactID, operatorID string) (domain.ReleasePublishRequest, error) {
	if service.releasePublishes == nil || service.business == nil {
		return domain.ReleasePublishRequest{}, ErrBusinessStoreUnavailable
	}
	if service.releasePublisher == nil || !service.releasePublisher.Configured() {
		return domain.ReleasePublishRequest{}, ErrReleasePublishNotConfigured
	}
	releaseArtifactID, operatorID = strings.TrimSpace(releaseArtifactID), strings.TrimSpace(operatorID)
	if releaseArtifactID == "" || operatorID == "" {
		return domain.ReleasePublishRequest{}, fmt.Errorf("%w: release artifact and operator are required", ErrValidation)
	}
	release, err := service.business.GetReleaseArtifact(ctx, releaseArtifactID)
	if err != nil {
		return domain.ReleasePublishRequest{}, err
	}
	if release.SourceCommitSHA == nil || !releaseCommitPattern.MatchString(*release.SourceCommitSHA) ||
		!sha256Pattern(release.SHA256) || release.SignatureRef == "" || release.RuntimeLinkage != "musl-static" {
		return domain.ReleasePublishRequest{}, fmt.Errorf("%w: only independently verified release artifacts can be published", ErrValidation)
	}
	now := service.now().UTC()
	request := domain.ReleasePublishRequest{ID: newID("publish", now), ReleaseArtifactID: release.ID, Version: release.Version,
		SourceCommitSHA: *release.SourceCommitSHA, TagName: "v" + release.Version, Status: "requested", RequestedBy: operatorID,
		CreatedAt: now, UpdatedAt: now}
	if err := service.releasePublishes.CreateReleasePublishRequest(ctx, request); err != nil {
		return domain.ReleasePublishRequest{}, err
	}
	return service.releasePublishes.GetReleasePublishRequest(ctx, request.ID)
}

func (service *Service) DecideReleasePublish(ctx context.Context, requestID string, input domain.ReleasePublishApprovalInput, operatorID string) (domain.ReleasePublishRequest, error) {
	if service.releasePublishes == nil {
		return domain.ReleasePublishRequest{}, ErrBusinessStoreUnavailable
	}
	requestID, operatorID = strings.TrimSpace(requestID), strings.TrimSpace(operatorID)
	input.Decision, input.Comment = strings.ToLower(strings.TrimSpace(input.Decision)), strings.TrimSpace(input.Comment)
	if requestID == "" || operatorID == "" || (input.Decision != "approved" && input.Decision != "rejected") || len(input.Comment) > 1000 {
		return domain.ReleasePublishRequest{}, fmt.Errorf("%w: publish approval input is invalid", ErrValidation)
	}
	request, err := service.releasePublishes.GetReleasePublishRequest(ctx, requestID)
	if err != nil {
		return domain.ReleasePublishRequest{}, err
	}
	if request.RequestedBy == operatorID {
		return domain.ReleasePublishRequest{}, fmt.Errorf("%w: requester cannot approve or reject their own publish request", ErrReleasePublishApproval)
	}
	if err := service.releasePublishes.DecideReleasePublishRequest(ctx, requestID, input.Decision, input.Comment, operatorID, service.now().UTC()); err != nil {
		return domain.ReleasePublishRequest{}, fmt.Errorf("%w: %v", ErrReleasePublishApproval, err)
	}
	return service.releasePublishes.GetReleasePublishRequest(ctx, requestID)
}

func (service *Service) ExecuteReleasePublish(ctx context.Context, requestID, operatorID string) (domain.ReleasePublishRequest, error) {
	if service.releasePublishes == nil || service.business == nil {
		return domain.ReleasePublishRequest{}, ErrBusinessStoreUnavailable
	}
	if service.releasePublisher == nil || !service.releasePublisher.Configured() {
		return domain.ReleasePublishRequest{}, ErrReleasePublishNotConfigured
	}
	requestID, operatorID = strings.TrimSpace(requestID), strings.TrimSpace(operatorID)
	if requestID == "" || operatorID == "" {
		return domain.ReleasePublishRequest{}, fmt.Errorf("%w: publish request and operator are required", ErrValidation)
	}
	request, err := service.releasePublishes.BeginReleasePublish(ctx, requestID, operatorID, service.now().UTC())
	if err != nil {
		return domain.ReleasePublishRequest{}, err
	}
	if err := service.executeReleasePublish(ctx, request); err != nil {
		return domain.ReleasePublishRequest{}, err
	}
	return service.releasePublishes.GetReleasePublishRequest(ctx, requestID)
}

func (service *Service) executeReleasePublish(ctx context.Context, request domain.ReleasePublishRequest) error {
	release, err := service.business.GetReleaseArtifact(ctx, request.ReleaseArtifactID)
	if err != nil {
		_ = service.releasePublishes.FailReleasePublish(context.WithoutCancel(ctx), request.ID, "RELEASE_PUBLISH_PROVENANCE_INVALID", service.now().UTC())
		return fmt.Errorf("%w: %v", ErrReleasePublish, err)
	}
	if release.SourceCommitSHA == nil || *release.SourceCommitSHA != request.SourceCommitSHA || release.Version != request.Version || "v"+release.Version != request.TagName {
		_ = service.releasePublishes.FailReleasePublish(context.WithoutCancel(ctx), request.ID, "RELEASE_PUBLISH_PROVENANCE_INVALID", service.now().UTC())
		return fmt.Errorf("%w: release provenance changed", ErrReleasePublish)
	}
	result, err := service.releasePublisher.Publish(ctx, release)
	if err != nil {
		_ = service.releasePublishes.FailReleasePublish(context.WithoutCancel(ctx), request.ID, "GITHUB_PUBLISH_FAILED", service.now().UTC())
		return fmt.Errorf("%w: %v", ErrReleasePublish, err)
	}
	if result.TagName != request.TagName || result.GitHubReleaseID <= 0 || result.HTMLURL == "" {
		_ = service.releasePublishes.FailReleasePublish(context.WithoutCancel(ctx), request.ID, "GITHUB_PUBLISH_FAILED", service.now().UTC())
		return fmt.Errorf("%w: publisher returned an invalid result", ErrReleasePublish)
	}
	if err := service.releasePublishes.CompleteReleasePublish(ctx, request.ID, result, service.now().UTC()); err != nil {
		return fmt.Errorf("%w: %v", ErrReleasePublish, err)
	}
	return nil
}

func (service *Service) ResumePendingReleasePublishes(ctx context.Context, limit int) []error {
	if service.releasePublishes == nil || service.releasePublisher == nil || service.business == nil {
		return nil
	}
	ids, err := service.releasePublishes.ListPublishingReleaseRequestIDs(ctx, limit)
	if err != nil {
		return []error{err}
	}
	errorsFound := make([]error, 0)
	for _, requestID := range ids {
		if ctx.Err() != nil {
			break
		}
		request, err := service.releasePublishes.GetReleasePublishRequest(ctx, requestID)
		if err == nil {
			err = service.executeReleasePublish(ctx, request)
		}
		if err != nil {
			errorsFound = append(errorsFound, fmt.Errorf("resume publish %s: %w", requestID, err))
		}
	}
	return errorsFound
}

func releaseTaskState(snapshot ports.ReleaseSnapshot) (string, string, *string) {
	if snapshot.Run.Status != "completed" {
		return "in_progress", "github_actions", nil
	}
	if snapshot.Run.Conclusion != nil && *snapshot.Run.Conclusion == "success" {
		return "verifying", "artifact_verification", nil
	}
	code := "GITHUB_WORKFLOW_FAILED"
	for _, job := range snapshot.Run.Jobs {
		if job.Conclusion == nil || (*job.Conclusion != "failure" && *job.Conclusion != "timed_out" && *job.Conclusion != "startup_failure") {
			continue
		}
		name := strings.ToLower(job.Name)
		if strings.Contains(name, "install") || strings.Contains(name, "runner") || strings.Contains(name, "rollback") {
			code = "RELEASE_LINUX_VALIDATION_FAILED"
			break
		}
		if strings.Contains(name, "build") || strings.Contains(name, "package") {
			code = "RELEASE_BUILD_FAILED"
		}
	}
	return "failed", "failed", &code
}

func (service *Service) CreateDelivery(ctx context.Context, input domain.DeliveryInput, operatorID string) (domain.DeliveryRecord, error) {
	if service.business == nil || service.artifacts == nil {
		return domain.DeliveryRecord{}, ErrBusinessStoreUnavailable
	}
	input.CustomerID = strings.TrimSpace(input.CustomerID)
	input.LicenseID = strings.TrimSpace(input.LicenseID)
	input.ReleaseArtifactID = strings.TrimSpace(input.ReleaseArtifactID)
	input.Channel = strings.TrimSpace(input.Channel)
	input.Recipient = strings.TrimSpace(input.Recipient)
	if input.CustomerID == "" || input.LicenseID == "" || input.ReleaseArtifactID == "" || input.Channel == "" || input.Recipient == "" || (input.OrderID == nil && input.TrialID == nil) || (input.OrderID != nil && input.TrialID != nil) {
		return domain.DeliveryRecord{}, fmt.Errorf("%w: delivery fields are invalid", ErrValidation)
	}
	release, err := service.business.GetReleaseArtifact(ctx, input.ReleaseArtifactID)
	if err != nil {
		return domain.DeliveryRecord{}, err
	}
	if err := service.business.ValidateDelivery(ctx, input); err != nil {
		return domain.DeliveryRecord{}, err
	}
	now := service.now().UTC()
	delivery := domain.DeliveryRecord{ID: newID("delivery", now), CustomerID: input.CustomerID, OrderID: input.OrderID, TrialID: input.TrialID,
		LicenseID: input.LicenseID, ReleaseArtifactID: input.ReleaseArtifactID, Channel: input.Channel, Recipient: input.Recipient, CreatedAt: now}
	receipt, err := json.Marshal(map[string]any{"schema": "aster.delivery-receipt.v1", "delivery_id": delivery.ID,
		"customer_ref": input.CustomerID, "order_id": input.OrderID, "trial_id": input.TrialID, "license_id": input.LicenseID,
		"release": map[string]any{"version": release.Version, "platform": release.Platform, "architecture": release.Architecture,
			"sha256": release.SHA256, "release_manifest_sha256": release.ReleaseManifestSHA256, "size_bytes": release.SizeBytes},
		"created_at": now.Format(time.RFC3339Nano)})
	if err != nil {
		return domain.DeliveryRecord{}, err
	}
	stored, err := service.artifacts.PutReceipt(ctx, receipt)
	if err != nil {
		return domain.DeliveryRecord{}, err
	}
	delivery.ReceiptObjectKey = stored.ObjectKey
	delivery.ReceiptSHA256 = stored.SHA256
	if err := service.business.CreateDelivery(ctx, delivery, operatorID); err != nil {
		return domain.DeliveryRecord{}, err
	}
	return delivery, nil
}

func (service *Service) ListDeliveries(ctx context.Context, limit int) ([]domain.DeliveryRecord, error) {
	if service.business == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	if limit < 1 || limit > 100 {
		limit = 50
	}
	return service.business.ListDeliveries(ctx, limit)
}

func (service *Service) GetDeliveryReceipt(ctx context.Context, deliveryID string) ([]byte, domain.DeliveryRecord, error) {
	if service.business == nil || service.artifacts == nil || strings.TrimSpace(deliveryID) == "" {
		return nil, domain.DeliveryRecord{}, ErrBusinessStoreUnavailable
	}
	delivery, err := service.business.GetDelivery(ctx, deliveryID)
	if err != nil {
		return nil, domain.DeliveryRecord{}, err
	}
	contents, err := service.artifacts.ReadObject(ctx, delivery.ReceiptObjectKey, 128*1024)
	if err != nil {
		return nil, domain.DeliveryRecord{}, err
	}
	digest := sha256.Sum256(contents)
	if hex.EncodeToString(digest[:]) != delivery.ReceiptSHA256 {
		return nil, domain.DeliveryRecord{}, errors.New("delivery receipt hash mismatch")
	}
	return contents, delivery, nil
}

func (service *Service) ListLicenseRecords(ctx context.Context, limit int) ([]ports.LicenseRecord, error) {
	if service.licenses == nil {
		return nil, ErrBusinessStoreUnavailable
	}
	if limit < 1 || limit > 100 {
		limit = 50
	}
	items, err := service.licenses.ListLicenseRecords(ctx, limit)
	if err != nil {
		return nil, err
	}
	now := service.now().UTC()
	for index := range items {
		if items[index].Status == "active" && !items[index].ValidUntil.After(now) {
			items[index].Status = "expired"
		}
	}
	return items, nil
}

func (service *Service) ListLicenseIssuances(ctx context.Context, licenseID string) ([]ports.LicenseIssuance, error) {
	if service.licenses == nil || !identifierPattern(strings.TrimSpace(licenseID)) {
		return nil, ErrBusinessStoreUnavailable
	}
	return service.licenses.ListLicenseIssuances(ctx, licenseID)
}

func normalizeCustomer(input domain.CustomerInput) domain.CustomerInput {
	input.Name = strings.TrimSpace(input.Name)
	input.LegalName = strings.TrimSpace(input.LegalName)
	input.Status = strings.ToLower(strings.TrimSpace(input.Status))
	input.ContactName = strings.TrimSpace(input.ContactName)
	input.ContactEmail = normalizeEmail(input.ContactEmail)
	input.ContactPhone = strings.TrimSpace(input.ContactPhone)
	input.ContactWeChat = strings.TrimSpace(input.ContactWeChat)
	input.Notes = strings.TrimSpace(input.Notes)
	if input.Status == "" {
		input.Status = "lead"
	}
	return input
}

func validateCustomer(input domain.CustomerInput) error {
	if len([]rune(input.Name)) < 2 || len([]rune(input.Name)) > 160 {
		return fmt.Errorf("%w: customer name must contain 2 to 160 characters", ErrValidation)
	}
	if len([]rune(input.LegalName)) > 200 || len([]rune(input.ContactName)) > 120 || len([]rune(input.ContactPhone)) > 64 || len([]rune(input.ContactWeChat)) > 120 || len([]rune(input.Notes)) > 4000 {
		return fmt.Errorf("%w: customer field exceeds its maximum length", ErrValidation)
	}
	if input.ContactEmail != "" {
		if err := validateEmail(input.ContactEmail); err != nil {
			return err
		}
	}
	switch input.Status {
	case "lead", "active", "inactive":
		return nil
	default:
		return fmt.Errorf("%w: customer status is invalid", ErrValidation)
	}
}

func validatePassword(password string) error {
	if len(password) < 12 || len(password) > 72 {
		return errors.New("bootstrap administrator password must contain 12 to 72 bytes")
	}
	return nil
}

func validateEmail(value string) error {
	address, err := mail.ParseAddress(value)
	if err != nil || normalizeEmail(address.Address) != value || len(value) > 320 {
		return fmt.Errorf("%w: email is invalid", ErrValidation)
	}
	return nil
}

func identifierPattern(value string) bool {
	if len(value) < 3 || len(value) > 128 {
		return false
	}
	for _, char := range value {
		if (char < 'a' || char > 'z') && (char < 'A' || char > 'Z') && (char < '0' || char > '9') && char != '_' && char != '-' && char != '.' && char != ':' {
			return false
		}
	}
	return true
}

func sha256Pattern(value string) bool {
	if len(value) != 64 {
		return false
	}
	for _, char := range value {
		if (char < '0' || char > '9') && (char < 'a' || char > 'f') {
			return false
		}
	}
	return true
}

func normalizeEmail(value string) string {
	return strings.ToLower(strings.TrimSpace(value))
}

func randomToken() (string, error) {
	bytes := make([]byte, 32)
	if _, err := rand.Read(bytes); err != nil {
		return "", err
	}
	return hex.EncodeToString(bytes), nil
}

func newID(prefix string, now time.Time) string {
	random := make([]byte, 8)
	if _, err := rand.Read(random); err != nil {
		panic("crypto/rand unavailable: " + err.Error())
	}
	return prefix + "_" + now.UTC().Format("20060102T150405.000000000") + "_" + hex.EncodeToString(random)
}
