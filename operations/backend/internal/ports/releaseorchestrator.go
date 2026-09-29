package ports

import (
	"context"
	"errors"
	"io"
	"time"

	"aster.local/team/operations/backend/internal/domain"
)

var (
	ErrReleaseArtifactPending = errors.New("release artifact is not ready")
	ErrReleaseArtifactInvalid = errors.New("release artifact verification failed")
)

type ReleasePreflight struct {
	CommitSHA string
}

type ReleaseDispatch struct {
	RunID     int64
	RunNumber *int64
	RunURL    string
	Status    string
	CreatedAt time.Time
}

type ReleaseDispatchInput struct {
	TaskID              string
	Version             string
	SourceRef           string
	SourceCommitSHA     string
	FreeDistributionID  string
	FreeLicenseSHA256   string
	FreeLicenseDocument []byte
}

type ReleaseSnapshot struct {
	Run       domain.ReleaseRun
	Artifacts []domain.ReleaseTaskArtifact
}

type VerifiedReleaseArtifact struct {
	ObjectKey          string
	SHA256             string
	SizeBytes          int64
	ManifestSHA256     string
	SignatureKeyID     string
	RuntimeLinkage     string
	SourceArtifactID   string
	GitHubDigestSHA256 string
}

type ReleaseArtifactVerificationError struct {
	Code string
	Err  error
}

func (releaseError *ReleaseArtifactVerificationError) Error() string {
	if releaseError == nil || releaseError.Err == nil {
		return "release artifact verification failed"
	}
	return releaseError.Err.Error()
}

func (releaseError *ReleaseArtifactVerificationError) Unwrap() error {
	if releaseError == nil {
		return ErrReleaseArtifactInvalid
	}
	return errors.Join(ErrReleaseArtifactInvalid, releaseError.Err)
}

type ReleaseArtifactSource interface {
	DownloadArtifact(context.Context, int64) (io.ReadCloser, error)
}

type ReleaseArtifactVerifier interface {
	Verify(context.Context, domain.ReleaseTask, domain.ReleaseTaskArtifact) (VerifiedReleaseArtifact, error)
}

type ReleaseOrchestrator interface {
	Capabilities() domain.ReleaseCapabilities
	Preflight(context.Context, string, string) (ReleasePreflight, error)
	Dispatch(context.Context, ReleaseDispatchInput) (ReleaseDispatch, error)
	Snapshot(context.Context, domain.ReleaseTask, int64) (ReleaseSnapshot, error)
}
