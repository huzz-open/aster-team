use super::*;
use std::{fs, os::unix::fs::MetadataExt as _, process::Command};

mod units;

pub(super) struct Installed<'a> {
    pub(super) layout: &'a InstallLayout,
    pub(super) forward: bool,
}

impl Host for Installed<'_> {
    fn clock(&mut self) -> Result<UpgradeClock, CliFailure> {
        LinuxUpgradeClock::sample()
    }

    fn verify(
        &mut self,
        journal: &PreparationJournal,
        deadline: Instant,
    ) -> Result<String, CliFailure> {
        let layout = self.layout;
        let intent = journal.intent();
        let candidate = journal
            .candidate()
            .ok_or_else(|| failed("candidate is not provisioned"))?;
        validate_binding(layout, journal)?;
        if self.forward {
            for slot in [&intent.previous, candidate] {
                crate::slot_preparation::require_settlement_support(
                    &layout.release(&slot.version),
                )?;
            }
        }
        units::verify(layout, candidate, deadline)?;
        let material = super::super::material::fingerprint(layout, candidate.slot)?;
        let runtime = RuntimeClient::new(layout, &intent.previous)?.status(deadline)?;
        let runner = SystemdRunnerHost::capture(intent.previous.slot, deadline)?;
        let proxy = CaddyClient::connect()?.observe(deadline)?;
        let clock = LinuxUpgradeClock::sample()?;
        if !intent.observes_previous(&runtime, &runner, &clock)
            || CaddyClient::snapshot(&proxy) != &intent.proxy
        {
            return Err(failed(
                "previous runtime or proxy changed during candidate startup",
            ));
        }
        Ok(material)
    }

    fn dispatch(
        &mut self,
        slot: ReleaseSlot,
        action: Action,
        deadline: Instant,
    ) -> Result<(), CliFailure> {
        let mut command = Command::new("systemctl");
        command.args(["--system", "--no-ask-password", "--no-pager"]);
        match action {
            Action::Enable => {
                command.arg("enable");
            }
            Action::Start => {
                command.args(["--no-block", "start"]);
            }
        }
        command.arg("--").args(units::names(slot));
        command.env("LC_ALL", "C").env("SYSTEMD_COLORS", "0");
        crate::preparation_command::run(
            command,
            deadline.min(Instant::now() + Duration::from_secs(6)),
            "candidate service startup",
        )
    }

    fn capture(
        &mut self,
        journal: &PreparationJournal,
        deadline: Instant,
    ) -> Result<Option<CandidateActivation>, CliFailure> {
        let candidate = journal
            .candidate()
            .ok_or_else(|| failed("candidate is not provisioned"))?;
        let client = RuntimeClient::new(self.layout, candidate)?;
        let (Ok(control), Ok(runner)) = (
            client.status(deadline),
            SystemdRunnerHost::capture(candidate.slot, deadline),
        ) else {
            return Ok(None);
        };
        if crate::control_process::SystemdControlObserver.observe_control(&control, deadline)?
            != ProcessProgress::Running
        {
            return Err(failed("candidate Control activation is not running"));
        }
        Ok(Some(CandidateActivation { control, runner }))
    }

    fn capture_partial(
        &mut self,
        journal: &PreparationJournal,
        deadline: Instant,
    ) -> Result<PartialCandidateActivation, CliFailure> {
        let candidate = journal
            .candidate()
            .ok_or_else(|| failed("candidate is not provisioned"))?;
        let control = match RuntimeClient::new(self.layout, candidate)?.status(deadline) {
            Ok(control) => {
                if crate::control_process::SystemdControlObserver
                    .observe_control(&control, deadline)?
                    != ProcessProgress::Running
                {
                    return Err(failed("partial candidate Control is not running"));
                }
                Some(control)
            }
            Err(_) => {
                crate::control_process::quiescence::observe(
                    &format!("aster-control@{}.service", candidate.slot.id()),
                    deadline,
                )?;
                None
            }
        };
        let runner = match SystemdRunnerHost::capture(candidate.slot, deadline) {
            Ok(runner) => Some(runner),
            Err(_) => {
                crate::control_process::quiescence::observe(
                    &format!("aster-runner@{}.service", candidate.slot.id()),
                    deadline,
                )?;
                None
            }
        };
        if control.is_some() && runner.is_some() {
            return Err(failed(
                "candidate completed startup during partial observation",
            ));
        }
        Ok(PartialCandidateActivation { control, runner })
    }

    fn pause(&mut self, deadline: Instant) {
        std::thread::sleep(
            Duration::from_millis(250).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn validate_binding(
    layout: &InstallLayout,
    journal: &PreparationJournal,
) -> Result<(), CliFailure> {
    let intent = journal.intent();
    let candidate = journal
        .candidate()
        .ok_or_else(|| failed("candidate is not provisioned"))?;
    let path = layout
        .upgrade_running()
        .join(format!("{}.json", intent.job.id));
    super::super::super::safe_owned_path(layout, &path)?;
    let job: MaintenanceJob = serde_json::from_slice(&read_regular(&path, 256 * 1024)?)
        .map_err(|_| failed("candidate startup job is invalid"))?;
    if !intent.matches_job(&job)
        || job.status != aster_upgrade_core::MaintenanceStatus::StartingCandidate
        || job.previous_release.as_ref() != Some(&layout.release(&intent.previous.version))
        || job.candidate_release.as_ref() != Some(&layout.release(&candidate.version))
    {
        return Err(failed("candidate startup belongs to a different task"));
    }
    let active: ActiveReleaseSlot =
        serde_json::from_slice(&read_regular(&layout.active_slot(), 8192)?)
            .map_err(|_| failed("active slot metadata is invalid"))?;
    if active != intent.previous {
        return Err(failed("active slot changed during candidate startup"));
    }
    for binding in [&intent.previous, candidate] {
        let release = layout.release(&binding.version);
        let signed = crate::verify_release_at(&release)?;
        if signed.claims().version != binding.version
            || signed.claims().platform != "linux"
            || signed.claims().architecture != "amd64"
            || binding.local_runner.as_ref().is_none_or(|runner| {
                crate::sha256_file(&release.join("RELEASE.json"))
                    .ok()
                    .as_ref()
                    != Some(&runner.manifest_sha256)
            })
        {
            return Err(failed("signed release changed during candidate startup"));
        }
        let link = layout.slot_release(binding.slot.id());
        protected_parent(layout, &link)?;
        if !fs::symlink_metadata(&link)
            .map_err(|e| failed(e.to_string()))?
            .file_type()
            .is_symlink()
            || fs::canonicalize(&link).map_err(|e| failed(e.to_string()))?
                != fs::canonicalize(&release).map_err(|e| failed(e.to_string()))?
        {
            return Err(failed(
                "slot release pointer changed during candidate startup",
            ));
        }
    }
    if fs::canonicalize(layout.current()).map_err(|e| failed(e.to_string()))?
        != fs::canonicalize(layout.release(&intent.previous.version))
            .map_err(|e| failed(e.to_string()))?
    {
        return Err(failed("current release changed during candidate startup"));
    }
    Ok(())
}

fn protected_parent(layout: &InstallLayout, path: &Path) -> Result<(), CliFailure> {
    if !path.starts_with(layout.root()) {
        return Err(failed("startup path escaped the installation"));
    }
    for parent in path
        .parent()
        .ok_or_else(|| failed("startup path has no parent"))?
        .ancestors()
    {
        let metadata = fs::symlink_metadata(parent).map_err(|e| failed(e.to_string()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.mode() & 0o022 != 0 {
            return Err(failed("startup path has an unsafe parent"));
        }
        if parent == layout.root() {
            return Ok(());
        }
    }
    Err(failed("startup path is outside the installation"))
}
