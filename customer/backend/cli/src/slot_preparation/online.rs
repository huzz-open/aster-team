//! Durable online intent around the shared credential preparation, while the
//! old slot continues serving. Candidate startup preserves closed admission.
use super::{CliFailure, InstallLayout, Path, ReleaseSlot, failed, read_regular};
use crate::{
    online_journal::JournalFile, proxy_client::CaddyClient, runner_process::SystemdRunnerHost,
    runtime_client::RuntimeClient, upgrade_clock::LinuxUpgradeClock,
};
use aster_upgrade_core::{
    ACTIVE_SLOT_RUNTIME_SCHEMA, ActiveLocalRunner, ActiveReleaseSlot, MaintenanceJob,
    migration::{MigrationInspection, MigrationPreflight},
    online::OnlineJournalStorage,
    preparation::{CandidateStartup, PreparationIntent, PreparationJournal},
    runtime::{SlotProxy, SlotRuntime, UpgradeClockSource},
};
use std::time::{Duration, Instant};

mod material;
mod staging;
mod startup;

pub(crate) fn reconcile_startup(
    layout: &InstallLayout,
    store: &mut JournalFile,
    journal: PreparationJournal,
) -> Result<PreparationJournal, CliFailure> {
    startup::run(layout, store, journal)
}

pub(crate) fn capture_startup_for_cancellation(
    layout: &InstallLayout,
    store: &JournalFile,
    journal: &PreparationJournal,
    deadline: Instant,
) -> Result<PreparationJournal, CliFailure> {
    startup::capture_for_cancellation(layout, store, journal, deadline)
}

pub(crate) fn capture_partial_for_cancellation(
    layout: &InstallLayout,
    journal: &PreparationJournal,
    deadline: Instant,
) -> Result<aster_upgrade_core::preparation::PartialCandidateActivation, CliFailure> {
    startup::capture_partial(layout, journal, deadline)
}

pub(super) struct Preparation {
    store: JournalFile,
    journal: PreparationJournal,
}

/// Called before the shared preparation path can rewrite any slot material.
pub(super) fn resume(
    layout: &InstallLayout,
    release: &Path,
    slot: ReleaseSlot,
    actor_id: &str,
    job_id: &str,
) -> Result<Option<ActiveLocalRunner>, CliFailure> {
    let mut store = JournalFile::open(layout)?;
    let Some(journal) = store.load_preparation()? else {
        return Ok(None);
    };
    if journal.abort_phase().is_some() {
        return Err(failed("preparation cancellation owns this task"));
    }
    if journal.startup().is_none() {
        return Ok(None);
    }
    let intent = journal.intent();
    if intent.job.id != job_id
        || intent.job.requested_by != actor_id
        || intent.job.candidate_release.as_deref() != Some(release)
        || journal
            .candidate()
            .is_none_or(|candidate| candidate.slot != slot)
    {
        return Err(failed(
            "candidate startup resume does not match its original task",
        ));
    }
    let started = startup::run(layout, &mut store, journal)?;
    Ok(started
        .candidate()
        .and_then(|candidate| candidate.local_runner.clone()))
}

pub(super) fn begin(
    layout: &InstallLayout,
    release: &Path,
    slot: ReleaseSlot,
    actor_id: &str,
    job_id: &str,
    previous: &ActiveReleaseSlot,
) -> Result<Preparation, CliFailure> {
    let mut store = JournalFile::open(layout)?;
    if store.load()?.is_some() {
        return Err(failed(
            "an online transition already owns this installation",
        ));
    }
    let job: MaintenanceJob = serde_json::from_slice(&read_regular(
        &layout.upgrade_running().join(format!("{job_id}.json")),
        256 * 1024,
    )?)
    .map_err(|_| failed("online preparation job is invalid"))?;
    if job.id != job_id
        || job.requested_by != actor_id
        || job.candidate_release.as_deref() != Some(release)
        || job.previous_release.as_ref() != Some(&layout.release(&previous.version))
        || slot != previous.slot.other()
    {
        return Err(failed(
            "online preparation no longer belongs to the expected task",
        ));
    }
    let candidate_release = crate::verify_release_at(release)?;
    let previous_release = crate::verify_release_at(&layout.release(&previous.version))?;
    if job.target_version.as_deref() != Some(candidate_release.claims().version.as_str())
        || previous_release.claims().version != previous.version
        || layout.release(&candidate_release.claims().version) != release
    {
        return Err(failed(
            "online preparation versions differ from their signed releases",
        ));
    }
    super::require_online_support(release)?;
    super::require_online_support(&layout.release(&previous.version))?;
    let previous_digest =
        crate::sha256_file(&layout.release(&previous.version).join("RELEASE.json"))?;
    if previous
        .local_runner
        .as_ref()
        .is_none_or(|runner| runner.manifest_sha256 != previous_digest)
    {
        return Err(failed(
            "previous runtime does not match its signed manifest",
        ));
    }
    let saved = store.load_preparation()?;
    if saved
        .as_ref()
        .is_some_and(|record| record.abort_phase().is_some())
    {
        return Err(failed("preparation cancellation owns this task"));
    }
    if saved
        .as_ref()
        .is_some_and(|record| record.startup().is_some())
    {
        return Err(failed(
            "candidate startup already owns prepared material; resume without rewriting credentials",
        ));
    }
    let candidate_migrations = inspect_migrations(layout, release)?;
    let migrations = match &saved {
        Some(saved) => {
            if !saved
                .intent()
                .migrations
                .accepts_progress(&candidate_migrations)
            {
                return Err(failed(
                    "candidate migration history changed during preparation",
                ));
            }
            saved.intent().migrations.clone()
        }
        None => {
            let evidence = MigrationPreflight {
                current: inspect_migrations(layout, &layout.release(&previous.version))?,
                candidate: candidate_migrations,
            };
            if !evidence.valid() {
                return Err(failed("current and candidate migration reports disagree"));
            }
            evidence
        }
    };
    let runtime = RuntimeClient::new(layout, previous)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let previous_process = runtime.status(deadline)?;
    let previous_runner_process = SystemdRunnerHost::capture(previous.slot, deadline)?;
    let proxy = CaddyClient::connect()?;
    let observation = proxy.observe(deadline)?;
    let intent = PreparationIntent {
        migrations,
        job,
        previous: previous.clone(),
        previous_process,
        previous_runner_process,
        candidate_manifest_sha256: crate::sha256_file(&release.join("RELEASE.json"))?,
        proxy: CaddyClient::snapshot(&observation).clone(),
        clock: LinuxUpgradeClock::sample()?,
    };
    if !intent.valid() {
        return Err(failed("online preparation observations are inconsistent"));
    }
    let journal = if let Some(saved) = saved {
        if !saved.intent().matches_job(&intent.job)
            || saved.intent().previous != intent.previous
            || saved.intent().candidate_manifest_sha256 != intent.candidate_manifest_sha256
            || saved.intent().proxy != intent.proxy
            || !saved.intent().observes_previous(
                &intent.previous_process,
                &intent.previous_runner_process,
                &intent.clock,
            )
        {
            return Err(failed(
                "online preparation identity changed; preserve the existing record",
            ));
        }
        saved
    } else {
        let next = PreparationJournal::create(intent)
            .ok_or_else(|| failed("invalid online preparation"))?;
        store.save_preparation(None, &next)?;
        next
    };
    // The sidecar and outer installation locks remain held throughout staging.
    super::assert_candidate_stopped(slot, crate::service_state)?;
    staging::stage(layout, &store, &journal)?;
    Ok(Preparation { store, journal })
}

impl Preparation {
    pub(super) fn finish(
        mut self,
        layout: &InstallLayout,
        runner: ActiveLocalRunner,
    ) -> Result<(), CliFailure> {
        let candidate = ActiveReleaseSlot {
            schema: ACTIVE_SLOT_RUNTIME_SCHEMA.into(),
            slot: self.journal.intent().previous.slot.other(),
            version: self
                .journal
                .intent()
                .job
                .target_version
                .clone()
                .ok_or_else(|| failed("online preparation has no candidate version"))?,
            local_runner: Some(runner),
        };
        let next = self
            .journal
            .provisioned(candidate.clone())
            .ok_or_else(|| failed("candidate identity differs from its durable preparation"))?;
        self.store.save_preparation(Some(&self.journal), &next)?;
        let material_sha256 = material::fingerprint(layout, candidate.slot)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let previous = &next.intent().previous;
        let runtime = RuntimeClient::new(layout, previous)?.status(deadline)?;
        let runner = SystemdRunnerHost::capture(previous.slot, deadline)?;
        let proxy = CaddyClient::connect()?.observe(deadline)?;
        let clock = LinuxUpgradeClock::sample()?;
        if !next.intent().observes_previous(&runtime, &runner, &clock)
            || CaddyClient::snapshot(&proxy) != &next.intent().proxy
        {
            return Err(failed(
                "old runtime or proxy changed while preparing the candidate",
            ));
        }
        let starting = next
            .starting(CandidateStartup {
                material_sha256,
                clock,
            })
            .ok_or_else(|| failed("candidate startup intent is inconsistent"))?;
        self.store.save_preparation(Some(&next), &starting)?;
        startup::run(layout, &mut self.store, starting)?;
        Ok(())
    }
}

fn inspect_migrations(
    layout: &InstallLayout,
    release: &Path,
) -> Result<MigrationInspection, CliFailure> {
    let mut command = super::control_command(layout, release);
    command.arg("inspect-database-migrations");
    let bytes =
        crate::control_process::bounded_output(command, Instant::now() + Duration::from_secs(6))
            .map_err(|_| failed("signed Control read-only migration inspection failed"))?;
    let report: MigrationInspection = serde_json::from_slice(&bytes)
        .map_err(|_| failed("signed Control migration report cannot be decoded"))?;
    if !report.valid() {
        return Err(failed("signed Control migration report is inconsistent"));
    }
    Ok(report)
}
