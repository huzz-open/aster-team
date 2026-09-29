#![forbid(unsafe_code)]

use std::{
    fmt::Write as _,
    fs::File,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

use aster_release_core::{RELEASE_MANIFEST_FILE, RELEASE_SCHEMA, ReleaseClaims, ReleaseFile, sign};
use clap::{Parser, Subcommand, ValueEnum};
use ed25519_dalek::SigningKey;
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};
use zeroize::Zeroizing;

#[derive(Debug, Parser)]
#[command(
    name = "aster-release-tool",
    version,
    about = "Internal Aster Team release signing tool"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Sign {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        private_key: PathBuf,
        #[arg(long)]
        key_id: String,
        #[arg(long)]
        version: String,
        #[arg(long)]
        architecture: Architecture,
        #[arg(long, default_value = "linux")]
        platform: Platform,
        #[arg(long)]
        runtime: Runtime,
        #[arg(long)]
        created_at: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Architecture {
    Amd64,
    Arm64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Platform {
    Linux,
    Windows,
    Macos,
}

impl Platform {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Windows => "windows",
            Self::Macos => "macos",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Runtime {
    MuslStatic,
    Msvc,
    Native,
}

impl Runtime {
    fn as_str(self) -> &'static str {
        match self {
            Self::MuslStatic => "musl-static",
            Self::Msvc => "msvc",
            Self::Native => "native",
        }
    }
}

impl Architecture {
    fn as_str(self) -> &'static str {
        match self {
            Self::Amd64 => "amd64",
            Self::Arm64 => "arm64",
        }
    }
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("release signing failed: {error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Sign {
            root,
            private_key,
            key_id,
            version,
            architecture,
            platform,
            runtime,
            created_at,
        } => {
            if !matches!(
                (platform, runtime),
                (Platform::Linux, Runtime::MuslStatic)
                    | (Platform::Windows, Runtime::Msvc)
                    | (Platform::Macos, Runtime::Native)
            ) {
                return Err("release platform and runtime do not match".into());
            }
            validate_exact_time(&created_at)?;
            let seed = Zeroizing::new(read_exact_seed(&private_key)?);
            let signing_key = SigningKey::from_bytes(&seed);
            let claims = ReleaseClaims {
                schema: RELEASE_SCHEMA.to_owned(),
                key_id,
                product: "aster-team".to_owned(),
                version,
                platform: platform.as_str().to_owned(),
                architecture: architecture.as_str().to_owned(),
                runtime: runtime.as_str().to_owned(),
                created_at,
                files: collect_files(&root)?,
            };
            let document = sign(claims, &signing_key)?;
            let mut encoded = serde_json::to_vec_pretty(&document)?;
            encoded.push(b'\n');
            write_manifest(&root.join(RELEASE_MANIFEST_FILE), &encoded)?;
            println!("{}", root.join(RELEASE_MANIFEST_FILE).display());
        }
    }
    Ok(())
}

fn collect_files(root: &Path) -> Result<Vec<ReleaseFile>, Box<dyn std::error::Error>> {
    let metadata = std::fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("release root must be a real directory".into());
    }
    let mut files = Vec::new();
    collect_directory(root, root, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    if files.is_empty() {
        return Err("release root is empty".into());
    }
    Ok(files)
}

fn collect_directory(
    root: &Path,
    directory: &Path,
    output: &mut Vec<ReleaseFile>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let metadata = std::fs::symlink_metadata(&path)?;
        let relative = path
            .strip_prefix(root)?
            .to_str()
            .ok_or("release paths must be valid UTF-8")?
            .replace('\\', "/");
        if file_type.is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(format!("release contains an unsafe file: {relative}").into());
        }
        if metadata.is_dir() {
            collect_directory(root, &path, output)?;
            continue;
        }
        if relative == RELEASE_MANIFEST_FILE {
            return Err("release manifest already exists; sign a clean staging tree".into());
        }
        let mut source = File::open(&path)?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt as _;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = relative.starts_with("bin/");
        output.push(ReleaseFile {
            path: relative,
            size: metadata.len(),
            sha256: lowercase_hex(&digest.finalize()),
            executable,
        });
    }
    Ok(())
}

fn lowercase_hex(value: &[u8]) -> String {
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn read_exact_seed(path: &Path) -> Result<[u8; 32], std::io::Error> {
    let value = std::fs::read(path)?;
    value.try_into().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "release signing key must contain exactly 32 raw bytes",
        )
    })
}

fn validate_exact_time(value: &str) -> Result<(), std::io::Error> {
    if value.len() != 24 || value.as_bytes().get(19) != Some(&b'.') || !value.ends_with('Z') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "created-at must use YYYY-MM-DDTHH:MM:SS.mmmZ",
        ));
    }
    let parsed = OffsetDateTime::parse(value, &Rfc3339).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "created-at is invalid")
    })?;
    if parsed.offset() != UtcOffset::UTC {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "created-at must use UTC",
        ));
    }
    Ok(())
}

fn write_manifest(path: &Path, value: &[u8]) -> Result<(), std::io::Error> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "manifest has no parent")
    })?;
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(value)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use aster_release_core::{TrustedReleaseKeys, verify, verify_release_tree};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn signs_a_clean_tree_that_customer_verification_accepts() {
        let directory = tempdir().expect("temp directory");
        let root = directory.path().join("release");
        std::fs::create_dir_all(root.join("bin")).expect("create bin");
        std::fs::write(root.join("bin/aster-control"), b"control").expect("write control");
        std::fs::write(root.join("VERSION"), b"1.0.0\n").expect("write version");
        let key = SigningKey::from_bytes(&[41_u8; 32]);
        let document = sign(
            ReleaseClaims {
                schema: RELEASE_SCHEMA.to_owned(),
                key_id: "release-test-01".to_owned(),
                product: "aster-team".to_owned(),
                version: "1.0.0".to_owned(),
                platform: "linux".to_owned(),
                architecture: "amd64".to_owned(),
                runtime: "musl-static".to_owned(),
                created_at: "2026-08-28T00:00:00.000Z".to_owned(),
                files: collect_files(&root).expect("collect files"),
            },
            &key,
        )
        .expect("sign release");
        let encoded = serde_json::to_vec_pretty(&document).expect("serialize release");
        write_manifest(&root.join(RELEASE_MANIFEST_FILE), &encoded).expect("write manifest");
        let mut keys = TrustedReleaseKeys::new();
        keys.insert("release-test-01", key.verifying_key())
            .expect("insert key");
        let verified = verify(&encoded, &keys).expect("verify signature");
        verify_release_tree(&root, &verified).expect("verify release tree");
        assert!(collect_files(&root).is_err());
    }
}
