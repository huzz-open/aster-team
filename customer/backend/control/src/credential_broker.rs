use aster_storage::GatewayRouteCandidate;
use zeroize::Zeroizing;

use crate::{ControlError, ControlState, IdentityRecord, RunnerTaskBinding};

pub struct AuthLease {
    provider_id: String,
    credential_instance_id: String,
    credential_revision: u32,
    upstream_subject_id: String,
    upstream_host: String,
    material: Zeroizing<Vec<u8>>,
}

impl AuthLease {
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    pub fn upstream_subject_id(&self) -> &str {
        &self.upstream_subject_id
    }

    pub fn upstream_host(&self) -> &str {
        &self.upstream_host
    }

    pub fn material(&self) -> &[u8] {
        &self.material
    }

    pub(crate) fn runner_binding(&self) -> RunnerTaskBinding {
        RunnerTaskBinding::credential(
            &self.provider_id,
            &self.credential_instance_id,
            self.credential_revision,
            &self.upstream_host,
        )
    }
}

pub struct CredentialBroker<'a> {
    state: &'a ControlState,
}

impl<'a> CredentialBroker<'a> {
    pub fn new(state: &'a ControlState) -> Self {
        Self { state }
    }

    pub async fn lease(
        &self,
        candidate: &GatewayRouteCandidate,
        upstream_host: &str,
    ) -> Result<AuthLease, ControlError> {
        if candidate.credential.account_id != candidate.account_id
            || candidate.credential.status != "active"
        {
            return Err(ControlError::CredentialInvalid);
        }
        self.lease_current(
            &candidate.provider,
            &candidate.account_id,
            &candidate.upstream_subject_id,
            &candidate.credential.id,
            upstream_host,
        )
        .await
    }

    pub async fn lease_current(
        &self,
        provider_id: &str,
        account_id: &str,
        upstream_subject_id: &str,
        credential_instance_id: &str,
        upstream_host: &str,
    ) -> Result<AuthLease, ControlError> {
        let decrypted = self
            .state
            .decrypt_credential_instance(credential_instance_id)
            .await?;
        if decrypted.account_id != account_id || decrypted.status != "active" {
            return Err(ControlError::CredentialRefreshConflict);
        }
        Ok(AuthLease {
            provider_id: provider_id.to_owned(),
            credential_instance_id: decrypted.id,
            credential_revision: decrypted.credential_revision,
            upstream_subject_id: upstream_subject_id.to_owned(),
            upstream_host: upstream_host.to_owned(),
            material: decrypted.plaintext_payload,
        })
    }

    pub async fn refresh(&self, credential_instance_id: &str) -> Result<(), ControlError> {
        self.state
            .refresh_openai_credential_via_runner_inner(credential_instance_id, None)
            .await
    }

    pub(crate) async fn refresh_with_actor(
        &self,
        credential_instance_id: &str,
        actor: &IdentityRecord,
    ) -> Result<(), ControlError> {
        self.state
            .refresh_openai_credential_via_runner_inner(credential_instance_id, Some(actor))
            .await
    }
}
