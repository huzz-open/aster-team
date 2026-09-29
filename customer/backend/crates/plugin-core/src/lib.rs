//! Signed, in-memory Lua bundles for the Control protocol adapter.
//!
//! This crate does not grant plugins network, credential, filesystem, or routing
//! access. The host supplies bounded JSON inputs and interprets the returned
//! intent only after its own authorization and schema checks.
#![forbid(unsafe_code)]

mod bundle;
pub mod contract;
mod runtime;
mod update;

pub use bundle::{
    Bundle, BundleDisplay, BundleError, BundleFile, BundleManifest, TrustedPublisher, verify_bundle,
};
pub use contract::{
    AdaptationChange, AdaptationPlan, AdapterError, CanonicalOperation, CanonicalResultV2,
    CanonicalUsageV2, ChannelDescriptor, CompatibilityMode, HttpIntent, PluginDescriptor,
    PluginResult, validate_http_intent,
};
pub use runtime::{ExecutionLimits, LuaRuntime, RuntimeError};
pub use update::{CandidateOutcome, PluginSlot};

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}").expect("writing hex to String cannot fail");
    }
    encoded
}
