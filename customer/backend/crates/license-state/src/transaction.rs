use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use aster_license_core::{VerifiedProductLicense, canonicalize};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit as _, Mac as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use time::OffsetDateTime;

use super::{
    LicenseImportAction, LicenseImportAudit, LicenseStateError, LicenseStateStore,
    MAX_LICENSE_DOCUMENT_BYTES, PendingLicenseActivation, StagedActivationOutcome, StateClaims,
    format_time, parse_time, state_from_license, validate_license_progress, validate_observation,
};

pub(super) const MAX_STATE_BYTES: usize = 32 * 1024;
const MAX_JOURNAL_BYTES: usize = 192 * 1024;
const MAX_STAGED_BYTES: usize = 128 * 1024;
const MAX_ACTIVATION_AUDIT_BYTES: usize = 32 * 1024;
const MAX_PENDING_ACTIVATION_AUDITS: usize = 64;
const UPDATE_SCHEMA: &str = "aster.license-update.v1";
const IMPORT_UPDATE_SCHEMA: &str = "aster.license-update.v2";
const STAGED_UPDATE_SCHEMA: &str = "aster.staged-license-update.v1";
const STAGED_SCHEMA: &str = "aster.staged-license.v1";
const ACTIVATION_AUDIT_SCHEMA: &str = "aster.license-activation-audit.v1";
const IMPORT_AUDIT_SCHEMA: &str = "aster.license-audit.v2";

/// The stable lock file is never renamed or removed. Separate store instances
/// and CLI processes therefore coordinate on the same OS lock, not just Arc.
pub(super) struct StateGuard<'a> {
    _file: File,
    _local: MutexGuard<'a, ()>,
}

/// Holds the current authorization stable until a short business transaction
/// has finished. The OS releases the lease on drop or process exit. It must
/// never cover an upstream request, streaming response, or interactive work.
#[must_use]
pub struct LicenseMutationGuard {
    _file: File,
}

/// Exclusive lease retained while Control checks all persisted quotas. It
/// cannot be constructed by a caller or reused for a different store/snapshot.
#[must_use]
pub struct FreeSwitchGuard {
    _mutation: LicenseMutationGuard,
    domain: Arc<Mutex<()>>,
    target: PathBuf,
    current: VerifiedProductLicense,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateClaims {
    schema: String,
    target: String,
    previous_license_sha256: Option<String>,
    previous_state_sha256: Option<String>,
    license: String,
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activation_audits: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_audits_sha256: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateWire {
    claims: UpdateClaims,
    mac: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StagedUpdateClaims {
    schema: String,
    target: String,
    previous_staged_sha256: Option<String>,
    previous_audits_sha256: Option<String>,
    staged: String,
    activation_audits: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StagedUpdateWire {
    claims: StagedUpdateClaims,
    mac: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StagedClaims {
    schema: String,
    target: String,
    license: String,
    license_sha256: String,
    license_id: String,
    issued_at: String,
    transfer_sequence: u32,
    not_before: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StagedWire {
    claims: StagedClaims,
    mac: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivationAuditClaims {
    schema: String,
    target: String,
    events: Vec<PendingLicenseActivation>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivationAuditWire {
    claims: ActivationAuditClaims,
    mac: String,
}

impl LicenseStateStore {
    pub(super) fn lock_mutations(
        &self,
        shared: bool,
    ) -> Result<LicenseMutationGuard, LicenseStateError> {
        let path = self.sidecar(".mutation");
        fs::create_dir_all(parent_directory(&path)?).map_err(|_| LicenseStateError::Io)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o640);
        }
        let file = options.open(path).map_err(|_| LicenseStateError::Io)?;
        // Never wait on a database transaction from a synchronous license
        // reader on the async runtime. New writers fail before recording an
        // intent; recovery of an existing intent retries after admissions finish.
        let result = if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        result.map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => LicenseStateError::MutationConflict,
            std::fs::TryLockError::Error(_) => LicenseStateError::Io,
        })?;
        Ok(LicenseMutationGuard { _file: file })
    }

    /// Rechecks the exact admitted snapshot under the state lock and retains
    /// a separate shared process lease through the caller's database commit.
    /// `None` is restricted to fresh-install bootstrap, never an unlicensed
    /// business request. When configured, the installed file must also match.
    pub fn guard_mutation(
        &self,
        license: Option<&VerifiedProductLicense>,
        target: Option<&Path>,
        now: OffsetDateTime,
    ) -> Result<LicenseMutationGuard, LicenseStateError> {
        let _state = self.lock_state()?;
        let mutation = self.lock_mutations(true)?;
        self.require_no_pending_update()?;
        match license {
            Some(license) => {
                let current = self.load()?;
                if current.last_license_sha256.as_deref()
                    != Some(super::license_digest(license).as_str())
                {
                    return Err(LicenseStateError::MutationConflict);
                }
                validate_observation(&current, license, now)?;
            }
            None => {
                if read_optional(&self.path, MAX_STATE_BYTES)?.is_some() {
                    return Err(LicenseStateError::MutationConflict);
                }
            }
        }
        if let Some(target) = target {
            self.validate_target(target)?;
            if read_optional(target, MAX_LICENSE_DOCUMENT_BYTES)?.as_deref()
                != license.map(VerifiedProductLicense::source)
            {
                return Err(LicenseStateError::MutationConflict);
            }
        }
        Ok(mutation)
    }

    pub(super) fn lock_state(&self) -> Result<StateGuard<'_>, LicenseStateError> {
        let local = self.gate.lock().map_err(|_| LicenseStateError::Io)?;
        let path = self.sidecar(".lock");
        fs::create_dir_all(parent_directory(&path)?).map_err(|_| LicenseStateError::Io)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o640);
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(LicenseStateError::Io);
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(LicenseStateError::Io);
            }
            _ => {}
        }
        let file = options.open(path).map_err(|_| LicenseStateError::Io)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(fs::TryLockError::WouldBlock) => return Err(LicenseStateError::Busy),
                Err(fs::TryLockError::Error(_)) => return Err(LicenseStateError::Io),
            }
        }
        Ok(StateGuard {
            _file: file,
            _local: local,
        })
    }

    /// Start an explicit administrator-requested switch. An expired current
    /// License is allowed, but missing/corrupt history or a stale snapshot is not.
    pub fn begin_free_switch(
        &self,
        target: &Path,
        current: &VerifiedProductLicense,
        now: OffsetDateTime,
    ) -> Result<FreeSwitchGuard, LicenseStateError> {
        let _state = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        let mutation = self.lock_mutations(false)?;
        self.require_no_pending_update()?;
        if !matches!(current, VerifiedProductLicense::V2(_)) {
            return Err(LicenseStateError::LicenseRollback);
        }
        self.validate_switch_snapshot(target, current, now)?;
        Ok(FreeSwitchGuard {
            _mutation: mutation,
            domain: Arc::clone(&self.gate),
            target: target.to_owned(),
            current: current.clone(),
        })
    }

    fn validate_switch_snapshot(
        &self,
        target: &Path,
        current: &VerifiedProductLicense,
        now: OffsetDateTime,
    ) -> Result<(), LicenseStateError> {
        let history = self.load()?;
        if history.last_license_sha256.as_deref() != Some(digest(current.source()).as_str())
            || read_optional(target, MAX_LICENSE_DOCUMENT_BYTES)?.as_deref()
                != Some(current.source())
        {
            return Err(LicenseStateError::MutationConflict);
        }
        validate_observation(&history, current, now)
    }

    /// The caller must check resource counts while holding `lease`. The
    /// same recoverable journal commits free rights, preserved paid watermark,
    /// and the administrator audit together. Scheduled paid rights stay intact.
    pub fn commit_free_switch(
        &self,
        lease: FreeSwitchGuard,
        free: &VerifiedProductLicense,
        now: OffsetDateTime,
        audit: LicenseImportAudit,
    ) -> Result<(), LicenseStateError> {
        let _state = self.lock_state()?;
        if !Arc::ptr_eq(&self.gate, &lease.domain) {
            return Err(LicenseStateError::MutationConflict);
        }
        self.require_no_pending_update()?;
        self.validate_switch_snapshot(&lease.target, &lease.current, now)?;
        let update = self.prepare_free_switch(&lease.target, &lease.current, free, now, audit)?;
        self.write_update_intent(&update)?;
        self.finish_update_locked(&lease.target, &update.claims)
    }

    fn prepare_free_switch(
        &self,
        target: &Path,
        current: &VerifiedProductLicense,
        free: &VerifiedProductLicense,
        now: OffsetDateTime,
        audit: LicenseImportAudit,
    ) -> Result<UpdateWire, LicenseStateError> {
        if free.source().is_empty() || free.source().len() > MAX_LICENSE_DOCUMENT_BYTES {
            return Err(LicenseStateError::InvalidJson);
        }
        if !super::is_public_free(free)
            || parse_time(free.as_ref().not_before())? > now
            || audit.action != LicenseImportAction::SwitchFree
        {
            return Err(LicenseStateError::LicenseRollback);
        }
        let mut update = self.prepare_update(target, current, now, false)?;
        let mut history =
            self.decode_state(&decode_bytes(&update.claims.state, MAX_STATE_BYTES)?)?;
        // Keep the previously accepted paid issue position even when the
        // public free file is newer. Otherwise an already-staged paid renewal
        // could be mistaken for a replay when its activation time arrives.
        history.last_license_id = Some(free.as_ref().license_id().to_owned());
        history.last_license_sha256 = Some(digest(free.source()));
        history.seat_over_limit_first_observed_at = None;
        update.claims.license = URL_SAFE_NO_PAD.encode(free.source());
        update.claims.state = URL_SAFE_NO_PAD.encode(self.encode_state(&history)?);
        self.attach_import_audit(&mut update, target, free, now, audit)?;
        Ok(update)
    }

    fn sidecar(&self, suffix: &str) -> PathBuf {
        let mut path = self.path.as_os_str().to_os_string();
        path.push(suffix);
        PathBuf::from(path)
    }

    pub(super) fn require_no_pending_update(&self) -> Result<(), LicenseStateError> {
        match read_optional(&self.sidecar(".pending"), MAX_JOURNAL_BYTES)? {
            Some(source) if !source.is_empty() => Err(LicenseStateError::RecoveryRequired),
            _ => Ok(()),
        }
    }

    /// Commits the exact verified document and its history under one recoverable
    /// transaction. A returned IO error after preparation is an uncertain result;
    /// the next read/retry completes the recorded intent, never resets history.
    pub fn install_document(
        &self,
        target: &Path,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        let _mutation = self.lock_mutations(false)?;
        let update = self.prepare_update(target, license, now, false)?;
        let encoded = serde_json::to_vec(&update).map_err(|_| LicenseStateError::InvalidJson)?;
        write_atomic(&self.sidecar(".pending"), &encoded)?;
        self.finish_update_locked(target, &update.claims)
    }

    /// Active import and its actor record share the same durable intent.
    pub fn install_document_with_audit(
        &self,
        target: &Path,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
        audit: LicenseImportAudit,
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        let _mutation = self.lock_mutations(false)?;
        let update = self.prepare_import_update(target, license, now, audit)?;
        self.write_update_intent(&update)?;
        self.finish_update_locked(target, &update.claims)
    }

    fn prepare_import_update(
        &self,
        target: &Path,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
        audit: LicenseImportAudit,
    ) -> Result<UpdateWire, LicenseStateError> {
        let mut update = self.prepare_update(target, license, now, false)?;
        if audit.action != LicenseImportAction::Install {
            return Err(LicenseStateError::InvalidJson);
        }
        self.attach_import_audit(&mut update, target, license, now, audit)?;
        Ok(update)
    }

    fn attach_import_audit(
        &self,
        update: &mut UpdateWire,
        target: &Path,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
        audit: LicenseImportAudit,
    ) -> Result<(), LicenseStateError> {
        let audits = self.prepare_import_audits(target, license, now, audit)?;
        update.claims.schema = IMPORT_UPDATE_SCHEMA.to_owned();
        update.claims.previous_audits_sha256 =
            read_optional(&self.sidecar(".activation"), MAX_ACTIVATION_AUDIT_BYTES)?
                .as_deref()
                .map(digest);
        update.claims.activation_audits = Some(URL_SAFE_NO_PAD.encode(audits));
        update.mac =
            URL_SAFE_NO_PAD.encode(self.update_mac(&update.claims)?.finalize().into_bytes());
        Ok(())
    }

    fn write_update_intent(&self, wire: &impl Serialize) -> Result<(), LicenseStateError> {
        let encoded = serde_json::to_vec(wire).map_err(|_| LicenseStateError::InvalidJson)?;
        if encoded.len() > MAX_JOURNAL_BYTES {
            return Err(LicenseStateError::InvalidJson);
        }
        write_atomic(&self.sidecar(".pending"), &encoded)
    }

    /// Reads after finishing any durable update intent. The caller must still
    /// verify the returned License signature, scope, binding and currentness.
    pub fn read_committed_document(
        &self,
        target: &Path,
    ) -> Result<Option<Vec<u8>>, LicenseStateError> {
        let _guard = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        let source = read_optional(target, MAX_LICENSE_DOCUMENT_BYTES)?;
        match (&source, read_optional(&self.path, MAX_STATE_BYTES)?) {
            (Some(_), Some(history)) => {
                self.decode_state(&history)?;
            }
            (None, None) => {}
            _ => return Err(LicenseStateError::Missing),
        }
        Ok(source)
    }

    /// Stores a verified future-effective document without advancing active
    /// history. The exact source and ordering metadata are protected with the
    /// installation-state key. Replacing an intact staged document must move
    /// forward under the same identity rules as an active replacement.
    pub fn stage_document(
        &self,
        target: &Path,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        let wire = self.prepare_staged(target, license, now)?;
        let encoded = serde_json::to_vec(&wire).map_err(|_| LicenseStateError::InvalidJson)?;
        write_atomic(&self.sidecar(".staged"), &encoded)
    }

    pub fn stage_document_with_audit(
        &self,
        target: &Path,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
        audit: LicenseImportAudit,
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        let _mutation = self.lock_mutations(false)?;
        if audit.action != LicenseImportAction::Schedule {
            return Err(LicenseStateError::InvalidJson);
        }
        let update = self.prepare_staged_update(target, license, now, audit)?;
        self.write_update_intent(&update)?;
        self.finish_staged_update(target, &update.claims)
    }

    fn prepare_staged_update(
        &self,
        target: &Path,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
        audit: LicenseImportAudit,
    ) -> Result<StagedUpdateWire, LicenseStateError> {
        let staged = serde_json::to_vec(&self.prepare_staged(target, license, now)?)
            .map_err(|_| LicenseStateError::InvalidJson)?;
        let audits = self.prepare_import_audits(target, license, now, audit)?;
        let claims = StagedUpdateClaims {
            schema: STAGED_UPDATE_SCHEMA.to_owned(),
            target: normalized_target(target)?,
            previous_staged_sha256: read_optional(&self.sidecar(".staged"), MAX_STAGED_BYTES)?
                .as_deref()
                .map(digest),
            previous_audits_sha256: read_optional(
                &self.sidecar(".activation"),
                MAX_ACTIVATION_AUDIT_BYTES,
            )?
            .as_deref()
            .map(digest),
            staged: URL_SAFE_NO_PAD.encode(staged),
            activation_audits: URL_SAFE_NO_PAD.encode(audits),
        };
        Ok(StagedUpdateWire {
            mac: URL_SAFE_NO_PAD.encode(self.staged_update_mac(&claims)?.finalize().into_bytes()),
            claims,
        })
    }

    fn prepare_staged(
        &self,
        target: &Path,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
    ) -> Result<StagedWire, LicenseStateError> {
        if license.source().is_empty() || license.source().len() > MAX_LICENSE_DOCUMENT_BYTES {
            return Err(LicenseStateError::InvalidJson);
        }

        let active_document = read_optional(target, MAX_LICENSE_DOCUMENT_BYTES)?;
        match read_optional(&self.path, MAX_STATE_BYTES)? {
            Some(source) => {
                let current = self.decode_state(&source)?;
                validate_observation(&current, license, now)?;
            }
            None if active_document.is_none() => {}
            None => return Err(LicenseStateError::Missing),
        }

        let path = self.sidecar(".staged");
        if let Some(source) = read_optional(&path, MAX_STAGED_BYTES)?
            && !source.is_empty()
            && let Ok(previous) = self.decode_staged(target, &source)
        {
            let previous_history = StateClaims {
                schema: super::LICENSE_STATE_SCHEMA.to_owned(),
                last_license_sha256: Some(previous.license_sha256),
                last_license_id: Some(previous.license_id),
                last_issued_at: previous.issued_at,
                last_transfer_sequence: previous.transfer_sequence,
                last_seen_at: format_time(now)?,
                seat_over_limit_first_observed_at: None,
            };
            validate_license_progress(&previous_history, license)?;
        }

        let claims = StagedClaims {
            schema: STAGED_SCHEMA.to_owned(),
            target: normalized_target(target)?,
            license: URL_SAFE_NO_PAD.encode(license.source()),
            license_sha256: digest(license.source()),
            license_id: license.as_ref().license_id().to_owned(),
            issued_at: license.as_ref().issued_at().to_owned(),
            transfer_sequence: license.as_ref().transfer_sequence(),
            not_before: license.as_ref().not_before().to_owned(),
        };
        Ok(StagedWire {
            mac: URL_SAFE_NO_PAD.encode(self.staged_mac(&claims)?.finalize().into_bytes()),
            claims,
        })
    }

    /// Reads the exact future-effective document. Invalid staged state is
    /// reported separately so callers can keep serving the active license.
    pub fn read_staged_document(
        &self,
        target: &Path,
    ) -> Result<Option<Vec<u8>>, LicenseStateError> {
        let _guard = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        let Some(source) = read_optional(&self.sidecar(".staged"), MAX_STAGED_BYTES)? else {
            return Ok(None);
        };
        if source.is_empty() {
            return Ok(None);
        }
        let staged = self.decode_staged(target, &source)?;
        decode_bytes(&staged.license, MAX_LICENSE_DOCUMENT_BYTES).map(Some)
    }

    /// Clears only the staged bytes the caller has already processed. A newer
    /// concurrent stage is preserved for its own activation time.
    pub fn clear_staged_document(
        &self,
        target: &Path,
        expected: &[u8],
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        let path = self.sidecar(".staged");
        let Some(source) = read_optional(&path, MAX_STAGED_BYTES)? else {
            return Ok(());
        };
        if source.is_empty() {
            return Ok(());
        }
        let staged = self.decode_staged(target, &source)?;
        let document = decode_bytes(&staged.license, MAX_LICENSE_DOCUMENT_BYTES)?;
        if document == expected {
            write_atomic(&path, &[])?;
        }
        Ok(())
    }

    /// Returns durable automatic-activation records that still need to be
    /// copied into the Customer audit chain.
    pub fn pending_activation_audits(
        &self,
        target: &Path,
    ) -> Result<Vec<PendingLicenseActivation>, LicenseStateError> {
        let _guard = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        self.read_activation_audits(target)
    }

    /// Removes one record only after its matching Customer audit event is
    /// durable. Other pending activations are preserved.
    pub fn acknowledge_activation_audit(
        &self,
        target: &Path,
        expected_sha256: &str,
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        let mut events = self.read_activation_audits(target)?;
        let before = events.len();
        events.retain(|event| event.import.is_some() || event.license_sha256 != expected_sha256);
        if events.len() == before {
            return Ok(());
        }
        let encoded = self.encode_activation_audits(target, events)?;
        write_atomic(&self.sidecar(".activation"), &encoded)
    }

    pub fn acknowledge_license_audit(
        &self,
        target: &Path,
        expected_event_id: &str,
    ) -> Result<(), LicenseStateError> {
        let _guard = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        let mut events = self.read_activation_audits(target)?;
        let before = events.len();
        events.retain(|event| event.event_id() != expected_event_id);
        if events.len() != before {
            let encoded = self.encode_activation_audits(target, events)?;
            write_atomic(&self.sidecar(".activation"), &encoded)?;
        }
        Ok(())
    }

    /// Atomically compares and activates the exact staged document under the
    /// same stable OS lock used by active installs. A stage replaced by another
    /// process is never cleared or activated by the old reader.
    pub fn activate_staged_document(
        &self,
        target: &Path,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
    ) -> Result<StagedActivationOutcome, LicenseStateError> {
        let _guard = self.lock_state()?;
        self.validate_target(target)?;
        self.recover_update(target)?;
        let staged_path = self.sidecar(".staged");
        let Some(source) = read_optional(&staged_path, MAX_STAGED_BYTES)? else {
            return Ok(StagedActivationOutcome::Changed);
        };
        if source.is_empty() {
            return Ok(StagedActivationOutcome::Changed);
        }
        let staged = self.decode_staged(target, &source)?;
        let document = decode_bytes(&staged.license, MAX_LICENSE_DOCUMENT_BYTES)?;
        if document != license.source() {
            return Ok(StagedActivationOutcome::Changed);
        }
        if read_optional(target, MAX_LICENSE_DOCUMENT_BYTES)?.as_deref()
            == Some(document.as_slice())
        {
            let history = read_optional(&self.path, MAX_STATE_BYTES)?
                .ok_or(LicenseStateError::Missing)
                .and_then(|source| self.decode_state(&source))?;
            if history.last_license_sha256.as_deref() == Some(digest(&document).as_str()) {
                let staged_cleared = write_atomic(&staged_path, &[]).is_ok();
                return Ok(StagedActivationOutcome::AlreadyActive { staged_cleared });
            }
        }
        let _mutation = self.lock_mutations(false)?;
        let update = match self.prepare_update(target, license, now, true) {
            Ok(update) => update,
            Err(LicenseStateError::LicenseRollback) => {
                let staged_cleared = write_atomic(&staged_path, &[]).is_ok();
                return Ok(StagedActivationOutcome::Superseded { staged_cleared });
            }
            Err(error) => return Err(error),
        };
        let encoded = serde_json::to_vec(&update).map_err(|_| LicenseStateError::InvalidJson)?;
        write_atomic(&self.sidecar(".pending"), &encoded)?;
        self.finish_update_locked(target, &update.claims)?;
        // Activation is already durable. Leaving the signed stage behind is
        // safe and retriable, so optional cleanup cannot revoke the active file.
        let staged_cleared = write_atomic(&staged_path, &[]).is_ok();
        Ok(StagedActivationOutcome::Activated { staged_cleared })
    }

    fn validate_target(&self, target: &Path) -> Result<(), LicenseStateError> {
        let target = normalized_target(target)?;
        for reserved in [
            &self.path,
            &self.sidecar(".lock"),
            &self.sidecar(".mutation"),
            &self.sidecar(".pending"),
            &self.sidecar(".staged"),
            &self.sidecar(".activation"),
        ] {
            let reserved = normalized_target(reserved)?;
            let equal = if cfg!(windows) {
                target.eq_ignore_ascii_case(&reserved)
            } else {
                target == reserved
            };
            if equal {
                return Err(LicenseStateError::Integrity);
            }
        }
        Ok(())
    }

    fn prepare_update(
        &self,
        target: &Path,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
        record_activation: bool,
    ) -> Result<UpdateWire, LicenseStateError> {
        if license.source().is_empty() || license.source().len() > MAX_LICENSE_DOCUMENT_BYTES {
            return Err(LicenseStateError::InvalidJson);
        }
        let old_license = read_optional(target, MAX_LICENSE_DOCUMENT_BYTES)?;
        let old_state = read_optional(&self.path, MAX_STATE_BYTES)?;
        let updated = match old_state.as_deref() {
            Some(source) => {
                let current = self.decode_state(source)?;
                super::observe(current, license, now)?
            }
            None if old_license.is_none() => state_from_license(license, now)?,
            None => return Err(LicenseStateError::Missing),
        };
        let claims = UpdateClaims {
            schema: if record_activation {
                IMPORT_UPDATE_SCHEMA
            } else {
                UPDATE_SCHEMA
            }
            .to_owned(),
            target: normalized_target(target)?,
            previous_license_sha256: old_license.as_deref().map(digest),
            previous_state_sha256: old_state.as_deref().map(digest),
            license: URL_SAFE_NO_PAD.encode(license.source()),
            state: URL_SAFE_NO_PAD.encode(self.encode_state(&updated)?),
            previous_audits_sha256: if record_activation {
                read_optional(&self.sidecar(".activation"), MAX_ACTIVATION_AUDIT_BYTES)?
                    .as_deref()
                    .map(digest)
            } else {
                None
            },
            activation_audits: if record_activation {
                let mut events = self.read_activation_audits(target)?;
                let license_sha256 = digest(license.source());
                if !events
                    .iter()
                    .any(|event| event.import.is_none() && event.license_sha256 == license_sha256)
                {
                    if events.len() >= MAX_PENDING_ACTIVATION_AUDITS {
                        return Err(LicenseStateError::Integrity);
                    }
                    events.push(PendingLicenseActivation {
                        license_id: license.as_ref().license_id().to_owned(),
                        license_sha256,
                        activated_at: format_time(now)?,
                        import: None,
                    });
                }
                Some(URL_SAFE_NO_PAD.encode(self.encode_activation_audits(target, events)?))
            } else {
                None
            },
        };
        let mac = self.update_mac(&claims)?;
        Ok(UpdateWire {
            claims,
            mac: URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()),
        })
    }

    fn recover_update(&self, target: &Path) -> Result<(), LicenseStateError> {
        let Some(source) = read_optional(&self.sidecar(".pending"), MAX_JOURNAL_BYTES)? else {
            return Ok(());
        };
        if source.is_empty() {
            return Ok(());
        }
        // This first pass selects only the exact schema. The second pass below
        // rejects every unknown/duplicate field in the selected wire format.
        #[derive(Deserialize)]
        struct Discriminator {
            claims: Schema,
        }
        #[derive(Deserialize)]
        struct Schema {
            schema: String,
        }
        let discriminator: Discriminator =
            serde_json::from_slice(&source).map_err(|_| LicenseStateError::InvalidJson)?;
        if discriminator.claims.schema == STAGED_UPDATE_SCHEMA {
            let wire: StagedUpdateWire =
                serde_json::from_slice(&source).map_err(|_| LicenseStateError::InvalidJson)?;
            let mac = URL_SAFE_NO_PAD
                .decode(&wire.mac)
                .map_err(|_| LicenseStateError::Integrity)?;
            self.staged_update_mac(&wire.claims)?
                .verify_slice(&mac)
                .map_err(|_| LicenseStateError::Integrity)?;
            let _mutation = self.lock_mutations(false)?;
            return self.finish_staged_update(target, &wire.claims);
        }
        if !matches!(
            discriminator.claims.schema.as_str(),
            UPDATE_SCHEMA | IMPORT_UPDATE_SCHEMA
        ) {
            return Err(LicenseStateError::InvalidJson);
        }
        let mut deserializer = serde_json::Deserializer::from_slice(&source);
        let update = UpdateWire::deserialize(&mut deserializer)
            .map_err(|_| LicenseStateError::InvalidJson)?;
        deserializer
            .end()
            .map_err(|_| LicenseStateError::InvalidJson)?;
        let mac = URL_SAFE_NO_PAD
            .decode(&update.mac)
            .map_err(|_| LicenseStateError::Integrity)?;
        self.update_mac(&update.claims)?
            .verify_slice(&mac)
            .map_err(|_| LicenseStateError::Integrity)?;
        self.finish_update(target, &update.claims)
    }

    fn update_mac(&self, claims: &UpdateClaims) -> Result<Hmac<Sha256>, LicenseStateError> {
        let value = serde_json::to_value(claims).map_err(|_| LicenseStateError::Crypto)?;
        let canonical = canonicalize(&value).map_err(|_| LicenseStateError::Crypto)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.state_key.as_ref())
            .map_err(|_| LicenseStateError::Crypto)?;
        mac.update(if claims.schema == IMPORT_UPDATE_SCHEMA {
            b"aster-team-license-update-v2\n"
        } else {
            b"aster-team-license-update-v1\n"
        });
        mac.update(&canonical);
        Ok(mac)
    }

    fn staged_update_mac(
        &self,
        claims: &StagedUpdateClaims,
    ) -> Result<Hmac<Sha256>, LicenseStateError> {
        let value = serde_json::to_value(claims).map_err(|_| LicenseStateError::Crypto)?;
        let canonical = canonicalize(&value).map_err(|_| LicenseStateError::Crypto)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.state_key.as_ref())
            .map_err(|_| LicenseStateError::Crypto)?;
        mac.update(b"aster-team-staged-license-update-v1\n");
        mac.update(&canonical);
        Ok(mac)
    }

    fn prepare_import_audits(
        &self,
        target: &Path,
        license: &VerifiedProductLicense,
        now: OffsetDateTime,
        audit: LicenseImportAudit,
    ) -> Result<Vec<u8>, LicenseStateError> {
        if !audit.valid() {
            return Err(LicenseStateError::InvalidJson);
        }
        let mut events = self.read_activation_audits(target)?;
        events.push(PendingLicenseActivation {
            license_id: license.as_ref().license_id().to_owned(),
            license_sha256: digest(license.source()),
            activated_at: format_time(now)?,
            import: Some(audit),
        });
        self.encode_activation_audits(target, events)
    }

    fn staged_mac(&self, claims: &StagedClaims) -> Result<Hmac<Sha256>, LicenseStateError> {
        let value = serde_json::to_value(claims).map_err(|_| LicenseStateError::Crypto)?;
        let canonical = canonicalize(&value).map_err(|_| LicenseStateError::Crypto)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.state_key.as_ref())
            .map_err(|_| LicenseStateError::Crypto)?;
        mac.update(b"aster-team-staged-license-v1\n");
        mac.update(&canonical);
        Ok(mac)
    }

    fn activation_audit_mac(
        &self,
        claims: &ActivationAuditClaims,
    ) -> Result<Hmac<Sha256>, LicenseStateError> {
        let value = serde_json::to_value(claims).map_err(|_| LicenseStateError::Crypto)?;
        let canonical = canonicalize(&value).map_err(|_| LicenseStateError::Crypto)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.state_key.as_ref())
            .map_err(|_| LicenseStateError::Crypto)?;
        mac.update(if claims.schema == IMPORT_AUDIT_SCHEMA {
            b"aster-team-license-audit-v2\n"
        } else {
            b"aster-team-license-activation-audit-v1\n"
        });
        mac.update(&canonical);
        Ok(mac)
    }

    fn read_activation_audits(
        &self,
        target: &Path,
    ) -> Result<Vec<PendingLicenseActivation>, LicenseStateError> {
        let Some(source) = read_optional(&self.sidecar(".activation"), MAX_ACTIVATION_AUDIT_BYTES)?
        else {
            return Ok(Vec::new());
        };
        if source.is_empty() {
            return Ok(Vec::new());
        }
        self.decode_activation_audits(target, &source)
    }

    fn encode_activation_audits(
        &self,
        target: &Path,
        events: Vec<PendingLicenseActivation>,
    ) -> Result<Vec<u8>, LicenseStateError> {
        let claims = ActivationAuditClaims {
            schema: if events.iter().any(|event| event.import.is_some()) {
                IMPORT_AUDIT_SCHEMA
            } else {
                ACTIVATION_AUDIT_SCHEMA
            }
            .to_owned(),
            target: normalized_target(target)?,
            events,
        };
        let wire = ActivationAuditWire {
            mac: URL_SAFE_NO_PAD
                .encode(self.activation_audit_mac(&claims)?.finalize().into_bytes()),
            claims,
        };
        let source = serde_json::to_vec(&wire).map_err(|_| LicenseStateError::InvalidJson)?;
        if source.len() > MAX_ACTIVATION_AUDIT_BYTES {
            return Err(LicenseStateError::InvalidJson);
        }
        self.decode_activation_audits(target, &source)?;
        Ok(source)
    }

    fn decode_activation_audits(
        &self,
        target: &Path,
        source: &[u8],
    ) -> Result<Vec<PendingLicenseActivation>, LicenseStateError> {
        let mut deserializer = serde_json::Deserializer::from_slice(source);
        let wire = ActivationAuditWire::deserialize(&mut deserializer)
            .map_err(|_| LicenseStateError::InvalidJson)?;
        deserializer
            .end()
            .map_err(|_| LicenseStateError::InvalidJson)?;
        if !matches!(
            wire.claims.schema.as_str(),
            ACTIVATION_AUDIT_SCHEMA | IMPORT_AUDIT_SCHEMA
        ) || (wire.claims.schema == ACTIVATION_AUDIT_SCHEMA
            && wire
                .claims
                .events
                .iter()
                .any(|event| event.import.is_some()))
            || wire.claims.target != normalized_target(target)?
            || wire.claims.events.len() > MAX_PENDING_ACTIVATION_AUDITS
            || wire.claims.events.iter().any(|event| {
                event.license_id.is_empty()
                    || event.license_id.len() > 128
                    || event.license_sha256.len() != 43
                    || parse_time(&event.activated_at).is_err()
                    || event.import.as_ref().is_some_and(|import| !import.valid())
            })
        {
            return Err(LicenseStateError::InvalidJson);
        }
        let mut ids = std::collections::HashSet::new();
        if wire
            .claims
            .events
            .iter()
            .any(|event| !ids.insert(event.event_id()))
        {
            return Err(LicenseStateError::InvalidJson);
        }
        let mac = URL_SAFE_NO_PAD
            .decode(&wire.mac)
            .map_err(|_| LicenseStateError::Integrity)?;
        self.activation_audit_mac(&wire.claims)?
            .verify_slice(&mac)
            .map_err(|_| LicenseStateError::Integrity)?;
        Ok(wire.claims.events)
    }

    fn decode_staged(
        &self,
        target: &Path,
        source: &[u8],
    ) -> Result<StagedClaims, LicenseStateError> {
        let mut deserializer = serde_json::Deserializer::from_slice(source);
        let wire = StagedWire::deserialize(&mut deserializer)
            .map_err(|_| LicenseStateError::InvalidJson)?;
        deserializer
            .end()
            .map_err(|_| LicenseStateError::InvalidJson)?;
        if wire.claims.schema != STAGED_SCHEMA
            || wire.claims.target != normalized_target(target)?
            || wire.claims.license_id.is_empty()
            || wire.claims.license_id.len() > 128
            || wire.claims.transfer_sequence > 10_000
            || parse_time(&wire.claims.issued_at).is_err()
            || parse_time(&wire.claims.not_before).is_err()
        {
            return Err(LicenseStateError::InvalidJson);
        }
        let mac = URL_SAFE_NO_PAD
            .decode(&wire.mac)
            .map_err(|_| LicenseStateError::Integrity)?;
        self.staged_mac(&wire.claims)?
            .verify_slice(&mac)
            .map_err(|_| LicenseStateError::Integrity)?;
        let document = decode_bytes(&wire.claims.license, MAX_LICENSE_DOCUMENT_BYTES)?;
        if digest(&document) != wire.claims.license_sha256 {
            return Err(LicenseStateError::Integrity);
        }
        Ok(wire.claims)
    }

    fn finish_update(&self, target: &Path, update: &UpdateClaims) -> Result<(), LicenseStateError> {
        let _mutation = self.lock_mutations(false)?;
        self.finish_update_locked(target, update)
    }

    fn finish_update_locked(
        &self,
        target: &Path,
        update: &UpdateClaims,
    ) -> Result<(), LicenseStateError> {
        if !matches!(update.schema.as_str(), UPDATE_SCHEMA | IMPORT_UPDATE_SCHEMA)
            || update.target != normalized_target(target)?
        {
            return Err(LicenseStateError::Integrity);
        }
        let license = decode_bytes(&update.license, MAX_LICENSE_DOCUMENT_BYTES)?;
        let history = decode_bytes(&update.state, MAX_STATE_BYTES)?;
        let activation_audits = update
            .activation_audits
            .as_deref()
            .map(|source| decode_bytes(source, MAX_ACTIVATION_AUDIT_BYTES))
            .transpose()?;
        self.decode_state(&history)?;
        if let Some(source) = activation_audits.as_deref() {
            self.decode_activation_audits(target, source)?;
        }
        if update.schema == IMPORT_UPDATE_SCHEMA {
            let audits = activation_audits
                .as_deref()
                .ok_or(LicenseStateError::Integrity)?;
            require_expected_file(
                &self.sidecar(".activation"),
                MAX_ACTIVATION_AUDIT_BYTES,
                update.previous_audits_sha256.as_deref(),
                audits,
            )?;
        } else if update.previous_audits_sha256.is_some() {
            return Err(LicenseStateError::Integrity);
        }
        require_expected_file(
            target,
            MAX_LICENSE_DOCUMENT_BYTES,
            update.previous_license_sha256.as_deref(),
            &license,
        )?;
        require_expected_file(
            &self.path,
            MAX_STATE_BYTES,
            update.previous_state_sha256.as_deref(),
            &history,
        )?;
        // While .pending exists, observations refuse to mix an old snapshot with
        // either half of this update. Recovery tolerates each completed rename.
        write_atomic(&self.path, &history)?;
        write_atomic(target, &license)?;
        if let Some(source) = activation_audits.as_deref() {
            write_atomic(&self.sidecar(".activation"), source)?;
        }
        // A durable empty marker ends the transaction. A plain unlink has no
        // directory durability barrier on Windows and could resurrect an old
        // intent after later observations have advanced the history.
        write_atomic(&self.sidecar(".pending"), &[])
    }

    fn finish_staged_update(
        &self,
        target: &Path,
        update: &StagedUpdateClaims,
    ) -> Result<(), LicenseStateError> {
        if update.schema != STAGED_UPDATE_SCHEMA || update.target != normalized_target(target)? {
            return Err(LicenseStateError::Integrity);
        }
        let staged = decode_bytes(&update.staged, MAX_STAGED_BYTES)?;
        let audits = decode_bytes(&update.activation_audits, MAX_ACTIVATION_AUDIT_BYTES)?;
        self.decode_staged(target, &staged)?;
        self.decode_activation_audits(target, &audits)?;
        require_expected_file(
            &self.sidecar(".staged"),
            MAX_STAGED_BYTES,
            update.previous_staged_sha256.as_deref(),
            &staged,
        )?;
        require_expected_file(
            &self.sidecar(".activation"),
            MAX_ACTIVATION_AUDIT_BYTES,
            update.previous_audits_sha256.as_deref(),
            &audits,
        )?;
        write_atomic(&self.sidecar(".staged"), &staged)?;
        write_atomic(&self.sidecar(".activation"), &audits)?;
        write_atomic(&self.sidecar(".pending"), &[])
    }
}

fn normalized_target(target: &Path) -> Result<String, LicenseStateError> {
    let parent = parent_directory(target)?;
    fs::create_dir_all(parent).map_err(|_| LicenseStateError::Io)?;
    let parent = fs::canonicalize(parent).map_err(|_| LicenseStateError::Io)?;
    let target = parent.join(target.file_name().ok_or(LicenseStateError::Io)?);
    target
        .to_str()
        .map(str::to_owned)
        .ok_or(LicenseStateError::Io)
}

fn digest(source: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(source))
}

fn decode_bytes(source: &str, maximum: usize) -> Result<Vec<u8>, LicenseStateError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(source)
        .map_err(|_| LicenseStateError::Integrity)?;
    if bytes.is_empty() || bytes.len() > maximum {
        return Err(LicenseStateError::InvalidJson);
    }
    Ok(bytes)
}

fn require_expected_file(
    path: &Path,
    maximum: usize,
    previous: Option<&str>,
    next: &[u8],
) -> Result<(), LicenseStateError> {
    let current = read_optional(path, maximum)?;
    let current_hash = current.as_deref().map(digest);
    if current_hash.as_deref() != previous && current.as_deref() != Some(next) {
        return Err(LicenseStateError::Integrity);
    }
    Ok(())
}

pub(super) fn read_optional(
    path: &Path,
    maximum: usize,
) -> Result<Option<Vec<u8>>, LicenseStateError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(LicenseStateError::Io),
    };
    let mut bytes = Vec::new();
    file.take((maximum + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| LicenseStateError::Io)?;
    if bytes.len() > maximum {
        return Err(LicenseStateError::InvalidJson);
    }
    Ok(Some(bytes))
}

pub(super) fn write_atomic(path: &Path, source: &[u8]) -> Result<(), LicenseStateError> {
    let parent = parent_directory(path)?;
    fs::create_dir_all(parent).map_err(|_| LicenseStateError::Io)?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|_| LicenseStateError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o640))
            .map_err(|_| LicenseStateError::Io)?;
    }
    temporary
        .write_all(source)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| LicenseStateError::Io)?;
    #[cfg(windows)]
    {
        // atomicwrites uses MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH).
        // NamedTempFile::persist alone does not request write-through on Windows.
        // keep() clears FILE_ATTRIBUTE_TEMPORARY before the durable rename.
        let (file, temporary_path) = temporary.keep().map_err(|_| LicenseStateError::Io)?;
        file.sync_all().map_err(|_| LicenseStateError::Io)?;
        drop(file);
        let result = atomicwrites::replace_atomic(&temporary_path, path);
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result.map_err(|_| LicenseStateError::Io)?;
    }
    #[cfg(not(windows))]
    temporary.persist(path).map_err(|_| LicenseStateError::Io)?;
    sync_parent(path)
}

fn parent_directory(path: &Path) -> Result<&Path, LicenseStateError> {
    path.parent()
        .map(|parent| {
            if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            }
        })
        .ok_or(LicenseStateError::Io)
}

fn sync_parent(path: &Path) -> Result<(), LicenseStateError> {
    #[cfg(unix)]
    File::open(parent_directory(path)?)
        .and_then(|parent| parent.sync_all())
        .map_err(|_| LicenseStateError::Io)?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
#[path = "free_switch_tests.rs"]
mod free_switch_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{license, v2_license};
    use std::{
        process::{Child, Command},
        sync::{Arc, Barrier},
        thread,
        time::{Duration, Instant},
    };
    use tempfile::tempdir;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-08-28 0:00 UTC);

    fn store(root: &Path) -> LicenseStateStore {
        LicenseStateStore::new(root.join("state.json"), &[91; 32]).unwrap()
    }

    fn import_audit(id: &str, action: LicenseImportAction) -> LicenseImportAudit {
        LicenseImportAudit {
            event_id: format!("license_import_{id}"),
            actor: crate::LicenseImportActor::Administrator {
                identity_id: "owner_original".to_owned(),
                role: "owner".to_owned(),
            },
            action,
        }
    }

    #[test]
    fn manual_import_recovers_actor_and_timestamp_at_every_commit_phase() {
        for phase in 0..=3 {
            let directory = tempdir().unwrap();
            let target = directory.path().join("license.json");
            let history = store(directory.path());
            let old = license(1, "2026-08-26T00:00:00.000Z");
            let next = license(2, "2026-08-27T00:00:00.000Z");
            history.install_document(&target, &old, NOW).unwrap();
            let audit = import_audit("accepted", LicenseImportAction::Install);
            let update = history
                .prepare_import_update(&target, &next, NOW, audit.clone())
                .unwrap();
            history.write_update_intent(&update).unwrap();
            if phase >= 1 {
                write_atomic(
                    &history.path,
                    &decode_bytes(&update.claims.state, MAX_STATE_BYTES).unwrap(),
                )
                .unwrap();
            }
            if phase >= 2 {
                write_atomic(&target, next.source()).unwrap();
            }
            if phase >= 3 {
                write_atomic(
                    &history.sidecar(".activation"),
                    &decode_bytes(
                        update.claims.activation_audits.as_deref().unwrap(),
                        MAX_ACTIVATION_AUDIT_BYTES,
                    )
                    .unwrap(),
                )
                .unwrap();
            }
            let restarted = store(directory.path());
            assert_eq!(
                restarted.read_committed_document(&target).unwrap().unwrap(),
                next.source()
            );
            let pending = restarted.pending_activation_audits(&target).unwrap();
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].import(), Some(&audit));
            assert_eq!(pending[0].activated_at(), "2026-08-28T00:00:00.000Z");
            assert!(fs::read(history.sidecar(".pending")).unwrap().is_empty());
        }
    }

    #[test]
    fn scheduled_import_recovers_each_file_and_keeps_its_separate_activation_event() {
        for phase in 0..=2 {
            let directory = tempdir().unwrap();
            let target = directory.path().join("license.json");
            let history = store(directory.path());
            let old = license(1, "2026-08-26T00:00:00.000Z");
            let next = license(2, "2026-08-27T00:00:00.000Z");
            history.install_document(&target, &old, NOW).unwrap();
            let audit = import_audit("scheduled", LicenseImportAction::Schedule);
            let update = history
                .prepare_staged_update(&target, &next, NOW, audit.clone())
                .unwrap();
            history.write_update_intent(&update).unwrap();
            if phase >= 1 {
                write_atomic(
                    &history.sidecar(".staged"),
                    &decode_bytes(&update.claims.staged, MAX_STAGED_BYTES).unwrap(),
                )
                .unwrap();
            }
            if phase >= 2 {
                write_atomic(
                    &history.sidecar(".activation"),
                    &decode_bytes(&update.claims.activation_audits, MAX_ACTIVATION_AUDIT_BYTES)
                        .unwrap(),
                )
                .unwrap();
            }
            let restarted = store(directory.path());
            assert_eq!(
                restarted.read_staged_document(&target).unwrap().unwrap(),
                next.source()
            );
            assert_eq!(
                restarted.read_committed_document(&target).unwrap().unwrap(),
                old.source()
            );
            restarted
                .activate_staged_document(&target, &next, NOW)
                .unwrap();
            let pending = restarted.pending_activation_audits(&target).unwrap();
            assert_eq!(pending.len(), 2);
            assert_eq!(pending[0].import(), Some(&audit));
            assert_eq!(pending[0].action(), "license.schedule");
            assert_eq!(pending[1].action(), "license.activate");
            assert_ne!(pending[0].event_id(), pending[1].event_id());
            restarted
                .acknowledge_activation_audit(&target, pending[1].license_sha256())
                .unwrap();
            assert_eq!(
                restarted.pending_activation_audits(&target).unwrap(),
                vec![pending[0].clone()]
            );
            restarted
                .acknowledge_license_audit(&target, &pending[0].event_id())
                .unwrap();
            assert!(
                restarted
                    .pending_activation_audits(&target)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn staged_recovery_rejects_changed_outbox_before_overwriting_any_file() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let history = store(directory.path());
        let next = license(1, "2026-08-27T00:00:00.000Z");
        let update = history
            .prepare_staged_update(
                &target,
                &next,
                NOW,
                import_audit("scheduled", LicenseImportAction::Schedule),
            )
            .unwrap();
        history.write_update_intent(&update).unwrap();
        fs::write(history.sidecar(".activation"), b"unexpected outbox").unwrap();
        assert_eq!(
            history.read_staged_document(&target),
            Err(LicenseStateError::Integrity)
        );
        assert!(!history.sidecar(".staged").exists());
        assert_eq!(
            fs::read(history.sidecar(".activation")).unwrap(),
            b"unexpected outbox"
        );
        assert!(!target.exists());
    }

    #[test]
    fn full_or_duplicate_audit_queue_rejects_import_before_any_license_change() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let history = store(directory.path());
        let current = license(1, "2026-08-26T00:00:00.000Z");
        history
            .install_document_with_audit(
                &target,
                &current,
                NOW,
                import_audit("one", LicenseImportAction::Install),
            )
            .unwrap();
        let before = fs::read(history.sidecar(".activation")).unwrap();
        assert!(
            history
                .install_document_with_audit(
                    &target,
                    &current,
                    NOW,
                    import_audit("one", LicenseImportAction::Install)
                )
                .is_err()
        );
        assert_eq!(fs::read(history.sidecar(".activation")).unwrap(), before);
        let event = history
            .pending_activation_audits(&target)
            .unwrap()
            .remove(0);
        let events = (0..MAX_PENDING_ACTIVATION_AUDITS)
            .map(|index| {
                let mut event = event.clone();
                event.import.as_mut().unwrap().event_id = format!("license_import_{index}");
                event
            })
            .collect();
        let full = history.encode_activation_audits(&target, events).unwrap();
        fs::write(history.sidecar(".activation"), &full).unwrap();
        let next = license(2, "2026-08-27T00:00:00.000Z");
        for action in [LicenseImportAction::Install, LicenseImportAction::Schedule] {
            let audit = import_audit("overflow", action);
            let result = match action {
                LicenseImportAction::Install => {
                    history.install_document_with_audit(&target, &next, NOW, audit)
                }
                LicenseImportAction::Schedule => {
                    history.stage_document_with_audit(&target, &next, NOW, audit)
                }
                LicenseImportAction::SwitchFree => unreachable!("covered by free-switch tests"),
            };
            assert!(result.is_err());
            assert_eq!(fs::read(&target).unwrap(), current.source());
            assert_eq!(fs::read(history.sidecar(".activation")).unwrap(), full);
            assert!(fs::read(history.sidecar(".pending")).unwrap().is_empty());
            assert!(!history.sidecar(".staged").exists());
        }
    }

    #[test]
    fn audited_import_contention_never_records_an_intent() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let history = store(directory.path());
        let current = license(1, "2026-08-26T00:00:00.000Z");
        history.install_document(&target, &current, NOW).unwrap();
        let _lease = history
            .guard_mutation(Some(&current), Some(&target), NOW)
            .unwrap();
        assert_eq!(
            history.install_document_with_audit(
                &target,
                &current,
                NOW,
                import_audit("active", LicenseImportAction::Install)
            ),
            Err(LicenseStateError::MutationConflict)
        );
        assert_eq!(
            history.stage_document_with_audit(
                &target,
                &current,
                NOW,
                import_audit("future", LicenseImportAction::Schedule)
            ),
            Err(LicenseStateError::MutationConflict)
        );
        assert!(fs::read(history.sidecar(".pending")).unwrap().is_empty());
        assert!(!history.sidecar(".activation").exists());
    }

    #[test]
    fn import_outbox_rejects_incomplete_actors_and_unknown_schema_or_fields() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let history = store(directory.path());
        let current = license(1, "2026-08-26T00:00:00.000Z");
        history
            .install_document_with_audit(
                &target,
                &current,
                NOW,
                import_audit("strict", LicenseImportAction::Install),
            )
            .unwrap();
        let source = fs::read(history.sidecar(".activation")).unwrap();
        let valid: serde_json::Value = serde_json::from_slice(&source).unwrap();
        assert_eq!(valid["claims"]["schema"], IMPORT_AUDIT_SCHEMA);
        for mutation in [
            "actor_missing",
            "actor_role",
            "action",
            "extra",
            "legacy_schema",
            "unknown_schema",
            "duplicate",
        ] {
            let mut candidate = valid.clone();
            let event = &mut candidate["claims"]["events"][0];
            match mutation {
                "actor_missing" => {
                    event["import"].as_object_mut().unwrap().remove("actor");
                }
                "actor_role" => event["import"]["actor"]["role"] = "member".into(),
                "action" => event["import"]["action"] = "allow_all".into(),
                "extra" => event["import"]["unrecognized"] = true.into(),
                "legacy_schema" => candidate["claims"]["schema"] = ACTIVATION_AUDIT_SCHEMA.into(),
                "unknown_schema" => {
                    candidate["claims"]["schema"] = "aster.license-audit.unknown".into()
                }
                "duplicate" => {
                    let cloned = event.clone();
                    candidate["claims"]["events"]
                        .as_array_mut()
                        .unwrap()
                        .push(cloned);
                }
                _ => unreachable!(),
            }
            // Even an authenticated but invalid producer value must be rejected.
            if let Ok(claims) =
                serde_json::from_value::<ActivationAuditClaims>(candidate["claims"].clone())
            {
                candidate["mac"] = URL_SAFE_NO_PAD
                    .encode(
                        history
                            .activation_audit_mac(&claims)
                            .unwrap()
                            .finalize()
                            .into_bytes(),
                    )
                    .into();
            }
            assert!(
                history
                    .decode_activation_audits(&target, &serde_json::to_vec(&candidate).unwrap())
                    .is_err(),
                "{mutation}"
            );
        }
        let pending = history.pending_activation_audits(&target).unwrap();
        history
            .acknowledge_license_audit(&target, &pending[0].event_id())
            .unwrap();
        let empty: serde_json::Value =
            serde_json::from_slice(&fs::read(history.sidecar(".activation")).unwrap()).unwrap();
        assert_eq!(empty["claims"]["schema"], ACTIVATION_AUDIT_SCHEMA);
    }

    #[test]
    fn old_update_wire_keeps_its_exact_fields_and_unknown_discriminators_do_not_recover() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let history = store(directory.path());
        let current = license(1, "2026-08-26T00:00:00.000Z");
        let update = history
            .prepare_update(&target, &current, NOW, false)
            .unwrap();
        let wire = serde_json::to_value(&update).unwrap();
        let fields: std::collections::BTreeSet<_> = wire["claims"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            fields,
            [
                "schema",
                "target",
                "previous_license_sha256",
                "previous_state_sha256",
                "license",
                "state"
            ]
            .into_iter()
            .collect()
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(history.state_key.as_ref()).unwrap();
        mac.update(b"aster-team-license-update-v1\n");
        mac.update(&canonicalize(&wire["claims"]).unwrap());
        assert_eq!(
            URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()),
            update.mac
        );
        let mut unknown = wire.clone();
        unknown["claims"]["schema"] = "aster.license-update.unknown".into();
        history.write_update_intent(&unknown).unwrap();
        assert_eq!(
            history.read_committed_document(&target),
            Err(LicenseStateError::InvalidJson)
        );
        assert!(!target.exists());
        assert!(!history.path.exists());
        history.write_update_intent(&wire).unwrap();
        assert_eq!(
            history.read_committed_document(&target).unwrap().unwrap(),
            current.source()
        );
    }

    fn stage(store: &LicenseStateStore, target: &Path, next: &VerifiedProductLicense, phase: u8) {
        let _guard = store.lock_state().unwrap();
        let update = store.prepare_update(target, next, NOW, false).unwrap();
        write_atomic(
            &store.sidecar(".pending"),
            &serde_json::to_vec(&update).unwrap(),
        )
        .unwrap();
        if phase >= 1 {
            write_atomic(
                &store.path,
                &decode_bytes(&update.claims.state, MAX_STATE_BYTES).unwrap(),
            )
            .unwrap();
        }
        if phase >= 2 {
            write_atomic(target, next.source()).unwrap();
        }
    }

    #[test]
    fn admitted_mutations_delay_replacement_and_stale_snapshots_cannot_commit() {
        let directory = tempdir().unwrap();
        let history = store(directory.path());
        let other = store(directory.path());
        let target = directory.path().join("license.json");
        let old = license(1, "2026-08-26T00:00:00.000Z");
        let next = license(2, "2026-08-27T00:00:00.000Z");
        history.install_document(&target, &old, NOW).unwrap();
        let first = history
            .guard_mutation(Some(&old), Some(&target), NOW)
            .unwrap();
        let second = other
            .guard_mutation(Some(&old), Some(&target), NOW)
            .unwrap();
        assert_eq!(
            other.accept_replacement(&next, NOW),
            Err(LicenseStateError::MutationConflict)
        );
        assert_eq!(
            other.check_and_observe(&next, NOW),
            Err(LicenseStateError::MutationConflict)
        );
        assert_eq!(
            other.install_document(&target, &next, NOW),
            Err(LicenseStateError::MutationConflict)
        );
        assert_eq!(fs::read(&target).unwrap(), old.source());
        assert!(fs::read(history.sidecar(".pending")).unwrap().is_empty());
        drop(
            history
                .guard_mutation(Some(&old), Some(&target), NOW)
                .unwrap(),
        );
        assert_eq!(
            other.read_committed_document(&target).unwrap().unwrap(),
            old.source()
        );
        // Inject an independently accepted, interrupted update to exercise the
        // recovery fence separately from ordinary contention (which has no intent).
        stage(&history, &target, &next, 0);
        assert!(matches!(
            history.guard_mutation(Some(&old), Some(&target), NOW),
            Err(LicenseStateError::RecoveryRequired)
        ));
        drop(first);
        assert_eq!(
            other.read_committed_document(&target),
            Err(LicenseStateError::MutationConflict)
        );
        drop(second);
        assert_eq!(
            other.read_committed_document(&target).unwrap().unwrap(),
            next.source()
        );
        assert!(matches!(
            history.guard_mutation(Some(&old), Some(&target), NOW),
            Err(LicenseStateError::MutationConflict)
        ));
        drop(
            history
                .guard_mutation(Some(&next), Some(&target), NOW)
                .unwrap(),
        );
        fs::write(&target, old.source()).unwrap();
        assert!(matches!(
            history.guard_mutation(Some(&next), Some(&target), NOW),
            Err(LicenseStateError::MutationConflict)
        ));
    }

    #[test]
    fn bootstrap_lease_blocks_first_license_and_cannot_be_reopened() {
        let directory = tempdir().unwrap();
        let history = store(directory.path());
        let target = directory.path().join("license.json");
        let next = license(1, "2026-08-26T00:00:00.000Z");
        let guard = history.guard_mutation(None, Some(&target), NOW).unwrap();
        assert_eq!(
            history.initialize(&next, NOW),
            Err(LicenseStateError::MutationConflict)
        );
        assert_eq!(
            history.install_document(&target, &next, NOW),
            Err(LicenseStateError::MutationConflict)
        );
        assert!(!target.exists());
        drop(guard);
        history.install_document(&target, &next, NOW).unwrap();
        history.read_committed_document(&target).unwrap();
        assert!(matches!(
            history.guard_mutation(None, Some(&target), NOW),
            Err(LicenseStateError::MutationConflict)
        ));
    }

    #[test]
    fn staged_activation_waits_for_an_admitted_mutation() {
        let directory = tempdir().unwrap();
        let history = store(directory.path());
        let target = directory.path().join("license.json");
        let old = license(1, "2026-08-26T00:00:00.000Z");
        let next = license(2, "2026-08-27T00:00:00.000Z");
        history.install_document(&target, &old, NOW).unwrap();
        history.stage_document(&target, &next, NOW).unwrap();
        let guard = history
            .guard_mutation(Some(&old), Some(&target), NOW)
            .unwrap();
        assert_eq!(
            history.activate_staged_document(&target, &next, NOW),
            Err(LicenseStateError::MutationConflict)
        );
        assert_eq!(fs::read(&target).unwrap(), old.source());
        drop(guard);
        history
            .activate_staged_document(&target, &next, NOW)
            .unwrap();
        assert_eq!(
            history.read_committed_document(&target).unwrap().unwrap(),
            next.source()
        );
        assert_eq!(history.pending_activation_audits(&target).unwrap().len(), 1);
    }

    #[test]
    fn mutation_child_process() {
        let Some(root) = std::env::var_os("ASTER_LICENSE_MUTATION_TEST_CHILD") else {
            return;
        };
        let root = PathBuf::from(root);
        let history = store(&root);
        let old = license(1, "2026-08-26T00:00:00.000Z");
        let _guard = history
            .guard_mutation(Some(&old), Some(&root.join("license.json")), NOW)
            .unwrap();
        fs::write(root.join("child-ready"), b"mutation admitted").unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        panic!("parent did not terminate mutation-test child");
    }

    #[test]
    fn terminated_mutation_process_releases_lease_and_pending_install_recovers() {
        let directory = tempdir().unwrap();
        let history = store(directory.path());
        let target = directory.path().join("license.json");
        let old = license(1, "2026-08-26T00:00:00.000Z");
        let next = license(2, "2026-08-27T00:00:00.000Z");
        history.install_document(&target, &old, NOW).unwrap();
        let mut child = ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "transaction::tests::mutation_child_process"])
                .env("ASTER_LICENSE_MUTATION_TEST_CHILD", directory.path())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !directory.path().join("child-ready").exists() {
            assert!(
                Instant::now() < deadline,
                "child did not acquire mutation lease"
            );
            assert!(child.0.try_wait().unwrap().is_none(), "child exited early");
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            history.install_document(&target, &next, NOW),
            Err(LicenseStateError::MutationConflict)
        );
        stage(&history, &target, &next, 0);
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert_eq!(
            history.read_committed_document(&target).unwrap().unwrap(),
            next.source()
        );
        drop(
            history
                .guard_mutation(Some(&next), Some(&target), NOW)
                .unwrap(),
        );
    }

    #[test]
    fn every_interrupted_update_phase_recovers_exact_document_and_history() {
        for phase in 0..=2 {
            let directory = tempdir().unwrap();
            let target = directory.path().join("license.json");
            let history = store(directory.path());
            let old = license(1, "2026-08-26T00:00:00.000Z");
            let next = license(2, "2026-08-27T00:00:00.000Z");
            history.install_document(&target, &old, NOW).unwrap();
            stage(&history, &target, &next, phase);
            assert_eq!(
                history.check_and_observe(&old, NOW),
                Err(LicenseStateError::RecoveryRequired)
            );
            let restarted = store(directory.path());
            assert_eq!(
                restarted.read_committed_document(&target).unwrap().unwrap(),
                next.source()
            );
            assert_eq!(
                restarted.validate_document_progress(&old),
                Err(LicenseStateError::LicenseRollback)
            );
            restarted
                .check_and_observe(&next, NOW + time::Duration::hours(1))
                .unwrap();
            assert!(fs::read(history.sidecar(".pending")).unwrap().is_empty());
            // The completion marker must not replay an old observation after restart.
            restarted.read_committed_document(&target).unwrap();
            assert_eq!(
                restarted.check_and_observe(&next, NOW),
                Err(LicenseStateError::ClockRollback)
            );
        }
    }

    #[test]
    fn interrupted_protocol_migrations_recover_one_exact_license_and_history() {
        for phase in 0..=2 {
            for old_is_v2 in [false, true] {
                let directory = tempdir().unwrap();
                let target = directory.path().join("license.json");
                let history = store(directory.path());
                let old = if old_is_v2 {
                    v2_license(1, "2026-08-26T00:00:00.000Z")
                } else {
                    license(1, "2026-08-26T00:00:00.000Z")
                };
                let next = if old_is_v2 {
                    license(2, "2026-08-27T00:00:00.000Z")
                } else {
                    v2_license(2, "2026-08-27T00:00:00.000Z")
                };
                history.install_document(&target, &old, NOW).unwrap();
                stage(&history, &target, &next, phase);

                let restarted = store(directory.path());
                assert_eq!(
                    restarted.read_committed_document(&target).unwrap().unwrap(),
                    next.source(),
                    "phase={phase} old_is_v2={old_is_v2}"
                );
                assert_eq!(
                    restarted.validate_document_progress(&old),
                    Err(LicenseStateError::LicenseRollback),
                    "phase={phase} old_is_v2={old_is_v2}"
                );
                restarted.check_and_observe(&next, NOW).unwrap();
                assert!(fs::read(history.sidecar(".pending")).unwrap().is_empty());
            }
        }
    }

    #[test]
    fn missing_history_is_not_treated_as_first_installation() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let history = store(directory.path());
        let current = license(1, "2026-08-26T00:00:00.000Z");
        history.install_document(&target, &current, NOW).unwrap();
        fs::remove_file(&history.path).unwrap();
        assert_eq!(
            history.install_document(&target, &current, NOW),
            Err(LicenseStateError::Missing)
        );
        assert_eq!(
            history.read_committed_document(&target),
            Err(LicenseStateError::Missing)
        );
        assert!(!history.path.exists());
        assert_eq!(fs::read(target).unwrap(), current.source());
    }

    #[test]
    fn corrupted_or_misdirected_intent_never_overwrites_files() {
        for mutation in ["mac", "target", "history", "unexpected-file"] {
            let directory = tempdir().unwrap();
            let target = directory.path().join("license.json");
            let history = store(directory.path());
            let old = license(1, "2026-08-26T00:00:00.000Z");
            let next = license(2, "2026-08-27T00:00:00.000Z");
            history.install_document(&target, &old, NOW).unwrap();
            let mut update = history.prepare_update(&target, &next, NOW, false).unwrap();
            match mutation {
                "mac" => update.mac = URL_SAFE_NO_PAD.encode([0; 32]),
                "target" => update.claims.target.push_str(".other"),
                "history" => update.claims.state = URL_SAFE_NO_PAD.encode(b"{}"),
                _ => fs::write(&target, b"changed outside transaction").unwrap(),
            }
            if mutation != "mac" {
                update.mac = URL_SAFE_NO_PAD.encode(
                    history
                        .update_mac(&update.claims)
                        .unwrap()
                        .finalize()
                        .into_bytes(),
                );
            }
            let before_license = fs::read(&target).unwrap();
            let before_history = fs::read(&history.path).unwrap();
            write_atomic(
                &history.sidecar(".pending"),
                &serde_json::to_vec(&update).unwrap(),
            )
            .unwrap();
            assert!(
                history.read_committed_document(&target).is_err(),
                "{mutation}"
            );
            assert_eq!(fs::read(&target).unwrap(), before_license, "{mutation}");
            assert_eq!(
                fs::read(&history.path).unwrap(),
                before_history,
                "{mutation}"
            );
        }
    }

    #[test]
    fn license_destination_cannot_alias_history_or_transaction_files() {
        let directory = tempdir().unwrap();
        let history = store(directory.path());
        let current = license(1, "2026-08-26T00:00:00.000Z");
        for target in [
            &history.path,
            &history.sidecar(".pending"),
            &history.sidecar(".lock"),
            &history.sidecar(".mutation"),
            &history.sidecar(".staged"),
            &history.sidecar(".activation"),
        ] {
            assert_eq!(
                history.install_document(target, &current, NOW),
                Err(LicenseStateError::Integrity)
            );
        }
    }

    #[test]
    fn staged_document_is_separate_from_active_history_and_clears_by_exact_bytes() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let history = store(directory.path());
        let active = license(1, "2026-08-26T00:00:00.000Z");
        let staged = license(2, "2026-08-27T00:00:00.000Z");
        history.install_document(&target, &active, NOW).unwrap();
        let active_history = fs::read(&history.path).unwrap();

        history.stage_document(&target, &staged, NOW).unwrap();
        assert_eq!(fs::read(&target).unwrap(), active.source());
        assert_eq!(fs::read(&history.path).unwrap(), active_history);
        assert_eq!(
            history.read_staged_document(&target).unwrap().unwrap(),
            staged.source()
        );

        history
            .clear_staged_document(&target, active.source())
            .unwrap();
        assert!(history.read_staged_document(&target).unwrap().is_some());
        history
            .clear_staged_document(&target, staged.source())
            .unwrap();
        assert_eq!(history.read_staged_document(&target).unwrap(), None);
    }

    #[test]
    fn staged_activation_persists_an_authenticated_audit_until_acknowledged() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let history = store(directory.path());
        let active = license(1, "2026-08-26T00:00:00.000Z");
        let staged = license(2, "2026-08-27T00:00:00.000Z");
        history.install_document(&target, &active, NOW).unwrap();
        history.stage_document(&target, &staged, NOW).unwrap();
        let staged_wire = fs::read(history.sidecar(".staged")).unwrap();

        assert!(matches!(
            history.activate_staged_document(&target, &staged, NOW),
            Ok(StagedActivationOutcome::Activated { .. })
        ));
        assert_eq!(fs::read(&target).unwrap(), staged.source());
        let pending = history.pending_activation_audits(&target).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].license_id(), staged.as_ref().license_id());
        assert_eq!(pending[0].activated_at(), "2026-08-28T00:00:00.000Z");
        assert_eq!(pending[0].license_sha256(), digest(staged.source()));

        write_atomic(&history.sidecar(".staged"), &staged_wire).unwrap();
        assert!(matches!(
            history.activate_staged_document(&target, &staged, NOW),
            Ok(StagedActivationOutcome::AlreadyActive { .. })
        ));
        assert_eq!(history.pending_activation_audits(&target).unwrap().len(), 1);

        history
            .acknowledge_activation_audit(&target, pending[0].license_sha256())
            .unwrap();
        assert!(
            history
                .pending_activation_audits(&target)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn interrupted_activation_recovers_the_license_history_and_audit_marker() {
        for phase in 0..=3 {
            let directory = tempdir().unwrap();
            let target = directory.path().join("license.json");
            let history = store(directory.path());
            let active = license(1, "2026-08-26T00:00:00.000Z");
            let staged = license(2, "2026-08-27T00:00:00.000Z");
            history.install_document(&target, &active, NOW).unwrap();
            history.stage_document(&target, &staged, NOW).unwrap();
            {
                let _guard = history.lock_state().unwrap();
                let update = history.prepare_update(&target, &staged, NOW, true).unwrap();
                write_atomic(
                    &history.sidecar(".pending"),
                    &serde_json::to_vec(&update).unwrap(),
                )
                .unwrap();
                if phase >= 1 {
                    write_atomic(
                        &history.path,
                        &decode_bytes(&update.claims.state, MAX_STATE_BYTES).unwrap(),
                    )
                    .unwrap();
                }
                if phase >= 2 {
                    write_atomic(&target, staged.source()).unwrap();
                }
                if phase >= 3 {
                    write_atomic(
                        &history.sidecar(".activation"),
                        &decode_bytes(
                            update.claims.activation_audits.as_deref().unwrap(),
                            MAX_ACTIVATION_AUDIT_BYTES,
                        )
                        .unwrap(),
                    )
                    .unwrap();
                }
            }

            let restarted = store(directory.path());
            assert_eq!(
                restarted.read_committed_document(&target).unwrap().unwrap(),
                staged.source(),
                "phase={phase}"
            );
            let pending = restarted.pending_activation_audits(&target).unwrap();
            assert_eq!(pending.len(), 1, "phase={phase}");
            assert_eq!(pending[0].license_sha256(), digest(staged.source()));
            assert!(fs::read(history.sidecar(".pending")).unwrap().is_empty());
        }
    }

    #[test]
    fn staged_replacement_cannot_move_backward_but_corruption_does_not_touch_active() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let history = store(directory.path());
        let active = license(1, "2026-08-26T00:00:00.000Z");
        let first = license(2, "2026-08-27T00:00:00.000Z");
        let later = license(3, "2026-08-28T00:00:00.000Z");
        history.install_document(&target, &active, NOW).unwrap();
        history.stage_document(&target, &later, NOW).unwrap();
        assert_eq!(
            history.stage_document(&target, &first, NOW),
            Err(LicenseStateError::LicenseRollback)
        );

        fs::write(history.sidecar(".staged"), b"corrupt staged state").unwrap();
        assert!(history.read_staged_document(&target).is_err());
        assert_eq!(
            history.read_committed_document(&target).unwrap().unwrap(),
            active.source()
        );

        history.stage_document(&target, &first, NOW).unwrap();
        assert_eq!(
            history.read_staged_document(&target).unwrap().unwrap(),
            first.source()
        );
        assert_eq!(fs::read(target).unwrap(), active.source());
    }

    #[test]
    fn independently_constructed_stores_serialize_competing_imports() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let barrier = Arc::new(Barrier::new(8));
        thread::scope(|scope| {
            for sequence in 1..=8 {
                let history = store(directory.path());
                let target = &target;
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    let current = license(sequence, "2026-08-26T00:00:00.000Z");
                    barrier.wait();
                    let result = history.install_document(target, &current, NOW);
                    assert!(result.is_ok() || result == Err(LicenseStateError::LicenseRollback));
                });
            }
        });
        let expected = license(8, "2026-08-26T00:00:00.000Z");
        assert_eq!(
            store(directory.path())
                .read_committed_document(&target)
                .unwrap()
                .unwrap(),
            expected.source()
        );
    }

    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn transaction_child_process() {
        let Some(root) = std::env::var_os("ASTER_LICENSE_STATE_TEST_CHILD") else {
            return;
        };
        let root = PathBuf::from(root);
        let history = store(&root);
        let target = root.join("license.json");
        let next = license(2, "2026-08-27T00:00:00.000Z");
        stage(&history, &target, &next, 1);
        let _guard = history.lock_state().unwrap();
        fs::write(root.join("child-ready"), b"locked").unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        panic!("parent did not terminate the crash-test child");
    }

    #[test]
    fn process_lock_is_released_after_crash_and_pending_update_recovers() {
        let directory = tempdir().unwrap();
        let history = store(directory.path());
        let target = directory.path().join("license.json");
        history
            .install_document(&target, &license(1, "2026-08-26T00:00:00.000Z"), NOW)
            .unwrap();
        let mut child = ChildGuard(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "transaction::tests::transaction_child_process"])
                .env("ASTER_LICENSE_STATE_TEST_CHILD", directory.path())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        while !directory.path().join("child-ready").exists() {
            assert!(
                Instant::now() < deadline,
                "child did not obtain process lock"
            );
            assert!(child.0.try_wait().unwrap().is_none(), "child exited early");
            thread::sleep(Duration::from_millis(10));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(history.sidecar(".lock"))
            .unwrap();
        assert!(matches!(
            lock.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        lock.try_lock().unwrap();
        lock.unlock().unwrap();
        assert_eq!(
            history.read_committed_document(&target).unwrap().unwrap(),
            license(2, "2026-08-27T00:00:00.000Z").source()
        );
    }

    #[cfg(windows)]
    #[test]
    fn actual_license_rename_failure_keeps_recoverable_intent() {
        use std::os::windows::fs::OpenOptionsExt as _;
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let history = store(directory.path());
        let old = license(1, "2026-08-26T00:00:00.000Z");
        let next = license(2, "2026-08-27T00:00:00.000Z");
        history.install_document(&target, &old, NOW).unwrap();
        // Reads remain possible while delete/rename is denied by this handle.
        let blocker = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&target)
            .unwrap();
        assert_eq!(
            history.install_document(&target, &next, NOW),
            Err(LicenseStateError::Io)
        );
        assert_eq!(history.load().unwrap().last_transfer_sequence, 2);
        assert_eq!(fs::read(&target).unwrap(), old.source());
        drop(blocker);
        assert_eq!(
            history.read_committed_document(&target).unwrap().unwrap(),
            next.source()
        );
    }

    #[cfg(windows)]
    #[test]
    fn committed_files_do_not_retain_windows_temporary_attribute() {
        use std::os::windows::fs::MetadataExt as _;
        let directory = tempdir().unwrap();
        let target = directory.path().join("license.json");
        let history = store(directory.path());
        let current = license(1, "2026-08-26T00:00:00.000Z");
        history.install_document(&target, &current, NOW).unwrap();
        for path in [&target, &history.path, &history.sidecar(".pending")] {
            assert_eq!(fs::metadata(path).unwrap().file_attributes() & 0x100, 0);
        }
    }
}
