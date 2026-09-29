package application

import (
	"context"
	"crypto/sha256"
	"errors"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/commercial"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/licenseprotocol"
	"aster.local/team/operations/backend/internal/ports"
	"golang.org/x/crypto/bcrypt"
)

type fakeStore struct {
	operator      domain.Operator
	passwordHash  string
	session       CreateSessionParams
	customers     []domain.Customer
	licenseRecord ports.LicenseRecord
	issuances     []ports.LicenseIssuance
}

func (store *fakeStore) OperatorCount(context.Context) (int64, error) {
	if store.operator.ID == "" {
		return 0, nil
	}
	return 1, nil
}

func (store *fakeStore) CreateOperator(_ context.Context, params CreateOperatorParams) (domain.Operator, error) {
	store.operator = domain.Operator{ID: params.ID, Email: params.Email, DisplayName: params.DisplayName, Status: "active", PasswordChangeRequired: params.PasswordChangeRequired, CreatedAt: params.CreatedAt}
	store.passwordHash = params.PasswordHash
	return store.operator, nil
}

func (store *fakeStore) FindOperatorForLogin(_ context.Context, email string) (domain.Operator, string, error) {
	if store.operator.Email != email {
		return domain.Operator{}, "", errors.New("not found")
	}
	return store.operator, store.passwordHash, nil
}

func (store *fakeStore) CreateSession(_ context.Context, params CreateSessionParams) error {
	store.session = params
	return nil
}

func (store *fakeStore) AuthenticateSession(_ context.Context, tokenHash [32]byte, now time.Time) (domain.AuthenticatedOperator, error) {
	if store.session.TokenHash != tokenHash || !store.session.ExpiresAt.After(now) {
		return domain.AuthenticatedOperator{}, errors.New("not found")
	}
	return domain.AuthenticatedOperator{Operator: store.operator, SessionID: store.session.ID, CSRFHash: store.session.CSRFHash, ExpiresAt: store.session.ExpiresAt}, nil
}

func (store *fakeStore) DeleteSession(_ context.Context, tokenHash [32]byte) error {
	if store.session.TokenHash == tokenHash {
		store.session = CreateSessionParams{}
	}
	return nil
}

func (*fakeStore) DeleteExpiredSessions(context.Context, time.Time) error { return nil }

func (store *fakeStore) CreateCustomer(_ context.Context, customer domain.Customer, _ string) error {
	store.customers = append(store.customers, customer)
	return nil
}

func (store *fakeStore) ListCustomers(_ context.Context, _ string, limit int) ([]domain.Customer, error) {
	if limit > len(store.customers) {
		limit = len(store.customers)
	}
	return store.customers[:limit], nil
}

func (store *fakeStore) Overview(context.Context) (domain.Overview, error) {
	return domain.Overview{CustomersTotal: int64(len(store.customers))}, nil
}

func (store *fakeStore) ListLicenseRecords(context.Context, int) ([]ports.LicenseRecord, error) {
	return []ports.LicenseRecord{store.licenseRecord}, nil
}

func (store *fakeStore) LicenseForIssuance(_ context.Context, licenseID string) (ports.LicenseRecord, error) {
	if store.licenseRecord.LicenseID != licenseID {
		return ports.LicenseRecord{}, ErrNotFound
	}
	return store.licenseRecord, nil
}

func (store *fakeStore) SaveLicenseIssuance(_ context.Context, issuance ports.LicenseIssuance, _ string) error {
	store.issuances = append([]ports.LicenseIssuance{issuance}, store.issuances...)
	return nil
}

func (store *fakeStore) ListLicenseIssuances(context.Context, string) ([]ports.LicenseIssuance, error) {
	return append([]ports.LicenseIssuance(nil), store.issuances...), nil
}

type fakeReleaseStore struct {
	*fakeStore
	tasks        []domain.ReleaseTask
	details      map[string]domain.ReleaseTaskDetail
	publishes    map[string]domain.ReleasePublishRequest
	distribution commercial.DistributionRecord
}

func (store *fakeReleaseStore) GetFreeDistribution(context.Context, string) (commercial.DistributionRecord, error) {
	if store.distribution.Snapshot.ID == "" {
		return commercial.DistributionRecord{}, ErrNotFound
	}
	return store.distribution, nil
}

func (store *fakeReleaseStore) HasPermission(context.Context, string, string) (bool, error) {
	return true, nil
}
func (store *fakeReleaseStore) ListReleaseTasks(_ context.Context, limit int) ([]domain.ReleaseTask, error) {
	if limit > len(store.tasks) {
		limit = len(store.tasks)
	}
	return append([]domain.ReleaseTask(nil), store.tasks[:limit]...), nil
}
func (store *fakeReleaseStore) GetReleaseTaskDetail(_ context.Context, taskID string) (domain.ReleaseTaskDetail, error) {
	detail, ok := store.details[taskID]
	if !ok {
		return domain.ReleaseTaskDetail{}, ErrNotFound
	}
	return detail, nil
}
func (store *fakeReleaseStore) CreateReleaseTask(_ context.Context, task domain.ReleaseTask, _ string) error {
	for _, existing := range store.tasks {
		if existing.Version == task.Version && existing.SourceCommitSHA == task.SourceCommitSHA && existing.Status != "failed" {
			return ErrReleaseDuplicate
		}
	}
	store.tasks = append([]domain.ReleaseTask{task}, store.tasks...)
	store.details[task.ID] = domain.ReleaseTaskDetail{Task: task, Runs: []domain.ReleaseRun{}, Artifacts: []domain.ReleaseTaskArtifact{}}
	return nil
}
func (store *fakeReleaseStore) RecordReleaseDispatch(_ context.Context, taskID string, run domain.ReleaseRun) error {
	detail := store.details[taskID]
	detail.Task.Status = "queued"
	detail.Runs = []domain.ReleaseRun{run}
	store.details[taskID] = detail
	return nil
}
func (store *fakeReleaseStore) MarkReleaseTaskFailed(_ context.Context, taskID, code string) error {
	detail := store.details[taskID]
	detail.Task.Status = "failed"
	detail.Task.ErrorCode = &code
	store.details[taskID] = detail
	return nil
}
func (store *fakeReleaseStore) ApplyReleaseSnapshot(_ context.Context, taskID string, snapshot ports.ReleaseSnapshot, status, phase string, errorCode *string, now time.Time) error {
	detail := store.details[taskID]
	detail.Task.Status, detail.Task.Phase, detail.Task.ErrorCode, detail.Task.UpdatedAt = status, phase, errorCode, now
	detail.Runs, detail.Artifacts = []domain.ReleaseRun{snapshot.Run}, snapshot.Artifacts
	store.details[taskID] = detail
	return nil
}
func (store *fakeReleaseStore) CompleteReleaseTaskArtifact(_ context.Context, taskID, artifactID string, release domain.ReleaseArtifact, signatureKeyID string, now time.Time) error {
	detail := store.details[taskID]
	for index := range detail.Artifacts {
		if detail.Artifacts[index].ID == artifactID {
			detail.Artifacts[index].SHA256 = &release.SHA256
			detail.Artifacts[index].ReleaseManifestSHA256 = &release.ReleaseManifestSHA256
			detail.Artifacts[index].SignatureKeyID = signatureKeyID
			detail.Artifacts[index].RuntimeLinkage = release.RuntimeLinkage
			detail.Artifacts[index].VerificationStatus = "verified"
			detail.Artifacts[index].ReleaseArtifactID = &release.ID
			detail.Artifacts[index].VerifiedAt = &now
		}
	}
	store.details[taskID] = detail
	return nil
}
func (store *fakeReleaseStore) FailReleaseTaskArtifact(_ context.Context, taskID, artifactID, code string, now time.Time) error {
	detail := store.details[taskID]
	for index := range detail.Artifacts {
		if detail.Artifacts[index].ID == artifactID {
			detail.Artifacts[index].VerificationStatus = "failed"
			detail.Artifacts[index].VerificationErrorCode = &code
		}
	}
	store.details[taskID] = detail
	return nil
}
func (store *fakeReleaseStore) PrepareReleaseTaskArtifactVerification(_ context.Context, taskID, artifactID, _ string, now time.Time) error {
	detail := store.details[taskID]
	detail.Task.Status, detail.Task.Phase, detail.Task.ErrorCode, detail.Task.UpdatedAt, detail.Task.CompletedAt = "verifying", "artifact_verification", nil, now, nil
	for index := range detail.Artifacts {
		if detail.Artifacts[index].ID == artifactID {
			detail.Artifacts[index].VerificationStatus = "queued"
			detail.Artifacts[index].VerificationErrorCode = nil
		}
	}
	store.details[taskID] = detail
	return nil
}
func (*fakeReleaseStore) ListPendingReleaseTaskIDs(context.Context, int) ([]string, error) {
	return nil, nil
}
func (store *fakeReleaseStore) ListReleasePublishRequests(context.Context, int) ([]domain.ReleasePublishRequest, error) {
	items := make([]domain.ReleasePublishRequest, 0, len(store.publishes))
	for _, item := range store.publishes {
		items = append(items, item)
	}
	return items, nil
}
func (store *fakeReleaseStore) GetReleasePublishRequest(_ context.Context, requestID string) (domain.ReleasePublishRequest, error) {
	item, ok := store.publishes[requestID]
	if !ok {
		return domain.ReleasePublishRequest{}, ErrNotFound
	}
	return item, nil
}
func (store *fakeReleaseStore) CreateReleasePublishRequest(_ context.Context, request domain.ReleasePublishRequest) error {
	store.publishes[request.ID] = request
	return nil
}
func (store *fakeReleaseStore) DecideReleasePublishRequest(_ context.Context, requestID, decision, comment, operatorID string, now time.Time) error {
	item := store.publishes[requestID]
	item.Status, item.ApprovalComment, item.ApprovedBy, item.ApprovedAt, item.UpdatedAt = decision, comment, &operatorID, &now, now
	store.publishes[requestID] = item
	return nil
}
func (store *fakeReleaseStore) BeginReleasePublish(context.Context, string, string, time.Time) (domain.ReleasePublishRequest, error) {
	return domain.ReleasePublishRequest{}, errors.New("unexpected BeginReleasePublish")
}
func (*fakeReleaseStore) CompleteReleasePublish(context.Context, string, ports.ReleasePublishResult, time.Time) error {
	return errors.New("unexpected CompleteReleasePublish")
}
func (*fakeReleaseStore) FailReleasePublish(context.Context, string, string, time.Time) error {
	return nil
}
func (*fakeReleaseStore) ListPublishingReleaseRequestIDs(context.Context, int) ([]string, error) {
	return nil, nil
}

type fakeReleaseOrchestrator struct {
	preflight  ports.ReleasePreflight
	dispatch   ports.ReleaseDispatch
	snapshot   ports.ReleaseSnapshot
	snapshots  *int
	dispatched *ports.ReleaseDispatchInput
}

type fakeReleaseArtifactVerifier struct {
	verified ports.VerifiedReleaseArtifact
	err      error
	calls    *int
}

func (verifier fakeReleaseArtifactVerifier) Verify(context.Context, domain.ReleaseTask, domain.ReleaseTaskArtifact) (ports.VerifiedReleaseArtifact, error) {
	if verifier.calls != nil {
		*verifier.calls++
	}
	return verifier.verified, verifier.err
}

func (fakeReleaseOrchestrator) Capabilities() domain.ReleaseCapabilities {
	return domain.ReleaseCapabilities{Configured: true, Repository: "huzz-max/aster-team", WorkflowFile: "customer-release.yml", Environment: "customer-release", DefaultSourceRef: "main", Targets: domain.ReleaseTargets()}
}
func (value fakeReleaseOrchestrator) Preflight(context.Context, string, string) (ports.ReleasePreflight, error) {
	return value.preflight, nil
}
func (value fakeReleaseOrchestrator) Dispatch(_ context.Context, input ports.ReleaseDispatchInput) (ports.ReleaseDispatch, error) {
	if value.dispatched != nil {
		*value.dispatched = input
	}
	return value.dispatch, nil
}
func (value fakeReleaseOrchestrator) Snapshot(context.Context, domain.ReleaseTask, int64) (ports.ReleaseSnapshot, error) {
	if value.snapshots != nil {
		*value.snapshots++
	}
	return value.snapshot, nil
}

func issuedReleaseDistribution(t *testing.T, store *fakeReleaseStore) Option {
	t.Helper()
	distributionStore, distributionService, _ := distributionFixture(t)
	definition := distributionStore.record.Snapshot.Plan.Definition
	definition.Offer.Expiry = &licenseprotocol.ExpiryV2{Mode: licenseprotocol.NoExpiryV2}
	plan, err := commercial.FreezePlan(distributionStore.record.Snapshot.Plan.PlanID, distributionStore.record.Snapshot.Plan.Version, definition)
	if err != nil {
		t.Fatal(err)
	}
	planSnapshot, err := plan.Snapshot()
	if err != nil {
		t.Fatal(err)
	}
	distributionStore.record.Snapshot.Plan = planSnapshot
	distributionStore.record.Snapshot.PlanSHA256 = plan.Digest()
	snapshotBytes, err := distributionStore.record.Snapshot.Bytes()
	if err != nil {
		t.Fatal(err)
	}
	distributionStore.record.SHA256 = commercial.ContentDigest(snapshotBytes)
	record, err := distributionService.IssueFreeDistribution(context.Background(), distributionStore.record.Snapshot.ID, "distribution-test", "operator_1")
	if err != nil {
		t.Fatal(err)
	}
	store.distribution = record
	return WithV2LicenseSigners(distributionService.licenseSignersV2)
}

func TestCreateReleaseTaskBindsRunAndClassifiesLinuxFailure(t *testing.T) {
	now := time.Date(2026, 8, 28, 14, 0, 0, 0, time.UTC)
	sha := strings.Repeat("a", 40)
	conclusion := "failure"
	runID := int64(321)
	var dispatched ports.ReleaseDispatchInput
	store := &fakeReleaseStore{fakeStore: &fakeStore{}, details: map[string]domain.ReleaseTaskDetail{}}
	orchestrator := fakeReleaseOrchestrator{preflight: ports.ReleasePreflight{CommitSHA: sha},
		dispatched: &dispatched,
		dispatch:   ports.ReleaseDispatch{RunID: runID, RunURL: "https://github.test/runs/321", Status: "queued", CreatedAt: now},
		snapshot: ports.ReleaseSnapshot{Run: domain.ReleaseRun{ID: "release_run_321", Attempt: 1, GitHubRunID: &runID,
			Status: "completed", Conclusion: &conclusion, CreatedAt: now, Jobs: []domain.ReleaseJob{{Name: "Verify customer install (ubuntu-24.04)", Conclusion: &conclusion}}}, Artifacts: []domain.ReleaseTaskArtifact{}}}
	service := NewService(store, time.Hour, WithReleaseOrchestrator(orchestrator), issuedReleaseDistribution(t, store))
	service.now = func() time.Time { return now }
	detail, err := service.CreateReleaseTask(context.Background(), domain.ReleaseTaskInput{Version: "2.0.0", SourceRef: "main", FreeDistributionID: store.distribution.Snapshot.ID}, "operator_1")
	if err != nil {
		t.Fatal(err)
	}
	if detail.Task.SourceCommitSHA != sha || detail.Task.Status != "failed" || detail.Task.ErrorCode == nil || *detail.Task.ErrorCode != "RELEASE_LINUX_VALIDATION_FAILED" {
		t.Fatalf("unexpected release task: %#v", detail.Task)
	}
	if len(detail.Runs) != 1 || detail.Runs[0].GitHubRunID == nil || *detail.Runs[0].GitHubRunID != runID {
		t.Fatalf("workflow run was not bound to task: %#v", detail.Runs)
	}
	if detail.Task.FreeDistributionID != store.distribution.Snapshot.ID || detail.Task.FreeLicenseSHA256 != store.distribution.DocumentSHA256 ||
		dispatched.FreeDistributionID != store.distribution.Snapshot.ID || dispatched.FreeLicenseSHA256 != store.distribution.DocumentSHA256 || len(dispatched.FreeLicenseDocument) == 0 {
		t.Fatalf("free license was not frozen into the release task and dispatch: %#v %#v", detail.Task, dispatched)
	}
	for _, version := range []string{"latest", "v2.0.0", "01.2.3", "1.0.0-alpha.01"} {
		if _, err := service.CreateReleaseTask(context.Background(), domain.ReleaseTaskInput{Version: version, SourceRef: "main", FreeDistributionID: store.distribution.Snapshot.ID}, "operator_1"); !errors.Is(err, ErrValidation) {
			t.Fatalf("invalid version %q error = %v", version, err)
		}
	}
}

func TestCreateReleaseTaskRejectsExpiringFreeDistribution(t *testing.T) {
	distributionStore, distributionService, _ := distributionFixture(t)
	record, err := distributionService.IssueFreeDistribution(context.Background(), distributionStore.record.Snapshot.ID, "distribution-test", "operator_1")
	if err != nil {
		t.Fatal(err)
	}
	store := &fakeReleaseStore{fakeStore: &fakeStore{}, distribution: record, details: map[string]domain.ReleaseTaskDetail{}}
	service := NewService(store, time.Hour,
		WithReleaseOrchestrator(fakeReleaseOrchestrator{preflight: ports.ReleasePreflight{CommitSHA: strings.Repeat("a", 40)}}),
		WithV2LicenseSigners(distributionService.licenseSignersV2),
	)
	_, err = service.CreateReleaseTask(context.Background(), domain.ReleaseTaskInput{Version: "2.0.0", SourceRef: "main", FreeDistributionID: record.Snapshot.ID}, "operator_1")
	if !errors.Is(err, ErrValidation) || !strings.Contains(err.Error(), "no expiry") {
		t.Fatalf("expiring free distribution error = %v", err)
	}
}

func TestCreateReleaseTaskCompletesOnlyAfterIndependentArtifactVerification(t *testing.T) {
	now := time.Date(2026, 8, 28, 14, 0, 0, 0, time.UTC)
	sha := strings.Repeat("a", 40)
	archiveSHA := strings.Repeat("b", 64)
	manifestSHA := strings.Repeat("c", 64)
	githubDigest := strings.Repeat("d", 64)
	conclusion := "success"
	runID := int64(654)
	store := &fakeReleaseStore{fakeStore: &fakeStore{}, details: map[string]domain.ReleaseTaskDetail{}}
	orchestrator := fakeReleaseOrchestrator{preflight: ports.ReleasePreflight{CommitSHA: sha},
		dispatch: ports.ReleaseDispatch{RunID: runID, RunURL: "https://github.test/runs/654", Status: "queued", CreatedAt: now},
		snapshot: ports.ReleaseSnapshot{Run: domain.ReleaseRun{ID: "release_run_654", Attempt: 1, GitHubRunID: &runID,
			Status: "completed", Conclusion: &conclusion, CompletedAt: &now, CreatedAt: now, Jobs: []domain.ReleaseJob{}},
			Artifacts: []domain.ReleaseTaskArtifact{{ID: "artifact_654", GitHubArtifactID: 654, Name: "customer-linux-amd64-2.0.0", Platform: "linux", Architecture: "amd64",
				GitHubDigestSHA256: &githubDigest, VerificationStatus: "pending", CreatedAt: now}, {ID: "artifact_windows", GitHubArtifactID: 655, Name: "customer-windows-amd64-2.0.0", Platform: "windows", Architecture: "amd64", GitHubDigestSHA256: &githubDigest, VerificationStatus: "pending", CreatedAt: now}}}}
	verifier := fakeReleaseArtifactVerifier{verified: ports.VerifiedReleaseArtifact{ObjectKey: "objects/bb/" + archiveSHA,
		SHA256: archiveSHA, SizeBytes: 1024, ManifestSHA256: manifestSHA, SignatureKeyID: "release-test-01", RuntimeLinkage: "musl-static"}}
	service := NewService(store, time.Hour, WithReleaseOrchestrator(orchestrator), WithReleaseArtifactVerifier(verifier), issuedReleaseDistribution(t, store))
	service.now = func() time.Time { return now }
	detail, err := service.CreateReleaseTask(context.Background(), domain.ReleaseTaskInput{Version: "2.0.0", SourceRef: "main", FreeDistributionID: store.distribution.Snapshot.ID}, "operator_1")
	if err != nil {
		t.Fatal(err)
	}
	if detail.Task.Status != "completed" || len(detail.Artifacts) != 2 || detail.Artifacts[0].VerificationStatus != "verified" || detail.Artifacts[1].VerificationStatus != "verified" ||
		detail.Artifacts[0].SHA256 == nil || *detail.Artifacts[0].SHA256 != archiveSHA || detail.Artifacts[0].RuntimeLinkage != "musl-static" {
		t.Fatalf("release was not completed from the verified artifact: %#v", detail)
	}
}

func TestReverifyReleaseTaskRequeuesExistingSuccessfulRunArtifact(t *testing.T) {
	now := time.Date(2026, 8, 29, 9, 0, 0, 0, time.UTC)
	conclusion := "success"
	digest := strings.Repeat("d", 64)
	errorCode := "RELEASE_PACKAGE_POLICY_FAILED"
	task := domain.ReleaseTask{ID: "release_task_reverify", Version: "2.0.0", Status: "failed", Phase: "failed", ErrorCode: &errorCode}
	artifact := domain.ReleaseTaskArtifact{ID: "artifact_reverify", Name: "customer-linux-amd64-2.0.0", Platform: "linux", Architecture: "amd64", GitHubArtifactID: 42,
		GitHubDigestSHA256: &digest, VerificationStatus: "failed", VerificationErrorCode: &errorCode}
	store := &fakeReleaseStore{fakeStore: &fakeStore{}, details: map[string]domain.ReleaseTaskDetail{
		task.ID: {Task: task, Runs: []domain.ReleaseRun{{Conclusion: &conclusion}}, Artifacts: []domain.ReleaseTaskArtifact{artifact}},
	}}
	runID := int64(42)
	store.details[task.ID] = domain.ReleaseTaskDetail{Task: task, Runs: []domain.ReleaseRun{{GitHubRunID: &runID, Conclusion: &conclusion}}, Artifacts: []domain.ReleaseTaskArtifact{artifact}}
	verifierCalls, snapshotCalls := 0, 0
	verified := ports.VerifiedReleaseArtifact{ObjectKey: "objects/aa/" + strings.Repeat("a", 64), SHA256: strings.Repeat("a", 64),
		SizeBytes: 1024, ManifestSHA256: strings.Repeat("b", 64), SignatureKeyID: "release-test-01", RuntimeLinkage: "musl-static"}
	service := NewService(store, time.Hour,
		WithReleaseOrchestrator(fakeReleaseOrchestrator{snapshots: &snapshotCalls}),
		WithReleaseArtifactVerifier(fakeReleaseArtifactVerifier{verified: verified, calls: &verifierCalls}))
	service.now = func() time.Time { return now }

	detail, err := service.ReverifyReleaseTask(context.Background(), task.ID, "artifact_reverify", "operator_1")
	if err != nil {
		t.Fatal(err)
	}
	if detail.Task.Status != "verifying" || detail.Task.Phase != "artifact_verification" || detail.Task.ErrorCode != nil || detail.Task.CompletedAt != nil {
		t.Fatalf("task was not requeued for verification: %#v", detail.Task)
	}
	if len(detail.Artifacts) != 1 || detail.Artifacts[0].VerificationStatus != "queued" || detail.Artifacts[0].VerificationErrorCode != nil {
		t.Fatalf("artifact was not reset to pending verification: %#v", detail.Artifacts)
	}
	if err := service.SyncReleaseTask(context.Background(), task.ID); err != nil {
		t.Fatal(err)
	}
	if verifierCalls != 1 || snapshotCalls != 0 {
		t.Fatalf("reverification calls = verifier %d, GitHub snapshots %d; want 1, 0", verifierCalls, snapshotCalls)
	}
	if store.details[task.ID].Task.Status != "completed" {
		t.Fatalf("reverified task did not complete: %#v", store.details[task.ID])
	}
}

func TestReverifyReleaseTaskRejectsBuildFailureWithoutArtifact(t *testing.T) {
	conclusion := "failure"
	task := domain.ReleaseTask{ID: "release_task_build_failed", Version: "2.0.0", Status: "failed", Phase: "failed"}
	store := &fakeReleaseStore{fakeStore: &fakeStore{}, details: map[string]domain.ReleaseTaskDetail{
		task.ID: {Task: task, Runs: []domain.ReleaseRun{{Conclusion: &conclusion}}, Artifacts: []domain.ReleaseTaskArtifact{}},
	}}
	service := NewService(store, time.Hour, WithReleaseArtifactVerifier(fakeReleaseArtifactVerifier{}))

	if _, err := service.ReverifyReleaseTask(context.Background(), task.ID, "artifact_reverify", "operator_1"); !errors.Is(err, ErrValidation) {
		t.Fatalf("build failure reverification error = %v, want validation error", err)
	}
}

func TestFormalPublishRequiresIndependentApprover(t *testing.T) {
	now := time.Date(2026, 8, 28, 14, 0, 0, 0, time.UTC)
	request := domain.ReleasePublishRequest{ID: "publish_1", Status: "requested", RequestedBy: "operator_1", CreatedAt: now, UpdatedAt: now}
	store := &fakeReleaseStore{fakeStore: &fakeStore{}, details: map[string]domain.ReleaseTaskDetail{}, publishes: map[string]domain.ReleasePublishRequest{request.ID: request}}
	service := NewService(store, time.Hour)
	service.now = func() time.Time { return now }
	input := domain.ReleasePublishApprovalInput{Decision: "approved", Comment: "reviewed"}
	if _, err := service.DecideReleasePublish(context.Background(), request.ID, input, "operator_1"); !errors.Is(err, ErrReleasePublishApproval) {
		t.Fatalf("self approval error = %v", err)
	}
	approved, err := service.DecideReleasePublish(context.Background(), request.ID, input, "operator_2")
	if err != nil || approved.Status != "approved" || approved.ApprovedBy == nil || *approved.ApprovedBy != "operator_2" {
		t.Fatalf("independent approval = %#v, %v", approved, err)
	}
}

func TestBootstrapLoginAuthenticateAndCSRF(t *testing.T) {
	store := &fakeStore{}
	service := NewService(store, time.Hour)
	fixedNow := time.Date(2026, 8, 24, 10, 30, 0, 0, time.UTC)
	service.now = func() time.Time { return fixedNow }

	if err := service.BootstrapAdmin(context.Background(), " Admin@Example.com ", "long-test-password"); err != nil {
		t.Fatalf("BootstrapAdmin() error = %v", err)
	}
	if store.operator.Email != "admin@example.com" || !store.operator.PasswordChangeRequired {
		t.Fatalf("unexpected bootstrap operator: %#v", store.operator)
	}
	if bcrypt.CompareHashAndPassword([]byte(store.passwordHash), []byte("long-test-password")) != nil {
		t.Fatal("bootstrap password was not stored as a bcrypt hash")
	}

	if _, err := service.Login(context.Background(), store.operator.Email, "wrong-password"); !errors.Is(err, ErrInvalidCredentials) {
		t.Fatalf("Login() error = %v, want ErrInvalidCredentials", err)
	}
	tokens, err := service.Login(context.Background(), store.operator.Email, "long-test-password")
	if err != nil {
		t.Fatalf("Login() error = %v", err)
	}
	operator, err := service.Authenticate(context.Background(), tokens.SessionToken)
	if err != nil {
		t.Fatalf("Authenticate() error = %v", err)
	}
	if !service.ValidateCSRF(operator, tokens.CSRFToken, tokens.CSRFToken) {
		t.Fatal("ValidateCSRF() rejected matching session tokens")
	}
	if service.ValidateCSRF(operator, tokens.CSRFToken, "different") {
		t.Fatal("ValidateCSRF() accepted a different header token")
	}
	if store.session.TokenHash != sha256.Sum256([]byte(tokens.SessionToken)) {
		t.Fatal("session token was not stored as a SHA-256 hash")
	}
}

func TestCreateCustomerNormalizesAndValidatesInput(t *testing.T) {
	store := &fakeStore{}
	service := NewService(store, time.Hour)
	service.now = func() time.Time { return time.Date(2026, 8, 24, 11, 0, 0, 0, time.UTC) }

	customer, err := service.CreateCustomer(context.Background(), domain.CustomerInput{
		Name: "  Example Customer  ", ContactEmail: " USER@EXAMPLE.COM ",
	}, "operator_1")
	if err != nil {
		t.Fatalf("CreateCustomer() error = %v", err)
	}
	if customer.Name != "Example Customer" || customer.ContactEmail != "user@example.com" || customer.Status != "lead" {
		t.Fatalf("unexpected normalized customer: %#v", customer)
	}
	if _, err := service.CreateCustomer(context.Background(), domain.CustomerInput{Name: "x"}, "operator_1"); !errors.Is(err, ErrValidation) {
		t.Fatalf("CreateCustomer() error = %v, want ErrValidation", err)
	}
}

func TestReauthenticateRequiresCurrentOperatorPassword(t *testing.T) {
	hash, err := bcrypt.GenerateFromPassword([]byte("current-password"), bcrypt.MinCost)
	if err != nil {
		t.Fatal(err)
	}
	store := &fakeStore{operator: domain.Operator{ID: "operator_1", Email: "admin@example.com", Status: "active"}, passwordHash: string(hash)}
	service := NewService(store, time.Hour)
	operator := domain.AuthenticatedOperator{Operator: store.operator}
	if err := service.Reauthenticate(context.Background(), operator, "wrong-password"); !errors.Is(err, ErrInvalidCredentials) {
		t.Fatalf("Reauthenticate() error = %v, want ErrInvalidCredentials", err)
	}
	if err := service.Reauthenticate(context.Background(), operator, "current-password"); err != nil {
		t.Fatalf("Reauthenticate() error = %v", err)
	}
}

func TestListLicenseRecordsReportsExpiredPolicy(t *testing.T) {
	now := time.Date(2026, 8, 27, 0, 0, 0, 0, time.UTC)
	store := &fakeStore{licenseRecord: ports.LicenseRecord{
		LicensePolicy: ports.LicensePolicy{LicenseID: "license_test_001", ValidUntil: now.Add(-time.Second)},
		Status:        "active",
	}}
	service := NewService(store, time.Hour)
	service.now = func() time.Time { return now }
	items, err := service.ListLicenseRecords(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(items) != 1 || items[0].Status != "expired" {
		t.Fatalf("unexpected license records: %#v", items)
	}
}

func TestSHA256PatternAcceptsOnlyCanonicalLowercaseDigest(t *testing.T) {
	if !sha256Pattern(strings.Repeat("a", 64)) {
		t.Fatal("sha256Pattern() rejected a canonical digest")
	}
	for _, value := range []string{strings.Repeat("a", 63), strings.Repeat("A", 64), strings.Repeat("z", 64)} {
		if sha256Pattern(value) {
			t.Fatalf("sha256Pattern(%q) accepted an invalid digest", value)
		}
	}
}
