use super::*;

pub(super) fn model_authorization() -> TaskAuthorization {
    TaskAuthorization::Model {
        subject: ModelSubject {
            identity_id: "identity-test-001".into(),
            api_key_id: "api-key-test-001".into(),
            request_id: "request-test-001".into(),
            reservation_id: "reservation-test-001".into(),
            reserved_tokens: 100,
            reservation_expires_at: datetime!(2026-08-28 0:05 UTC).unix_timestamp(),
        },
        resource: ModelResource {
            account_id: "account-test-001".into(),
            public_model: "gpt-test".into(),
            upstream_model: "gpt-test-upstream".into(),
        },
        license: license(),
    }
}

fn license() -> TaskLicense {
    TaskLicense {
        license_id: "license-test-001".into(),
        license_sha256: "ab".repeat(32),
        expiry: SignedExpiry::Never {},
    }
}

fn authorizations() -> Vec<TaskAuthorization> {
    let mut cases = vec![
        model_authorization(),
        TaskAuthorization::DiscoverModels {
            actor: MaintenanceActor::Service {
                service: ControlService::ModelCatalog,
            },
            account_id: "account-test-001".into(),
            license: license(),
        },
        TaskAuthorization::RefreshCredential {
            actor: MaintenanceActor::Service {
                service: ControlService::CredentialBroker,
            },
            account_id: "account-test-001".into(),
            lease_sha256: "cd".repeat(32),
            lease_expires_at: datetime!(2026-08-28 0:05 UTC).unix_timestamp(),
            license: license(),
        },
        TaskAuthorization::AuthorizeCredential {
            actor: AdminSubject {
                identity_id: "admin-test-001".into(),
            },
            enrollment_id: "enrollment-test-001".into(),
            session_expires_at: datetime!(2026-08-28 0:05 UTC).unix_timestamp(),
            license: license(),
        },
    ];
    for case in cases.clone() {
        let mut value = serde_json::to_value(case).unwrap();
        value["license"]["expiry"] = serde_json::json!({ "kind": "fixed", "expires_at": datetime!(2026-08-28 0:05 UTC).unix_timestamp() });
        if value["actor"]["kind"] == "service" {
            value["actor"] =
                serde_json::json!({ "kind": "admin", "identity_id": "admin-test-001" });
        }
        cases.push(serde_json::from_value(value).unwrap());
    }
    cases
}

#[test]
fn model_name_limits_count_unicode_scalars_and_reject_controls() {
    for model in [
        "a".repeat(256),
        "模".repeat(256),
        "🚀".repeat(256),
        "e\u{301}".repeat(128),
    ] {
        let mut value = serde_json::to_value(model_authorization()).unwrap();
        value["resource"]["public_model"] = serde_json::json!(model);
        assert!(issue(serde_json::from_value(value.clone()).unwrap()).is_ok());
        value["resource"]["public_model"] = serde_json::json!(format!("{model}a"));
        assert!(issue(serde_json::from_value(value).unwrap()).is_err());
    }
    for control in ['\u{0}', '\u{7f}', '\u{85}'] {
        let mut value = serde_json::to_value(model_authorization()).unwrap();
        value["resource"]["upstream_model"] = serde_json::json!(format!("model{control}"));
        assert!(issue(serde_json::from_value(value).unwrap()).is_err());
    }
}

#[test]
fn duplicate_raw_fields_are_rejected_before_signature_verification() {
    fn first_field<'a>(
        value: &'a serde_json::Value,
        wanted: &str,
    ) -> Option<&'a serde_json::Value> {
        for (key, child) in value.as_object()? {
            if key == wanted {
                return Some(child);
            }
            if let Some(found) = first_field(child, wanted) {
                return Some(found);
            }
        }
        None
    }
    let mut keys = TrustedTaskKeys::new();
    keys.insert(
        "runner-task-test-01",
        SigningKey::from_bytes(&[71_u8; 32]).verifying_key(),
    )
    .unwrap();
    for case in authorizations() {
        let value = serde_json::to_value(issue(case).unwrap()).unwrap();
        let raw = serde_json::to_string(&value).unwrap();
        for key in [
            "authorization",
            "kind",
            "identity_id",
            "expires_at",
            "license_id",
            "credential_revision",
        ] {
            let Some(field) = first_field(&value, key) else {
                continue;
            };
            let needle = format!("\"{key}\":{}", serde_json::to_string(field).unwrap());
            assert!(raw.contains(&needle));
            // Both occurrences have the valid original value. The original
            // signature would still verify if a decoder silently kept either.
            let duplicate = raw.replacen(&needle, &format!("{needle},{needle}"), 1);
            assert_eq!(
                verify_task_ticket(
                    duplicate.as_bytes(),
                    &keys,
                    "runner-test-001",
                    b"payload",
                    datetime!(2026-08-28 0:00 UTC)
                ),
                Err(RunnerProtocolError::InvalidJson),
                "duplicate {key}"
            );
        }
    }
}

#[test]
fn numeric_authorization_edges_do_not_wrap_or_become_unlimited() {
    let original = serde_json::to_value(issue(model_authorization()).unwrap()).unwrap();
    for amount in [0, -1] {
        let mut value = original.clone();
        value["authorization"]["subject"]["reserved_tokens"] = serde_json::json!(amount);
        resign(&mut value);
        assert_eq!(verify(&value), Err(RunnerProtocolError::InvalidField));
    }
    for (pointer, value) in [
        (
            "/authorization/subject/reserved_tokens",
            serde_json::json!(i64::MAX),
        ),
        ("/credential_revision", serde_json::json!(u32::MAX)),
        (
            "/authorization/subject/reservation_expires_at",
            serde_json::json!(i64::MAX),
        ),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        resign(&mut changed);
        assert!(verify(&changed).is_ok(), "valid maximum {pointer}");
    }
    for (pointer, value) in [
        (
            "/authorization/subject/reserved_tokens",
            serde_json::json!(i64::MAX as u64 + 1),
        ),
        (
            "/credential_revision",
            serde_json::json!(u64::from(u32::MAX) + 1),
        ),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert_eq!(
            verify(&changed),
            Err(RunnerProtocolError::InvalidJson),
            "overflow {pointer}"
        );
    }
    let mut changed = original;
    changed["authorization"]["subject"]["reservation_expires_at"] = serde_json::json!(i64::MIN);
    resign(&mut changed);
    assert_eq!(verify(&changed), Err(RunnerProtocolError::InvalidField));
}

fn issue(authorization: TaskAuthorization) -> Result<TaskTicketDocument, RunnerProtocolError> {
    let signing_key = SigningKey::from_bytes(&[71_u8; 32]);
    let enrollment = authorization.command() == TaskCommand::AuthorizeCredential;
    issue_task_ticket(TaskTicketIssue {
        key_id: "runner-task-test-01",
        signing_key: &signing_key,
        task_id: "task-test-001",
        runner_id: "runner-test-001",
        provider_id: "openai",
        credential_instance_id: (!enrollment).then_some("credential-test-001"),
        credential_revision: (!enrollment).then_some(7),
        upstream_host: "api.openai.com",
        command: authorization.command(),
        authorization,
        payload: b"payload",
        nonce: "nonce-test-001",
        now: datetime!(2026-08-28 0:00 UTC),
        execution_timeout_ms: MAX_TASK_EXECUTION_MILLISECONDS,
    })
}

fn verify(value: &serde_json::Value) -> Result<VerifiedTaskTicket, RunnerProtocolError> {
    let mut keys = TrustedTaskKeys::new();
    keys.insert(
        "runner-task-test-01",
        SigningKey::from_bytes(&[71_u8; 32]).verifying_key(),
    )
    .unwrap();
    verify_task_ticket(
        &serde_json::to_vec(value).unwrap(),
        &keys,
        "runner-test-001",
        b"payload",
        datetime!(2026-08-28 0:00 UTC),
    )
}

fn resign(value: &mut serde_json::Value) {
    value.as_object_mut().unwrap().remove("signature");
    let signature = SigningKey::from_bytes(&[71_u8; 32]).sign(&canonicalize(value).unwrap());
    value["signature"] = serde_json::json!(encode_base64url(&signature.to_bytes()));
}

#[test]
fn each_command_round_trips_its_distinct_authorization() {
    for authorization in authorizations() {
        let document = issue(authorization.clone()).unwrap();
        let verified = verify(&serde_json::to_value(document).unwrap()).unwrap();
        assert_eq!(verified.claims().authorization, authorization);
    }
}

#[test]
fn every_nested_field_is_required_and_unknown_fields_are_rejected() {
    fn check_objects(original: &serde_json::Value, pointer: &str) {
        let object = original.pointer(pointer).unwrap().as_object().unwrap();
        for (key, child) in object {
            let mut missing = original.clone();
            missing
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(key);
            resign(&mut missing);
            assert_eq!(
                verify(&missing),
                Err(RunnerProtocolError::InvalidJson),
                "missing {pointer}/{key}"
            );
            if child.is_object() {
                check_objects(original, &format!("{pointer}/{key}"));
            }
        }
        let mut extra = original.clone();
        extra.pointer_mut(pointer).unwrap()["bypass"] = serde_json::json!(true);
        resign(&mut extra);
        assert_eq!(
            verify(&extra),
            Err(RunnerProtocolError::InvalidJson),
            "unknown at {pointer}"
        );
    }
    for authorization in authorizations() {
        let original = serde_json::to_value(issue(authorization).unwrap()).unwrap();
        check_objects(&original, "/authorization");
        for key in [
            "authorization",
            "credential_instance_id",
            "credential_revision",
        ] {
            let mut missing = original.clone();
            missing.as_object_mut().unwrap().remove(key);
            resign(&mut missing);
            assert_eq!(
                verify(&missing),
                Err(RunnerProtocolError::InvalidJson),
                "missing {key}"
            );
        }
    }
}

#[test]
fn signed_command_actor_and_credential_mismatches_are_rejected() {
    for authorization in authorizations() {
        let original = serde_json::to_value(issue(authorization).unwrap()).unwrap();
        for command in [
            "execute",
            "discover_models",
            "refresh_credential",
            "authorize_credential",
        ] {
            if original["command"] == command {
                continue;
            }
            let mut wrong = original.clone();
            wrong["command"] = serde_json::json!(command);
            resign(&mut wrong);
            assert_eq!(verify(&wrong), Err(RunnerProtocolError::InvalidField));
        }
        let mut wrong = original.clone();
        if original["command"] == "authorize_credential" {
            wrong["credential_instance_id"] = serde_json::json!("credential-test-001");
            wrong["credential_revision"] = serde_json::json!(7);
        } else {
            wrong["credential_instance_id"] = serde_json::Value::Null;
            wrong["credential_revision"] = serde_json::Value::Null;
        }
        resign(&mut wrong);
        assert_eq!(verify(&wrong), Err(RunnerProtocolError::InvalidField));
        if original["authorization"]["actor"]["kind"] == "service" {
            let mut wrong = original.clone();
            wrong["authorization"]["actor"]["service"] =
                serde_json::json!(if original["command"] == "discover_models" {
                    "credential_broker"
                } else {
                    "model_catalog"
                });
            resign(&mut wrong);
            assert_eq!(verify(&wrong), Err(RunnerProtocolError::InvalidField));
        }
    }
}

#[test]
fn authorization_cannot_be_changed_without_the_signing_key() {
    let original = serde_json::to_value(issue(model_authorization()).unwrap()).unwrap();
    for (pointer, replacement) in [
        (
            "/authorization/subject/identity_id",
            serde_json::json!("other-identity"),
        ),
        (
            "/authorization/subject/api_key_id",
            serde_json::json!("other-api-key"),
        ),
        (
            "/authorization/subject/request_id",
            serde_json::json!("other-request"),
        ),
        (
            "/authorization/subject/reservation_id",
            serde_json::json!("other-reservation"),
        ),
        (
            "/authorization/subject/reserved_tokens",
            serde_json::json!(999),
        ),
        (
            "/authorization/subject/reservation_expires_at",
            serde_json::json!(datetime!(2026-08-29 0:00 UTC).unix_timestamp()),
        ),
        (
            "/authorization/resource/account_id",
            serde_json::json!("other-account"),
        ),
        (
            "/authorization/resource/public_model",
            serde_json::json!("other-model"),
        ),
        (
            "/authorization/resource/upstream_model",
            serde_json::json!("other-upstream"),
        ),
        (
            "/authorization/license/license_id",
            serde_json::json!("other-license"),
        ),
        (
            "/authorization/license/license_sha256",
            serde_json::json!("ef".repeat(32)),
        ),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        assert_eq!(
            verify(&changed),
            Err(RunnerProtocolError::InvalidSignature),
            "{pointer}"
        );
    }
}

#[test]
fn license_and_operation_deadlines_bound_the_outer_ticket() {
    let issued_at = datetime!(2026-08-28 0:00 UTC).unix_timestamp();
    for authorization in authorizations() {
        for offset in [-1, 0, 1, 60, 120, 121] {
            let mut value = serde_json::to_value(&authorization).unwrap();
            value["license"]["expiry"] =
                serde_json::json!({ "kind": "fixed", "expires_at": issued_at + offset });
            let bounded = serde_json::from_value(value).unwrap();
            let document = issue(bounded);
            if offset <= 0 {
                assert!(document.is_err());
                continue;
            }
            let document = document.unwrap();
            assert_eq!(document.claims.expires_at, issued_at + offset.min(120));
            assert!(verify(&serde_json::to_value(document).unwrap()).is_ok());
        }
        let pointer = match authorization.command() {
            TaskCommand::Execute => "/subject/reservation_expires_at",
            TaskCommand::RefreshCredential => "/lease_expires_at",
            TaskCommand::AuthorizeCredential => "/session_expires_at",
            TaskCommand::DiscoverModels | TaskCommand::Probe | TaskCommand::FetchAsset => continue,
        };
        for offset in [-1, 0, 1, 60, 121] {
            let mut value = serde_json::to_value(&authorization).unwrap();
            *value.pointer_mut(pointer).unwrap() = serde_json::json!(issued_at + offset);
            let document = issue(serde_json::from_value(value).unwrap());
            if offset <= 0 {
                assert!(document.is_err());
                continue;
            }
            let mut value = serde_json::to_value(document.unwrap()).unwrap();
            assert_eq!(value["expires_at"], issued_at + offset.min(120));
            assert!(verify(&value).is_ok());
            if offset < 120 {
                value["expires_at"] = serde_json::json!(issued_at + 120);
                resign(&mut value);
                assert_eq!(verify(&value), Err(RunnerProtocolError::InvalidField));
            }
        }
    }
}

#[test]
fn old_v2_tickets_are_not_a_fallback() {
    let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../../../contracts/test-vectors/runner-task.v2.json"
    ))
    .unwrap();
    let mut keys = TrustedTaskKeys::new();
    keys.insert_spki_base64url(
        "runner-task-vector-v2",
        fixture["public_key_spki"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        verify_task_ticket(
            &serde_json::to_vec(&fixture["ticket"]).unwrap(),
            &keys,
            "runner-vector-123",
            fixture["payload_json"].as_str().unwrap().as_bytes(),
            OffsetDateTime::from_unix_timestamp(1_787_565_660).unwrap()
        ),
        Err(RunnerProtocolError::InvalidJson)
    );
    let mut changed = serde_json::to_value(issue(model_authorization()).unwrap()).unwrap();
    changed["schema"] = serde_json::json!("aster.runner-task.v2");
    resign(&mut changed);
    assert_eq!(verify(&changed), Err(RunnerProtocolError::InvalidField));
}
