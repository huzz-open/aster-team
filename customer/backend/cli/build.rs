use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

fn git_output(manifest_directory: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(manifest_directory)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn register_git_inputs(manifest_directory: &Path) {
    if let Some(path) = git_output(manifest_directory, &["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={path}");
    }
    if let Some(reference) = git_output(manifest_directory, &["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git_output(manifest_directory, &["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn build_commit(manifest_directory: &Path) -> String {
    let value = env::var("ASTER_BUILD_COMMIT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| git_output(manifest_directory, &["rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_owned())
        .trim()
        .to_ascii_lowercase();
    if value == "unknown" {
        return value;
    }
    if !(7..=64).contains(&value.len()) || !value.bytes().all(|value| value.is_ascii_hexdigit()) {
        panic!("ASTER_BUILD_COMMIT must be a hexadecimal Git commit");
    }
    value.chars().take(12).collect()
}

fn normalized_timestamp(value: &str, source: &str) -> String {
    let parsed = OffsetDateTime::parse(value, &Rfc3339)
        .unwrap_or_else(|_| panic!("{source} must be an RFC 3339 timestamp"));
    parsed
        .to_offset(UtcOffset::UTC)
        .replace_nanosecond(0)
        .expect("zero nanoseconds are valid")
        .format(&Rfc3339)
        .expect("UTC timestamps format as RFC 3339")
}

fn build_timestamp() -> String {
    if let Ok(value) = env::var("ASTER_BUILD_TIMESTAMP")
        && !value.trim().is_empty()
    {
        return normalized_timestamp(value.trim(), "ASTER_BUILD_TIMESTAMP");
    }
    if let Ok(value) = env::var("ASTER_RELEASE_CREATED_AT")
        && !value.trim().is_empty()
    {
        return normalized_timestamp(value.trim(), "ASTER_RELEASE_CREATED_AT");
    }
    if let Ok(value) = env::var("SOURCE_DATE_EPOCH")
        && !value.trim().is_empty()
    {
        let seconds = value
            .parse::<i64>()
            .expect("SOURCE_DATE_EPOCH must be an integer Unix timestamp");
        return OffsetDateTime::from_unix_timestamp(seconds)
            .expect("SOURCE_DATE_EPOCH must be representable")
            .format(&Rfc3339)
            .expect("UTC timestamps format as RFC 3339");
    }
    OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("zero nanoseconds are valid")
        .format(&Rfc3339)
        .expect("UTC timestamps format as RFC 3339")
}

fn main() {
    println!("cargo:rerun-if-env-changed=ASTER_RELEASE_TRUSTED_KEYS_JSON");
    println!("cargo:rerun-if-env-changed=ASTER_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=ASTER_BUILD_TIMESTAMP");
    println!("cargo:rerun-if-env-changed=ASTER_RELEASE_CREATED_AT");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=src");
    let manifest_directory = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is provided by Cargo"),
    );
    register_git_inputs(&manifest_directory);
    let release_keyring =
        env::var("ASTER_RELEASE_TRUSTED_KEYS_JSON").unwrap_or_else(|_| "[]".to_owned());
    let release = env::var("PROFILE").is_ok_and(|profile| profile == "release");
    if release && release_keyring.trim() == "[]" {
        panic!("release customer builds require ASTER_RELEASE_TRUSTED_KEYS_JSON");
    }
    let output_directory =
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is provided by Cargo"));
    let output = output_directory.join("trusted_release_keys.rs");
    fs::write(
        output,
        format!(
            "pub const COMPILED_RELEASE_KEYS_JSON: &str = {:?};\n",
            release_keyring
        ),
    )
    .expect("write compiled Release keyring");
    fs::write(
        output_directory.join("build_info.rs"),
        format!(
            "pub const BUILD_COMMIT: &str = {:?};\npub const BUILD_TIMESTAMP: &str = {:?};\n",
            build_commit(&manifest_directory),
            build_timestamp(),
        ),
    )
    .expect("write CLI build metadata");
}
