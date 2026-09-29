//! Candidate dependency checks. These are not the complete cutover verdict:
//! runner evidence, asset availability and deployment compatibility also matter.
use super::*;

#[derive(serde::Serialize)]
pub(crate) struct DependencyReport {
    pub database_read_write: bool,
    pub business_state: bool,
    pub model_routes: bool,
}

impl DependencyReport {
    pub fn passed(&self) -> bool {
        self.database_read_write && self.business_state && self.model_routes
    }
}

impl ControlState {
    pub(crate) async fn configured_readiness_models(&self) -> Result<Vec<String>, ControlError> {
        // enabled_models() filters away models with no active credential. Such
        // a configured-but-broken model must fail readiness, not disappear.
        let mut models: Vec<_> = self
            .credential_storage()?
            .list_models()
            .await?
            .into_iter()
            .filter(|model| model.enabled)
            .map(|model| model.public_name)
            .collect();
        models.sort();
        if !aster_upgrade_core::runtime::ReadinessModelInventory::valid_models(&models) {
            return Err(ControlError::GatewayRequestInvalid);
        }
        Ok(models)
    }

    pub(crate) async fn check_dependencies(&self, models: &[String]) -> DependencyReport {
        let database = async {
            match self.storage.as_ref() {
                #[cfg(feature = "mariadb")]
                Some(ControlStorage::MariaDb(store)) => {
                    store.verify_supported_server().await.is_ok()
                        && store.verify_read_write().await.is_ok()
                }
                // SQLite is deliberately not eligible for the online candidate
                // path. Never turn a successful SQLite query into that promise.
                _ => false,
            }
        };
        let business = async {
            let state = self.verify_business_state().await.is_ok();
            let routes = state && self.verify_model_dependencies(models).await.is_ok();
            (state, routes)
        };
        let (database_read_write, (business_state, model_routes)) =
            tokio::join!(database, business);
        DependencyReport {
            database_read_write,
            business_state,
            model_routes,
        }
    }

    async fn verify_business_state(&self) -> Result<(), ControlError> {
        self.auth_core()?;
        self.credential_vault()?;
        self.task_issuer
            .as_ref()
            .ok_or(ControlError::InvalidUpstreamResponse)?;
        authorize_non_consuming_feature(self, "runner")?;
        authorize_model_consumption(self, "member").await?;
        // Installed runtime configuration must actually exist; its ordinary
        // read-time default is insufficient evidence for a candidate cutover.
        self.credential_storage()?
            .runtime_setting(RUNTIME_CONFIGURATION_KEY)
            .await?
            .ok_or(ControlError::DataIntegrityInvalid)?;
        let configuration = self.runtime_configuration().await?;
        if configuration.public_api_base_url.is_empty() {
            return Err(ControlError::DataIntegrityInvalid);
        }
        let (_, identities) = self.verified_seat_registry().await?;
        // Exercise the real quota snapshot and HMAC path on one deterministic
        // existing consumer. This is not an audit of every historical ledger.
        if let Some(identity) = identities.first() {
            self.verified_quota_snapshot(identity).await?;
        }
        Ok(())
    }

    async fn verify_model_dependencies(&self, models: &[String]) -> Result<(), ControlError> {
        if models.is_empty() || models.len() > 32 {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let storage = self.credential_storage()?;
        let available = storage.enabled_models().await?;
        for expected_model in models {
            let model = if gateway::capabilities::is_image_model(expected_model) {
                gateway::capabilities::select_image_host_model(
                    available.iter().map(|model| model.public_name.as_str()),
                )
                .ok_or(ControlError::ModelNotFound)?
            } else {
                expected_model.clone()
            };
            if !available
                .iter()
                .any(|candidate| candidate.public_name == model)
            {
                return Err(ControlError::ModelNotFound);
            }
            let candidates = storage.gateway_route_candidates(&model).await?;
            let vault = self.credential_vault()?;
            let decryptable = candidates.iter().any(|candidate| {
                let record = &candidate.credential;
                vault
                    .decrypt(
                        &CredentialContext {
                            credential_id: &record.id,
                            account_id: &record.account_id,
                            revision: record.credential_revision,
                        },
                        &credential_material(record),
                    )
                    .is_ok()
            });
            if !decryptable {
                return Err(ControlError::DataIntegrityInvalid);
            }
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "sqlite-dev"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn business_checks_verify_real_configuration_quota_and_credential_integrity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("customer.db");
        let store = Arc::new(StdMutex::new(
            aster_storage::SqlCipherStore::initialize(&path, &[41; 32]).unwrap(),
        ));
        let now = time::macros::datetime!(2026-08-28 0:00 UTC);
        let license = crate::tests::verified_license();
        let license_state = Arc::new(
            LicenseStateStore::new(directory.path().join("license-state"), &[42; 32]).unwrap(),
        );
        license_state.initialize(&license, now).unwrap();
        let state = ControlState::new("0.1.0", Some(license))
            .with_storage(ControlStorage::SqlCipher(store.clone()))
            .with_auth_core(AuthCore::new(&[43; 32], "installation-test").unwrap())
            .with_credential_vault(CredentialVault::new(&[44; 32], "installation-test").unwrap())
            .with_task_issuer(RunnerTaskIssuer::new(
                "task-key",
                SigningKey::from_bytes(&[45; 32]),
            ))
            .with_license_state(license_state)
            .with_now(now);
        let owner = state
            .initialize_owner_identity(
                "owner@example.com",
                "Owner",
                Zeroizing::new(b"owner-password-strong".to_vec()),
            )
            .await
            .unwrap();
        state
            .reset_admin_password(
                "owner@example.com",
                Zeroizing::new(b"owner-password-ready".to_vec()),
            )
            .await
            .unwrap();
        let actor = state
            .credential_storage()
            .unwrap()
            .identity_by_id(&owner.id)
            .await
            .unwrap()
            .unwrap();
        let member = state
            .create_member_identity(
                &actor,
                "member@example.com",
                "Member",
                Zeroizing::new(b"member-password-strong".to_vec()),
            )
            .await
            .unwrap();
        assert!(
            state.verify_business_state().await.is_err(),
            "missing configuration must not pass through defaults"
        );
        state
            .initialize_runtime_configuration("https://api.example.com")
            .await
            .unwrap();
        state.verify_business_state().await.unwrap();
        {
            let mut store = store.lock().unwrap();
            store
                .insert_upstream_account_unchecked(
                    "account-test",
                    "openai",
                    "subject-test",
                    "owner@example.com",
                    "2026-08-28T00:00:00.000Z",
                )
                .unwrap();
            store
                .replace_account_models(
                    "account-test",
                    &[aster_storage::DiscoveredModel {
                        id: "model-test".into(),
                        public_name: "gpt-test".into(),
                        display_name: "Test".into(),
                        upstream_name: "upstream-test".into(),
                    }],
                    "2026-08-28T00:00:00.000Z",
                )
                .unwrap();
        }
        let models = vec![
            "gpt-image-1".into(),
            "gpt-image-2".into(),
            "gpt-image-2.5-flare".into(),
            "gpt-image-2.5-sunburst".into(),
            "gpt-test".into(),
        ];
        assert_eq!(state.configured_readiness_models().await.unwrap(), models);
        assert!(
            state
                .credential_storage()
                .unwrap()
                .enabled_models()
                .await
                .unwrap()
                .is_empty()
        );
        {
            use tower::ServiceExt as _;
            let runtime = crate::runtime_control::RuntimeControl::new(
                state.clone(),
                "installation-test".into(),
                aster_upgrade_core::ReleaseSlot::Green,
                &[91; 32],
            )
            .unwrap();
            let authorization = format!(
                "Bearer {}",
                base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, [91; 32],)
            );
            let response = runtime
                .router()
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/v1/status")
                        .header("Authorization", &authorization)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let observed: aster_upgrade_core::runtime::RuntimeSnapshot = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 8192)
                    .await
                    .unwrap(),
            )
            .unwrap();
            let response = runtime
                .router()
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/v1/readiness-models")
                        .header("Authorization", &authorization)
                        .header("Content-Type", "application/json")
                        .body(Body::from(
                            serde_json::json!({"instance_id":observed.instance_id,
                    "expected_revision":observed.lifecycle.revision})
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let inventory: aster_upgrade_core::runtime::ReadinessModelInventory =
                serde_json::from_slice(
                    &axum::body::to_bytes(response.into_body(), 8192)
                        .await
                        .unwrap(),
                )
                .unwrap();
            assert!(inventory.valid_for(&observed));
            assert_eq!(
                inventory.models, models,
                "missing credentials cannot hide an enabled model"
            );
        }
        assert!(
            state.verify_model_dependencies(&models).await.is_err(),
            "a model without a credential is not routable"
        );
        state
            .create_credential_instance(
                "account-test",
                b"fixture-refresh",
                br#"{"access_token":"fixture-access","refresh_token":"fixture-refresh"}"#,
                "2026-08-28T01:00:00.000Z",
            )
            .await
            .unwrap();
        state.verify_model_dependencies(&models).await.unwrap();
        state
            .verify_model_dependencies(&["gpt-image-2".into()])
            .await
            .unwrap();
        assert!(
            state
                .verify_model_dependencies(&["missing-model".into()])
                .await
                .is_err()
        );
        let wrong_key = state
            .clone()
            .with_credential_vault(CredentialVault::new(&[46; 32], "installation-test").unwrap());
        assert!(wrong_key.verify_model_dependencies(&models).await.is_err());
        let report = state.check_dependencies(&models).await;
        assert!(report.business_state && report.model_routes);
        assert!(
            !report.database_read_write && !report.passed(),
            "SQLite must remain ineligible for online upgrade"
        );
        let raw = rusqlite::Connection::open(path).unwrap();
        let before: String = raw
            .query_row(
                "SELECT value FROM runtime_settings WHERE key=?",
                [RUNTIME_CONFIGURATION_KEY],
                |row| row.get(0),
            )
            .unwrap();
        raw.execute(
            "UPDATE runtime_settings SET value='{}' WHERE key=?",
            [RUNTIME_CONFIGURATION_KEY],
        )
        .unwrap();
        assert!(state.verify_business_state().await.is_err());
        raw.execute(
            "UPDATE runtime_settings SET value=? WHERE key=?",
            [&before, RUNTIME_CONFIGURATION_KEY],
        )
        .unwrap();
        state.verify_business_state().await.unwrap();
        raw.execute(
            "UPDATE user_balances SET balance_tokens=balance_tokens+1 WHERE identity_id=?",
            [&member.id],
        )
        .unwrap();
        assert!(
            state.verify_business_state().await.is_err(),
            "tampered quota must fail even though DB queries succeed"
        );
    }
}
