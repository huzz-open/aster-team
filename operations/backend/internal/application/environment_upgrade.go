package application

import (
	"context"
	"crypto/x509"
	"fmt"
	"net"
	"net/url"
	"slices"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

const PermissionEnvironmentWrite = "release.environment.write"
const PermissionEnvironmentUpgrade = "release.environment.upgrade"

type EnvironmentUpgradeStore interface {
	RotateUpgradeCredentials(context.Context, string, string, string) error
	CreateUpgradeEnvironment(context.Context, domain.UpgradeEnvironment, string, string) error
	ListUpgradeEnvironments(context.Context) ([]domain.UpgradeEnvironment, error)
	GetUpgradeEnvironment(context.Context, string) (domain.UpgradeEnvironment, string, error)
	CreateEnvironmentUpgrade(context.Context, domain.EnvironmentUpgrade) error
	ListEnvironmentUpgrades(context.Context) ([]domain.EnvironmentUpgrade, error)
	GetEnvironmentUpgrade(context.Context, string, int64) (domain.EnvironmentUpgradeDetail, error)
	ClaimEnvironmentUpgrade(context.Context, string) (domain.EnvironmentUpgrade, bool, error)
	RenewEnvironmentUpgrade(context.Context, string, string) error
	SaveEnvironmentUpgrade(context.Context, domain.EnvironmentUpgrade, string, []domain.UpgradeProbeSample, bool) error
}

func (s *Service) RotateUpgradeCredentials(ctx context.Context, id string, credentials domain.UpgradeCredentials, actor string) error {
	if err := s.upgradesConfigured(); err != nil {
		return err
	}
	if err := s.RequirePermission(ctx, actor, PermissionEnvironmentWrite); err != nil {
		return err
	}
	for _, value := range []string{credentials.AdminEmail, credentials.AdminPassword, credentials.APIKey} {
		if len(value) == 0 || len(value) > 4096 {
			return ErrValidation
		}
	}
	if _, _, err := s.environmentUpgrades.GetUpgradeEnvironment(ctx, id); err != nil {
		return err
	}
	sealed, err := s.upgradeSecrets.Seal(id, credentials)
	if err != nil {
		return err
	}
	return s.environmentUpgrades.RotateUpgradeCredentials(ctx, id, sealed, actor)
}

func WithEnvironmentUpgrades(store EnvironmentUpgradeStore, secrets ports.UpgradeSecretStore, targets ports.UpgradeTargetFactory) Option {
	return func(s *Service) {
		s.environmentUpgrades = store
		s.upgradeSecrets = secrets
		s.upgradeTargets = targets
	}
}

func (s *Service) upgradesConfigured() error {
	if s.environmentUpgrades == nil || s.upgradeSecrets == nil || s.upgradeTargets == nil {
		return fmt.Errorf("%w: environment upgrades are not configured", ErrBusinessStoreUnavailable)
	}
	return nil
}

func normalizeTargetOrigin(value string) (string, error) {
	u, err := url.Parse(strings.TrimSpace(value))
	if err != nil || u.User != nil || u.Host == "" || u.RawQuery != "" || u.Fragment != "" || (u.Path != "" && u.Path != "/") {
		return "", ErrValidation
	}
	loopback := u.Hostname() == "localhost"
	if ip := net.ParseIP(u.Hostname()); ip != nil {
		loopback = ip.IsLoopback()
	}
	if u.Scheme != "https" && !(u.Scheme == "http" && loopback) {
		return "", fmt.Errorf("%w: target requires HTTPS (HTTP is limited to loopback)", ErrValidation)
	}
	u.Path = ""
	return u.String(), nil
}

func (s *Service) CreateUpgradeEnvironment(ctx context.Context, input domain.UpgradeEnvironmentInput, actor string) (domain.UpgradeEnvironment, error) {
	var env domain.UpgradeEnvironment
	if err := s.upgradesConfigured(); err != nil {
		return env, err
	}
	if err := s.RequirePermission(ctx, actor, PermissionEnvironmentWrite); err != nil {
		return env, err
	}
	if len(strings.TrimSpace(input.Name)) < 1 || len(input.Name) > 100 || !identifierPattern(input.InstallationID) || len(input.InstallationID) > 128 || input.Model == "" || len(input.Model) > 128 || input.Credentials.AdminEmail == "" || input.Credentials.AdminPassword == "" || input.Credentials.APIKey == "" || len(input.CAPEM) > 65536 {
		return env, ErrValidation
	}
	for _, secret := range []string{input.Credentials.AdminEmail, input.Credentials.AdminPassword, input.Credentials.APIKey} {
		if len(secret) > 4096 {
			return env, ErrValidation
		}
	}
	if input.CAPEM != "" && !x509.NewCertPool().AppendCertsFromPEM([]byte(input.CAPEM)) {
		return env, ErrValidation
	}
	var err error
	env = domain.UpgradeEnvironment{ID: newID("env", s.now()), Name: strings.TrimSpace(input.Name), InstallationID: input.InstallationID, CAPEM: input.CAPEM, Model: input.Model, CredentialVersion: 1, CreatedAt: s.now().UTC()}
	if env.AdminURL, err = normalizeTargetOrigin(input.AdminURL); err != nil {
		return env, err
	}
	if env.MemberURL, err = normalizeTargetOrigin(input.MemberURL); err != nil {
		return env, err
	}
	if env.APIURL, err = normalizeTargetOrigin(input.APIURL); err != nil {
		return env, err
	}
	sealed, err := s.upgradeSecrets.Seal(env.ID, input.Credentials)
	if err != nil {
		return env, err
	}
	err = s.environmentUpgrades.CreateUpgradeEnvironment(ctx, env, sealed, actor)
	return env, err
}

func (s *Service) ListUpgradeEnvironments(ctx context.Context) ([]domain.UpgradeEnvironment, error) {
	if err := s.upgradesConfigured(); err != nil {
		return nil, err
	}
	return s.environmentUpgrades.ListUpgradeEnvironments(ctx)
}

func (s *Service) connectUpgradeTarget(ctx context.Context, id string) (domain.UpgradeEnvironment, ports.UpgradeTarget, error) {
	env, sealed, err := s.environmentUpgrades.GetUpgradeEnvironment(ctx, id)
	if err != nil {
		return env, nil, err
	}
	credentials, err := s.upgradeSecrets.Open(env.ID, sealed)
	if err != nil {
		return env, nil, err
	}
	target, err := s.upgradeTargets.Connect(env, credentials)
	return env, target, err
}

func validateUpgradeTarget(env domain.UpgradeEnvironment, status domain.TargetMaintenance) error {
	if status.InstallationID != env.InstallationID {
		return fmt.Errorf("%w: target installation identity mismatch", ErrValidation)
	}
	if !status.CorrelatedUpgrades {
		return fmt.Errorf("%w: target requires the correlated maintenance API bridge version", ErrValidation)
	}
	if !slices.Contains(status.Capabilities.SupportedModes, "maintenance") {
		return fmt.Errorf("%w: target does not support maintenance upgrades", ErrValidation)
	}
	return nil
}

func (s *Service) InspectUpgradeEnvironment(ctx context.Context, id string) (domain.TargetMaintenance, error) {
	if err := s.upgradesConfigured(); err != nil {
		return domain.TargetMaintenance{}, err
	}
	env, target, err := s.connectUpgradeTarget(ctx, id)
	if err != nil {
		return domain.TargetMaintenance{}, err
	}
	defer target.Close()
	status, err := target.Status(ctx, "")
	if err == nil {
		err = validateUpgradeTarget(env, status)
	}
	// Target task details are deliberately excluded from environment inspection.
	status.Jobs = nil
	return status, err
}

func (s *Service) CreateEnvironmentUpgrade(ctx context.Context, envID, artifactID, actor string) (domain.EnvironmentUpgrade, error) {
	var task domain.EnvironmentUpgrade
	if err := s.upgradesConfigured(); err != nil {
		return task, err
	}
	if err := s.RequirePermission(ctx, actor, PermissionEnvironmentUpgrade); err != nil {
		return task, err
	}
	if s.business == nil || s.artifacts == nil {
		return task, ErrBusinessStoreUnavailable
	}
	artifact, err := s.business.GetReleaseArtifact(ctx, artifactID)
	if err != nil {
		return task, err
	}
	if artifact.RuntimeLinkage == "unverified" || artifact.RuntimeLinkage == "" || !strings.HasPrefix(artifact.SignatureRef, "release-key:") || !sha256Pattern(artifact.SHA256) || artifact.GitHubRunID == nil || artifact.SizeBytes < 1 || artifact.SizeBytes > 1<<30 {
		return task, fmt.Errorf("%w: select an independently verified artifact up to 1 GiB", ErrValidation)
	}
	status, err := s.InspectUpgradeEnvironment(ctx, envID)
	if err != nil {
		return task, err
	}
	if !upgradeArtifactMatches(artifact, status) {
		return task, fmt.Errorf("%w: target busy or artifact platform/version mismatch", ErrValidation)
	}

	now := s.now().UTC()
	task = domain.EnvironmentUpgrade{Summary: []domain.UpgradeProbeSummary{}, ID: newID("upgrade", now), EnvironmentID: envID, ArtifactID: artifactID, ArtifactSHA256: artifact.SHA256, TargetVersion: artifact.Version, CreatedBy: actor, Mode: "maintenance", Phase: "queued", UpgradeResult: "pending", ContinuityResult: "not_applicable", RecoveryResult: "pending", CreatedAt: now, UpdatedAt: now, ProbeUntil: now.Add(30 * time.Minute)}
	task.TargetJobID = strings.ReplaceAll(task.ID, ".", "_")
	err = s.environmentUpgrades.CreateEnvironmentUpgrade(ctx, task)
	return task, err
}

func (s *Service) ListEnvironmentUpgrades(ctx context.Context) ([]domain.EnvironmentUpgrade, error) {
	if err := s.upgradesConfigured(); err != nil {
		return nil, err
	}
	return s.environmentUpgrades.ListEnvironmentUpgrades(ctx)
}

func (s *Service) GetEnvironmentUpgrade(ctx context.Context, id string, after int64) (domain.EnvironmentUpgradeDetail, error) {
	if err := s.upgradesConfigured(); err != nil {
		return domain.EnvironmentUpgradeDetail{}, err
	}
	return s.environmentUpgrades.GetEnvironmentUpgrade(ctx, id, after)
}

func upgradeArtifactMatches(artifact domain.ReleaseArtifact, status domain.TargetMaintenance) bool {
	architecture := status.Architecture
	if architecture == "x86_64" {
		architecture = "amd64"
	}
	if architecture == "aarch64" {
		architecture = "arm64"
	}
	return !status.Busy && artifact.Platform == status.Platform && artifact.Architecture == architecture && domain.IsNewerSemanticVersion(artifact.Version, status.CurrentVersion)
}
