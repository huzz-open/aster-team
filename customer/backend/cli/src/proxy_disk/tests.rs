use super::*;
use std::time::Duration;

pub(crate) fn fixture() -> (tempfile::TempDir, InstallLayout) {
    let temporary = tempfile::tempdir().unwrap();
    let layout = InstallLayout::new(temporary.path()).unwrap();
    let path = layout.caddy_upstreams();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        super::super::maintenance_executor::render_upstreams(ReleaseSlot::Blue, Default::default()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    }
    (temporary, layout)
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(3)
}

#[test]
fn disk_intent_survives_reopen_and_repeated_preparation_is_idempotent() {
    let (_temporary, layout) = fixture();
    let file = InstalledUpstreams::open(&layout).unwrap();
    file.assert_target(ReleaseSlot::Blue, deadline()).unwrap();
    file.prepare(ReleaseSlot::Green, deadline()).unwrap();
    drop(file);
    let file = InstalledUpstreams::open(&layout).unwrap();
    file.assert_target(ReleaseSlot::Green, deadline()).unwrap();
    file.prepare(ReleaseSlot::Green, deadline()).unwrap();
    file.assert_target(ReleaseSlot::Green, deadline()).unwrap();
}

#[test]
fn unrecognized_changed_missing_and_oversized_files_are_preserved() {
    for value in [b"custom upstream".to_vec(), vec![b'x'; 8193]] {
        let (_temporary, layout) = fixture();
        let file = InstalledUpstreams::open(&layout).unwrap();
        fs::write(layout.caddy_upstreams(), &value).unwrap();
        assert!(file.prepare(ReleaseSlot::Green, deadline()).is_err());
        assert_eq!(fs::read(layout.caddy_upstreams()).unwrap(), value);
    }
    let (_temporary, layout) = fixture();
    let file = InstalledUpstreams::open(&layout).unwrap();
    fs::remove_file(layout.caddy_upstreams()).unwrap();
    assert!(file.prepare(ReleaseSlot::Green, deadline()).is_err());
    assert!(!layout.caddy_upstreams().exists());
}

#[test]
fn expired_operation_does_not_write_disk_intent() {
    let (_temporary, layout) = fixture();
    let file = InstalledUpstreams::open(&layout).unwrap();
    assert!(file.prepare(ReleaseSlot::Green, Instant::now()).is_err());
    file.assert_target(ReleaseSlot::Blue, deadline()).unwrap();
}

#[cfg(unix)]
#[test]
fn atomic_replacement_preserves_caddy_read_permissions_and_rejects_links() {
    use std::os::unix::fs::{MetadataExt as _, symlink};
    let (temporary, layout) = fixture();
    let before = fs::metadata(layout.caddy_upstreams()).unwrap();
    let file = InstalledUpstreams::open(&layout).unwrap();
    file.prepare(ReleaseSlot::Green, deadline()).unwrap();
    let after = fs::metadata(layout.caddy_upstreams()).unwrap();
    assert_eq!(
        (after.uid(), after.gid(), after.mode() & 0o7777),
        (before.uid(), before.gid(), 0o640)
    );
    assert_ne!(after.ino(), before.ino());
    let linked = temporary.path().join("hard-link");
    fs::hard_link(layout.caddy_upstreams(), &linked).unwrap();
    assert!(file.prepare(ReleaseSlot::Blue, deadline()).is_err());
    fs::remove_file(&linked).unwrap();
    fs::rename(layout.caddy_upstreams(), &linked).unwrap();
    symlink(&linked, layout.caddy_upstreams()).unwrap();
    assert!(file.prepare(ReleaseSlot::Blue, deadline()).is_err());
    assert!(
        fs::symlink_metadata(layout.caddy_upstreams())
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
