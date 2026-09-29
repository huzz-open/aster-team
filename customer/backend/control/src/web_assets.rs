//! Readiness verifies the immutable signed release, including lazy page assets.
use aster_release_core::{RELEASE_MANIFEST_FILE, TrustedReleaseKeys};
use sha2::{Digest as _, Sha256};
use std::{
    io::Read as _,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) struct WebAssets {
    admin: PathBuf,
    member: PathBuf,
    worker: Arc<tokio::sync::Semaphore>,
}

impl WebAssets {
    pub fn new(admin: PathBuf, member: PathBuf) -> Self {
        Self {
            admin,
            member,
            worker: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    pub async fn verify(&self, version: String, expected_manifest: String) -> bool {
        let Ok(permit) = self.worker.clone().try_acquire_owned() else {
            return false;
        };
        let admin = self.admin.clone();
        let member = self.member.clone();
        // The worker retains the permit if its HTTP waiter is cancelled. A slow
        // filesystem cannot accumulate unlimited detached verification workers.
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let Ok(keys) = crate::compiled_release_keys() else {
                return false;
            };
            let Ok(executable) = std::env::current_exe() else {
                return false;
            };
            verify(
                &admin,
                &member,
                &executable,
                &version,
                &expected_manifest,
                &keys,
            )
            .is_ok()
        })
        .await
        .unwrap_or(false)
    }
}

fn verify(
    admin: &Path,
    member: &Path,
    executable: &Path,
    version: &str,
    expected_manifest: &str,
    keys: &TrustedReleaseKeys,
) -> Result<(), ()> {
    let original_admin = admin;
    let original_member = member;
    let admin = admin.canonicalize().map_err(|_| ())?;
    let member = member.canonicalize().map_err(|_| ())?;
    let root = admin.parent().ok_or(())?;
    if admin.file_name() != Some(std::ffi::OsStr::new("admin")) || member != root.join("member") {
        return Err(());
    }
    let mut document = Vec::new();
    std::fs::File::open(root.join(RELEASE_MANIFEST_FILE))
        .map_err(|_| ())?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut document)
        .map_err(|_| ())?;
    if document.len() > 16 * 1024 * 1024
        || crate::hex_encode(&Sha256::digest(&document)) != expected_manifest
    {
        return Err(());
    }
    let release = aster_release_core::verify(&document, keys).map_err(|_| ())?;
    if release.claims().version != version {
        return Err(());
    }
    let binary = if release.claims().platform == "windows" {
        "bin/aster-control.exe"
    } else {
        "bin/aster-control"
    };
    if executable.canonicalize().map_err(|_| ())?
        != root.join(binary).canonicalize().map_err(|_| ())?
    {
        return Err(());
    }
    for required in ["admin/index.html", "member/index.html", binary] {
        if !release
            .claims()
            .files
            .iter()
            .any(|file| file.path == required && file.size > 0)
        {
            return Err(());
        }
    }
    aster_release_core::verify_release_tree(root, &release).map_err(|_| ())?;
    if original_admin.canonicalize().map_err(|_| ())? != admin
        || original_member.canonicalize().map_err(|_| ())? != member
    {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_pages_include_lazy_assets_and_match_the_running_release() {
        let root = tempfile::tempdir().unwrap();
        let mut files = Vec::new();
        for (path, data) in [
            ("admin/index.html", "admin"),
            ("admin/assets/lazy.js", "lazy-module"),
            ("member/index.html", "member"),
            ("bin/aster-control", "binary"),
        ] {
            let target = root.path().join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, data).unwrap();
            files.push(aster_release_core::ReleaseFile {
                path: path.into(),
                size: data.len() as u64,
                sha256: crate::hex_encode(&Sha256::digest(data)),
                executable: false,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let key = ed25519_dalek::SigningKey::from_bytes(&[33; 32]);
        let release = aster_release_core::sign(
            aster_release_core::ReleaseClaims {
                schema: aster_release_core::RELEASE_SCHEMA.into(),
                key_id: "release-test".into(),
                product: "aster-team".into(),
                version: "0.1.0".into(),
                platform: "linux".into(),
                architecture: "amd64".into(),
                runtime: "musl-static".into(),
                created_at: "2026-08-28T00:00:00.000Z".into(),
                files,
            },
            &key,
        )
        .unwrap();
        let document = serde_json::to_vec(&release).unwrap();
        std::fs::write(root.path().join(RELEASE_MANIFEST_FILE), &document).unwrap();
        let manifest = crate::hex_encode(&Sha256::digest(&document));
        let mut keys = TrustedReleaseKeys::new();
        keys.insert("release-test", key.verifying_key()).unwrap();
        let admin = root.path().join("admin");
        let member = root.path().join("member");
        let executable = root.path().join("bin/aster-control");
        assert!(verify(&admin, &member, &executable, "0.1.0", &manifest, &keys).is_ok());
        assert!(verify(&admin, &member, &executable, "0.2.0", &manifest, &keys).is_err());
        assert!(
            verify(
                &admin,
                &member,
                &executable,
                "0.1.0",
                &"0".repeat(64),
                &keys
            )
            .is_err()
        );
        assert!(
            verify(
                &admin,
                &member,
                &executable,
                "0.1.0",
                &manifest,
                &TrustedReleaseKeys::new()
            )
            .is_err()
        );
        let other = root.path().join("different-control");
        std::fs::write(&other, "binary").unwrap();
        assert!(verify(&admin, &member, &other, "0.1.0", &manifest, &keys).is_err());
        std::fs::remove_file(other).unwrap();
        std::fs::write(admin.join("assets/lazy.js"), "corrupt-js!").unwrap();
        assert!(verify(&admin, &member, &executable, "0.1.0", &manifest, &keys).is_err());
        std::fs::remove_file(admin.join("assets/lazy.js")).unwrap();
        assert!(verify(&admin, &member, &executable, "0.1.0", &manifest, &keys).is_err());
    }
}
