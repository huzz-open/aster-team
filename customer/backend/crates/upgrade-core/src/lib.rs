#![forbid(unsafe_code)]

pub mod migration;
pub mod online;
pub mod preparation;
pub mod retirement;
pub mod runtime;
pub mod settlement;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAINTENANCE_JOB_SCHEMA: &str = "aster.maintenance-job.v1";
pub const ACTIVE_SLOT_SCHEMA: &str = "aster.active-release-slot.v1";
pub const ACTIVE_SLOT_RUNTIME_SCHEMA: &str = "aster.active-release-slot.v2";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeMode {
    Maintenance,
    BlueGreen,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UpgradeCapabilities {
    pub database_driver: String,
    pub supported_modes: Vec<UpgradeMode>,
    pub unavailable_reason: String,
}

impl UpgradeCapabilities {
    #[must_use]
    pub fn for_database(database_driver: &str) -> Self {
        Self {
            database_driver: database_driver.to_owned(),
            supported_modes: if database_driver == "sqlcipher"
                || (database_driver == "mariadb"
                    && cfg!(all(target_os = "linux", target_arch = "x86_64")))
            {
                vec![UpgradeMode::Maintenance]
            } else {
                Vec::new()
            },
            unavailable_reason: match database_driver {
                "sqlcipher" => "sqlite_requires_maintenance",
                "mariadb" if cfg!(all(target_os = "linux", target_arch = "x86_64")) => {
                    "blue_green_runtime_not_available"
                }
                "mariadb" => "external_database_platform_not_supported",
                _ => "database_driver_unknown",
            }
            .to_owned(),
        }
    }

    #[must_use]
    pub fn supports(&self, mode: UpgradeMode) -> bool {
        self.supported_modes.contains(&mode)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseSlot {
    Blue,
    Green,
}

impl ReleaseSlot {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Green => "green",
        }
    }

    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Blue => Self::Green,
            Self::Green => Self::Blue,
        }
    }

    /// Private loopback runtime endpoint; only provisioned on supported Linux deployments.
    #[must_use]
    pub const fn runtime_port(self) -> u16 {
        match self {
            Self::Blue => 11_383,
            Self::Green => 11_483,
        }
    }

    #[must_use]
    pub const fn ports(self) -> ReleaseSlotPorts {
        match self {
            Self::Blue => ReleaseSlotPorts {
                api: 11_380,
                member: 11_381,
                admin: 11_382,
            },
            Self::Green => ReleaseSlotPorts {
                api: 11_480,
                member: 11_481,
                admin: 11_482,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReleaseSlotPorts {
    pub api: u16,
    pub member: u16,
    pub admin: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveLocalRunner {
    pub runner_id: String,
    pub manifest_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveReleaseSlot {
    pub schema: String,
    pub slot: ReleaseSlot,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_runner: Option<ActiveLocalRunner>,
}

impl ActiveReleaseSlot {
    pub fn validate(&self) -> Result<(), MaintenanceJobError> {
        match &self.local_runner {
            None if self.schema == ACTIVE_SLOT_SCHEMA => {}
            Some(runner)
                if self.schema == ACTIVE_SLOT_RUNTIME_SCHEMA
                    && runner.runner_id.strip_prefix("runner_").is_some_and(|id| {
                        id.len() == 32
                            && id
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
                    && runner.manifest_sha256.len() == 64
                    && runner
                        .manifest_sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) => {}
            _ => return Err(MaintenanceJobError::InvalidSchema),
        }
        if !valid_version(&self.version) {
            return Err(MaintenanceJobError::InvalidVersion);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceOperation {
    Upgrade {
        archive: PathBuf,
        archive_sha256: String,
    },
    DeleteVersion {
        version: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceStatus {
    Queued,
    Verifying,
    Staging,
    StoppingServices,
    RestoringPrevious,
    StartingCandidate,
    Migrating,
    SwitchingTraffic,
    DrainingPrevious,
    Succeeded,
    Failed,
}

impl MaintenanceStatus {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceJob {
    pub schema: String,
    pub id: String,
    pub requested_by: String,
    pub operation: MaintenanceOperation,
    pub status: MaintenanceStatus,
    // Missing on historical jobs; do not relabel an old blue/green run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade_mode: Option<UpgradeMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_was_running: Option<bool>,
    pub current_version: String,
    pub target_version: Option<String>,
    pub previous_release: Option<PathBuf>,
    pub candidate_release: Option<PathBuf>,
    pub message: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MaintenanceJobError {
    #[error("maintenance job schema is invalid")]
    InvalidSchema,
    #[error("maintenance job identifier is invalid")]
    InvalidIdentifier,
    #[error("maintenance job checksum is invalid")]
    InvalidChecksum,
    #[error("maintenance job version is invalid")]
    InvalidVersion,
    #[error("maintenance job archive path is invalid")]
    InvalidArchive,
}

impl MaintenanceJob {
    pub fn validate(&self) -> Result<(), MaintenanceJobError> {
        if self.schema != MAINTENANCE_JOB_SCHEMA {
            return Err(MaintenanceJobError::InvalidSchema);
        }
        if !valid_identifier(&self.id) || !valid_identifier(&self.requested_by) {
            return Err(MaintenanceJobError::InvalidIdentifier);
        }
        if !valid_version(&self.current_version)
            || self
                .target_version
                .as_deref()
                .is_some_and(|version| !valid_version(version))
        {
            return Err(MaintenanceJobError::InvalidVersion);
        }
        match &self.operation {
            MaintenanceOperation::Upgrade {
                archive,
                archive_sha256,
            } => {
                if !archive.is_absolute() {
                    return Err(MaintenanceJobError::InvalidArchive);
                }
                if !valid_checksum(archive_sha256) {
                    return Err(MaintenanceJobError::InvalidChecksum);
                }
            }
            MaintenanceOperation::DeleteVersion { version } => {
                if !valid_version(version) {
                    return Err(MaintenanceJobError::InvalidVersion);
                }
            }
        }
        Ok(())
    }
}

#[must_use]
pub fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'@'))
}

fn valid_checksum(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_version(value: &str) -> bool {
    let core = value.split_once(['-', '+']).map_or(value, |(core, _)| core);
    let parts = core.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_topology_is_part_of_one_commit_and_never_falls_back_from_v2() {
        let legacy: ActiveReleaseSlot = serde_json::from_value(
            serde_json::json!({ "schema": ACTIVE_SLOT_SCHEMA, "slot": "blue", "version": "2.0.0" }),
        )
        .unwrap();
        legacy.validate().unwrap();
        assert!(
            serde_json::to_value(&legacy)
                .unwrap()
                .get("local_runner")
                .is_none()
        );
        let mut active = legacy.clone();
        active.schema = ACTIVE_SLOT_RUNTIME_SCHEMA.into();
        assert!(active.validate().is_err());
        active.local_runner = Some(ActiveLocalRunner {
            runner_id: format!("runner_{}", "a".repeat(32)),
            manifest_sha256: "b".repeat(64),
        });
        active.validate().unwrap();
        assert_eq!(
            serde_json::from_slice::<ActiveReleaseSlot>(&serde_json::to_vec(&active).unwrap())
                .unwrap(),
            active
        );
        active.schema = ACTIVE_SLOT_SCHEMA.into();
        assert!(active.validate().is_err());
        active.schema = ACTIVE_SLOT_RUNTIME_SCHEMA.into();
        active.local_runner.as_mut().unwrap().runner_id = "../other".into();
        assert!(active.validate().is_err());
    }

    #[test]
    fn validates_upgrade_and_delete_jobs() {
        let upgrade = MaintenanceJob {
            schema: MAINTENANCE_JOB_SCHEMA.to_owned(),
            id: "job_123".to_owned(),
            requested_by: "admin_123".to_owned(),
            operation: MaintenanceOperation::Upgrade {
                archive: std::env::temp_dir().join("aster-team-release.tar.gz"),
                archive_sha256: "a".repeat(64),
            },
            status: MaintenanceStatus::Queued,
            upgrade_mode: Some(UpgradeMode::Maintenance),
            runner_was_running: None,
            current_version: "2.0.0".to_owned(),
            target_version: None,
            previous_release: None,
            candidate_release: None,
            message: String::new(),
            created_at: "2026-08-31T00:00:00.000Z".to_owned(),
            updated_at: "2026-08-31T00:00:00.000Z".to_owned(),
        };
        upgrade.validate().unwrap();

        let mut delete = upgrade;
        delete.operation = MaintenanceOperation::DeleteVersion {
            version: "2.0.1-rc.1".to_owned(),
        };
        delete.validate().unwrap();
    }

    #[test]
    fn release_slots_have_stable_non_overlapping_ports() {
        assert_eq!(ReleaseSlot::Blue.other(), ReleaseSlot::Green);
        assert_eq!(ReleaseSlot::Blue.ports().api, 11_380);
        assert_eq!(ReleaseSlot::Green.ports().admin, 11_482);
        assert_ne!(ReleaseSlot::Blue.ports(), ReleaseSlot::Green.ports());
        let mut unique = std::collections::BTreeSet::new();
        for slot in [ReleaseSlot::Blue, ReleaseSlot::Green] {
            let ports = slot.ports();
            for port in [ports.api, ports.member, ports.admin, slot.runtime_port()] {
                assert!(unique.insert(port));
            }
        }
        assert_eq!(ReleaseSlot::Blue.runtime_port(), 11_383);
        assert_eq!(ReleaseSlot::Green.runtime_port(), 11_483);
    }

    #[test]
    fn supported_database_platforms_allow_maintenance_but_never_claim_blue_green() {
        for driver in ["sqlcipher", "mariadb", "mysql", "unknown"] {
            let capabilities = UpgradeCapabilities::for_database(driver);
            assert_eq!(
                capabilities.supports(UpgradeMode::Maintenance),
                driver == "sqlcipher"
                    || (driver == "mariadb"
                        && cfg!(all(target_os = "linux", target_arch = "x86_64")))
            );
            assert!(!capabilities.supports(UpgradeMode::BlueGreen));
        }
    }
}
