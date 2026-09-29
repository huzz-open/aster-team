package ports

import (
	"aster.local/team/operations/backend/internal/domain"
	"context"
	"io"
)

type UpgradeSecretStore interface {
	Seal(string, domain.UpgradeCredentials) (string, error)
	Open(string, string) (domain.UpgradeCredentials, error)
}

type UpgradeTarget interface {
	Status(context.Context, string) (domain.TargetMaintenance, error)
	Upload(context.Context, string, string, io.Reader) error
	Probe(context.Context, string) domain.UpgradeProbeSample
	Close()
}

type UpgradeTargetFactory interface {
	Connect(domain.UpgradeEnvironment, domain.UpgradeCredentials) (UpgradeTarget, error)
}
