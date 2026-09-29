//! Installed Linux recovery composes the actual runtime, Caddy and process
//! adapters. Starting a new online plan remains gated by staging compatibility.
use super::{
    CliFailure, control_process::SystemdControlObserver, online_journal::JournalFile,
    proxy_client::CaddyClient, runner_process::SystemdRunnerHost, runtime_client::RuntimeClient,
    upgrade_clock::LinuxUpgradeClock,
};
use aster_install_layout::InstallLayout;
use aster_upgrade_core::{
    online::{OnlineDeployment, OnlineJournal, OnlineJournalStorage, OnlinePhase, OnlineProgress},
    runtime::{
        ControlProcessObserver, ProcessProgress, RuntimeSnapshot, SlotProxy, SlotRuntime,
        UpgradeClockSource,
    },
};
use std::{
    thread,
    time::{Duration, Instant},
};

mod preparation_abort;
mod prepared;
#[cfg(test)]
mod protocol_tests;
mod switchback;

pub(super) fn run_prepared(layout: &InstallLayout) -> Result<bool, CliFailure> {
    if preparation_abort::resume(layout)? {
        return Ok(true);
    }
    match prepared::resume(layout) {
        Ok(false) => return Ok(false),
        Ok(true) => {}
        Err(error) => {
            if preparation_abort::claim(layout)? {
                return preparation_abort::resume(layout);
            }
            return Err(error);
        }
    }
    // The complete transition is durable before this releases/reacquires the
    // journal lock. The caller still owns the installation maintenance lock.
    run_existing(layout)
}

fn failed(message: impl Into<String>) -> CliFailure {
    CliFailure::new(aster_error_catalog::delivery::UPGRADE_FAILED, message)
}

struct InstalledDeployment<'a> {
    layout: &'a InstallLayout,
    store: JournalFile,
    proxy: CaddyClient,
    previous: &'a RuntimeClient,
    candidate: &'a RuntimeClient,
}

impl OnlineJournalStorage for InstalledDeployment<'_> {
    type Error = CliFailure;
    fn load(&self) -> Result<Option<OnlineJournal>, Self::Error> {
        self.store.load()
    }
    fn assert_current(&mut self, journal: &OnlineJournal) -> Result<(), Self::Error> {
        self.store.assert_current(journal)?;
        super::online_commit::load_job(self.layout, journal, false)?;
        Ok(())
    }
    fn save(
        &mut self,
        previous: Option<&OnlineJournal>,
        next: &OnlineJournal,
    ) -> Result<(), Self::Error> {
        self.store.save(previous, next)
    }
}
impl OnlineDeployment for InstalledDeployment<'_> {
    fn traffic(&mut self, deadline: Instant) -> Result<RuntimeSnapshot, Self::Error> {
        let journal = self
            .load()?
            .ok_or_else(|| failed("online journal is missing"))?;
        let observed = self.proxy.observe_transition(&journal, deadline)?;
        let slot = CaddyClient::snapshot(&observed).slot;
        let (runtime, expected) = if slot == journal.plan().previous.slot {
            (self.previous, &journal.plan().previous_process)
        } else {
            (self.candidate, &journal.plan().candidate_process)
        };
        let state = runtime.status(deadline)?;
        if !state.same_process(expected) {
            return Err(failed("proxy targets a replacement Control"));
        }
        Ok(state)
    }
    fn switch_to(
        &mut self,
        candidate: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<(), Self::Error> {
        let journal = self
            .load()?
            .ok_or_else(|| failed("online journal is missing"))?;
        self.assert_current(&journal)?;
        if journal.phase() != OnlinePhase::SwitchingTraffic
            || !candidate.same_process(&journal.plan().candidate_process)
        {
            return Err(failed(
                "cutover does not match the durable switching intent",
            ));
        }
        let observed = self.proxy.observe_transition(&journal, deadline)?;
        self.proxy.switch_to(
            observed,
            candidate.slot,
            journal.plan().proxy.stream_close_delay_ms,
            deadline,
        )?;
        Ok(())
    }
    fn observe_control(
        &self,
        drained: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<ProcessProgress, Self::Error> {
        SystemdControlObserver.observe_control(drained, deadline)
    }
    fn retire(
        &mut self,
        journal: &OnlineJournal,
        deadline: Instant,
    ) -> Result<ProcessProgress, Self::Error> {
        self.assert_current(journal)?;
        // Do not disable either old unit here: this would erase systemd's exit
        // evidence before the core durably checkpoints Committing.
        SystemdRunnerHost::retire(journal, deadline)
    }
    fn commit(&mut self, journal: &OnlineJournal, deadline: Instant) -> Result<(), Self::Error> {
        self.assert_current(journal)?;
        if journal.phase() != OnlinePhase::Committing {
            return Err(failed("online exits are not durably committed"));
        }
        self.traffic(deadline)?;
        super::online_commit::commit(self.layout, journal, deadline)
    }
}

/// Called under the existing exclusive installation maintenance lock, before
/// maintenance can interpret or stop either slot. Errors retain the journal.
pub(super) fn run_existing(layout: &InstallLayout) -> Result<bool, CliFailure> {
    if !JournalFile::pending(layout)? {
        return Ok(false);
    }
    let mut store = JournalFile::open(layout)?;
    if let Some(journal) =
        aster_upgrade_core::online::switchback::SwitchbackJournalStorage::load(&store)?
    {
        return switchback::run(layout, store, journal);
    }
    let mut journal = store
        .load()?
        .ok_or_else(|| failed("online journal disappeared"))?;
    super::online_commit::load_job(layout, &journal, journal.phase() == OnlinePhase::Complete)?;
    if journal.phase() == OnlinePhase::Complete {
        store.archive_preparation(&journal)?;
        super::online_commit::finish(layout, &journal)?;
        store.archive(&journal)?;
        return Ok(true);
    }
    let protocol = require_forward_protocol(layout, &journal);
    let clock = LinuxUpgradeClock::sample()?;
    if clock.boot_id != journal.plan().clock.boot_id {
        return Err(failed(
            "machine boot changed; preserve both online slots for recovery",
        ));
    }
    let previous = RuntimeClient::new(layout, &journal.plan().previous)?;
    let candidate = RuntimeClient::new(layout, &journal.plan().candidate)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    if SystemdRunnerHost::capture(journal.plan().candidate.slot, deadline)?
        != journal.plan().candidate_runner_process
    {
        return Err(failed(
            "candidate Runner activation changed during online recovery",
        ));
    }
    let mut deployment = InstalledDeployment {
        layout,
        store,
        proxy: CaddyClient::connect()?,
        previous: &previous,
        candidate: &candidate,
    };
    // A persisted forward journal can bypass preparation after a CLI update.
    // Frozen releases, not the current pointer (which may already have switched),
    // determine compatibility. Reverse recovery and final archival remain usable.
    if let Err(error) = protocol {
        if let Some(reverse) = switchback::claim(&mut deployment)? {
            return switchback::run(layout, deployment.store, reverse);
        }
        return Err(error);
    }
    deployment.store.archive_preparation(&journal)?;
    loop {
        let clock = LinuxUpgradeClock::sample()?;
        let progress = match journal.advance(
            &previous,
            &candidate,
            &mut deployment,
            &clock,
            Instant::now() + Duration::from_secs(30),
        ) {
            Ok(progress) => progress,
            Err(error) => {
                if let Some(reverse) = switchback::claim(&mut deployment)? {
                    return switchback::run(layout, deployment.store, reverse);
                }
                return Err(failed(error.to_string()));
            }
        };
        match progress {
            OnlineProgress::Complete => {
                super::online_commit::finish(layout, &journal)?;
                deployment.store.archive(&journal)?;
                return Ok(true);
            }
            OnlineProgress::DrainBudgetExhausted => {
                if let Some(reverse) = switchback::claim(&mut deployment)? {
                    return switchback::run(layout, deployment.store, reverse);
                }
                return Err(failed(
                    "online drain budget exhausted; keep both slots and the recovery journal",
                ));
            }
            OnlineProgress::Draining { .. } | OnlineProgress::RetiringPrevious => {
                thread::sleep(Duration::from_millis(250))
            }
            OnlineProgress::Advanced(_) => {}
        }
    }
}

fn require_forward_protocol(
    layout: &InstallLayout,
    journal: &OnlineJournal,
) -> Result<(), CliFailure> {
    require_forward_protocol_with_keys(layout, journal, &crate::compiled_release_keys()?)
}

fn require_forward_protocol_with_keys(
    layout: &InstallLayout,
    journal: &OnlineJournal,
    keys: &aster_release_core::TrustedReleaseKeys,
) -> Result<(), CliFailure> {
    if !journal.valid() {
        return Err(failed("online recovery journal is invalid"));
    }
    for slot in [&journal.plan().previous, &journal.plan().candidate] {
        let release = layout.release(&slot.version);
        let signed = crate::verify_release_at_with_keys(&release, keys)?;
        if signed.claims().version != slot.version
            || signed.claims().platform != "linux"
            || signed.claims().architecture != "amd64"
            || slot.local_runner.as_ref().is_none_or(|runner| {
                crate::sha256_file(&release.join("RELEASE.json"))
                    .ok()
                    .as_ref()
                    != Some(&runner.manifest_sha256)
            })
        {
            return Err(failed(
                "forward recovery signed release differs from its frozen plan",
            ));
        }
        // Committing follows durable retirement of both old processes. Existing
        // commit checks still revalidate their exits; no coexistence is created.
        // Keep frozen release authentication even for this terminal recovery.
        if !matches!(
            journal.phase(),
            OnlinePhase::Committing | OnlinePhase::Complete
        ) {
            crate::slot_preparation::require_settlement_support(&release)?;
        }
    }
    Ok(())
}
