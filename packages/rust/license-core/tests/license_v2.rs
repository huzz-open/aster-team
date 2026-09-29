use aster_license_core::{
    LicenseError, TrustedLicenseKeys, VerifiedProductLicense,
    catalog::QuotaId,
    entitlements::{EntitlementError, Entitlements, QuotaLimit},
    v2::{self, BindingKind, ExpiryKind, IssuerPolicy, SourceKind},
    verify_product_license,
};
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../../../contracts/test-vectors/license.v2.json"
    ))
    .unwrap()
}
fn policy(ceiling: Entitlements, source: SourceKind) -> IssuerPolicy {
    IssuerPolicy {
        sources: vec![source],
        bindings: vec![BindingKind::Unbound, BindingKind::Installation],
        expiries: vec![ExpiryKind::Fixed, ExpiryKind::None],
        entitlement_ceiling: ceiling,
    }
}
fn document(index: usize) -> v2::Document {
    serde_json::from_value(fixture()["cases"][index]["document"].clone()).unwrap()
}
fn keys(policy: IssuerPolicy) -> TrustedLicenseKeys {
    let mut keys = TrustedLicenseKeys::new();
    keys.insert_scoped_spki_base64url(
        "test-only-v2",
        fixture()["public_key_spki"].as_str().unwrap(),
        policy,
    )
    .unwrap();
    keys
}
fn encode(document: &v2::Document) -> Vec<u8> {
    serde_json::to_vec(document).unwrap()
}

#[test]
fn shared_vectors_have_identical_canonical_bytes_and_signatures_in_rust() {
    let key = SigningKey::from_bytes(&[42; 32]);
    for case in fixture()["cases"].as_array().unwrap() {
        let doc: v2::Document = serde_json::from_value(case["document"].clone()).unwrap();
        let canonical = v2::canonical_claims(&doc.claims).unwrap();
        assert_eq!(
            canonical,
            case["canonical_claims"].as_str().unwrap().as_bytes()
        );
        assert_eq!(v2::sign(doc.claims.clone(), &key).unwrap(), doc);
        let verified = v2::verify(
            &encode(&doc),
            &keys(policy(
                doc.claims.entitlements.clone(),
                doc.claims.source.kind(),
            )),
        )
        .unwrap();
        assert_eq!(verified.claims(), &doc.claims);
        assert_eq!(verified.document(), &doc);
    }
}

#[test]
fn every_authorization_field_is_signed_and_missing_modes_are_rejected() {
    let doc = document(0);
    let trusted = keys(policy(
        doc.claims.entitlements.clone(),
        doc.claims.source.kind(),
    ));
    let original = serde_json::to_value(&doc).unwrap();
    for (pointer, replacement) in [
        ("/claims/edition", json!("paid")),
        ("/claims/plan_id", json!("another")),
        ("/claims/plan_version", json!(2)),
        ("/claims/serial", json!("NEW")),
        ("/claims/license_id", json!("another")),
        ("/claims/source/distribution_id", json!("another")),
        ("/claims/entitlements/quotas/0/limit/value", json!(4)),
        ("/claims/entitlements/features", json!([])),
        ("/claims/minimum_version", json!("1.2.4")),
        (
            "/claims/validity/not_before",
            json!("2026-09-02T00:00:00.000Z"),
        ),
        ("/claims/issued_at", json!("2026-08-31T00:00:00.000Z")),
        ("/claims/validity/expiry", json!({"mode":"none"})),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = replacement;
        assert_eq!(
            v2::verify(&serde_json::to_vec(&changed).unwrap(), &trusted).unwrap_err(),
            v2::Error::License(LicenseError::InvalidSignature),
            "{pointer}"
        );
    }
    let mut missing = original.clone();
    missing["claims"].as_object_mut().unwrap().remove("binding");
    assert!(v2::verify(&serde_json::to_vec(&missing).unwrap(), &trusted).is_err());
    for binding in [
        json!({}),
        json!({"mode":"unknown"}),
        json!({"mode":"unbound","installation_id":"test_installation"}),
        Value::Null,
    ] {
        let mut changed = original.clone();
        changed["claims"]["binding"] = binding;
        assert!(v2::verify(&serde_json::to_vec(&changed).unwrap(), &trusted).is_err());
    }
}

#[test]
fn signed_licenses_cannot_exceed_free_issuer_scope() {
    let key = SigningKey::from_bytes(&[42; 32]);
    let doc = document(0);
    let mut scope = policy(doc.claims.entitlements.clone(), doc.claims.source.kind());
    scope.sources = vec![SourceKind::FreeDistribution];
    scope.bindings = vec![BindingKind::Unbound];
    scope.expiries = vec![ExpiryKind::Fixed];
    let trusted = keys(scope);
    for index in [1, 2, 3] {
        assert_eq!(
            v2::verify(&encode(&document(index)), &trusted).unwrap_err(),
            v2::Error::IssuerScope
        );
    }
    let mut claims = doc.claims;
    claims.entitlements.quotas[0].limit =
        aster_license_core::entitlements::QuotaLimit::Limited { value: 4 };
    let over = v2::sign(claims, &key).unwrap();
    assert_eq!(
        v2::verify(&encode(&over), &trusted).unwrap_err(),
        v2::Error::Entitlements(EntitlementError::IssuerScope)
    );
    let mut commercial = document(2).claims;
    commercial.binding = v2::Binding::Unbound {};
    assert_eq!(
        v2::sign(commercial, &key).unwrap_err(),
        v2::Error::SourcePolicy
    );
}

#[test]
fn strict_wire_rejects_duplicate_fields_nulls_unknown_fields_and_trailing_input() {
    let doc = document(0);
    let raw = String::from_utf8(encode(&doc)).unwrap();
    let trusted = keys(policy(
        doc.claims.entitlements.clone(),
        doc.claims.source.kind(),
    ));
    for (from, to) in [
        (
            "\"mode\":\"unbound\"",
            "\"mode\":\"unbound\",\"mode\":\"unbound\"",
        ),
        ("\"mode\":\"unbound\"", "\"mode\":\"unbound\",\"skip\":true"),
        ("\"mode\":\"limited\"", "\"mode\":\"limited\",\"value\":3"),
        ("\"plan_version\":1", "\"plan_version\":null"),
        ("\"plan_version\":1", "\"Plan_Version\":1"),
        (
            "\"catalog_version\":1",
            "\"catalog_version\":1,\"catalog_version\":1",
        ),
        (
            "\"kind\":\"free_distribution\"",
            "\"kind\":\"free_distribution\",\"kind\":\"free_distribution\"",
        ),
    ] {
        assert!(raw.contains(from));
        assert!(
            v2::verify(raw.replacen(from, to, 1).as_bytes(), &trusted).is_err(),
            "{to}"
        );
    }
    for suffix in [" {}", " garbage", " false"] {
        assert!(v2::verify(format!("{raw}{suffix}").as_bytes(), &trusted).is_err());
    }
    let no_expiry = document(1);
    let raw = String::from_utf8(encode(&no_expiry)).unwrap().replace(
        "\"mode\":\"none\"",
        "\"mode\":\"none\",\"expires_at\":\"2027-09-01T00:00:00.000Z\"",
    );
    assert!(v2::verify(raw.as_bytes(), &trusted).is_err());
}

#[test]
fn v2_keys_cannot_be_reused_as_legacy_unrestricted_keys_and_duplicates_are_atomic() {
    let signing = SigningKey::from_bytes(&[42; 32]);
    let other = SigningKey::from_bytes(&[43; 32]);
    let doc = document(0);
    let scope = policy(doc.claims.entitlements.clone(), doc.claims.source.kind());
    let mut trusted = keys(scope.clone());
    assert!(
        trusted
            .insert("test-only-v2", other.verifying_key())
            .is_err()
    );
    assert!(
        trusted
            .insert_scoped("test-only-v2", other.verifying_key(), scope.clone())
            .is_err()
    );
    assert!(v2::verify(&encode(&doc), &trusted).is_ok());
    let mut legacy = TrustedLicenseKeys::new();
    legacy
        .insert("test-only-v2", signing.verifying_key())
        .unwrap();
    assert!(
        legacy
            .insert_scoped("test-only-v2", signing.verifying_key(), scope)
            .is_err()
    );
    assert!(v2::verify(&encode(&doc), &legacy).is_err());
    assert!(aster_license_core::verify(&encode(&doc), &trusted).is_err());
}

#[test]
fn protocol_versions_dates_and_source_modes_fail_closed() {
    let key = SigningKey::from_bytes(&[42; 32]);
    for (pointer, value) in [
        ("/quota_policy_version", json!(0)),
        ("/plan_version", json!(0)),
        ("/entitlements/catalog_version", json!(99)),
        ("/minimum_version", json!("1.02.3")),
        ("/minimum_version", json!("18446744073709551616.0.0")),
        ("/issued_at", json!("2026-09-01T00:00:60.000Z")),
        ("/issued_at", json!("2027-09-01T00:00:00.000Z")),
        (
            "/validity/expiry/expires_at",
            json!("2026-09-01T00:00:00.000Z"),
        ),
    ] {
        let mut claims = serde_json::to_value(document(0).claims).unwrap();
        *claims.pointer_mut(pointer).unwrap() = value;
        let claims: v2::Claims = serde_json::from_value(claims).unwrap();
        assert!(v2::sign(claims, &key).is_err(), "{pointer}");
    }
    let mut claims = document(2).claims;
    claims.validity.expiry = v2::Expiry::None {};
    assert!(v2::sign(claims, &key).is_err());
    let mut claims = document(0).claims;
    claims.entitlements.features.clear();
    let doc = v2::sign(claims, &key).unwrap();
    assert!(
        v2::verify(
            &encode(&doc),
            &keys(policy(
                doc.claims.entitlements.clone(),
                doc.claims.source.kind()
            ))
        )
        .is_ok()
    );
}

#[test]
fn free_key_cannot_sign_a_legacy_document_and_duplicate_legacy_insert_preserves_key() {
    let fixture: Value = serde_json::from_slice(include_bytes!(
        "../../../../contracts/test-vectors/license.v1.json"
    ))
    .unwrap();
    let original = serde_json::to_vec(&fixture["document"]).unwrap();
    let public_key = fixture["public_key_spki"].as_str().unwrap();
    let mut legacy = TrustedLicenseKeys::new();
    legacy
        .insert_spki_base64url("license-cross-language-v1", public_key)
        .unwrap();
    let other = SigningKey::from_bytes(&[42; 32]);
    assert!(
        legacy
            .insert("license-cross-language-v1", other.verifying_key())
            .is_err()
    );
    assert!(aster_license_core::verify(&original, &legacy).is_ok());
    let mut raw_claims = fixture["document"].clone();
    raw_claims.as_object_mut().unwrap().remove("signature");
    raw_claims["key_id"] = json!("test-only-v2");
    let claims: aster_license_core::LicenseClaims = serde_json::from_value(raw_claims).unwrap();
    let forged_scope = aster_license_core::sign(claims, &other).unwrap();
    let trusted = keys(policy(
        document(0).claims.entitlements,
        SourceKind::FreeDistribution,
    ));
    assert_eq!(
        aster_license_core::verify(&serde_json::to_vec(&forged_scope).unwrap(), &trusted)
            .unwrap_err(),
        LicenseError::UntrustedKey("test-only-v2".into())
    );
}

#[test]
fn base64_cannot_ignore_newlines_in_signatures_or_trusted_public_keys() {
    let original = document(0);
    let scope = policy(
        original.claims.entitlements.clone(),
        original.claims.source.kind(),
    );
    let trusted = keys(scope.clone());
    for separator in ["\n", "\r", "\r\n"] {
        let mut changed = original.clone();
        changed.signature.insert_str(12, separator);
        assert!(v2::verify(&encode(&changed), &trusted).is_err());
        let mut spki = fixture()["public_key_spki"].as_str().unwrap().to_owned();
        spki.insert_str(12, separator);
        assert!(
            TrustedLicenseKeys::new()
                .insert_scoped_spki_base64url("test-only-v2", &spki, scope.clone())
                .is_err()
        );
    }
}

#[test]
fn scoped_public_key_cannot_be_registered_again_under_a_different_identifier() {
    let signing = SigningKey::from_bytes(&[42; 32]);
    let scope = policy(
        document(0).claims.entitlements,
        SourceKind::FreeDistribution,
    );
    let mut trusted = keys(scope.clone());
    assert!(
        trusted
            .insert("legacy-alias", signing.verifying_key())
            .is_err()
    );
    assert!(
        trusted
            .insert_scoped("scoped-alias", signing.verifying_key(), scope.clone())
            .is_err()
    );
    let mut legacy_first = TrustedLicenseKeys::new();
    legacy_first
        .insert("legacy-alias", signing.verifying_key())
        .unwrap();
    assert!(
        legacy_first
            .insert_scoped("test-only-v2", signing.verifying_key(), scope)
            .is_err()
    );
}

#[test]
fn later_issuance_keeps_the_purchased_start_and_end_dates() {
    let mut claims = document(2).claims;
    claims.issued_at = "2026-10-01T00:00:00.000Z".into();
    let signed = v2::sign(claims.clone(), &SigningKey::from_bytes(&[42; 32])).unwrap();
    let verified = v2::verify(
        &encode(&signed),
        &keys(policy(claims.entitlements, claims.source.kind())),
    )
    .unwrap();
    assert_eq!(
        verified.claims().validity.not_before,
        "2026-09-01T00:00:00.000Z"
    );
    assert_eq!(
        verified.claims().validity.expiry,
        v2::Expiry::Fixed {
            expires_at: "2027-09-01T00:00:00.000Z".into()
        }
    );
}

#[test]
fn product_protocol_bridge_preserves_exact_v2_bytes_and_signed_quotas() {
    let doc = document(0);
    let trusted = keys(policy(
        doc.claims.entitlements.clone(),
        doc.claims.source.kind(),
    ));
    let mut source = serde_json::to_vec_pretty(&doc).unwrap();
    source.push(b'\n');

    let verified = verify_product_license(&source, &trusted).unwrap();
    assert!(matches!(verified, VerifiedProductLicense::V2(_)));
    assert_eq!(verified.source(), source);
    assert_eq!(verified.as_ref().protocol_schema(), v2::SCHEMA);
    assert_eq!(
        verified.as_ref().quota(QuotaId::MemberSeats),
        QuotaLimit::Limited { value: 3 }
    );
    assert_eq!(
        verified.as_ref().quota(QuotaId::ApiKeysPerMember),
        QuotaLimit::Limited { value: 2 }
    );
}

#[test]
fn product_protocol_bridge_does_not_let_the_shape_probe_weaken_strict_decoding() {
    let doc = document(0);
    let trusted = keys(policy(
        doc.claims.entitlements.clone(),
        doc.claims.source.kind(),
    ));
    let raw = String::from_utf8(encode(&doc)).unwrap();
    let cases = [
        raw.replacen(
            "{\"claims\":",
            "{\"schema\":\"aster.license.v1\",\"claims\":",
            1,
        ),
        raw.replacen(
            "\"schema\":\"aster.license.v2\"",
            "\"schema\":\"aster.license.v2\",\"schema\":\"aster.license.v2\"",
            1,
        ),
        format!("{raw} {{}}"),
    ];
    for changed in cases {
        assert!(verify_product_license(changed.as_bytes(), &trusted).is_err());
    }
}

#[test]
fn standard_subscription_is_signed_commercial_only_and_resolves_for_runtime() {
    use aster_license_core::{VerifiedLicenseRef, catalog::FeatureSetId};
    let mut paid = fixture()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| serde_json::from_value::<v2::Document>(case["document"].clone()).unwrap())
        .find(|doc| doc.claims.source.kind() == SourceKind::CommercialOrder)
        .unwrap();
    paid.claims.entitlements.features.clear();
    paid.claims.entitlements.feature_sets = vec![FeatureSetId::Standard];
    let trusted = keys(policy(
        paid.claims.entitlements.clone(),
        SourceKind::CommercialOrder,
    ));
    let signed = v2::sign(paid.claims, &SigningKey::from_bytes(&[42; 32])).unwrap();
    let verified = v2::verify(&encode(&signed), &trusted).unwrap();
    let view = VerifiedLicenseRef::from(&verified);
    assert!(view.has_feature("gateway"));
    assert!(view.features().contains(&"member"));
    assert!(!view.has_feature("unknown_extension"));
    let mut tampered = signed.clone();
    tampered.claims.entitlements.feature_sets.clear();
    assert!(v2::verify(&encode(&tampered), &trusted).is_err());
    let free_policy = policy(
        signed.claims.entitlements.clone(),
        SourceKind::FreeDistribution,
    );
    assert!(free_policy.validate().is_err());
    let mut free = document(0).claims;
    free.entitlements.feature_sets = vec![FeatureSetId::Standard];
    assert!(v2::sign(free, &SigningKey::from_bytes(&[42; 32])).is_err());
}
