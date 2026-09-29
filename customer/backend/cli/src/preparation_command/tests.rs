use super::*;
use std::{fs, path::Path};

fn shell(script: &str, root: &Path) -> Command {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script, "preparation-test"]).arg(root);
    command
}

fn not_running(path: &Path) {
    let pid = fs::read_to_string(path).unwrap();
    let status = fs::read_to_string(format!("/proc/{}/status", pid.trim()));
    if let Ok(status) = status {
        assert!(
            status
                .lines()
                .any(|line| line.starts_with("State:") && line.contains('Z')),
            "owned descendant is still running: {status}"
        );
    }
}

#[test]
fn deadline_before_spawn_and_failures_never_report_success() {
    let root = tempfile::tempdir().unwrap();
    let deadline = || Instant::now() + Duration::from_secs(3);
    assert!(
        run(
            shell("touch \"$1/started\"", root.path()),
            Instant::now(),
            "expired"
        )
        .is_err()
    );
    assert!(!root.path().join("started").exists());
    run(shell("exit 0", root.path()), deadline(), "success").unwrap();
    assert!(run(shell("exit 7", root.path()), deadline(), "nonzero").is_err());
    assert!(run(shell("kill -TERM $$", root.path()), deadline(), "signal").is_err());
    assert!(
        run(
            Command::new(root.path().join("absent")),
            deadline(),
            "missing"
        )
        .is_err()
    );
}

#[test]
fn timeout_terminates_descendants_without_touching_an_unrelated_child() {
    let root = tempfile::tempdir().unwrap();
    struct Sentinel(Child);
    impl Drop for Sentinel {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut sentinel = Sentinel(Command::new("sleep").arg("10").spawn().unwrap());
    let start = Instant::now();
    let result = run(
        shell(
            "echo $$ > \"$1/parent\"; (sleep 1; touch \"$1/late\") & echo $! > \"$1/child\"; wait",
            root.path(),
        ),
        start + Duration::from_millis(200),
        "timeout",
    );
    assert!(result.is_err());
    assert!(start.elapsed() < Duration::from_secs(3));
    let parent = fs::read_to_string(root.path().join("parent")).unwrap();
    assert!(
        !Path::new(&format!("/proc/{}", parent.trim())).exists(),
        "leader was not reaped"
    );
    std::thread::sleep(Duration::from_millis(1200));
    not_running(&root.path().join("child"));
    assert!(!root.path().join("late").exists());
    assert!(sentinel.0.try_wait().unwrap().is_none());
}

#[test]
fn successful_leader_cannot_leave_background_work_after_return() {
    let root = tempfile::tempdir().unwrap();
    run(
        shell(
            "(sleep 1; touch \"$1/late\") & echo $! > \"$1/child\"; exit 0",
            root.path(),
        ),
        Instant::now() + Duration::from_secs(3),
        "background",
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    not_running(&root.path().join("child"));
    assert!(!root.path().join("late").exists());
}

#[test]
fn observing_exit_retains_leader_identity_until_group_cleanup() {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exit 0"]).process_group(0);
    let mut group = ChildGroup {
        child: command.spawn().unwrap(),
        reaped: false,
    };
    let pid = group.child.id();
    let deadline = Instant::now() + Duration::from_secs(3);
    while group.observe().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        Path::new(&format!("/proc/{pid}")).exists(),
        "observation must not reap"
    );
    assert!(group.observe().unwrap().is_some());
    assert!(group.finish().unwrap().success());
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}

#[test]
#[ignore = "requires Linux root and runuser; run with --ignored in the isolated Linux builder"]
fn root_runuser_descendant_is_owned_by_the_preparation_group() {
    assert!(rustix::process::geteuid().is_root());
    let root = tempfile::tempdir().unwrap();
    let user = Command::new("id").args(["-u", "nobody"]).output().unwrap();
    assert!(user.status.success());
    let uid = std::str::from_utf8(&user.stdout)
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    // Keep tempfile's private permissions; grant ownership only to this test user.
    std::os::unix::fs::chown(root.path(), Some(uid), None).unwrap();
    let mut command = Command::new("runuser");
    command
        .args([
            "-u",
            "nobody",
            "--",
            "/bin/sh",
            "-c",
            "echo $$ > \"$1/child\"; sleep 1; touch \"$1/late\"",
            "runuser-test",
        ])
        .arg(root.path());
    let start = Instant::now();
    assert!(run(command, start + Duration::from_millis(300), "runuser").is_err());
    assert!(start.elapsed() < Duration::from_secs(3));
    std::thread::sleep(Duration::from_millis(1200));
    not_running(&root.path().join("child"));
    assert!(!root.path().join("late").exists());
}
