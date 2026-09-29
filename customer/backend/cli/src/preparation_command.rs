//! Own the entire short-lived preparation command group, including runuser's
//! child. Never use this for a systemd business service or the old Runner.
use super::CliFailure;
use rustix::process::{
    Pid, Signal, WaitId, WaitIdOptions, WaitIdStatus, kill_process_group, waitid,
};
use std::{
    io,
    os::unix::process::CommandExt as _,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

struct ChildGroup {
    child: Child,
    reaped: bool,
}

impl ChildGroup {
    fn observe(&self) -> io::Result<Option<WaitIdStatus>> {
        loop {
            match waitid(
                WaitId::Pid(Pid::from_child(&self.child)),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            ) {
                Err(rustix::io::Errno::INTR) => continue,
                result => return result.map_err(Into::into),
            }
        }
    }

    fn finish(&mut self) -> io::Result<ExitStatus> {
        // WNOWAIT retains ownership of the leader PID even after it exits.
        // Kill remaining group members before reaping; otherwise a reused PID
        // could name a different process group. An externally reaped child is
        // not authority to signal a numerical PID or group.
        self.observe()?;
        match kill_process_group(Pid::from_child(&self.child), Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => {}
            Err(error) => return Err(error.into()),
        }
        let result = self.child.wait();
        if result.is_ok() {
            self.reaped = true;
        }
        result
    }
}

impl Drop for ChildGroup {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.finish();
        }
    }
}

/// Preserve task and preparation evidence on every error. Cancellation of the
/// helper never proves that its database transaction did not already commit.
pub(super) fn run(mut command: Command, deadline: Instant, label: &str) -> Result<(), CliFailure> {
    let fail = |detail: &str| {
        CliFailure::new(
            aster_error_catalog::delivery::UPGRADE_FAILED,
            format!("{label}: {detail}; preserve preparation evidence before retrying"),
        )
    };
    if Instant::now() >= deadline {
        return Err(fail("preparation command deadline elapsed"));
    }
    command.process_group(0).stdin(Stdio::null());
    let mut child = ChildGroup {
        child: command.spawn().map_err(|error| fail(&error.to_string()))?,
        reaped: false,
    };
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| fail("preparation command deadline elapsed"))?;
        if child
            .observe()
            .map_err(|error| fail(&error.to_string()))?
            .is_some()
        {
            let status = child.finish().map_err(|error| fail(&error.to_string()))?;
            if !status.success() {
                return Err(fail(&format!("command exited with {status}")));
            }
            if Instant::now() >= deadline {
                return Err(fail("preparation command deadline elapsed"));
            }
            return Ok(());
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

#[cfg(test)]
mod tests;
