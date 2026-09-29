package application

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"

	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

func (store *fakeReleaseStore) FinishReleaseTaskVerification(_ context.Context, taskID string, taskError *string, now time.Time) error {
	detail := store.details[taskID]
	for _, artifact := range detail.Artifacts {
		if artifact.VerificationStatus == "queued" {
			return nil
		}
		if artifact.VerificationStatus == "failed" {
			taskError = artifact.VerificationErrorCode
		}
	}
	detail.Task.Status = "completed"
	if taskError != nil {
		detail.Task.Status = "failed"
	}
	detail.Task.Phase, detail.Task.ErrorCode, detail.Task.CompletedAt = detail.Task.Status, taskError, &now
	store.details[taskID] = detail
	return nil
}

type packageVerifierFunc func(domain.ReleaseTaskArtifact) (ports.VerifiedReleaseArtifact, error)

func (fn packageVerifierFunc) Verify(_ context.Context, _ domain.ReleaseTask, artifact domain.ReleaseTaskArtifact) (ports.VerifiedReleaseArtifact, error) {
	return fn(artifact)
}

func TestPackageVerificationFailureDoesNotBlockOtherPlatform(t *testing.T) {
	for _, failing := range []string{"linux", "windows"} {
		t.Run(failing, func(t *testing.T) {
			runID := int64(42)
			success := "success"
			detail := domain.ReleaseTaskDetail{Task: domain.ReleaseTask{ID: "task", Version: "2.0.0", Status: "verifying", Phase: "artifact_verification"}, Runs: []domain.ReleaseRun{{GitHubRunID: &runID, Conclusion: &success}}}
			for _, target := range domain.ReleaseTargets() {
				detail.Artifacts = append(detail.Artifacts, domain.ReleaseTaskArtifact{ID: target.Platform, Name: target.ArtifactName("2.0.0"), Platform: target.Platform, Architecture: target.Architecture, VerificationStatus: "queued"})
			}
			store := &fakeReleaseStore{fakeStore: &fakeStore{}, details: map[string]domain.ReleaseTaskDetail{"task": detail}}
			var calls []string
			service := NewService(store, time.Hour, WithReleaseOrchestrator(fakeReleaseOrchestrator{}), WithReleaseArtifactVerifier(packageVerifierFunc(func(artifact domain.ReleaseTaskArtifact) (ports.VerifiedReleaseArtifact, error) {
				calls = append(calls, artifact.Platform)
				if artifact.Platform == failing {
					return ports.VerifiedReleaseArtifact{}, &ports.ReleaseArtifactVerificationError{Code: "RELEASE_SIGNATURE_INVALID", Err: errors.New("invalid signature")}
				}
				return ports.VerifiedReleaseArtifact{ObjectKey: "objects/test", SHA256: strings.Repeat("a", 64), ManifestSHA256: strings.Repeat("b", 64), SizeBytes: 100, SignatureKeyID: "test", RuntimeLinkage: "musl-static"}, nil
			})))
			if err := service.SyncReleaseTask(context.Background(), "task"); err != nil {
				t.Fatal(err)
			}
			if len(calls) != 2 {
				t.Fatalf("verification calls = %v", calls)
			}
			result := store.details["task"]
			if result.Task.Status != "failed" {
				t.Fatalf("task status = %s", result.Task.Status)
			}
			for _, artifact := range result.Artifacts {
				if artifact.Platform == failing {
					if artifact.VerificationStatus != "failed" || artifact.ReleaseArtifactID != nil {
						t.Fatalf("failed package was made downloadable: %#v", artifact)
					}
				} else if artifact.VerificationStatus != "verified" || artifact.ReleaseArtifactID == nil {
					t.Fatalf("other platform was blocked: %#v", artifact)
				}
			}
		})
	}
}

func TestHistoricalWindowsPackageRequiresExplicitVerification(t *testing.T) {
	runID := int64(42)
	success := "success"
	digest := strings.Repeat("d", 64)
	releaseID := "release_linux"
	detail := domain.ReleaseTaskDetail{Task: domain.ReleaseTask{ID: "task", Version: "2.0.0", Status: "completed"}, Runs: []domain.ReleaseRun{{GitHubRunID: &runID, Conclusion: &success}}, Artifacts: []domain.ReleaseTaskArtifact{
		{ID: "linux", Name: "customer-linux-amd64-2.0.0", Platform: "linux", Architecture: "amd64", VerificationStatus: "verified", ReleaseArtifactID: &releaseID},
		{ID: "windows", Name: "customer-windows-amd64-2.0.0", Platform: "windows", Architecture: "amd64", VerificationStatus: "pending", GitHubDigestSHA256: &digest},
	}}
	store := &fakeReleaseStore{fakeStore: &fakeStore{}, details: map[string]domain.ReleaseTaskDetail{"task": detail}}
	var calls []string
	service := NewService(store, time.Hour, WithReleaseOrchestrator(fakeReleaseOrchestrator{}), WithReleaseArtifactVerifier(packageVerifierFunc(func(artifact domain.ReleaseTaskArtifact) (ports.VerifiedReleaseArtifact, error) {
		calls = append(calls, artifact.ID)
		return ports.VerifiedReleaseArtifact{ObjectKey: "objects/windows", SHA256: strings.Repeat("b", 64), ManifestSHA256: strings.Repeat("c", 64), SizeBytes: 100, SignatureKeyID: "test", RuntimeLinkage: "msvc"}, nil
	})))
	if err := service.SyncReleaseTask(context.Background(), "task"); err != nil {
		t.Fatal(err)
	}
	if len(calls) != 0 {
		t.Fatal("historical package was automatically verified")
	}
	if _, err := service.ReverifyReleaseTask(context.Background(), "task", "linux", "operator"); !errors.Is(err, ErrValidation) {
		t.Fatalf("verified package reopened: %v", err)
	}
	if _, err := service.ReverifyReleaseTask(context.Background(), "task", "windows", "operator"); err != nil {
		t.Fatal(err)
	}
	if err := service.SyncReleaseTask(context.Background(), "task"); err != nil {
		t.Fatal(err)
	}
	if len(calls) != 1 || calls[0] != "windows" {
		t.Fatalf("verification calls = %v", calls)
	}
	if *store.details["task"].Artifacts[0].ReleaseArtifactID != releaseID {
		t.Fatal("Linux association changed")
	}
}
