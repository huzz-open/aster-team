use std::{
    fs::File,
    io::Read,
    path::Path,
    sync::{Arc, Mutex, RwLock},
};

use crate::sha256_hex;

use crate::{
    Bundle, BundleError, ExecutionLimits, LuaRuntime, TrustedPublisher, bundle::verify_bundle,
};

const MAX_ARCHIVE: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateOutcome {
    Absent,
    Unchanged,
    Rejected { digest: String, reason: String },
    Activated { digest: String },
}

struct SlotState {
    active: Option<Arc<Bundle>>,
    last_submission: Option<String>,
    last_outcome: CandidateOutcome,
}

/// In-memory version switch. Each request clones the active Arc once, so an
/// in-flight workflow retains its original source after activation.
pub struct PluginSlot {
    state: RwLock<SlotState>,
    update_lock: Mutex<()>,
    publishers: Vec<TrustedPublisher>,
    host_version: String,
    canonical_schema: u32,
    limits: ExecutionLimits,
}

impl PluginSlot {
    pub fn new(
        publishers: Vec<TrustedPublisher>,
        host_version: String,
        canonical_schema: u32,
        limits: ExecutionLimits,
    ) -> Self {
        Self {
            state: RwLock::new(SlotState {
                active: None,
                last_submission: None,
                last_outcome: CandidateOutcome::Absent,
            }),
            update_lock: Mutex::new(()),
            publishers,
            host_version,
            canonical_schema,
            limits,
        }
    }

    pub fn active(&self) -> Option<Arc<Bundle>> {
        self.state
            .read()
            .expect("plugin slot poisoned")
            .active
            .clone()
    }

    pub fn inspect_archive(&self, bytes: &[u8]) -> Result<Bundle, BundleError> {
        verify_bundle(
            bytes,
            &self.publishers,
            &self.host_version,
            self.canonical_schema,
        )
    }

    pub fn last_outcome(&self) -> CandidateOutcome {
        self.state
            .read()
            .expect("plugin slot poisoned")
            .last_outcome
            .clone()
    }

    /// Poll the fixed same-name submission path. A malformed partial copy never
    /// reaches Lua; a failed complete submission is not retried until bytes
    /// change or the caller explicitly invokes submit again.
    pub fn poll(
        &self,
        path: &Path,
        persist: impl FnOnce(&Bundle, &[u8]) -> Result<(), String>,
    ) -> CandidateOutcome {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return CandidateOutcome::Absent;
            }
            Err(error) => {
                return CandidateOutcome::Rejected {
                    digest: String::new(),
                    reason: error.to_string(),
                };
            }
        };
        let mut bytes = Vec::new();
        if let Err(error) = file.take(MAX_ARCHIVE + 1).read_to_end(&mut bytes) {
            return CandidateOutcome::Rejected {
                digest: String::new(),
                reason: error.to_string(),
            };
        }
        let digest = sha256_hex(&bytes);
        if self
            .state
            .read()
            .expect("plugin slot poisoned")
            .last_submission
            .as_deref()
            == Some(&digest)
        {
            return CandidateOutcome::Unchanged;
        }
        self.submit_inner(&bytes, digest, persist)
    }

    /// Explicit upload is a new submission even when its bytes match a prior
    /// failed submission. The caller must hold the Control upgrade permit for
    /// the entire validation/self-test/activation transaction.
    pub fn submit(
        &self,
        bytes: &[u8],
        persist: impl FnOnce(&Bundle, &[u8]) -> Result<(), String>,
    ) -> CandidateOutcome {
        let digest = sha256_hex(bytes);
        self.submit_inner(bytes, digest, persist)
    }

    fn submit_inner(
        &self,
        bytes: &[u8],
        digest: String,
        persist: impl FnOnce(&Bundle, &[u8]) -> Result<(), String>,
    ) -> CandidateOutcome {
        let _guard = self
            .update_lock
            .lock()
            .expect("plugin update lock poisoned");
        let outcome = match verify_bundle(
            bytes,
            &self.publishers,
            &self.host_version,
            self.canonical_schema,
        ) {
            Ok(bundle) => {
                let runtime = LuaRuntime::new(Arc::new(bundle.clone()), self.limits);
                match runtime.self_test() {
                    Ok(()) => match persist(&bundle, bytes) {
                        Ok(()) => {
                            self.state.write().expect("plugin slot poisoned").active =
                                Some(Arc::new(bundle));
                            CandidateOutcome::Activated {
                                digest: digest.clone(),
                            }
                        }
                        Err(reason) => CandidateOutcome::Rejected {
                            digest: digest.clone(),
                            reason,
                        },
                    },
                    Err(error) => CandidateOutcome::Rejected {
                        digest: digest.clone(),
                        reason: error.to_string(),
                    },
                }
            }
            Err(error) => CandidateOutcome::Rejected {
                digest: digest.clone(),
                reason: display_bundle_error(error),
            },
        };
        let mut state = self.state.write().expect("plugin slot poisoned");
        state.last_submission = Some(digest);
        state.last_outcome = outcome.clone();
        outcome
    }
}

fn display_bundle_error(error: BundleError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::signed_fixture;

    #[test]
    fn rejected_submission_keeps_prior_version_and_does_not_retry() {
        let (first, key) = signed_fixture(b"return { self_test = function() return true end }");
        let slot = PluginSlot::new(vec![key], "2.1.1".into(), 2, ExecutionLimits::default());
        assert!(matches!(
            slot.submit(&first, |_, _| Ok(())),
            CandidateOutcome::Activated { .. }
        ));
        let prior = slot.active().unwrap().digest.clone();
        let (invalid, _) = signed_fixture(b"return { self_test = function() return false end }");
        assert!(matches!(
            slot.submit(&invalid, |_, _| Ok(())),
            CandidateOutcome::Rejected { .. }
        ));
        assert_eq!(slot.active().unwrap().digest, prior);
        let path =
            std::env::temp_dir().join(format!("aster-plugin-{}.asterlua", std::process::id()));
        std::fs::write(&path, &invalid).unwrap();
        assert_eq!(slot.poll(&path, |_, _| Ok(())), CandidateOutcome::Unchanged);
        std::fs::remove_file(&path).unwrap();
    }
}
