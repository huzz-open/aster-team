//! Offline publisher tool. The signing key stays in the release environment.
use std::{
    collections::BTreeMap,
    env, fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

use aster_plugin_core::{
    BundleDisplay, BundleFile, BundleManifest, TrustedPublisher, sha256_hex, verify_bundle,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use zeroize::Zeroizing;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

fn main() {
    if let Err(error) = run() {
        eprintln!("plugin signing failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 8 {
        return Err(
            "usage: aster-sign-plugin SOURCE OUTPUT KEY_FILE KEY_ID BUNDLE_ID BUNDLE_VERSION MIN_HOST_VERSION CANONICAL_SCHEMA"
                .into(),
        );
    }
    let source = PathBuf::from(&args[0]);
    let output = PathBuf::from(&args[1]);
    let key_file = PathBuf::from(&args[2]);
    let key_id = args[3].to_string_lossy().into_owned();
    let bundle_id = args[4].to_string_lossy().into_owned();
    let bundle_version = args[5].to_string_lossy().into_owned();
    let min_host_version = args[6].to_string_lossy().into_owned();
    let canonical_schema: u32 = args[7].to_string_lossy().parse()?;
    if key_id.is_empty() || bundle_id.is_empty() {
        return Err("key id and bundle id are required".into());
    }
    let secret = Zeroizing::new(fs::read(key_file)?);
    let secret_bytes: &[u8; 32] = secret
        .as_slice()
        .try_into()
        .map_err(|_| "signing key must contain exactly 32 raw bytes")?;
    let signing_key = SigningKey::from_bytes(secret_bytes);
    let entrypoints: BTreeMap<String, String> =
        serde_json::from_slice(&fs::read(source.join("entrypoints.json"))?)?;
    let display: BTreeMap<String, BundleDisplay> = match fs::read(source.join("plugin-info.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(error) => return Err(error.into()),
    };
    let mut files = Vec::new();
    collect_sources(&source, &source, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err("source has no plugin files".into());
    }
    let manifest = BundleManifest {
        format_version: 1,
        bundle_id: bundle_id.clone(),
        bundle_version,
        display,
        host_api: 1,
        canonical_schema,
        min_host_version: min_host_version.clone(),
        entrypoints,
        files: files
            .iter()
            .map(|(path, data)| BundleFile {
                path: path.clone(),
                size: data.len() as u64,
                sha256: sha256_hex(data),
            })
            .collect(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let mut signed = b"ASTER-LUA-BUNDLE-V1\0".to_vec();
    signed.extend_from_slice(&manifest_bytes);
    let signature = signing_key.sign(&signed);
    let signature_bytes = serde_json::to_vec(&json!({
        "algorithm": "ed25519",
        "key_id": key_id.clone(),
        "signature_base64": STANDARD.encode(signature.to_bytes()),
    }))?;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer.start_file("manifest.json", options)?;
    writer.write_all(&manifest_bytes)?;
    writer.start_file("signature.json", options)?;
    writer.write_all(&signature_bytes)?;
    for (path, data) in files {
        writer.start_file(path, options)?;
        writer.write_all(&data)?;
    }
    let archive = writer.finish()?.into_inner();
    verify_bundle(
        &archive,
        &[TrustedPublisher {
            key_id,
            bundle_id,
            public_key: signing_key.verifying_key(),
        }],
        &min_host_version,
        canonical_schema,
    )?;
    fs::write(&output, &archive)?;
    println!("{}  {}", sha256_hex(&archive), output.display());
    Ok(())
}

fn collect_sources(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(format!("symlink in plugin source: {}", path.display()).into());
        }
        if file_type.is_dir() {
            collect_sources(root, &path, files)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(format!("unsupported plugin source: {}", path.display()).into());
        }
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if relative == "entrypoints.json" || relative == "plugin-info.json" {
            continue;
        }
        if !relative.ends_with(".lua") && !relative.ends_with(".json") {
            return Err(format!("unsupported source file: {relative}").into());
        }
        files.push((relative, fs::read(path)?));
    }
    Ok(())
}
