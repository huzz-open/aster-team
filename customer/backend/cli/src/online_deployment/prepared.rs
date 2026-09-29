//! Reconcile preparation before ordinary maintenance can touch either slot.
//! No new task is admitted here; the durable preparation owns the same job.
use super::*;
use aster_upgrade_core::{
    online::OnlinePlan,
    preparation::{CandidateStartup, PreparationJournal},
    runtime::ReadinessExpectation,
};

pub(super) fn resume(layout: &InstallLayout) -> Result<bool, CliFailure> {
    let initial = {
        let store = JournalFile::open(layout)?;
        if store.load()?.is_some() {
            return Err(failed(
                "online transition appeared before preparation recovery",
            ));
        }
        let Some(initial) = store.load_preparation()? else {
            return Ok(false);
        };
        initial
    };
    let intent = initial.intent();
    let release = intent
        .job
        .candidate_release
        .as_ref()
        .ok_or_else(|| failed("prepared task has no candidate path"))?;
    let runner = crate::slot_preparation::prepare_if_supported(
        layout,
        release,
        intent.previous.slot.other(),
        &intent.job.requested_by,
        &intent.job.id,
    )?
    .ok_or_else(|| failed("prepared online candidate cannot resume on this installation"))?;
    let mut store = JournalFile::open(layout)?;
    let prepared = store
        .load_preparation()?
        .ok_or_else(|| failed("candidate preparation disappeared during recovery"))?;
    if prepared.intent() != initial.intent()
        || prepared
            .candidate()
            .and_then(|candidate| candidate.local_runner.as_ref())
            != Some(&runner)
    {
        return Err(failed(
            "recovered candidate does not belong to the original preparation",
        ));
    }
    let prepared =
        crate::slot_preparation::online::reconcile_startup(layout, &mut store, prepared)?;
    let deadline = remaining_deadline(&prepared, &LinuxUpgradeClock::sample()?)?;
    let plan = observe_plan(layout, &prepared, deadline)?;
    // Refresh actual signed material, unit definitions and all pinned process
    // identities after the potentially slow runtime inspection, before writing.
    let checked =
        crate::slot_preparation::online::reconcile_startup(layout, &mut store, prepared.clone())?;
    if checked != prepared || Instant::now() >= deadline {
        return Err(failed(
            "preparation changed or expired while constructing cutover",
        ));
    }
    handoff(&mut store, &prepared, plan)?;
    Ok(true)
}

fn remaining_deadline(
    prepared: &PreparationJournal,
    clock: &aster_upgrade_core::runtime::UpgradeClock,
) -> Result<Instant, CliFailure> {
    let startup = prepared
        .startup()
        .ok_or_else(|| failed("candidate has no startup intent"))?;
    if !startup.accepts_observation(&startup.material_sha256, clock) {
        return Err(failed(
            "prepared candidate original startup budget is exhausted",
        ));
    }
    let remaining = startup
        .clock
        .uptime_ms
        .checked_add(CandidateStartup::BUDGET_MS)
        .and_then(|end| end.checked_sub(clock.uptime_ms))
        .ok_or_else(|| failed("prepared startup clock overflowed"))?;
    Instant::now()
        .checked_add(Duration::from_millis(remaining))
        .ok_or_else(|| failed("prepared startup deadline overflowed"))
}

fn observe_plan(
    layout: &InstallLayout,
    prepared: &PreparationJournal,
    deadline: Instant,
) -> Result<OnlinePlan, CliFailure> {
    let intent = prepared.intent();
    let candidate_binding = prepared
        .candidate()
        .ok_or_else(|| failed("candidate identity is missing"))?;
    let previous = RuntimeClient::new(layout, &intent.previous)?;
    let candidate = RuntimeClient::new(layout, candidate_binding)?;
    let previous_process = previous.status(deadline)?;
    let candidate_process = candidate.status(deadline)?;
    let drain_budget_ms = previous.request_budget_ms(&previous_process, deadline)?;
    let previous_models = previous.configured_models(&previous_process, deadline)?;
    let models = candidate.configured_models(&candidate_process, deadline)?;
    if models != previous_models {
        return Err(failed(
            "candidate and previous Control disagree on configured models",
        ));
    }
    let runner = candidate_binding
        .local_runner
        .as_ref()
        .ok_or_else(|| failed("candidate local Runner is missing"))?;
    let proxy = CaddyClient::connect()?.observe(deadline)?;
    let plan = OnlinePlan {
        job_id: intent.job.id.clone(),
        previous: intent.previous.clone(),
        candidate: candidate_binding.clone(),
        previous_process,
        candidate_process,
        previous_runner_process: SystemdRunnerHost::capture(intent.previous.slot, deadline)?,
        candidate_runner_process: SystemdRunnerHost::capture(candidate_binding.slot, deadline)?,
        readiness: ReadinessExpectation {
            manifest_sha256: runner.manifest_sha256.clone(),
            models,
            runner_ids: vec![runner.runner_id.clone()],
        },
        proxy: CaddyClient::snapshot(&proxy).clone(),
        clock: LinuxUpgradeClock::sample()?,
        drain_budget_ms,
    };
    if Instant::now() >= deadline || !prepared.permits_handoff(&plan) {
        return Err(failed(
            "live state cannot authorize the prepared online handoff",
        ));
    }
    Ok(plan)
}

fn handoff(
    store: &mut JournalFile,
    prepared: &PreparationJournal,
    plan: OnlinePlan,
) -> Result<OnlineJournal, CliFailure> {
    if store.load()?.is_some()
        || store.load_preparation()?.as_ref() != Some(prepared)
        || !prepared.permits_handoff(&plan)
    {
        return Err(failed(
            "prepared online handoff lost ownership or original bindings",
        ));
    }
    let journal = OnlineJournal::create(plan, store).map_err(|error| failed(error.to_string()))?;
    // On archival failure, preserve both records. run_existing will reconcile
    // the already-created transition; never recreate or erase its intent.
    store.archive_preparation(&journal)?;
    Ok(journal)
}

#[cfg(test)]
mod tests;
