//! Fingerprint the exact slot inputs before allowing service startup. No secret
//! bytes or filesystem paths are serialized into the startup journal.
use super::*;
use sha2::{Digest as _, Sha256};
use std::os::unix::fs::MetadataExt as _;

pub(super) fn fingerprint(layout: &InstallLayout, slot: ReleaseSlot) -> Result<String, CliFailure> {
    let mut digest = Sha256::new();
    digest.update(b"aster-online-slot-material-v1");
    digest.update(slot.id().as_bytes());
    for (name, path, required, max_bytes) in inputs(layout, slot) {
        super::super::safe_owned_path(layout, &path)?;
        digest.update((name.len() as u64).to_be_bytes());
        digest.update(name.as_bytes());
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => {
                digest.update([0]);
                continue;
            }
            Err(error) => return Err(failed(error.to_string())),
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.nlink() != 1
            || metadata.mode() & 0o022 != 0
        {
            return Err(failed(
                "online startup material must be a protected plain file",
            ));
        }
        let bytes = read_regular(&path, max_bytes)?;
        digest.update([1]);
        digest.update(metadata.uid().to_be_bytes());
        digest.update(metadata.gid().to_be_bytes());
        digest.update(metadata.mode().to_be_bytes());
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes.as_slice());
    }
    Ok(crate::lowercase_hex(&digest.finalize()))
}

fn inputs(
    layout: &InstallLayout,
    slot: ReleaseSlot,
) -> Vec<(&'static str, std::path::PathBuf, bool, usize)> {
    vec![
        (
            "common-control",
            layout.control_environment(),
            true,
            128 * 1024,
        ),
        (
            "slot-control",
            layout.control_slot_environment(slot.id()),
            true,
            128 * 1024,
        ),
        (
            "runtime-token",
            layout.control_slot_runtime_token(slot.id()),
            true,
            128 * 1024,
        ),
        (
            "slot-runner",
            layout.runner_slot_environment(slot.id()),
            true,
            128 * 1024,
        ),
        (
            "runner-identity",
            layout.runner_slot_identity(slot.id()),
            true,
            128 * 1024,
        ),
        (
            "runner-task-keys",
            layout.runner_slot_task_keys(slot.id()),
            true,
            128 * 1024,
        ),
        (
            "runner-upstream-ca",
            layout.runner_slot_upstream_ca(slot.id()),
            false,
            1024 * 1024,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt as _, symlink},
    };

    #[test]
    fn ca_bundle_preserves_the_existing_one_mib_limit() {
        let root = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(root.path()).unwrap();
        let slot = ReleaseSlot::Green;
        for (_, path, required, _) in inputs(&layout, slot) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            if required {
                fs::write(path, b"fixture material").unwrap();
            }
        }
        let ca = layout.runner_slot_upstream_ca(slot.id());
        // Content parsing belongs to Runner preflight; fingerprinting preserves
        // its accepted byte limit and binds every byte, including the tail.
        let mut bundle = vec![b'x'; 1024 * 1024];
        fs::write(&ca, &bundle).unwrap();
        let original = fingerprint(&layout, slot).unwrap();
        *bundle.last_mut().unwrap() = b'y';
        fs::write(&ca, &bundle).unwrap();
        assert_ne!(original, fingerprint(&layout, slot).unwrap());
        bundle.push(b'z');
        fs::write(&ca, &bundle).unwrap();
        assert!(fingerprint(&layout, slot).is_err());
        fs::remove_file(&ca).unwrap();
        fs::write(
            layout.control_slot_runtime_token(slot.id()),
            vec![b'x'; 128 * 1024 + 1],
        )
        .unwrap();
        assert!(fingerprint(&layout, slot).is_err());
    }

    #[test]
    fn material_bytes_presence_and_permissions_are_bound_without_modifying_inputs() {
        let root = tempfile::tempdir().unwrap();
        let layout = InstallLayout::new(root.path()).unwrap();
        let slot = ReleaseSlot::Green;
        for (_, path, required, _) in inputs(&layout, slot) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            if required {
                fs::write(path, b"fixture material").unwrap();
            }
        }
        let original = fingerprint(&layout, slot).unwrap();
        assert_eq!(original, fingerprint(&layout, slot).unwrap());
        let key = layout.control_slot_runtime_token(slot.id());
        fs::write(&key, b"different material").unwrap();
        assert_ne!(original, fingerprint(&layout, slot).unwrap());
        fs::write(&key, b"fixture material").unwrap();
        assert_eq!(original, fingerprint(&layout, slot).unwrap());
        let ca = layout.runner_slot_upstream_ca(slot.id());
        fs::write(&ca, b"optional cert").unwrap();
        assert_ne!(original, fingerprint(&layout, slot).unwrap());
        fs::remove_file(&ca).unwrap();
        let before = fs::metadata(&key).unwrap().permissions().mode();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        if before & 0o777 != 0o600 {
            assert_ne!(original, fingerprint(&layout, slot).unwrap());
        }
        fs::set_permissions(&key, fs::Permissions::from_mode(before)).unwrap();
        fs::remove_file(&key).unwrap();
        assert!(fingerprint(&layout, slot).is_err());
        symlink(layout.control_environment(), &key).unwrap();
        assert!(fingerprint(&layout, slot).is_err());
        assert_eq!(fs::read_link(&key).unwrap(), layout.control_environment());
    }
}
