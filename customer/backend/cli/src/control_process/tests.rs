use super::*;

fn expected() -> RuntimeSnapshot {
    let mut value = crate::online_journal::tests::plan().previous_process;
    value.lifecycle.accepting = false;
    value.lifecycle.in_flight = 0;
    value
}

fn running() -> String {
    format!(
        "Id=aster-control@blue.service\nLoadState=loaded\nUnitFileState=enabled\nActiveState=active\nSubState=running\nType=simple\nRestart=on-failure\nInvocationID={}\nMainPID=101\nControlPID=0\nExecMainPID=101\nExecMainCode=0\nExecMainStatus=0\nExecMainExitTimestampMonotonic=0\nResult=success\n",
        "a".repeat(32)
    )
}

fn exited() -> String {
    running()
        .replace("ActiveState=active", "ActiveState=inactive")
        .replace("SubState=running", "SubState=dead")
        .replace("\nMainPID=101", "\nMainPID=0")
        .replace("ExecMainCode=0", "ExecMainCode=1")
        .replace(
            "ExecMainExitTimestampMonotonic=0",
            "ExecMainExitTimestampMonotonic=1234567",
        )
}

#[test]
fn observes_running_stopping_and_normal_exit_of_the_same_activation() {
    assert_eq!(
        classify(&running(), &expected()).unwrap(),
        ProcessProgress::Running
    );
    let stopping = running()
        .replace("ActiveState=active", "ActiveState=deactivating")
        .replace("SubState=running", "SubState=stop-sigterm");
    assert_eq!(
        classify(&stopping, &expected()).unwrap(),
        ProcessProgress::Stopping
    );
    assert_eq!(
        classify(&exited(), &expected()).unwrap(),
        ProcessProgress::Exited
    );
}

#[test]
fn crashes_replacements_and_missing_units_never_count_as_retired() {
    for source in [
        exited().replace("Result=success", "Result=signal"),
        exited().replace("ExecMainCode=1", "ExecMainCode=2"),
        exited().replace("ExecMainStatus=0", "ExecMainStatus=15"),
        exited().replace("ControlPID=0", "ControlPID=456"),
        exited().replace("ExecMainPID=101", "ExecMainPID=456"),
        exited().replace(&"a".repeat(32), &"b".repeat(32)), // PID reuse, new activation.
        exited().replace("LoadState=loaded", "LoadState=not-found"),
        running().replace("UnitFileState=enabled", "UnitFileState=disabled"),
        exited().replace("UnitFileState=enabled", "UnitFileState=disabled"),
        exited().replace("ActiveState=inactive", "ActiveState=failed"),
        exited().replace("Restart=on-failure", "Restart=always"),
        exited().replace(
            "Id=aster-control@blue.service",
            "Id=aster-control@green.service",
        ),
        exited().replace(
            "ExecMainExitTimestampMonotonic=1234567",
            "ExecMainExitTimestampMonotonic=0",
        ),
        running().replace("SubState=running", "SubState=auto-restart"),
    ] {
        assert!(classify(&source, &expected()).is_err(), "accepted {source}");
    }
}

#[test]
fn exit_requires_prior_drained_evidence_and_a_valid_service_binding() {
    let mut snapshot = expected();
    snapshot.lifecycle.in_flight = 1;
    assert!(classify(&exited(), &snapshot).is_err());
    snapshot.lifecycle.in_flight = 0;
    snapshot.lifecycle.accepting = true;
    assert!(classify(&exited(), &snapshot).is_err());
    snapshot.lifecycle.accepting = false;
    snapshot.service = None;
    assert!(classify(&exited(), &snapshot).is_err());
    assert!(
        SystemdControlObserver
            .observe_control(&snapshot, Instant::now())
            .is_err()
    );
    assert!(
        SystemdControlObserver
            .observe_control(&expected(), Instant::now())
            .is_err()
    );
}

#[test]
fn malformed_or_incomplete_properties_are_not_exit_evidence() {
    for source in [
        String::new(),
        exited().replace("MainPID=0\n", ""),
        exited() + "MainPID=0\n",
        exited() + "Unknown=0\n",
        exited().replace("ExecMainPID=101", "ExecMainPID=-1"),
        exited().replace("ExecMainPID=101", "ExecMainPID=18446744073709551616"),
        "x".repeat(16 * 1024 + 1),
    ] {
        assert!(classify(&source, &expected()).is_err());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn systemd_query_child_is_bounded_and_errors_do_not_become_empty_success() {
    use std::{process::Command, time::Duration};
    let command = |script: &str| {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        command
    };
    let deadline = || Instant::now() + Duration::from_secs(3);
    assert_eq!(
        bounded_output(command("printf ok"), deadline()).unwrap(),
        b"ok"
    );
    assert!(bounded_output(command("exit 2"), deadline()).is_err());
    assert!(bounded_output(command("head -c 16385 /dev/zero"), deadline()).is_err());
    let start = Instant::now();
    assert!(bounded_output(command("exec sleep 10"), start + Duration::from_millis(100)).is_err());
    assert!(start.elapsed() < Duration::from_secs(3));
}
