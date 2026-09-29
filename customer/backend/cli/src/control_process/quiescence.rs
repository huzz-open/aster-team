//! Inactivity is distinct from the normal exit of a pinned activation. This
//! proof is suitable before startup or after exits have already been persisted;
//! it must never replace ControlProcessObserver during request retirement.
use super::*;

const PROPERTIES: &[&str] = &[
    "Id",
    "LoadState",
    "UnitFileState",
    "ActiveState",
    "SubState",
    "Type",
    "Restart",
    "KillMode",
    "MainPID",
    "ControlPID",
    "ControlGroup",
    "TasksCurrent",
    "Job",
];

/// Require the full service group to be absent and no systemd job pending.
/// MainPID=0 alone also occurs during activation and automatic restart.
pub(crate) fn classify(source: &str, unit: &str) -> Result<String, CliFailure> {
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
    if values.len() != PROPERTIES.len()
        || values["Id"] != unit
        || values["LoadState"] != "loaded"
        || !matches!(values["UnitFileState"], "enabled" | "disabled")
        || values["Type"] != "simple"
        || values["Restart"] != "on-failure"
        || values["KillMode"] != "control-group"
        || values["MainPID"] != "0"
        || values["ControlPID"] != "0"
        || !values["Job"].is_empty()
        || !values["ControlGroup"].is_empty()
        || !matches!(values["TasksCurrent"], "0" | "[not set]")
        || !matches!(
            (values["ActiveState"], values["SubState"]),
            ("inactive", "dead") | ("failed", "failed")
        )
    {
        return Err(failed());
    }
    Ok(values["ActiveState"].to_owned())
}

#[cfg(target_os = "linux")]
pub(crate) fn observe(unit: &str, deadline: Instant) -> Result<String, CliFailure> {
    let mut command = std::process::Command::new("systemctl");
    command
        .args(["--system", "--no-ask-password", "--no-pager", "show"])
        .arg(format!("--property={}", PROPERTIES.join(",")))
        .arg("--")
        .arg(unit)
        .env("LC_ALL", "C")
        .env("SYSTEMD_COLORS", "0");
    let bytes = bounded_output(command, deadline)?;
    let state = classify(std::str::from_utf8(&bytes).map_err(|_| failed())?, unit)?;
    if Instant::now() >= deadline {
        return Err(failed());
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Values observed from the supported Ubuntu 20.04 systemd image after a
    // simple service exits: its cgroup is released and TasksCurrent is unset.
    fn quiescent() -> String {
        "Id=aster-control@green.service\nLoadState=loaded\nUnitFileState=enabled\nActiveState=inactive\nSubState=dead\nType=simple\nRestart=on-failure\nKillMode=control-group\nMainPID=0\nControlPID=0\nControlGroup=\nTasksCurrent=[not set]\nJob=\n".into()
    }
    const UNIT: &str = "aster-control@green.service";

    #[test]
    fn inactive_loaded_services_may_be_enabled_disabled_or_collected_then_loaded() {
        for enabled in ["enabled", "disabled"] {
            for tasks in ["0", "[not set]"] {
                let source = quiescent()
                    .replace("UnitFileState=enabled", &format!("UnitFileState={enabled}"))
                    .replace("TasksCurrent=[not set]", &format!("TasksCurrent={tasks}"));
                assert_eq!(classify(&source, UNIT).unwrap(), "inactive");
            }
        }
        let stopped_failure = quiescent()
            .replace("ActiveState=inactive", "ActiveState=failed")
            .replace("SubState=dead", "SubState=failed");
        assert_eq!(classify(&stopped_failure, UNIT).unwrap(), "failed");
    }

    #[test]
    fn queued_start_automatic_restart_and_remaining_children_are_not_quiescent() {
        for (from, to) in [
            ("Job=", "Job=125"),
            ("MainPID=0", "MainPID=123"),
            ("ControlPID=0", "ControlPID=124"),
            (
                "ControlGroup=",
                "ControlGroup=/system.slice/aster-control@green.service",
            ),
            ("TasksCurrent=[not set]", "TasksCurrent=1"),
            ("ActiveState=inactive", "ActiveState=activating"),
            ("SubState=dead", "SubState=auto-restart"),
            ("KillMode=control-group", "KillMode=process"),
            ("Restart=on-failure", "Restart=always"),
        ] {
            assert!(
                classify(&quiescent().replace(from, to), UNIT).is_err(),
                "accepted {to}"
            );
        }
        let auto_restart = quiescent()
            .replace("ActiveState=inactive", "ActiveState=activating")
            .replace("SubState=dead", "SubState=auto-restart");
        assert!(classify(&auto_restart, UNIT).is_err());
    }

    #[test]
    fn incomplete_unknown_or_mismatched_service_observations_fail_closed() {
        for source in [
            quiescent().replace("LoadState=loaded", "LoadState=not-found"),
            quiescent().replace("UnitFileState=enabled", "UnitFileState=masked"),
            quiescent().replace("Type=simple", "Type=forking"),
            quiescent().replace("TasksCurrent=[not set]", "TasksCurrent=unknown"),
            quiescent().replace("Job=\n", ""),
            format!("{}Job=\n", quiescent()),
            format!("{}Unknown=x\n", quiescent()),
            "x".repeat(16 * 1024 + 1),
            quiescent().replace("MainPID=0", "MainPID=00"),
        ] {
            assert!(classify(&source, UNIT).is_err());
        }
        assert!(classify(&quiescent(), "aster-runner@green.service").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires an isolated systemd Docker container; see upgrade verification design"]
    fn real_systemd_preserves_exit_identity_and_rejects_pending_or_restarting_units() {
        use std::{fs, io::Write as _, process::Command, thread, time::Duration};
        assert_eq!(
            fs::read_to_string("/run/systemd/container").unwrap().trim(),
            "docker",
            "this test must run in the disposable systemd container"
        );
        struct Probe {
            unit: String,
            path: std::path::PathBuf,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                let _ = Command::new("systemctl")
                    .args(["stop", &self.unit])
                    .output();
                let _ = Command::new("systemctl")
                    .args(["disable", &self.unit])
                    .output();
                let _ = fs::remove_file(&self.path);
                let _ = Command::new("systemctl").arg("daemon-reload").output();
                let _ = Command::new("systemctl")
                    .args(["reset-failed", &self.unit])
                    .output();
            }
        }
        let run = |args: &[&str]| {
            let output = Command::new("systemctl").args(args).output().unwrap();
            assert!(
                output.status.success(),
                "systemctl {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        let deadline = || Instant::now() + Duration::from_secs(5);
        let create = |suffix: &str, service: &str| {
            let unit = format!("aster-quiescence-{}-{suffix}.service", std::process::id());
            let path = std::path::Path::new("/etc/systemd/system").join(&unit);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            let probe = Probe { unit, path };
            write!(file, "[Unit]\nStartLimitIntervalSec=60\nStartLimitBurst=1\n[Service]\nType=simple\nRestart=on-failure\nRestartSec=2s\nKillMode=control-group\n{service}\n[Install]\nWantedBy=multi-user.target\n").unwrap();
            file.sync_all().unwrap();
            run(&["daemon-reload"]);
            run(&["enable", &probe.unit]);
            probe
        };
        let wait_state = |unit: &str, state: &str| {
            let end = Instant::now() + Duration::from_secs(10);
            loop {
                if observe(unit, deadline()).is_ok_and(|observed| observed == state) {
                    break;
                }
                assert!(Instant::now() < end, "unit never became {state}: {unit}");
                thread::sleep(Duration::from_millis(30));
            }
        };
        let normal = create("normal", "ExecStart=/bin/sleep 2");
        assert_eq!(observe(&normal.unit, deadline()).unwrap(), "inactive");
        run(&["start", &normal.unit]);
        let running = super::super::show_service(&normal.unit, deadline()).unwrap();
        let (identity, progress) =
            super::super::service_snapshot(std::str::from_utf8(&running).unwrap(), &normal.unit)
                .unwrap();
        assert_eq!(progress, ProcessProgress::Running);
        assert!(observe(&normal.unit, deadline()).is_err());
        wait_state(&normal.unit, "inactive");
        let exited = super::super::show_service(&normal.unit, deadline()).unwrap();
        assert_eq!(
            super::super::service_snapshot(std::str::from_utf8(&exited).unwrap(), &normal.unit)
                .unwrap(),
            (identity, ProcessProgress::Exited)
        );
        run(&["disable", &normal.unit]);
        assert_eq!(observe(&normal.unit, deadline()).unwrap(), "inactive");
        let queued = create(
            "queued",
            "ExecStartPre=/bin/sleep 10\nExecStart=/bin/sleep 2",
        );
        run(&["--no-block", "start", &queued.unit]);
        assert!(observe(&queued.unit, deadline()).is_err());
        let crashing = create("crash", "ExecStart=/bin/false");
        run(&["--no-block", "start", &crashing.unit]);
        assert!(observe(&crashing.unit, deadline()).is_err());
        wait_state(&crashing.unit, "failed");
        assert_eq!(observe(&crashing.unit, deadline()).unwrap(), "failed");
    }
}
