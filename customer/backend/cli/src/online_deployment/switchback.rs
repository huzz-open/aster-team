//! Recover a durable reverse operation before the frozen forward executor.
use super::*;
use crate::online_commit::switchback as commit;
use aster_upgrade_core::{
    ReleaseSlot,
    online::switchback::{
        SwitchbackDeployment, SwitchbackJournal, SwitchbackJournalStorage, SwitchbackPhase,
        SwitchbackProgress,
    },
};

struct InstalledSwitchback<'a> {
    layout: &'a InstallLayout,
    store: JournalFile,
    proxy: CaddyClient,
}

impl SwitchbackJournalStorage for InstalledSwitchback<'_> {
    type Error = CliFailure;
    fn load(&self) -> Result<Option<SwitchbackJournal>, CliFailure> {
        SwitchbackJournalStorage::load(&self.store)
    }
    fn assert_current(&mut self, journal: &SwitchbackJournal) -> Result<(), CliFailure> {
        SwitchbackJournalStorage::assert_current(&mut self.store, journal)?;
        commit::load_job(self.layout, journal)?;
        Ok(())
    }
    fn save(
        &mut self,
        previous: Option<&SwitchbackJournal>,
        next: &SwitchbackJournal,
    ) -> Result<(), CliFailure> {
        SwitchbackJournalStorage::save(&mut self.store, previous, next)
    }
}
impl SwitchbackDeployment for InstalledSwitchback<'_> {
    fn observe_control(
        &self,
        drained: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<ProcessProgress, CliFailure> {
        SystemdControlObserver.observe_control(drained, deadline)
    }
    fn traffic(
        &mut self,
        journal: &SwitchbackJournal,
        deadline: Instant,
    ) -> Result<ReleaseSlot, CliFailure> {
        self.assert_current(journal)?;
        let observed = self.proxy.observe_switchback(journal, deadline)?;
        Ok(CaddyClient::snapshot(&observed).slot)
    }
    fn switch_back(
        &mut self,
        journal: &SwitchbackJournal,
        deadline: Instant,
    ) -> Result<(), CliFailure> {
        self.assert_current(journal)?;
        if journal.phase() != SwitchbackPhase::SwitchingBack {
            return Err(failed("reverse cutover has no durable intent"));
        }
        let observed = self.proxy.observe_switchback(journal, deadline)?;
        self.proxy.switch_to(
            observed,
            journal.plan().previous.slot,
            journal.plan().proxy.stream_close_delay_ms,
            deadline,
        )?;
        Ok(())
    }
    fn retire_candidate_runner(
        &mut self,
        journal: &SwitchbackJournal,
        deadline: Instant,
    ) -> Result<ProcessProgress, CliFailure> {
        self.assert_current(journal)?;
        SystemdRunnerHost::retire_switchback(journal, deadline)
    }
    fn commit_previous(
        &mut self,
        journal: &SwitchbackJournal,
        deadline: Instant,
    ) -> Result<(), CliFailure> {
        self.assert_current(journal)?;
        self.traffic(journal, deadline)?;
        commit::commit(self.layout, journal, deadline)
    }
}

pub(super) fn run(
    layout: &InstallLayout,
    store: JournalFile,
    mut journal: SwitchbackJournal,
) -> Result<bool, CliFailure> {
    commit::load_job(layout, &journal)?;
    let mut deployment = InstalledSwitchback {
        layout,
        store,
        proxy: CaddyClient::connect()?,
    };
    if journal.phase() == SwitchbackPhase::Complete {
        deployment.traffic(&journal, Instant::now() + Duration::from_secs(30))?;
        commit::finish(layout, &journal)?;
        deployment.store.archive_switchback(&journal)?;
        return Ok(true);
    }
    let previous = RuntimeClient::new(layout, &journal.plan().original.plan().previous)?;
    let candidate = RuntimeClient::new(layout, &journal.plan().original.plan().candidate)?;
    loop {
        let clock = LinuxUpgradeClock::sample()?;
        match journal
            .advance(
                &previous,
                &candidate,
                &mut deployment,
                &clock,
                Instant::now() + Duration::from_secs(30),
            )
            .map_err(|error| failed(error.to_string()))?
        {
            SwitchbackProgress::Complete => {
                commit::finish(layout, &journal)?;
                deployment.store.archive_switchback(&journal)?;
                return Ok(true);
            }
            SwitchbackProgress::DrainBudgetExhausted => {
                return Err(failed(
                    "candidate drain budget exhausted; preserve both slots and switchback journal",
                ));
            }
            SwitchbackProgress::Draining { .. } | SwitchbackProgress::Retiring => {
                thread::sleep(Duration::from_millis(250))
            }
            SwitchbackProgress::Advanced(_) => {}
        }
    }
}

/// A failed forward step may have durably advanced before losing its reply.
/// Always reload that record; only claim a reversible, candidate-routed task.
pub(super) fn claim(
    deployment: &mut InstalledDeployment<'_>,
) -> Result<Option<SwitchbackJournal>, CliFailure> {
    use aster_upgrade_core::{online::switchback::SwitchbackPlan, runtime::RuntimeRequestBudget};
    let Some(original) = reversible_journal(deployment)? else {
        return Ok(None);
    };
    let clock = LinuxUpgradeClock::sample()?;
    if clock.boot_id != original.plan().clock.boot_id {
        return Err(failed(
            "cannot claim reverse operation across machine boots",
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let observed = deployment.proxy.observe_transition(&original, deadline)?;
    if CaddyClient::snapshot(&observed).slot != original.plan().candidate.slot {
        return Ok(None);
    }
    let previous = deployment.previous.status(deadline)?;
    let candidate = deployment.candidate.status(deadline)?;
    let budget = deployment
        .candidate
        .request_budget_ms(&candidate, deadline)?;
    let plan = SwitchbackPlan {
        original: original.clone(),
        previous: previous.clone(),
        candidate: candidate.clone(),
        candidate_budget: RuntimeRequestBudget {
            runtime: candidate.clone(),
            request_budget_ms: budget,
        },
        proxy: CaddyClient::snapshot(&observed).clone(),
        clock: LinuxUpgradeClock::sample()?,
    };
    if !plan.valid() {
        return Err(failed(
            "observed processes cannot safely claim the original reverse operation",
        ));
    }
    // Prove the old signed release/Runner/model coverage before freezing forward
    // execution. A timeout or missing candidate never becomes drain evidence.
    let _permit = deployment
        .previous
        .readiness(&previous, &plan.previous_readiness(), deadline)?;
    for (slot, expected) in [
        (
            original.plan().previous.slot,
            &original.plan().previous_runner_process,
        ),
        (
            original.plan().candidate.slot,
            &original.plan().candidate_runner_process,
        ),
    ] {
        if SystemdRunnerHost::capture(slot, deadline)? != *expected {
            return Err(failed("Runner activation changed before switchback claim"));
        }
    }
    let same_admission = |now: &RuntimeSnapshot, old: &RuntimeSnapshot| {
        now.same_process(old)
            && now.lifecycle.accepting == old.lifecycle.accepting
            && now.lifecycle.stopping == old.lifecycle.stopping
            && now.lifecycle.revision == old.lifecycle.revision
    };
    if !same_admission(&deployment.previous.status(deadline)?, &previous)
        || !same_admission(&deployment.candidate.status(deadline)?, &candidate)
    {
        return Err(failed("runtime changed during switchback observation"));
    }
    let proxy = deployment.proxy.observe_transition(&original, deadline)?;
    if CaddyClient::snapshot(&proxy) != &plan.proxy || Instant::now() >= deadline {
        return Err(failed("proxy or deadline changed before switchback claim"));
    }
    OnlineJournalStorage::assert_current(deployment, &original)?;
    SwitchbackJournal::create(plan, &mut deployment.store)
        .map(Some)
        .map_err(|error| failed(error.to_string()))
}

fn reversible_journal<S: OnlineJournalStorage<Error = CliFailure>>(
    store: &mut S,
) -> Result<Option<OnlineJournal>, CliFailure> {
    let original = store
        .load()?
        .ok_or_else(|| failed("forward journal disappeared before recovery"))?;
    store.assert_current(&original)?;
    Ok(matches!(
        original.phase(),
        OnlinePhase::SwitchingTraffic
            | OnlinePhase::ClosingPrevious
            | OnlinePhase::DrainingPrevious
            | OnlinePhase::DrainBudgetExhausted
    )
    .then_some(original))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt as _};

    #[test]
    fn lost_forward_reply_uses_durable_phase_and_never_reverses_retirement_or_commit() {
        for phase in [
            OnlinePhase::OpeningCandidate,
            OnlinePhase::SwitchingTraffic,
            OnlinePhase::ClosingPrevious,
            OnlinePhase::DrainingPrevious,
            OnlinePhase::DrainBudgetExhausted,
            OnlinePhase::RetiringPrevious,
            OnlinePhase::Committing,
            OnlinePhase::Complete,
        ] {
            let (_directory, layout) = crate::proxy_disk::tests::fixture();
            let durable =
                crate::proxy_client::tests::transition_journal(&layout, ReleaseSlot::Blue, phase);
            // Emulate a successful checkpoint whose acknowledgement was lost.
            // No old in-memory journal is passed to recovery selection.
            let path = layout.upgrade_state().join("online-transition.json");
            fs::write(&path, serde_json::to_vec(&durable).unwrap()).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let mut store = JournalFile::open(&layout).unwrap();
            let observed = reversible_journal(&mut store).unwrap();
            if matches!(
                phase,
                OnlinePhase::OpeningCandidate
                    | OnlinePhase::RetiringPrevious
                    | OnlinePhase::Committing
                    | OnlinePhase::Complete
            ) {
                assert!(observed.is_none());
            } else {
                assert_eq!(observed, Some(durable));
            }
        }
    }

    #[test]
    fn another_reverse_owner_or_corrupt_checkpoint_prevents_any_new_claim() {
        for corrupt_forward in [false, true] {
            let (_directory, layout) = crate::proxy_disk::tests::fixture();
            let durable = crate::proxy_client::tests::transition_journal(
                &layout,
                ReleaseSlot::Blue,
                OnlinePhase::SwitchingTraffic,
            );
            let path = layout.upgrade_state().join("online-transition.json");
            fs::write(
                &path,
                if corrupt_forward {
                    b"{".to_vec()
                } else {
                    serde_json::to_vec(&durable).unwrap()
                },
            )
            .unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            if !corrupt_forward {
                let reverse = layout.upgrade_state().join("online-switchback.json");
                fs::write(&reverse, b"{").unwrap();
                fs::set_permissions(reverse, fs::Permissions::from_mode(0o600)).unwrap();
            }
            let before = fs::read(&path).unwrap();
            let mut store = JournalFile::open(&layout).unwrap();
            assert!(reversible_journal(&mut store).is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }
}
