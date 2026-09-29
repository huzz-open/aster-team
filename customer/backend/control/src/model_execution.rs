use super::*;
use aster_runner_protocol::{
    AdminSubject, ControlService, ImageModelSubject, MaintenanceActor, ModelResource, ModelSubject,
    SignedExpiry, TaskAuthorization, TaskLicense,
};
use aster_storage::{ConnectionCredential, ConnectionRecord, ConnectionRoute};

pub(crate) struct AuthorizedRunnerTask {
    pub(crate) license: VerifiedProductLicense,
    pub(crate) claims: TaskAuthorization,
}

fn task_license(license: &VerifiedProductLicense) -> Result<TaskLicense, ControlError> {
    let verified = license.as_ref();
    Ok(TaskLicense {
        license_id: verified.license_id().to_owned(),
        license_sha256: sha256_hex(verified.source()),
        expiry: match verified.expires_at() {
            Some(value) => SignedExpiry::Fixed {
                expires_at: OffsetDateTime::parse(value, &Rfc3339)
                    .map_err(|_| ControlError::DataIntegrityInvalid)?
                    .unix_timestamp(),
            },
            None => SignedExpiry::Never {},
        },
    })
}

/// Immutable request scope, not a cached permission. Every provider and Runner
/// attempt revalidates the consumer, signed reservation and current License.
pub(crate) struct ModelExecution<'a> {
    consumer: &'a ModelConsumer,
    reservation: &'a QuotaReservation,
    pub(crate) request_id: &'a GatewayRequestIds,
    pub(crate) public_model: &'a str,
    operation: &'a providers::CanonicalOperation,
}

impl<'a> ModelExecution<'a> {
    fn access_resource_model(&self) -> &str {
        match self.operation {
            providers::CanonicalOperation::Text(_) => self.public_model,
            providers::CanonicalOperation::ImageGeneration(operation) => &operation.public_model,
            providers::CanonicalOperation::ImageEdit(operation) => &operation.public_model,
        }
    }

    pub(crate) fn new(
        consumer: &'a ModelConsumer,
        reservation: &'a QuotaReservation,
        request_id: &'a GatewayRequestIds,
        public_model: &'a str,
        operation: &'a providers::CanonicalOperation,
    ) -> Self {
        Self {
            consumer,
            reservation,
            request_id,
            public_model,
            operation,
        }
    }

    async fn revalidate(
        &self,
        state: &ControlState,
    ) -> Result<(VerifiedProductLicense, ModelSubject), ControlError> {
        revalidate_model_subject(state, self.consumer, self.reservation, self.request_id).await
    }

    pub(crate) async fn prepare(
        &self,
        state: &ControlState,
        excluded: &HashSet<String>,
        force_refresh: Option<&str>,
    ) -> Result<ModelAttempt<'_>, ControlError> {
        self.revalidate(state).await?;
        state
            .assert_model_access(self.consumer.identity_id(), self.access_resource_model())
            .await?;
        let (candidate, binding, request) = prepare_provider_request(
            state,
            self.public_model,
            self.request_id,
            self.operation,
            excluded,
            force_refresh,
        )
        .await?;
        let encoded = Zeroizing::new(
            serde_json::to_vec(&request).map_err(|_| ControlError::GatewayRequestInvalid)?,
        );
        Ok(ModelAttempt {
            execution: self,
            candidate,
            binding,
            request,
            payload_digest: sha256_hex(encoded.as_slice()),
        })
    }
}

async fn revalidate_model_subject(
    state: &ControlState,
    consumer: &ModelConsumer,
    reservation: &QuotaReservation,
    request_id: &GatewayRequestIds,
) -> Result<(VerifiedProductLicense, ModelSubject), ControlError> {
    let subject = consumer.revalidate(state).await?;
    if reservation.identity_id != subject.identity.id
        || reservation.request_id != request_id.as_str()
    {
        return Err(ControlError::ExternalApiKeyInvalid);
    }
    if let Some(frozen) = &reservation.billing {
        if reservation._money_permit.is_none()
            || frozen.selected_price.public_model.is_empty()
            || reservation.reserved_tokens <= 0
        {
            return Err(ControlError::QuotaSettlementConflict);
        }
        let (license, _) = authorize_model_consumption_license(state, "member").await?;
        authorize_licensed_feature(&license, "runner", &state.product_version, (state.now)())
            .map_err(ControlError::Policy)?;
        let expires_at = (state.now)()
            .checked_add(time::Duration::minutes(10))
            .ok_or(ControlError::DataIntegrityInvalid)?;
        return Ok((
            license,
            ModelSubject {
                identity_id: subject.identity.id,
                api_key_id: subject.api_key.id,
                request_id: reservation.request_id.clone(),
                reservation_id: reservation.id.clone(),
                reserved_tokens: reservation.reserved_tokens,
                reservation_expires_at: expires_at.unix_timestamp(),
            },
        ));
    }
    let snapshot = state.verified_quota_snapshot(&subject.identity.id).await?;
    let record = snapshot
        .active_reservations
        .iter()
        .find(|record| record.id == reservation.id)
        .ok_or(ControlError::QuotaSettlementConflict)?;
    if record.api_key_id != subject.api_key.id
        || record.request_id != request_id.as_str()
        || record.reserved_tokens != reservation.reserved_tokens
        || record.identity_id != subject.identity.id
    {
        return Err(ControlError::ExternalApiKeyInvalid);
    }
    let expires_at = OffsetDateTime::parse(&record.expires_at, &Rfc3339)
        .map_err(|_| ControlError::DataIntegrityInvalid)?;
    if expires_at <= (state.now)() {
        return Err(ControlError::QuotaSettlementConflict);
    }
    let (license, _) = authorize_model_consumption_license(state, "member").await?;
    // Preserve the existing Execute requirements, using one License instead
    // of reading member and runner from potentially different documents.
    authorize_licensed_feature(&license, "runner", &state.product_version, (state.now)())
        .map_err(ControlError::Policy)?;
    Ok((
        license,
        ModelSubject {
            identity_id: subject.identity.id.clone(),
            api_key_id: subject.api_key.id.clone(),
            request_id: record.request_id.clone(),
            reservation_id: record.id.clone(),
            reserved_tokens: record.reserved_tokens,
            reservation_expires_at: expires_at.unix_timestamp(),
        },
    ))
}

/// One immutable API-key dispatch. The model, channel, credential revision and
/// complete HTTP payload are rechecked at Runner admission, after reservation.
pub(crate) struct PluginModelAttempt<'a> {
    pub(crate) consumer: &'a ModelConsumer,
    pub(crate) reservation: &'a QuotaReservation,
    pub(crate) request_id: &'a GatewayRequestIds,
    pub(crate) public_model: &'a str,
    pub(crate) route: ConnectionRoute,
    pub(crate) binding: RunnerTaskBinding,
    pub(crate) request: UpstreamHttpRequest,
    pub(crate) payload_digest: String,
}

impl PluginModelAttempt<'_> {
    pub(crate) fn new<'a>(
        consumer: &'a ModelConsumer,
        reservation: &'a QuotaReservation,
        request_id: &'a GatewayRequestIds,
        public_model: &'a str,
        route: ConnectionRoute,
        binding: RunnerTaskBinding,
        request: UpstreamHttpRequest,
    ) -> Result<PluginModelAttempt<'a>, ControlError> {
        let encoded = Zeroizing::new(
            serde_json::to_vec(&request).map_err(|_| ControlError::GatewayRequestInvalid)?,
        );
        Ok(PluginModelAttempt {
            consumer,
            reservation,
            request_id,
            public_model,
            route,
            binding,
            request,
            payload_digest: sha256_hex(encoded.as_slice()),
        })
    }

    async fn authorize(
        &self,
        state: &ControlState,
        binding: &RunnerTaskBinding,
        payload: &[u8],
    ) -> Result<AuthorizedRunnerTask, ControlError> {
        if binding != &self.binding || sha256_hex(payload) != self.payload_digest {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let current = state
            .credential_storage()?
            .api_key_connection_route(self.public_model)
            .await?
            .ok_or(ControlError::ModelAttemptStale)?;
        if current.connection.id != self.route.connection.id
            || current.connection.revision != self.route.connection.revision
            || current.connection.channel_id != self.route.connection.channel_id
            || current.binding != self.route.binding
            || current.credential.id != self.route.credential.id
            || current.credential.credential_revision != self.route.credential.credential_revision
        {
            return Err(ControlError::ModelAttemptStale);
        }
        let (license, subject) =
            revalidate_model_subject(state, self.consumer, self.reservation, self.request_id)
                .await?;
        state
            .admit_model_attempt(
                &subject.identity_id,
                self.public_model,
                self.request_id.as_str(),
            )
            .await?;
        let claims = TaskAuthorization::Model {
            subject,
            resource: ModelResource {
                account_id: self.route.connection.id.clone(),
                public_model: self.public_model.to_owned(),
                upstream_model: self.route.binding.upstream_name.clone(),
            },
            license: task_license(&license)?,
        };
        Ok(AuthorizedRunnerTask { license, claims })
    }
}

/// An image dispatch is authorized against the admitted monetary request.
pub(crate) struct PluginImageProgress {
    pub(crate) requested_count: i64,
    pub(crate) confirmed_images: bool,
}

pub(crate) struct PluginImageAttempt<'a> {
    pub(crate) consumer: &'a ModelConsumer,
    pub(crate) reservation: &'a QuotaReservation,
    pub(crate) requested_count: i64,
    pub(crate) confirmed_images: bool,
    pub(crate) request_id: &'a GatewayRequestIds,
    pub(crate) public_model: &'a str,
    pub(crate) route: ConnectionRoute,
    pub(crate) binding: RunnerTaskBinding,
    pub(crate) request: UpstreamHttpRequest,
    pub(crate) payload_digest: String,
}

impl PluginImageAttempt<'_> {
    pub(crate) fn new<'a>(
        consumer: &'a ModelConsumer,
        reservation: &'a QuotaReservation,
        progress: PluginImageProgress,
        request_id: &'a GatewayRequestIds,
        public_model: &'a str,
        route: ConnectionRoute,
        wire: (RunnerTaskBinding, UpstreamHttpRequest),
    ) -> Result<PluginImageAttempt<'a>, ControlError> {
        let (binding, request) = wire;
        let encoded = Zeroizing::new(
            serde_json::to_vec(&request).map_err(|_| ControlError::GatewayRequestInvalid)?,
        );
        Ok(PluginImageAttempt {
            consumer,
            reservation,
            requested_count: progress.requested_count,
            confirmed_images: progress.confirmed_images,
            request_id,
            public_model,
            route,
            binding,
            request,
            payload_digest: sha256_hex(encoded.as_slice()),
        })
    }

    async fn authorize(
        &self,
        state: &ControlState,
        binding: &RunnerTaskBinding,
        payload: &[u8],
    ) -> Result<AuthorizedRunnerTask, ControlError> {
        if binding != &self.binding || sha256_hex(payload) != self.payload_digest {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let asset_fetch = self.request.method == "GET";
        if !asset_fetch {
            let current = state
                .credential_storage()?
                .api_key_connection_route(self.public_model)
                .await?
                .ok_or(ControlError::ModelAttemptStale)?;
            if current.connection.id != self.route.connection.id
                || current.connection.revision != self.route.connection.revision
                || current.binding != self.route.binding
                || current.credential.id != self.route.credential.id
                || current.credential.credential_revision
                    != self.route.credential.credential_revision
            {
                return Err(ControlError::ModelAttemptStale);
            }
            self.consumer.revalidate(state).await?;
        }
        self.consumer.revalidate(state).await?;
        if self.reservation.billing.is_none()
            || self.reservation._money_permit.is_none()
            || self.reservation.identity_id != self.consumer.identity_id()
            || self.reservation.request_id != self.request_id.as_str()
            || self.requested_count <= 0
            || (asset_fetch && !self.confirmed_images)
        {
            return Err(ControlError::QuotaSettlementConflict);
        }
        let expires_at = (state.now)()
            .checked_add(time::Duration::minutes(10))
            .ok_or(ControlError::DataIntegrityInvalid)?;
        if !asset_fetch {
            state
                .admit_model_attempt(
                    &self.reservation.identity_id,
                    self.public_model,
                    self.request_id.as_str(),
                )
                .await?;
        }
        let (license, _) = authorize_model_consumption_license(state, "member").await?;
        authorize_licensed_feature(&license, "runner", &state.product_version, (state.now)())
            .map_err(ControlError::Policy)?;
        let image_subject = ImageModelSubject {
            identity_id: self.reservation.identity_id.clone(),
            api_key_id: self.consumer.api_key_id().to_owned(),
            request_id: self.reservation.request_id.clone(),
            reservation_id: self.reservation.id.clone(),
            reserved_images: self.requested_count,
            reservation_expires_at: expires_at.unix_timestamp(),
        };
        let resource = ModelResource {
            account_id: self.route.connection.id.clone(),
            public_model: self.public_model.into(),
            upstream_model: self.route.binding.upstream_name.clone(),
        };
        let claims = if self.request.method == "GET" {
            TaskAuthorization::FetchAsset {
                subject: image_subject,
                resource,
                license: task_license(&license)?,
            }
        } else {
            TaskAuthorization::ImageModel {
                subject: image_subject,
                resource,
                license: task_license(&license)?,
            }
        };
        Ok(AuthorizedRunnerTask { license, claims })
    }
}

/// Constructed only by the provider preparation path. An attempt cannot be
/// dispatched with another credential binding or a substituted HTTP payload.
pub(crate) struct ModelAttempt<'a> {
    execution: &'a ModelExecution<'a>,
    candidate: GatewayRouteCandidate,
    binding: RunnerTaskBinding,
    request: UpstreamHttpRequest,
    payload_digest: String,
}

impl ModelAttempt<'_> {
    pub(crate) fn candidate(&self) -> &GatewayRouteCandidate {
        &self.candidate
    }
    pub(crate) fn binding(&self) -> &RunnerTaskBinding {
        &self.binding
    }
    pub(crate) fn request(&self) -> &UpstreamHttpRequest {
        &self.request
    }
    pub(crate) fn into_candidate(self) -> GatewayRouteCandidate {
        self.candidate
    }

    async fn authorize(
        &self,
        state: &ControlState,
        binding: &RunnerTaskBinding,
        payload: &[u8],
    ) -> Result<AuthorizedRunnerTask, ControlError> {
        if binding != &self.binding || sha256_hex(payload) != self.payload_digest {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let candidates = state
            .credential_storage()?
            .gateway_route_candidates(self.execution.public_model)
            .await?;
        if !candidates.iter().any(|candidate| {
            candidate.account_id == self.candidate.account_id
                && candidate.provider == self.candidate.provider
                && candidate.upstream_model == self.candidate.upstream_model
                && candidate.upstream_subject_id == self.candidate.upstream_subject_id
                && candidate.credential.id == self.candidate.credential.id
                && Some(candidate.credential.credential_revision) == binding.credential_revision
        }) {
            return Err(ControlError::ModelAttemptStale);
        }
        let (license, subject) = self.execution.revalidate(state).await?;
        state
            .admit_model_attempt(
                &subject.identity_id,
                self.execution.access_resource_model(),
                self.execution.request_id.as_str(),
            )
            .await?;
        let claims = TaskAuthorization::Model {
            subject,
            resource: ModelResource {
                account_id: self.candidate.account_id.clone(),
                public_model: self.execution.public_model.to_owned(),
                upstream_model: self.candidate.upstream_model.clone(),
            },
            license: task_license(&license)?,
        };
        Ok(AuthorizedRunnerTask { license, claims })
    }
}

#[derive(Clone, Copy)]
pub(crate) enum RunnerTaskAuthorization<'a> {
    Model(&'a ModelAttempt<'a>),
    PluginModel(&'a PluginModelAttempt<'a>),
    PluginImage(&'a PluginImageAttempt<'a>),
    DiscoverModels {
        actor: Option<&'a IdentityRecord>,
        credential: &'a DecryptedCredentialInstance,
    },
    PluginDiscover {
        actor: &'a IdentityRecord,
        connection: &'a ConnectionRecord,
        credential: &'a ConnectionCredential,
    },
    RefreshCredential {
        actor: Option<&'a IdentityRecord>,
        lease: &'a CredentialRefreshLease,
    },
    AuthorizeCredential {
        actor: &'a IdentityRecord,
        enrollment_id: &'a str,
        expires_at: OffsetDateTime,
    },
}

impl RunnerTaskAuthorization<'_> {
    pub(crate) fn command(self) -> TaskCommand {
        match self {
            Self::Model(_) | Self::PluginModel(_) => TaskCommand::Execute,
            Self::PluginImage(attempt) if attempt.request.method == "GET" => {
                TaskCommand::FetchAsset
            }
            Self::PluginImage(_) => TaskCommand::Execute,
            Self::DiscoverModels { .. } | Self::PluginDiscover { .. } => {
                TaskCommand::DiscoverModels
            }
            Self::RefreshCredential { .. } => TaskCommand::RefreshCredential,
            Self::AuthorizeCredential { .. } => TaskCommand::AuthorizeCredential,
        }
    }

    pub(crate) async fn authorize(
        self,
        state: &ControlState,
        binding: &RunnerTaskBinding,
        payload: &[u8],
    ) -> Result<AuthorizedRunnerTask, ControlError> {
        if let Self::Model(attempt) = self {
            return attempt.authorize(state, binding, payload).await;
        }
        if let Self::PluginModel(attempt) = self {
            return attempt.authorize(state, binding, payload).await;
        }
        if let Self::PluginImage(attempt) = self {
            return attempt.authorize(state, binding, payload).await;
        }
        // Automatic token renewal is part of model execution and did not
        // require the administrator's gateway feature at its original entry.
        let license = match self {
            Self::DiscoverModels { .. } | Self::PluginDiscover { .. } => {
                authorize_non_consuming_operation(state, BusinessOperationId::UpstreamSync)?
            }
            Self::RefreshCredential { actor: Some(_), .. } => {
                authorize_non_consuming_operation(state, BusinessOperationId::UpstreamRefresh)?
            }
            Self::AuthorizeCredential { .. } => {
                authorize_non_consuming_operation(state, BusinessOperationId::UpstreamAuthorize)?
            }
            Self::RefreshCredential { actor: None, .. } => {
                authorize_non_consuming_feature(state, "runner")?
            }
            Self::Model(_) | Self::PluginModel(_) | Self::PluginImage(_) => {
                unreachable!("model handled above")
            }
        };
        let signed_license = task_license(&license)?;
        let claims = match self {
            Self::Model(_) | Self::PluginModel(_) | Self::PluginImage(_) => {
                unreachable!("model handled above")
            }
            Self::PluginDiscover {
                actor,
                connection,
                credential,
            } => {
                crate::validate_admin_actor(state, actor).await?;
                let current = state
                    .credential_storage()?
                    .api_key_connection(&connection.id)
                    .await?
                    .ok_or(ControlError::UpstreamAccountNotFound)?;
                if current.0 != *connection || current.1 != *credential {
                    return Err(ControlError::UpstreamConnectionRevisionConflict);
                }
                let (expected_binding, expected_request) =
                    crate::plugins::connection_discovery_request(state, &current.0, &current.1)?
                        .ok_or(ControlError::UpstreamAccountInputInvalid)?;
                let request: UpstreamHttpRequest =
                    decode_exact_json(payload).map_err(|_| ControlError::GatewayRequestInvalid)?;
                if binding != &expected_binding || request != expected_request {
                    return Err(ControlError::GatewayRequestInvalid);
                }
                TaskAuthorization::DiscoverModels {
                    actor: maintenance_actor(state, Some(actor), ControlService::ModelCatalog)
                        .await?,
                    account_id: connection.id.clone(),
                    license: signed_license,
                }
            }
            Self::DiscoverModels { actor, credential } => {
                validate_credential_binding(binding, credential, "chatgpt.com")?;
                let request: UpstreamHttpRequest =
                    decode_exact_json(payload).map_err(|_| ControlError::GatewayRequestInvalid)?;
                validate_maintenance_request(&request, "GET", OPENAI_CODEX_MODELS_ENDPOINT)?;
                TaskAuthorization::DiscoverModels {
                    actor: maintenance_actor(state, actor, ControlService::ModelCatalog).await?,
                    account_id: credential.account_id.clone(),
                    license: signed_license,
                }
            }
            Self::RefreshCredential { actor, lease } => {
                validate_credential_binding(binding, &lease.credential, "auth.openai.com")?;
                let task: CredentialRefreshTask =
                    decode_exact_json(payload).map_err(|_| ControlError::CredentialInvalid)?;
                if task.credential_id != lease.credential.id
                    || task.expected_revision != lease.credential.credential_revision
                    || lease.lease_token.is_empty()
                {
                    return Err(ControlError::CredentialRefreshLeaseInvalid);
                }
                validate_maintenance_request(&task.request, "POST", OPENAI_TOKEN_ENDPOINT)?;
                TaskAuthorization::RefreshCredential {
                    actor: maintenance_actor(state, actor, ControlService::CredentialBroker)
                        .await?,
                    account_id: lease.credential.account_id.clone(),
                    lease_sha256: sha256_hex(lease.lease_token.as_bytes()),
                    lease_expires_at: OffsetDateTime::parse(&lease.expires_at, &Rfc3339)
                        .map_err(|_| ControlError::CredentialRefreshLeaseInvalid)?
                        .unix_timestamp(),
                    license: signed_license,
                }
            }
            Self::AuthorizeCredential {
                actor,
                enrollment_id,
                expires_at,
            } => {
                validate_admin_actor(state, actor).await?;
                if binding != &RunnerTaskBinding::enrollment("openai", "auth.openai.com") {
                    return Err(ControlError::OAuthSessionInvalid);
                }
                let request: UpstreamHttpRequest =
                    decode_exact_json(payload).map_err(|_| ControlError::OAuthSessionInvalid)?;
                validate_maintenance_request(&request, "POST", OPENAI_TOKEN_ENDPOINT)?;
                let sessions = state.oauth_sessions.lock().await;
                let session = sessions
                    .get(enrollment_id)
                    .ok_or(ControlError::OAuthSessionInvalid)?;
                if !session.exchanging
                    || session.admin_identity_id != actor.id
                    || session.expires_at != expires_at
                    || expires_at <= (state.now)()
                {
                    return Err(ControlError::OAuthSessionInvalid);
                }
                TaskAuthorization::AuthorizeCredential {
                    actor: AdminSubject {
                        identity_id: actor.id.clone(),
                    },
                    enrollment_id: enrollment_id.to_owned(),
                    session_expires_at: expires_at.unix_timestamp(),
                    license: signed_license,
                }
            }
        };
        if claims
            .deadline()
            .is_some_and(|expiry| expiry <= (state.now)().unix_timestamp())
        {
            return Err(ControlError::RunnerTicket(RunnerProtocolError::InvalidTime));
        }
        Ok(AuthorizedRunnerTask { license, claims })
    }
}

async fn maintenance_actor(
    state: &ControlState,
    actor: Option<&IdentityRecord>,
    service: ControlService,
) -> Result<MaintenanceActor, ControlError> {
    match actor {
        Some(actor) => {
            validate_admin_actor(state, actor).await?;
            Ok(MaintenanceActor::Admin {
                identity_id: actor.id.clone(),
            })
        }
        None => Ok(MaintenanceActor::Service { service }),
    }
}

fn validate_credential_binding(
    binding: &RunnerTaskBinding,
    credential: &DecryptedCredentialInstance,
    host: &str,
) -> Result<(), ControlError> {
    if credential.status != "active"
        || binding
            != &RunnerTaskBinding::credential(
                "openai",
                &credential.id,
                credential.credential_revision,
                host,
            )
    {
        return Err(ControlError::CredentialInvalid);
    }
    Ok(())
}

fn validate_maintenance_request(
    request: &UpstreamHttpRequest,
    method: &str,
    url: &str,
) -> Result<(), ControlError> {
    if request.method != method || request.url != url {
        return Err(ControlError::GatewayRequestInvalid);
    }
    Ok(())
}
