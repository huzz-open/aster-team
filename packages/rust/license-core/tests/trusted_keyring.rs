use aster_license_core::{LicenseDocument, TrustedLicenseKeys, sign, v2, verify};
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../../../contracts/test-vectors/license-trust.v1.json"
    ))
    .unwrap()
}
fn v2_fixture() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../../../contracts/test-vectors/license.v2.json"
    ))
    .unwrap()
}
fn parse(value: &Value) -> Result<TrustedLicenseKeys, v2::Error> {
    TrustedLicenseKeys::from_json(&serde_json::to_vec(value).unwrap())
}

#[test]
fn customer_keyring_rejects_legacy_and_enforces_the_operations_free_scope() {
    let trusted = parse(&fixture()["keyring"]).unwrap();
    let legacy: Value = serde_json::from_slice(include_bytes!(
        "../../../../contracts/test-vectors/license.v1.json"
    ))
    .unwrap();
    let legacy_raw = serde_json::to_vec(&legacy["document"]).unwrap();
    assert!(verify(&legacy_raw, &trusted).is_err());
    assert!(parse(&json!([fixture()["legacy_entry"]])).is_err());
    for (index, case) in v2_fixture()["cases"].as_array().unwrap().iter().enumerate() {
        let result = v2::verify(&serde_json::to_vec(&case["document"]).unwrap(), &trusted);
        assert_eq!(result.is_ok(), index < 2, "{}", case["name"]);
    }
    // Correctly signed, over-ceiling claims must be rejected by the loaded scope.
    let mut doc: v2::Document =
        serde_json::from_value(v2_fixture()["cases"][0]["document"].clone()).unwrap();
    doc.claims.entitlements.quotas[0].limit =
        aster_license_core::entitlements::QuotaLimit::Limited { value: 4 };
    let signed = v2::sign(doc.claims, &SigningKey::from_bytes(&[42; 32])).unwrap();
    assert!(v2::verify(&serde_json::to_vec(&signed).unwrap(), &trusted).is_err());
    // Removing policy cannot turn a v2 document into valid legacy authorization.
    let mut without_scope = fixture()["keyring"].clone();
    without_scope[1].as_object_mut().unwrap().remove("policy");
    assert!(parse(&without_scope).is_err());
    // A free signer cannot issue v1 through the actual Customer keyring.
    let mut legacy_keys = TrustedLicenseKeys::new();
    legacy_keys
        .insert_spki_base64url(
            fixture()["legacy_entry"]["key_id"].as_str().unwrap(),
            fixture()["legacy_entry"]["public_key_spki"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    let mut legacy_doc: LicenseDocument =
        verify(&legacy_raw, &legacy_keys).unwrap().into_document();
    legacy_doc.claims.key_id = "test-only-v2".to_owned();
    let forged_legacy = sign(legacy_doc.claims, &SigningKey::from_bytes(&[42; 32])).unwrap();
    assert!(verify(&serde_json::to_vec(&forged_legacy).unwrap(), &trusted).is_err());
}

#[test]
fn malformed_policy_never_selects_legacy_registration() {
    let original = fixture()["keyring"].clone();
    for value in [Value::Null, json!({}), json!(false), json!("legacy")] {
        let mut changed = original.clone();
        changed[1]["policy"] = value;
        assert!(parse(&changed).is_err());
    }
    for (pointer, value) in [
        ("/1/policy/sources", json!([])),
        (
            "/1/policy/sources",
            json!(["free_distribution", "commercial_order"]),
        ),
        (
            "/1/policy/sources",
            json!(["approved_trial", "free_distribution"]),
        ),
        (
            "/1/policy/sources",
            json!(["free_distribution", "free_distribution"]),
        ),
        ("/1/policy/sources", json!([{ "free_distribution": null }])),
        ("/1/policy/bindings", json!([{"unbound": null}])),
        ("/1/policy/expiries", json!([{"none": null}])),
        ("/1/policy/expiries", json!(["unknown"])),
        ("/1/policy/entitlement_ceiling/quotas", json!([])),
        ("/1/policy/entitlement_ceiling/features", json!(["unknown"])),
        ("/1/policy/entitlement_ceiling/catalog_version", json!(2)),
        (
            "/1/policy/entitlement_ceiling/quotas/0/limit",
            json!({"mode":"unlimited","value":3}),
        ),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        assert!(parse(&changed).is_err(), "{pointer}");
    }
    for value in [
        json!([]),
        json!({}),
        Value::Null,
        json!(vec![original[0].clone(); 9]),
    ] {
        assert!(parse(&value).is_err());
    }
}

#[test]
fn keyring_rejects_aliases_duplicate_json_and_noncanonical_keys() {
    let original = fixture()["keyring"].clone();
    for order in [vec![0, 1], vec![1, 0]] {
        let mut changed = original.clone();
        changed[0]["public_key_spki"] = changed[1]["public_key_spki"].clone();
        assert!(parse(&json!([changed[order[0]], changed[order[1]]])).is_err());
    }
    let mut duplicated = original.clone();
    duplicated[1]["key_id"] = duplicated[0]["key_id"].clone();
    assert!(parse(&duplicated).is_err());
    for value in [
        "",
        "unknown-key",
        "MCowBQYDK2VwAyEAGX9rI-FshTLGq8g4-s1ep4m-DHaykgM0A5v6iz02jWE=",
        "\n",
    ] {
        let mut changed = original.clone();
        changed[1]["public_key_spki"] = json!(value);
        assert!(parse(&changed).is_err());
    }
    let raw = serde_json::to_string(&original).unwrap();
    for (from, to) in [
        ("\"policy\":{", "\"policy\":null,\"policy\":{"),
        ("\"sources\":[", "\"sources\":[],\"sources\":["),
        ("\"key_id\":", "\"key_id\":\"alias\",\"key_id\":"),
        ("\"bindings\":", "\"b\\u0069ndings\":[],\"bindings\":"),
        ("\"policy\":{", "\"Policy\":{"),
        (
            "\"catalog_version\":1",
            "\"catalog_version\":1,\"extra\":true",
        ),
    ] {
        assert!(raw.contains(from));
        assert!(
            TrustedLicenseKeys::from_json(raw.replacen(from, to, 1).as_bytes()).is_err(),
            "{to}"
        );
    }
    assert!(TrustedLicenseKeys::from_json(format!("{raw} {{}}").as_bytes()).is_err());
    assert!(TrustedLicenseKeys::from_json(&vec![b' '; (1 << 20) + 1]).is_err());
}
