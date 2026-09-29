use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

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

fn register_git_path(manifest_directory: &Path, name: &str) -> bool {
    let Some(path) = git_output(manifest_directory, &["rev-parse", "--git-path", name]) else {
        return false;
    };
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        manifest_directory.join(path)
    };
    if !path.is_file() {
        return false;
    }
    println!("cargo:rerun-if-changed={}", path.display());
    true
}

fn register_git_inputs(manifest_directory: &Path) {
    register_git_path(manifest_directory, "HEAD");
    if let Some(reference) = git_output(manifest_directory, &["symbolic-ref", "-q", "HEAD"])
        && !register_git_path(manifest_directory, &reference)
    {
        register_git_path(manifest_directory, "packed-refs");
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

fn main() {
    println!("cargo:rerun-if-env-changed=ASTER_BUILD_COMMIT");
    println!("cargo:rerun-if-changed=src");
    let manifest_directory = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is provided by Cargo"),
    );
    register_git_inputs(&manifest_directory);
    let output_directory =
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is provided by Cargo"));
    fs::write(
        output_directory.join("build_info.rs"),
        format!(
            "pub const BUILD_COMMIT: &str = {:?};\n",
            build_commit(&manifest_directory),
        ),
    )
    .expect("write asterctl build metadata");
}
