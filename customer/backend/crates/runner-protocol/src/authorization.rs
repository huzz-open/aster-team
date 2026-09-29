use super::{RunnerProtocolError, TaskCommand, validate_identifier};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SignedExpiry {
    Fixed { expires_at: i64 },
    Never {},
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskLicense {
    pub license_id: String,
    pub license_sha256: String,
    pub expiry: SignedExpiry,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdminSubject {
    pub identity_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlService {
    CredentialBroker,
    ModelCatalog,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaintenanceActor {
    Admin { identity_id: String },
    Service { service: ControlService },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSubject {
    pub identity_id: String,
    pub api_key_id: String,
    pub request_id: String,
    pub reservation_id: String,
    pub reserved_tokens: i64,
    pub reservation_expires_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageModelSubject {
    pub identity_id: String,
    pub api_key_id: String,
    pub request_id: String,
    pub reservation_id: String,
    pub reserved_images: i64,
    pub reservation_expires_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelResource {
    pub account_id: String,
    pub public_model: String,
    pub upstream_model: String,
}

/// Authorization is part of the signed ticket. Variants cannot borrow another
/// command's subject or omit an expiry to acquire unrestricted authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskAuthorization {
    Probe {},
    Model {
        subject: ModelSubject,
        resource: ModelResource,
        license: TaskLicense,
    },
    ImageModel {
        subject: ImageModelSubject,
        resource: ModelResource,
        license: TaskLicense,
    },
    FetchAsset {
        subject: ImageModelSubject,
        resource: ModelResource,
        license: TaskLicense,
    },
    DiscoverModels {
        actor: MaintenanceActor,
        account_id: String,
        license: TaskLicense,
    },
    RefreshCredential {
        actor: MaintenanceActor,
        account_id: String,
        lease_sha256: String,
        lease_expires_at: i64,
        license: TaskLicense,
    },
    AuthorizeCredential {
        actor: AdminSubject,
        enrollment_id: String,
        session_expires_at: i64,
        license: TaskLicense,
    },
}

impl TaskAuthorization {
    pub fn command(&self) -> TaskCommand {
        match self {
            Self::Probe {} => TaskCommand::Probe,
            Self::Model { .. } | Self::ImageModel { .. } => TaskCommand::Execute,
            Self::FetchAsset { .. } => TaskCommand::FetchAsset,
            Self::DiscoverModels { .. } => TaskCommand::DiscoverModels,
            Self::RefreshCredential { .. } => TaskCommand::RefreshCredential,
            Self::AuthorizeCredential { .. } => TaskCommand::AuthorizeCredential,
        }
    }

    pub fn license(&self) -> Option<&TaskLicense> {
        match self {
            Self::Model { license, .. }
            | Self::ImageModel { license, .. }
            | Self::FetchAsset { license, .. }
            | Self::DiscoverModels { license, .. }
            | Self::RefreshCredential { license, .. }
            | Self::AuthorizeCredential { license, .. } => Some(license),
            Self::Probe {} => None,
        }
    }

    pub fn deadline(&self) -> Option<i64> {
        let license = self.license().and_then(|license| match license.expiry {
            SignedExpiry::Fixed { expires_at } => Some(expires_at),
            SignedExpiry::Never {} => None,
        });
        let operation = match self {
            Self::Model { subject, .. } => Some(subject.reservation_expires_at),
            Self::ImageModel { subject, .. } => Some(subject.reservation_expires_at),
            Self::FetchAsset { subject, .. } => Some(subject.reservation_expires_at),
            Self::Probe {} | Self::DiscoverModels { .. } => None,
            Self::RefreshCredential {
                lease_expires_at, ..
            } => Some(*lease_expires_at),
            Self::AuthorizeCredential {
                session_expires_at, ..
            } => Some(*session_expires_at),
        };
        license.into_iter().chain(operation).min()
    }

    pub(super) fn validate(
        &self,
        command: TaskCommand,
        expires_at: i64,
    ) -> Result<(), RunnerProtocolError> {
        if self.command() != command
            || self
                .deadline()
                .is_some_and(|deadline| expires_at > deadline)
        {
            return Err(RunnerProtocolError::InvalidField);
        }
        if let Some(license) = self.license() {
            validate_identifier(&license.license_id)?;
            validate_sha256(&license.license_sha256)?;
        }
        match self {
            Self::Probe {} => {}
            Self::Model {
                subject, resource, ..
            } => {
                for id in [
                    &subject.identity_id,
                    &subject.api_key_id,
                    &subject.request_id,
                    &subject.reservation_id,
                    &resource.account_id,
                ] {
                    validate_identifier(id)?;
                }
                if subject.reserved_tokens <= 0 {
                    return Err(RunnerProtocolError::InvalidField);
                }
                for model in [&resource.public_model, &resource.upstream_model] {
                    if model.is_empty()
                        || model.chars().count() > 256
                        || model.chars().any(char::is_control)
                    {
                        return Err(RunnerProtocolError::InvalidField);
                    }
                }
            }
            Self::ImageModel {
                subject, resource, ..
            }
            | Self::FetchAsset {
                subject, resource, ..
            } => {
                for id in [
                    &subject.identity_id,
                    &subject.api_key_id,
                    &subject.request_id,
                    &subject.reservation_id,
                    &resource.account_id,
                ] {
                    validate_identifier(id)?;
                }
                if !(1..=10).contains(&subject.reserved_images) {
                    return Err(RunnerProtocolError::InvalidField);
                }
                for model in [&resource.public_model, &resource.upstream_model] {
                    if model.is_empty()
                        || model.chars().count() > 256
                        || model.chars().any(char::is_control)
                    {
                        return Err(RunnerProtocolError::InvalidField);
                    }
                }
            }
            Self::DiscoverModels {
                actor, account_id, ..
            } => {
                validate_actor(actor, ControlService::ModelCatalog)?;
                validate_identifier(account_id)?;
            }
            Self::RefreshCredential {
                actor,
                account_id,
                lease_sha256,
                ..
            } => {
                validate_actor(actor, ControlService::CredentialBroker)?;
                validate_identifier(account_id)?;
                validate_sha256(lease_sha256)?;
            }
            Self::AuthorizeCredential {
                actor,
                enrollment_id,
                ..
            } => {
                validate_identifier(&actor.identity_id)?;
                validate_identifier(enrollment_id)?;
            }
        }
        Ok(())
    }
}

fn validate_actor(
    actor: &MaintenanceActor,
    expected_service: ControlService,
) -> Result<(), RunnerProtocolError> {
    match actor {
        MaintenanceActor::Admin { identity_id } => validate_identifier(identity_id),
        MaintenanceActor::Service { service } if *service == expected_service => Ok(()),
        _ => Err(RunnerProtocolError::InvalidField),
    }
}

fn validate_sha256(value: &str) -> Result<(), RunnerProtocolError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(RunnerProtocolError::InvalidField);
    }
    Ok(())
}
