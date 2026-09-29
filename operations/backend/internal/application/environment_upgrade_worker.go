package application

import (
	"context"
	"time"

	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
)

// RunEnvironmentUpgrades is independent of browser requests. A renewable,
// fenced database lease owns each task; losing it cancels all network work.
func (s *Service) RunEnvironmentUpgrades(ctx context.Context) {
	if s.upgradesConfigured() != nil {
		return
	}
	for ctx.Err() == nil {
		owner := newID("worker", s.now())
		task, claimed, err := s.environmentUpgrades.ClaimEnvironmentUpgrade(ctx, owner)
		if err == nil && claimed {
			s.runEnvironmentUpgrade(ctx, task, owner)
			continue
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(3 * time.Second):
		}
	}
}

type upgradeSchedule struct{ tick, baseline, observation, health, page, model time.Duration }

var defaultUpgradeSchedule = upgradeSchedule{time.Second, 30 * time.Second, 60 * time.Second, time.Second, 2 * time.Second, 5 * time.Second}

func (s *Service) runEnvironmentUpgrade(parent context.Context, task domain.EnvironmentUpgrade, owner string) {
	s.runEnvironmentUpgradeWithSchedule(parent, task, owner, defaultUpgradeSchedule)
}

func (s *Service) runEnvironmentUpgradeWithSchedule(parent context.Context, task domain.EnvironmentUpgrade, owner string, schedule upgradeSchedule) {
	ctx, cancel := context.WithCancel(parent)
	defer cancel()
	heartbeatDone := make(chan struct{})
	go func() {
		defer close(heartbeatDone)
		ticker := time.NewTicker(10 * time.Second)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				check, stop := context.WithTimeout(ctx, 5*time.Second)
				err := s.environmentUpgrades.RenewEnvironmentUpgrade(check, task.ID, owner)
				stop()
				if err != nil {
					cancel()
					return
				}
			}
		}
	}()
	defer func() { cancel(); <-heartbeatDone }()
	save := func(samples []domain.UpgradeProbeSample, terminal bool) bool {
		task.UpdatedAt = s.now().UTC()
		if s.environmentUpgrades.SaveEnvironmentUpgrade(ctx, task, owner, samples, terminal) != nil {
			cancel()
			return false
		}
		return true
	}
	env, target, err := s.connectUpgradeTarget(ctx, task.EnvironmentID)
	if err != nil {
		task.ErrorCode = "credentials_unavailable"
		task.CoverageGap = true
		save(nil, false)
		return
	}
	defer func() { target.Close() }()
	if task.Phase == "queued" {
		status, statusErr := target.Status(ctx, "")
		if statusErr != nil || validateUpgradeTarget(env, status) != nil || status.Busy {
			task.Phase = "completed"
			task.UpgradeResult = "failed"
			task.ErrorCode = "target_preflight_failed"
			task.RecoveryResult = "not_started"
			save(nil, true)
			return
		}
		task.Phase = "baseline"
		task.ProbeUntil = s.now().Add(30 * time.Minute)
		task.BaselineUntil = s.now().Add(schedule.baseline)
		if !save(nil, false) {
			return
		}
	}
	samples := make(chan domain.UpgradeProbeSample, 10)
	inflight := map[string]bool{}
	next := map[string]time.Time{}
	baselineKinds := map[string]bool{}
	postKinds := map[string]bool{}
	if task.CoverageGap && task.Phase == "baseline" {
		task.BaselineUntil = s.now().Add(schedule.baseline)
		task.BaselineFailed = false
	}
	if task.CoverageGap && task.Phase == "observing" {
		task.ObserveUntil = s.now().Add(schedule.observation)
	}
	nextCredentialCheck := s.now().Add(time.Minute)
	type statusResult struct {
		status domain.TargetMaintenance
		err    error
	}
	statusResults := make(chan statusResult, 1)
	statusPending := false
	nextStatus := time.Time{}
	var uploadResult <-chan error
	ticker := time.NewTicker(schedule.tick)
	defer ticker.Stop()
	for ctx.Err() == nil {
		select {
		case <-ctx.Done():
			return
		case sample := <-samples:
			recordUpgradeSample(&task, sample)
			inflight[sample.Kind] = false
			if sample.Phase == "baseline" {
				baselineKinds[sample.Kind] = true
				if !sample.OK {
					task.BaselineFailed = true
				}
			}
			if sample.Phase == "observing" {
				postKinds[sample.Kind] = true
				if !sample.OK {
					task.PostFailed = true
				}
			}
			if !save([]domain.UpgradeProbeSample{sample}, false) {
				return
			}
		case result := <-statusResults:
			statusPending = false
			status := result.status
			if result.err == nil && status.InstallationID == env.InstallationID {
				for _, job := range status.Jobs {
					if job.ID != task.TargetJobID {
						continue
					}
					if (job.Status == "succeeded" || job.Status == "failed") && !status.Busy {
						task.UpgradeResult = "failed"
						if job.Status == "succeeded" && job.TargetVersion == task.TargetVersion && status.CurrentVersion == task.TargetVersion {
							task.UpgradeResult = "succeeded"
						}
						task.Phase = "observing"
						task.ObserveUntil = s.now().Add(schedule.observation)
						task.ErrorCode = ""
						if !save(nil, false) {
							return
						}
					}
				}
			}
			if task.Phase != "observing" && !s.now().Before(task.ProbeUntil) {
				task.CoverageGap = true
				task.UpgradeResult = "unknown"
				task.ErrorCode = "observation_budget_exhausted"
				save(nil, false)
				return
			}
		case uploadErr := <-uploadResult:
			uploadResult = nil
			task.Phase = "tracking"
			if uploadErr != nil {
				task.ErrorCode = "upload_response_unconfirmed"
				task.UpgradeResult = "unknown"
			}
			if !save(nil, false) {
				return
			}
		case <-ticker.C:
			now := s.now()
			pending := false
			for _, active := range inflight {
				pending = pending || active
			}
			if uploadResult == nil && !statusPending && !pending && !now.Before(nextCredentialCheck) {
				latest, _, readErr := s.environmentUpgrades.GetUpgradeEnvironment(ctx, env.ID)
				nextCredentialCheck = now.Add(time.Minute)
				if readErr == nil && latest.CredentialVersion != env.CredentialVersion {
					refreshedEnv, refreshedTarget, refreshErr := s.connectUpgradeTarget(ctx, env.ID)
					if refreshErr == nil {
						target.Close()
						env, target = refreshedEnv, refreshedTarget
					}
				}
			}
			baselineEnded := task.Phase == "baseline" && !now.Before(task.BaselineUntil)
			observationEnded := task.Phase == "observing" && !now.Before(task.ObserveUntil)
			if baselineEnded && !pending {
				if task.BaselineFailed || len(baselineKinds) != 5 {
					task.Phase = "completed"
					task.UpgradeResult = "failed"
					task.ErrorCode = "baseline_failed_or_incomplete"
					task.RecoveryResult = "not_started"
					save(nil, true)
					return
				}
				status, statusErr := target.Status(ctx, "")
				if statusErr != nil || validateUpgradeTarget(env, status) != nil || status.Busy {
					task.Phase = "completed"
					task.UpgradeResult = "failed"
					task.ErrorCode = "target_preflight_failed"
					task.RecoveryResult = "not_started"
					save(nil, true)
					return
				}
				artifact, reader, openErr := s.OpenReleaseArtifact(ctx, task.ArtifactID)
				if openErr != nil || artifact.SHA256 != task.ArtifactSHA256 || !upgradeArtifactMatches(artifact, status) {
					if reader != nil {
						reader.Close()
					}
					task.Phase = "completed"
					task.UpgradeResult = "failed"
					task.ErrorCode = "artifact_unavailable"
					task.RecoveryResult = "not_started"
					save(nil, true)
					return
				}
				// Persist before sending any bytes. A resumed worker only queries
				// this ID; it must never blindly retransmit an uncertain upload.
				task.Phase = "uploading"
				if !save(nil, false) {
					reader.Close()
					return
				}
				result := make(chan error, 1)
				uploadResult = result
				go func() {
					defer reader.Close()
					result <- target.Upload(ctx, task.TargetJobID, task.ArtifactSHA256, reader)
				}()
			}
			if observationEnded && !pending {
				task.Phase = "completed"
				task.RecoveryResult = "recovered"
				if task.PostFailed {
					task.RecoveryResult = "failed"
				} else if len(postKinds) != 5 {
					task.RecoveryResult = "insufficient_coverage"
				}
				if task.CoverageGap && task.RecoveryResult == "recovered" {
					task.RecoveryResult = "recovered_with_observation_gap"
				}
				save(nil, true)
				return
			}
			if (task.Phase == "tracking" || task.Phase == "uploading") && uploadResult == nil && !statusPending && !now.Before(nextStatus) {
				statusPending = true
				nextStatus = now.Add(2 * time.Second)
				go func(current ports.UpgradeTarget) {
					status, statusErr := current.Status(ctx, task.TargetJobID)
					select {
					case statusResults <- statusResult{status, statusErr}:
					case <-ctx.Done():
					}
				}(target)
			}
			if !now.Before(task.ProbeUntil) {
				if task.Phase == "baseline" {
					task.Phase = "completed"
					task.UpgradeResult = "failed"
					task.RecoveryResult = "not_started"
					task.ErrorCode = "baseline_budget_exhausted"
					task.CoverageGap = true
					save(nil, true)
					return
				}
				task.CoverageGap = true
				if task.Phase != "observing" {
					task.UpgradeResult = "unknown"
					task.ErrorCode = "observation_budget_exhausted"
				}
				if !save(nil, false) {
					return
				}
				// Keep the target locked, but let this worker service other
				// environments before the next leased status reconciliation.
				if task.Phase != "observing" && uploadResult == nil && !statusPending {
					return
				}
				continue
			}
			if baselineEnded || observationEnded {
				continue
			}
			for _, kind := range []string{"health", "admin", "member", "model", "stream"} {
				if inflight[kind] || now.Before(next[kind]) {
					continue
				}
				interval := schedule.health
				if kind == "admin" || kind == "member" {
					interval = schedule.page
				}
				if kind == "model" || kind == "stream" {
					interval = schedule.model
					if task.ModelRequests >= 120 {
						task.CoverageGap = true
						continue
					}
					task.ModelRequests++
					if !save(nil, false) {
						return
					}
				}
				inflight[kind] = true
				next[kind] = now.Add(interval)
				phase := task.Phase
				go func(kind, phase string) {
					sample := target.Probe(ctx, kind)
					sample.Phase = phase
					select {
					case samples <- sample:
					case <-ctx.Done():
					}
				}(kind, phase)
			}
		}
	}
}
