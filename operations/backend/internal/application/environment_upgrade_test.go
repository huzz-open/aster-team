package application

import (
	"bytes"
	"context"
	"errors"
	"io"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

type upgradeMemory struct {
	EnvironmentUpgradeStore
	BusinessStore
	env      domain.UpgradeEnvironment
	artifact domain.ReleaseArtifact
	mu       sync.Mutex
	task     domain.EnvironmentUpgrade
	samples  []domain.UpgradeProbeSample
	terminal bool
	failSave bool
}

func (m *upgradeMemory) GetUpgradeEnvironment(context.Context, string) (domain.UpgradeEnvironment, string, error) {
	return m.env, "sealed", nil
}
func (m *upgradeMemory) GetReleaseArtifact(context.Context, string) (domain.ReleaseArtifact, error) {
	return m.artifact, nil
}
func (m *upgradeMemory) RenewEnvironmentUpgrade(context.Context, string, string) error { return nil }
func (m *upgradeMemory) SaveEnvironmentUpgrade(_ context.Context, task domain.EnvironmentUpgrade, _ string, samples []domain.UpgradeProbeSample, terminal bool) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.failSave {
		return errors.New("lease lost")
	}
	m.task = task
	m.samples = append(m.samples, samples...)
	m.terminal = terminal
	return nil
}

type fakeUpgradeSecrets struct{}

func (fakeUpgradeSecrets) Seal(string, domain.UpgradeCredentials) (string, error) {
	return "sealed", nil
}
func (fakeUpgradeSecrets) Open(string, string) (domain.UpgradeCredentials, error) {
	return domain.UpgradeCredentials{}, nil
}

type fakeUpgradeFactory struct{ target *fakeUpgradeTarget }

func (f fakeUpgradeFactory) Connect(domain.UpgradeEnvironment, domain.UpgradeCredentials) (ports.UpgradeTarget, error) {
	return f.target, nil
}

type fakeUpgradeTarget struct {
	statusDelay  time.Duration
	uploads      atomic.Int32
	modelCalls   atomic.Int32
	uploaded     atomic.Bool
	failBaseline bool
	failAfter    bool
	loseResponse bool
	missingJob   bool
}

func (f *fakeUpgradeTarget) Close() {}
func (f *fakeUpgradeTarget) Status(ctx context.Context, id string) (domain.TargetMaintenance, error) {
	if id != "" && f.statusDelay > 0 {
		select {
		case <-ctx.Done():
			return domain.TargetMaintenance{}, ctx.Err()
		case <-time.After(f.statusDelay):
		}
	}
	status := domain.TargetMaintenance{InstallationID: "installation_test", Platform: "linux", Architecture: "x86_64", CurrentVersion: "2.0.1", CorrelatedUpgrades: true}
	status.Capabilities.DatabaseDriver = "sqlcipher"
	status.Capabilities.SupportedModes = []string{"maintenance"}
	if id != "" && !f.missingJob {
		status.CurrentVersion = "2.0.2"
		status.Jobs = []domain.TargetUpgradeJob{{ID: id, Status: "succeeded", TargetVersion: "2.0.2"}}
	}
	return status, nil
}
func (f *fakeUpgradeTarget) Upload(_ context.Context, _ string, _ string, r io.Reader) error {
	io.Copy(io.Discard, r)
	f.uploads.Add(1)
	f.uploaded.Store(true)
	if f.loseResponse {
		return errors.New("response lost")
	}
	return nil
}
func (f *fakeUpgradeTarget) Probe(_ context.Context, kind string) domain.UpgradeProbeSample {
	if kind == "model" || kind == "stream" {
		f.modelCalls.Add(1)
	}
	ok := true
	if kind == "stream" {
		ok = !(f.failBaseline && !f.uploaded.Load() || f.failAfter && f.uploaded.Load())
	}
	return domain.UpgradeProbeSample{Kind: kind, At: time.Now().UTC(), Status: 200, OK: ok}
}

type fakeUpgradeArtifacts struct{ ports.ArtifactStore }
type upgradeReader struct{ *bytes.Reader }

func (upgradeReader) Close() error { return nil }
func (fakeUpgradeArtifacts) OpenObject(context.Context, string, int64) (ports.ReadSeekCloser, int64, error) {
	return upgradeReader{bytes.NewReader([]byte("package"))}, 7, nil
}

func upgradeWorkerFixture() (*Service, *upgradeMemory, *fakeUpgradeTarget, domain.EnvironmentUpgrade) {
	memory := &upgradeMemory{env: domain.UpgradeEnvironment{ID: "environment_test", InstallationID: "installation_test"}, artifact: domain.ReleaseArtifact{ID: "artifact_test", Version: "2.0.2", Platform: "linux", Architecture: "amd64", SHA256: strings.Repeat("a", 64), SizeBytes: 7}}
	target := &fakeUpgradeTarget{}
	service := &Service{now: time.Now, environmentUpgrades: memory, upgradeSecrets: fakeUpgradeSecrets{}, upgradeTargets: fakeUpgradeFactory{target}, business: memory, artifacts: fakeUpgradeArtifacts{}}
	task := domain.EnvironmentUpgrade{ID: "upgrade_test_12345", EnvironmentID: memory.env.ID, ArtifactID: memory.artifact.ID, ArtifactSHA256: memory.artifact.SHA256, TargetJobID: "upgrade_test_12345", TargetVersion: "2.0.2", Phase: "queued", Mode: "maintenance", ContinuityResult: "not_applicable", ProbeUntil: time.Now().Add(time.Minute)}
	return service, memory, target, task
}

var fastUpgradeSchedule = upgradeSchedule{2 * time.Millisecond, 200 * time.Millisecond, 200 * time.Millisecond, 5 * time.Millisecond, 5 * time.Millisecond, 10 * time.Millisecond}

func TestUpgradeWorkerKeepsUploadAndProbeResultsSeparate(t *testing.T) {
	for _, tc := range []struct {
		name                  string
		baseline, after, lost bool
		uploads               int32
		result, recovery      string
	}{
		{"success", false, false, false, 1, "succeeded", "recovered"},
		{"bad_baseline", true, false, false, 0, "failed", "not_started"},
		{"lost_response", false, false, true, 1, "succeeded", "recovered"},
		{"post_stream_failure", false, true, false, 1, "succeeded", "failed"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s, m, target, task := upgradeWorkerFixture()
			target.failBaseline = tc.baseline
			target.failAfter = tc.after
			target.loseResponse = tc.lost
			ctx, cancel := context.WithTimeout(context.Background(), time.Second)
			defer cancel()
			s.runEnvironmentUpgradeWithSchedule(ctx, task, "owner", fastUpgradeSchedule)
			if !m.terminal || target.uploads.Load() != tc.uploads || m.task.UpgradeResult != tc.result || m.task.RecoveryResult != tc.recovery || m.task.ContinuityResult != "not_applicable" {
				t.Fatalf("unexpected result: %+v uploads=%d", m.task, target.uploads.Load())
			}
			if len(m.samples) == 0 {
				t.Fatal("missing probe evidence")
			}
		})
	}
}

func TestResumedUploadOnlyQueriesCorrelationAndPreservesGap(t *testing.T) {
	s, m, target, task := upgradeWorkerFixture()
	task.Phase = "uploading"
	task.CoverageGap = true
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	s.runEnvironmentUpgradeWithSchedule(ctx, task, "owner", fastUpgradeSchedule)
	if target.uploads.Load() != 0 || !m.terminal || !m.task.CoverageGap || m.task.RecoveryResult != "recovered_with_observation_gap" {
		t.Fatalf("unsafe recovery: %+v", m.task)
	}
}

func TestUnconfirmedUploadAndExhaustedBudgetCannotUnlockTarget(t *testing.T) {
	s, m, target, task := upgradeWorkerFixture()
	task.Phase = "uploading"
	task.ProbeUntil = time.Now().Add(-time.Minute)
	target.missingJob = true
	ctx, cancel := context.WithTimeout(context.Background(), 25*time.Millisecond)
	defer cancel()
	s.runEnvironmentUpgradeWithSchedule(ctx, task, "owner", fastUpgradeSchedule)
	if target.uploads.Load() != 0 || target.modelCalls.Load() != 0 || m.terminal || m.task.UpgradeResult != "unknown" || !m.task.CoverageGap {
		t.Fatalf("uncertainty was hidden: %+v", m.task)
	}
}

func TestLeaseLossStopsBeforeUploadingAndBudgetIsReserved(t *testing.T) {
	s, m, target, task := upgradeWorkerFixture()
	m.failSave = true
	s.runEnvironmentUpgradeWithSchedule(context.Background(), task, "owner", fastUpgradeSchedule)
	if target.uploads.Load() != 0 || target.modelCalls.Load() != 0 {
		t.Fatal("work continued after lease loss")
	}
	s, m, target, task = upgradeWorkerFixture()
	task.ModelRequests = 119
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	s.runEnvironmentUpgradeWithSchedule(ctx, task, "owner", fastUpgradeSchedule)
	if m.task.ModelRequests != 120 || target.modelCalls.Load() != 1 || target.uploads.Load() != 0 {
		t.Fatalf("model budget not bounded: %+v", m.task)
	}
}

func TestEnvironmentOriginsAndCapabilitiesFailClosed(t *testing.T) {
	for _, origin := range []string{"https://user:password@example.com", "https://example.com/path", "http://10.0.0.140", "https://example.com?target=evil", "file:///tmp"} {
		if _, err := normalizeTargetOrigin(origin); err == nil {
			t.Errorf("accepted %s", origin)
		}
	}
	for _, origin := range []string{"https://10.0.0.140", "http://127.0.0.1:11082", "https://admin.example.com/"} {
		if _, err := normalizeTargetOrigin(origin); err != nil {
			t.Errorf("rejected %s", origin)
		}
	}
	env := domain.UpgradeEnvironment{InstallationID: "expected"}
	status := domain.TargetMaintenance{InstallationID: "different", CorrelatedUpgrades: true}
	status.Capabilities.SupportedModes = []string{"maintenance"}
	if validateUpgradeTarget(env, status) == nil {
		t.Fatal("installation mismatch accepted")
	}
	status.InstallationID = "expected"
	status.CorrelatedUpgrades = false
	if validateUpgradeTarget(env, status) == nil {
		t.Fatal("legacy target accepted")
	}
}

func TestProbeSummaryKeepsOriginalFailureWindow(t *testing.T) {
	var task domain.EnvironmentUpgrade
	start := time.Unix(100, 0)
	for i, ok := range []bool{false, false, true} {
		recordUpgradeSample(&task, domain.UpgradeProbeSample{Kind: "stream", At: start.Add(time.Duration(i) * time.Second), DurationMS: 100, OK: ok})
	}
	if len(task.Summary) != 1 {
		t.Fatal("summary missing")
	}
	item := task.Summary[0]
	if item.Attempts != 3 || item.Failures != 2 || item.LongestFailureMS != 2100 || !item.FailureSince.IsZero() || item.LastRecoveryAt.IsZero() {
		t.Fatalf("failed samples were overwritten: %+v", item)
	}
}

type upgradeAllowPermission struct{}

func (upgradeAllowPermission) HasPermission(context.Context, string, string) (bool, error) {
	return true, nil
}
func (m *upgradeMemory) CreateEnvironmentUpgrade(_ context.Context, task domain.EnvironmentUpgrade) error {
	m.task = task
	return nil
}

func TestCreatedUpgradeUsesSafeStableTargetCorrelation(t *testing.T) {
	s, m, _, _ := upgradeWorkerFixture()
	s.permissions = upgradeAllowPermission{}
	run := int64(1)
	m.artifact.RuntimeLinkage = "musl-static"
	m.artifact.SignatureRef = "release-key:test"
	m.artifact.GitHubRunID = &run
	task, err := s.CreateEnvironmentUpgrade(context.Background(), m.env.ID, m.artifact.ID, "operator_test")
	if err != nil {
		t.Fatal(err)
	}
	if task.TargetJobID == "" || task.TargetJobID != m.task.TargetJobID {
		t.Fatal("target correlation was not persisted")
	}
	for _, ch := range task.TargetJobID {
		if !(ch >= 'a' && ch <= 'z' || ch >= 'A' && ch <= 'Z' || ch >= '0' && ch <= '9' || ch == '_') {
			t.Fatalf("generated target ID violates Customer contract: %q", task.TargetJobID)
		}
	}
	if task.Mode != "maintenance" || task.ArtifactSHA256 != m.artifact.SHA256 {
		t.Fatal("upgrade inputs not bound")
	}
}

func TestTargetStatusTimeoutDoesNotPauseProbeScheduling(t *testing.T) {
	s, m, target, task := upgradeWorkerFixture()
	task.Phase = "tracking"
	target.statusDelay = time.Second
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	s.runEnvironmentUpgradeWithSchedule(ctx, task, "owner", fastUpgradeSchedule)
	health := 0
	for _, sample := range m.samples {
		if sample.Kind == "health" {
			health++
		}
	}
	if health < 3 || target.modelCalls.Load() < 2 {
		t.Fatalf("target status blocked independent probes: health=%d model=%d", health, target.modelCalls.Load())
	}
}
