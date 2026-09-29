//! Test handoff for actual Operations-issued v2 documents. This example is not
//! shipped in Customer packages and cannot install or activate a License.
use std::{env, error::Error, fs, io, process::ExitCode};

use aster_license_core::{TrustedLicenseKeys, v2};

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: verify_issued_license <document.json> <trusted-public-profiles.json>",
        )
        .into());
    }
    let trusted = TrustedLicenseKeys::from_json(&fs::read(&args[1])?)?;
    let verified = v2::verify(&fs::read(&args[0])?, &trusted)?;
    // Output only claims authenticated by the production Rust verifier, never
    // the unverified parsed input. Runtime time/installation checks are separate.
    println!("{}", serde_json::to_string(verified.claims())?);
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
