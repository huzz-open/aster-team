//! Read-only systemd proof for the specific Control activation in the journal.
//! This never stops a unit and never treats an inaccessible runtime as exited.
use std::{collections::BTreeMap, time::Instant};

use aster_error_catalog::delivery;
use aster_upgrade_core::runtime::{
    ControlProcessObserver, ProcessProgress, RuntimeSnapshot, ServiceInvocation,
};

use super::CliFailure;

pub(super) mod quiescence;

pub(super) struct SystemdControlObserver;

const PROPERTIES: &[&str] = &[
    "Id",
    "LoadState",
    "UnitFileState",
    "ActiveState",
    "SubState",
    "Type",
    "Restart",
    "InvocationID",
    "MainPID",
    "ControlPID",
    "ExecMainPID",
    "ExecMainCode",
    "ExecMainStatus",
    "ExecMainExitTimestampMonotonic",
    "Result",
];

fn failed() -> CliFailure {
    CliFailure::new(
        delivery::UPGRADE_FAILED,
        "systemd could not prove the pinned Control state; preserve the online transition",
    )
}

impl ControlProcessObserver for SystemdControlObserver {
    type Error = CliFailure;

    fn observe_control(
        &self,
        expected: &RuntimeSnapshot,
        deadline: Instant,
    ) -> Result<ProcessProgress, CliFailure> {
        if !expected.same_process(expected)
            || !expected
                .service
                .as_ref()
                .is_some_and(|service| service.valid())
            || Instant::now() >= deadline
        {
            return Err(failed());
        }
        #[cfg(target_os = "linux")]
        {
            let unit = format!("aster-control@{}.service", expected.slot.id());
            let bytes = show_service(&unit, deadline)?;
            let result = classify(std::str::from_utf8(&bytes).map_err(|_| failed())?, expected)?;
            if Instant::now() >= deadline {
                return Err(failed());
            }
            Ok(result)
        }
        #[cfg(not(target_os = "linux"))]
        Err(failed())
    }
}

fn classify(source: &str, expected: &RuntimeSnapshot) -> Result<ProcessProgress, CliFailure> {
    let identity = expected
        .service
        .as_ref()
        .filter(|service| service.valid())
        .ok_or_else(failed)?;
    let (observed, progress) = service_snapshot(
        source,
        &format!("aster-control@{}.service", expected.slot.id()),
    )?;
    if observed != *identity
        || (progress != ProcessProgress::Running
            && (expected.lifecycle.accepting || expected.lifecycle.in_flight != 0))
    {
        return Err(failed());
    }
    Ok(progress)
}

/// A service name alone is never proof of which activation exited.
/// Keep units enabled until the journal records both exits: systemd may garbage
/// collect an unreferenced disabled unit and erase its activation/exit evidence.
pub(super) fn service_snapshot(
    source: &str,
    unit: &str,
) -> Result<(ServiceInvocation, ProcessProgress), CliFailure> {
    if source.len() > 16 * 1024 {
        return Err(failed());
    }
    let mut values = BTreeMap::new();
    for line in source.lines() {
        let (key, value) = line.split_once('=').ok_or_else(failed)?;
        if !PROPERTIES.contains(&key) || values.insert(key, value).is_some() {
            return Err(failed());
        }
    }
    if values.len() != PROPERTIES.len() {
        return Err(failed());
    }
    let number = |key: &str| -> Result<u64, CliFailure> {
        let value = values.get(key).ok_or_else(failed)?;
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(failed());
        }
        value.parse().map_err(|_| failed())
    };
    let identity = ServiceInvocation {
        process_id: u32::try_from(number("ExecMainPID")?).map_err(|_| failed())?,
        invocation_id: values["InvocationID"].to_owned(),
    };
    if !identity.valid()
        || values["Id"] != unit
        || values["LoadState"] != "loaded"
        || values["UnitFileState"] != "enabled"
        || values["Type"] != "simple"
        || values["Restart"] != "on-failure"
        || values["Result"] != "success"
    {
        return Err(failed());
    }
    let pid = number("MainPID")?;
    let control_pid = number("ControlPID")?;
    let exit_code = number("ExecMainCode")?;
    let exit_status = number("ExecMainStatus")?;
    let exited_at = number("ExecMainExitTimestampMonotonic")?;
    let progress = match (values["ActiveState"], values["SubState"]) {
        ("active", "running")
            if pid == u64::from(identity.process_id)
                && control_pid == 0
                && exit_code == 0
                && exit_status == 0
                && exited_at == 0 =>
        {
            Ok(ProcessProgress::Running)
        }
        ("deactivating", "stop" | "stop-sigterm" | "stop-post" | "final-sigterm")
            if (pid == 0 || pid == u64::from(identity.process_id)) =>
        {
            Ok(ProcessProgress::Stopping)
        }
        ("inactive", "dead")
            if pid == 0
                && control_pid == 0
                && exit_code == 1
                && exit_status == 0
                && exited_at > 0 =>
        {
            Ok(ProcessProgress::Exited)
        }
        _ => Err(failed()),
    }?;
    Ok((identity, progress))
}

#[cfg(target_os = "linux")]
pub(super) fn show_service(unit: &str, deadline: Instant) -> Result<Vec<u8>, CliFailure> {
    let mut command = std::process::Command::new("systemctl");
    command
        .args(["--system", "--no-ask-password", "--no-pager", "show"])
        .arg(format!("--property={}", PROPERTIES.join(",")))
        .arg("--")
        .arg(unit)
        .env("LC_ALL", "C")
        .env("SYSTEMD_COLORS", "0");
    bounded_output(command, deadline)
}

#[cfg(target_os = "linux")]
pub(super) fn bounded_output(
    mut command: std::process::Command,
    deadline: Instant,
) -> Result<Vec<u8>, CliFailure> {
    use std::{io::Read as _, process::Stdio, sync::mpsc, time::Duration};
    let deadline = deadline.min(Instant::now() + Duration::from_secs(6));
    if Instant::now() >= deadline {
        return Err(failed());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            // Only this command child is terminated. A mutation may already have
            // happened; its caller must reconcile durable state before retrying.
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }
    let mut child = Child(command.spawn().map_err(|_| failed())?);
    let output = child.0.stdout.take().ok_or_else(failed)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = output
            .take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(failed)?;
        match child.0.try_wait().map_err(|_| failed())? {
            Some(status) => {
                if !status.success() {
                    return Err(failed());
                }
                let bytes = receiver
                    .recv_timeout(remaining)
                    .map_err(|_| failed())?
                    .map_err(|_| failed())?;
                if bytes.len() > 16 * 1024 || Instant::now() >= deadline {
                    return Err(failed());
                }
                return Ok(bytes);
            }
            None => std::thread::sleep(remaining.min(Duration::from_millis(10))),
        }
    }
}

#[cfg(test)]
mod tests;
