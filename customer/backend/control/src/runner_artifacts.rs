use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

use axum::{
    body::{Body, Bytes},
    extract::{Path as AxumPath, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{
            AUTHORIZATION, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, ETAG,
        },
    },
    response::{IntoResponse, Response},
};
use flate2::{Compression, write::GzEncoder};
use sha2::{Digest as _, Sha256};

use super::{
    ControlError, ControlState, authorize_non_consuming_feature, compiled_release_keys,
    format_database_time, sha256_hex, valid_runner_secret,
};

const MAX_RUNNER_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;

fn runner_bundle_paths(platform: &str) -> Result<&'static [&'static str], ControlError> {
    match platform {
        "windows" => Ok(&[
            "RELEASE.json",
            "VERSION",
            "bin/aster-runner.exe",
            "bin/aster-team-cli.exe",
            "init.ps1",
            "libexec/install.ps1",
            "libexec/restore-backup.ps1",
            "windows/service-launch.ps1",
        ]),
        "linux" => Ok(&[
            "RELEASE.json",
            "VERSION",
            "bin/aster-runner",
            "bin/aster-team-cli",
            "init.sh",
            "libexec/install.sh",
            "libexec/restore-backup.sh",
            "libexec/service-health.sh",
            "systemd/aster-runner.service",
        ]),
        _ => Err(ControlError::RunnerInputInvalid),
    }
}

struct RunnerInstallArtifact {
    file_name: String,
    path: PathBuf,
    sha256: String,
    size_bytes: u64,
}

fn artifact_error(error: impl std::fmt::Display) -> ControlError {
    ControlError::MaintenanceUnavailable(format!("Runner install package is unavailable: {error}"))
}

async fn authorize_pending_enrollment(
    state: &ControlState,
    headers: &HeaderMap,
) -> Result<(), ControlError> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| valid_runner_secret(value, "aren_", 40, 160))
        .ok_or(ControlError::RunnerUnauthenticated)?;
    let now =
        format_database_time((state.now)()).map_err(|_| ControlError::TimeConversionFailed)?;
    let token_hash = sha256_hex(token.as_bytes());
    if !state
        .credential_storage()?
        .pending_runner_enrollment(&token_hash, &now)
        .await?
    {
        return Err(ControlError::RunnerUnauthenticated);
    }
    authorize_non_consuming_feature(state, "runner")?;
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, ControlError> {
    let mut file = File::open(path).map_err(artifact_error)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(artifact_error)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn checked_regular_file(path: &Path) -> Result<Option<u64>, ControlError> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && (1..=MAX_RUNNER_PACKAGE_BYTES).contains(&metadata.len()) =>
        {
            Ok(Some(metadata.len()))
        }
        Ok(_) => Err(artifact_error(
            "cached package is not a bounded regular file",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(artifact_error(error)),
    }
}

fn build_runner_install_artifact(
    layout: &aster_install_layout::InstallLayout,
    version: &str,
    platform: &str,
    architecture: &str,
) -> Result<RunnerInstallArtifact, ControlError> {
    if !matches!(platform, "linux" | "windows") || architecture != "amd64" {
        return Err(ControlError::RunnerInputInvalid);
    }
    semver::Version::parse(version).map_err(|_| ControlError::DataIntegrityInvalid)?;
    if let Some(artifact) = cached_runner_install_artifact(layout, version, platform, architecture)?
    {
        return Ok(artifact);
    }

    let expected_platform = std::env::consts::OS;
    let expected_architecture = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        value => value,
    };
    if platform != expected_platform || architecture != expected_architecture {
        return Err(ControlError::RunnerInputInvalid);
    }

    let release = layout.release(version);
    let current = layout.current();
    if release.exists() || current.exists() {
        let release_canonical = release.canonicalize().map_err(artifact_error)?;
        let current_canonical = current.canonicalize().map_err(artifact_error)?;
        if release_canonical != current_canonical
            || !release_canonical.join("RELEASE.json").is_file()
            || !release_canonical.join("VERSION").is_file()
        {
            return Err(artifact_error("the active signed release is incomplete"));
        }
        return cache_runner_install_artifact(
            layout,
            &release_canonical,
            version,
            platform,
            architecture,
        );
    }

    let full_archive_name = format!("aster-team-{version}-{platform}-{architecture}.tar.gz");
    let full_archive = layout
        .runtime()
        .join("runner-install-packages")
        .join(full_archive_name);
    if checked_regular_file(&full_archive)?.is_some() {
        let trusted_keys = compiled_release_keys()?;
        return cache_runner_install_artifact_from_full_archive(
            layout,
            &full_archive,
            version,
            platform,
            architecture,
            &trusted_keys,
        );
    }

    Err(artifact_error(
        "neither the active signed release nor a cached full release package is available",
    ))
}

fn cache_runner_install_artifact_from_full_archive(
    layout: &aster_install_layout::InstallLayout,
    full_archive: &Path,
    version: &str,
    platform: &str,
    architecture: &str,
    trusted_keys: &aster_release_core::TrustedReleaseKeys,
) -> Result<RunnerInstallArtifact, ControlError> {
    let cache = layout.runtime().join("runner-install-packages");
    fs::create_dir_all(&cache).map_err(artifact_error)?;
    let temporary = tempfile::tempdir_in(&cache).map_err(artifact_error)?;
    let release_root = temporary.path().join("release");
    fs::create_dir(&release_root).map_err(artifact_error)?;

    let archive_root = format!("aster-team-{version}-{platform}-{architecture}");
    let expected = runner_bundle_paths(platform)?
        .iter()
        .map(|relative| (format!("{archive_root}/{relative}"), *relative))
        .collect::<BTreeMap<_, _>>();
    let input = File::open(full_archive).map_err(artifact_error)?;
    let decoder = flate2::read::GzDecoder::new(input);
    let mut archive = tar::Archive::new(decoder);
    let mut extracted = BTreeSet::new();
    for entry in archive.entries().map_err(artifact_error)? {
        let mut entry = entry.map_err(artifact_error)?;
        let path = entry
            .path()
            .map_err(artifact_error)?
            .to_string_lossy()
            .replace('\\', "/");
        let Some(relative) = expected.get(&path).copied() else {
            continue;
        };
        if !entry.header().entry_type().is_file() || !extracted.insert(relative) {
            return Err(artifact_error(format!(
                "cached full release contains an unsafe Runner file: {relative}"
            )));
        }
        let destination = release_root.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(artifact_error)?;
        }
        entry.unpack(&destination).map_err(artifact_error)?;
    }
    if extracted.len() != expected.len() {
        let missing = expected
            .values()
            .find(|relative| !extracted.contains(**relative))
            .copied()
            .unwrap_or("Runner file");
        return Err(artifact_error(format!(
            "cached full release is missing Runner file {missing}"
        )));
    }

    let manifest = fs::read(release_root.join("RELEASE.json")).map_err(artifact_error)?;
    let verified = aster_release_core::verify(&manifest, trusted_keys).map_err(artifact_error)?;
    if verified.claims().version != version
        || verified.claims().platform != platform
        || verified.claims().architecture != architecture
    {
        return Err(artifact_error(
            "cached full release identity does not match the requested Runner package",
        ));
    }
    aster_release_core::verify_release_subset(
        &release_root,
        &verified,
        runner_bundle_paths(platform)?
            .iter()
            .copied()
            .filter(|relative| *relative != "RELEASE.json"),
    )
    .map_err(artifact_error)?;
    cache_runner_install_artifact(layout, &release_root, version, platform, architecture)
}

fn cached_runner_install_artifact(
    layout: &aster_install_layout::InstallLayout,
    version: &str,
    platform: &str,
    architecture: &str,
) -> Result<Option<RunnerInstallArtifact>, ControlError> {
    let file_name = format!("aster-team-runner-{version}-{platform}-{architecture}.tar.gz");
    let path = layout
        .runtime()
        .join("runner-install-packages")
        .join(&file_name);
    let Some(size_bytes) = checked_regular_file(&path)? else {
        return Ok(None);
    };
    Ok(Some(RunnerInstallArtifact {
        file_name,
        sha256: hash_file(&path)?,
        path,
        size_bytes,
    }))
}

fn cache_runner_install_artifact(
    layout: &aster_install_layout::InstallLayout,
    release_canonical: &Path,
    version: &str,
    platform: &str,
    architecture: &str,
) -> Result<RunnerInstallArtifact, ControlError> {
    if let Some(artifact) = cached_runner_install_artifact(layout, version, platform, architecture)?
    {
        return Ok(artifact);
    }
    let cache = layout.runtime().join("runner-install-packages");
    fs::create_dir_all(&cache).map_err(artifact_error)?;
    let file_name = format!("aster-team-runner-{version}-{platform}-{architecture}.tar.gz");
    let path = cache.join(&file_name);
    let size_bytes = {
        let partial = cache.join(format!(".{file_name}.{}.partial", std::process::id()));
        match fs::symlink_metadata(&partial) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                fs::remove_file(&partial).map_err(artifact_error)?;
            }
            Ok(_) => return Err(artifact_error("temporary package path is unsafe")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(artifact_error(error)),
        }
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(artifact_error)?;
        let encoder = GzEncoder::new(output, Compression::best());
        let mut archive = tar::Builder::new(encoder);
        archive.follow_symlinks(false);
        archive.mode(tar::HeaderMode::Deterministic);
        let archive_root = format!("aster-team-runner-{version}-{platform}-{architecture}");
        let append_result = (|| -> Result<(), ControlError> {
            for relative in runner_bundle_paths(platform)? {
                let source = release_canonical.join(relative);
                let metadata = fs::symlink_metadata(&source).map_err(artifact_error)?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err(artifact_error(format!(
                        "the active release is missing Runner file {relative}"
                    )));
                }
                let destination = format!("{archive_root}/{relative}");
                archive
                    .append_path_with_name(&source, &destination)
                    .map_err(artifact_error)?;
            }
            Ok(())
        })();
        if let Err(error) = append_result {
            drop(archive);
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
        let encoder = archive.into_inner().map_err(artifact_error)?;
        let mut output = encoder.finish().map_err(artifact_error)?;
        output.flush().map_err(artifact_error)?;
        output.sync_all().map_err(artifact_error)?;
        drop(output);
        let size = checked_regular_file(&partial)?
            .ok_or_else(|| artifact_error("generated package disappeared"))?;
        fs::rename(&partial, &path).map_err(artifact_error)?;
        size
    };
    let sha256 = hash_file(&path)?;
    Ok(RunnerInstallArtifact {
        file_name,
        path,
        sha256,
        size_bytes,
    })
}

async fn runner_install_artifact(
    state: &ControlState,
    platform: String,
    architecture: String,
) -> Result<RunnerInstallArtifact, ControlError> {
    let manager = state.maintenance.as_ref().ok_or_else(|| {
        ControlError::MaintenanceUnavailable("maintenance is not configured".to_owned())
    })?;
    let _package_build = manager.submissions.lock().await;
    let layout = manager.layout.clone();
    let version = state.product_version.to_string();
    super::lifecycle::spawn_blocking(move || {
        build_runner_install_artifact(&layout, &version, &platform, &architecture)
    })
    .await
    .map_err(ControlError::storage)?
}

pub(super) async fn download_runner_install_package(
    State(state): State<ControlState>,
    headers: HeaderMap,
    AxumPath((platform, architecture)): AxumPath<(String, String)>,
) -> Result<Response, ControlError> {
    authorize_pending_enrollment(&state, &headers).await?;
    let artifact = runner_install_artifact(&state, platform, architecture).await?;
    let disposition =
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", artifact.file_name))
            .map_err(|_| ControlError::DataIntegrityInvalid)?;
    let content_length = HeaderValue::from_str(&artifact.size_bytes.to_string())
        .map_err(|_| ControlError::DataIntegrityInvalid)?;
    let etag = HeaderValue::from_str(&format!("\"sha256:{}\"", artifact.sha256))
        .map_err(|_| ControlError::DataIntegrityInvalid)?;
    let file = tokio::fs::File::open(&artifact.path)
        .await
        .map_err(artifact_error)?;
    let stream = futures_util::stream::try_unfold(file, |mut file| async move {
        let mut buffer = vec![0_u8; 64 * 1024];
        let count = tokio::io::AsyncReadExt::read(&mut file, &mut buffer).await?;
        if count == 0 {
            Ok(None)
        } else {
            buffer.truncate(count);
            Ok::<_, std::io::Error>(Some((Bytes::from(buffer), file)))
        }
    });
    let mut response = Body::from_stream(stream).into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/gzip"));
    response
        .headers_mut()
        .insert(CONTENT_DISPOSITION, disposition);
    response
        .headers_mut()
        .insert(CONTENT_LENGTH, content_length);
    response.headers_mut().insert(ETAG, etag);
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    Ok(response)
}

pub(super) async fn runner_install_package_checksum(
    State(state): State<ControlState>,
    headers: HeaderMap,
    AxumPath((platform, architecture)): AxumPath<(String, String)>,
) -> Result<Response, ControlError> {
    authorize_pending_enrollment(&state, &headers).await?;
    let artifact = runner_install_artifact(&state, platform, architecture).await?;
    let mut response = (
        StatusCode::OK,
        format!("{}  {}\n", artifact.sha256, artifact.file_name),
    )
        .into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use aster_release_core::{ReleaseClaims, ReleaseFile, TrustedReleaseKeys, sign};
    use ed25519_dalek::SigningKey;

    use super::*;
    use tempfile::tempdir;

    fn signed_full_release_archive(
        layout: &aster_install_layout::InstallLayout,
        version: &str,
        platform: &str,
        architecture: &str,
    ) -> (PathBuf, TrustedReleaseKeys) {
        let source = tempdir().expect("full release source");
        let extra = match platform {
            "windows" => "bin/aster-control.exe",
            _ => "bin/aster-control",
        };
        let mut paths = runner_bundle_paths(platform)
            .unwrap()
            .iter()
            .copied()
            .filter(|relative| *relative != "RELEASE.json")
            .chain([extra])
            .collect::<Vec<_>>();
        paths.sort_unstable();
        let mut files = Vec::new();
        for relative in &paths {
            let contents = format!("signed fixture: {relative}").into_bytes();
            let target = source.path().join(relative);
            fs::create_dir_all(target.parent().unwrap()).expect("fixture parent");
            fs::write(&target, &contents).expect("fixture file");
            files.push(ReleaseFile {
                path: (*relative).to_owned(),
                size: contents.len() as u64,
                sha256: sha256_hex(&contents),
                executable: false,
            });
        }
        let signing_key = SigningKey::from_bytes(&[42_u8; 32]);
        let document = sign(
            ReleaseClaims {
                schema: aster_release_core::RELEASE_SCHEMA.to_owned(),
                key_id: "runner-package-test".to_owned(),
                product: "aster-team".to_owned(),
                version: version.to_owned(),
                platform: platform.to_owned(),
                architecture: architecture.to_owned(),
                runtime: if platform == "windows" {
                    "msvc".to_owned()
                } else {
                    "musl-static".to_owned()
                },
                created_at: "2026-09-20T00:00:00.000Z".to_owned(),
                files,
            },
            &signing_key,
        )
        .expect("sign fixture");
        fs::write(
            source.path().join("RELEASE.json"),
            serde_json::to_vec(&document).expect("manifest JSON"),
        )
        .expect("manifest");

        let cache = layout.runtime().join("runner-install-packages");
        fs::create_dir_all(&cache).expect("package cache");
        let archive_root = format!("aster-team-{version}-{platform}-{architecture}");
        let archive_path = cache.join(format!("{archive_root}.tar.gz"));
        let output = File::create(&archive_path).expect("full archive");
        let encoder = GzEncoder::new(output, Compression::fast());
        let mut archive = tar::Builder::new(encoder);
        archive
            .append_dir_all(&archive_root, source.path())
            .expect("archive full release");
        let encoder = archive.into_inner().expect("finish tar");
        encoder.finish().expect("finish gzip");

        let mut trusted_keys = TrustedReleaseKeys::new();
        trusted_keys
            .insert("runner-package-test", signing_key.verifying_key())
            .expect("trusted key");
        (archive_path, trusted_keys)
    }

    #[test]
    fn package_builder_creates_a_cached_local_release_archive() {
        let directory = tempdir().expect("install root");
        let layout = aster_install_layout::InstallLayout::new(directory.path()).expect("layout");
        fs::create_dir_all(layout.runtime()).expect("runtime");
        let release = layout.release("2.1.1");
        let platform = std::env::consts::OS;
        let architecture = match std::env::consts::ARCH {
            "x86_64" => "amd64",
            value => value,
        };
        for relative in runner_bundle_paths(platform).expect("supported platform") {
            let path = release.join(relative);
            fs::create_dir_all(path.parent().expect("parent")).expect("release directory");
            fs::write(path, relative.as_bytes()).expect("release file");
        }
        let artifact = cache_runner_install_artifact(
            &layout,
            &release.canonicalize().expect("canonical release"),
            "2.1.1",
            platform,
            architecture,
        )
        .expect("artifact");
        assert!(artifact.path.is_file());
        assert!(artifact.file_name.starts_with("aster-team-runner-"));
        assert!(artifact.size_bytes > 0);
        assert_eq!(artifact.sha256.len(), 64);
        let archive = File::open(&artifact.path).expect("open archive");
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        let entries = archive
            .entries()
            .expect("archive entries")
            .map(|entry| {
                entry
                    .expect("archive entry")
                    .path()
                    .expect("archive path")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), runner_bundle_paths(platform).unwrap().len());
        assert!(entries.iter().all(|path| !path.contains("aster-control")));
        assert!(entries.iter().all(|path| !path.contains("caddy")));
        let cached = build_runner_install_artifact(&layout, "2.1.1", platform, architecture)
            .expect("cached artifact without an installed current link");
        assert_eq!(cached.sha256, artifact.sha256);
        assert!(build_runner_install_artifact(&layout, "2.1.1", "other", architecture).is_err());
    }

    #[test]
    fn package_builder_extracts_a_verified_subset_from_a_cached_full_release() {
        let directory = tempdir().expect("install root");
        let layout = aster_install_layout::InstallLayout::new(directory.path()).expect("layout");
        fs::create_dir_all(layout.runtime()).expect("runtime");
        let version = "2.1.1";
        let platform = std::env::consts::OS;
        let architecture = match std::env::consts::ARCH {
            "x86_64" => "amd64",
            value => value,
        };
        let (full_archive, trusted_keys) =
            signed_full_release_archive(&layout, version, platform, architecture);
        let artifact = cache_runner_install_artifact_from_full_archive(
            &layout,
            &full_archive,
            version,
            platform,
            architecture,
            &trusted_keys,
        )
        .expect("Runner subset");
        assert!(artifact.path.is_file());
        assert_eq!(
            artifact.file_name,
            format!("aster-team-runner-{version}-{platform}-{architecture}.tar.gz")
        );
        let archive = File::open(&artifact.path).expect("open Runner archive");
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        let entries = archive
            .entries()
            .expect("archive entries")
            .map(|entry| {
                entry
                    .expect("archive entry")
                    .path()
                    .expect("archive path")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), runner_bundle_paths(platform).unwrap().len());
        assert!(entries.iter().all(|path| !path.contains("aster-control")));
    }
}
