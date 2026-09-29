use aster_license_core::{
    catalog::{CapabilityId, QuotaId},
    entitlements::{EntitlementError, Entitlements, QuotaLimit},
};

const VECTOR: &str = include_str!("../../../../contracts/test-vectors/entitlements.v1.json");

fn sample() -> Entitlements {
    serde_json::from_str(VECTOR).expect("shared entitlement vector")
}

#[test]
fn shared_vector_round_trips_without_expanding_rights() {
    let value = sample();
    value.validate().expect("valid entitlements");
    assert_eq!(
        serde_json::to_value(&value).unwrap(),
        serde_json::from_str::<serde_json::Value>(VECTOR).unwrap()
    );
    assert_eq!(
        value.quota(QuotaId::MemberSeats),
        Some(QuotaLimit::Limited { value: 3 })
    );
    assert_eq!(
        value.quota(QuotaId::ApiKeysPerMember),
        Some(QuotaLimit::Limited { value: 1 })
    );
}

#[test]
fn missing_duplicate_and_unsupported_grants_fail_closed() {
    let mut value = sample();
    value.quotas.pop();
    assert_eq!(value.validate(), Err(EntitlementError::IncompleteQuotas));
    let mut value = sample();
    value.quotas[1].id = QuotaId::MemberSeats;
    assert_eq!(value.validate(), Err(EntitlementError::IncompleteQuotas));
    let mut value = sample();
    value.features.push(CapabilityId::Member);
    assert_eq!(value.validate(), Err(EntitlementError::DuplicateCapability));
    let mut value = sample();
    value.catalog_version += 1;
    assert_eq!(value.validate(), Err(EntitlementError::CatalogVersion));
    assert!(serde_json::from_str::<Entitlements>(&VECTOR.replace("gateway", "unknown")).is_err());
    assert!(
        serde_json::from_str::<Entitlements>(&VECTOR.replace("member_seats", "unknown")).is_err()
    );
    assert!(
        serde_json::from_str::<Entitlements>(&VECTOR.replacen("\"catalog_version\": 1,", "", 1))
            .is_err()
    );
    assert!(
        serde_json::from_str::<Entitlements>(&VECTOR.replacen(
            "\"catalog_version\": 1,",
            "\"catalog_version\": 1, \"catalog_version\": 1,",
            1
        ))
        .is_err()
    );
}

#[test]
fn finite_zero_and_unlimited_are_distinct_and_issuer_scope_is_enforced() {
    let zero = QuotaLimit::Limited { value: 0 };
    assert!(zero.permits(0));
    assert!(!zero.permits(1));
    assert!(QuotaLimit::Unlimited {}.permits(u32::MAX));
    let ceiling = sample();
    let mut grant = sample();
    grant.ensure_within(&ceiling).unwrap();
    grant.quotas[0].limit = QuotaLimit::Unlimited {};
    assert_eq!(
        grant.ensure_within(&ceiling),
        Err(EntitlementError::IssuerScope)
    );
    grant.quotas[0].limit = QuotaLimit::Limited { value: 4 };
    assert_eq!(
        grant.ensure_within(&ceiling),
        Err(EntitlementError::IssuerScope)
    );
    grant.quotas[0].limit = zero;
    grant.ensure_within(&ceiling).unwrap();
    let mut narrow = sample();
    narrow.features.clear();
    narrow.validate().unwrap();
    assert_eq!(
        grant.ensure_within(&narrow),
        Err(EntitlementError::IssuerScope)
    );
}

#[test]
fn limit_encoding_cannot_use_missing_null_or_unknown_fields_for_unlimited() {
    for invalid in [
        r#"{}"#,
        r#"null"#,
        r#"{"mode":"limited"}"#,
        r#"{"mode":"limited","value":-1}"#,
        r#"{"mode":"limited","value":4294967296}"#,
        r#"{"mode":"unlimited","value":3}"#,
        r#"{"mode":"all"}"#,
        r#"{"mode":"limited","value":1,"value":2}"#,
    ] {
        assert!(
            serde_json::from_str::<QuotaLimit>(invalid).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn catalog_ids_only_accept_json_strings_like_go_and_the_schema() {
    assert!(serde_json::from_str::<CapabilityId>(r#"{"gateway":null}"#).is_err());
    assert!(serde_json::from_str::<QuotaId>(r#"{"member_seats":null}"#).is_err());
}

#[test]
fn symbolic_sets_require_symbolic_ceiling_and_keep_quota_limits() {
    use aster_license_core::catalog::FeatureSetId;
    let explicit = sample();
    let mut subscription = sample();
    subscription.features.clear();
    subscription.feature_sets = vec![FeatureSetId::Standard];
    assert_eq!(subscription.effective_features(), explicit.features);
    assert_eq!(
        subscription.ensure_within(&explicit),
        Err(EntitlementError::IssuerScope)
    );
    assert!(explicit.ensure_within(&subscription).is_ok());
    assert!(subscription.ensure_within(&subscription).is_ok());
    let mut excessive = subscription.clone();
    excessive.quotas[0].limit = QuotaLimit::Unlimited {};
    assert_eq!(
        excessive.ensure_within(&subscription),
        Err(EntitlementError::IssuerScope)
    );
    subscription.feature_sets.push(FeatureSetId::Standard);
    assert_eq!(
        subscription.validate(),
        Err(EntitlementError::DuplicateCapability)
    );
}

#[test]
fn malformed_feature_sets_are_not_default_grants() {
    for sets in [
        serde_json::json!(null),
        serde_json::json!(["*"]),
        serde_json::json!(["unknown"]),
        serde_json::json!([{"standard": null}]),
    ] {
        let mut value = serde_json::to_value(sample()).unwrap();
        value["feature_sets"] = sets;
        assert!(serde_json::from_value::<Entitlements>(value).is_err());
    }
}
