package ports

import (
	"context"

	"aster.local/team/operations/backend/internal/domain"
)

type ReleasePublishResult struct {
	GitHubReleaseID int64
	HTMLURL         string
	TagName         string
}

type ReleasePublisher interface {
	Configured() bool
	Publish(context.Context, domain.ReleaseArtifact) (ReleasePublishResult, error)
}
