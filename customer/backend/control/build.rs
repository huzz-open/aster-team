use std::{env, fs, path::PathBuf};

#[path = "build/entrypoint_audit.rs"]
mod entrypoint_audit;

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=build/entrypoint_audit.rs");
    for source in entrypoint_audit::audit_crate(&PathBuf::from("src"))
        .expect("all Control routes must use the classified entrypoint registry")
    {
        println!("cargo:rerun-if-changed={}", source.display());
    }
    println!("cargo:rerun-if-env-changed=ASTER_LICENSE_TRUSTED_KEYS_JSON");
    println!("cargo:rerun-if-env-changed=ASTER_RELEASE_TRUSTED_KEYS_JSON");
    println!("cargo:rerun-if-env-changed=ASTER_PLUGIN_TRUSTED_KEYS_JSON");
    let license_keyring =
        env::var("ASTER_LICENSE_TRUSTED_KEYS_JSON").unwrap_or_else(|_| "[]".to_owned());
    let release_keyring =
        env::var("ASTER_RELEASE_TRUSTED_KEYS_JSON").unwrap_or_else(|_| "[]".to_owned());
    let plugin_keyring =
        env::var("ASTER_PLUGIN_TRUSTED_KEYS_JSON").unwrap_or_else(|_| "[]".to_owned());
    let release = env::var("PROFILE").is_ok_and(|profile| profile == "release");
    let local_demo = env::var_os("CARGO_FEATURE_LOCAL_DEMO").is_some();
    if release && !local_demo {
        if license_keyring.trim() == "[]" {
            panic!("release customer builds require ASTER_LICENSE_TRUSTED_KEYS_JSON");
        }
        if release_keyring.trim() == "[]" {
            panic!("release customer builds require ASTER_RELEASE_TRUSTED_KEYS_JSON");
        }
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is provided by Cargo"))
        .join("trusted_license_keys.rs");
    fs::write(
        output,
        format!(
            "pub const COMPILED_LICENSE_KEYS_JSON: &str = {:?};\n\
             pub const COMPILED_RELEASE_KEYS_JSON: &str = {:?};\n\
             pub const COMPILED_PLUGIN_KEYS_JSON: &str = {:?};\n",
            license_keyring, release_keyring, plugin_keyring
        ),
    )
    .expect("write compiled license keyring");
}
