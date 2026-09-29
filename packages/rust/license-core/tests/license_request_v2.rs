use aster_license_core::{canonicalize, parse_request, request_v2};
use serde_json::Value;

#[test]
fn shared_request_compatibility_and_strict_parsing() {
    let fixture: Value = serde_json::from_slice(include_bytes!(
        "../../../../contracts/test-vectors/license-request.v2.json"
    ))
    .unwrap();
    let raw = serde_json::to_vec(&fixture["request"]).unwrap();
    let request = request_v2::parse_request(&raw).unwrap();
    assert!(parse_request(&raw).is_err());
    let canonical = canonicalize(&serde_json::to_value(&request).unwrap()).unwrap();
    assert_eq!(
        canonical,
        fixture["canonical_request"].as_str().unwrap().as_bytes()
    );
    for item in fixture["invalid_raw"].as_array().unwrap() {
        assert!(
            request_v2::parse_request(item["raw"].as_str().unwrap().as_bytes()).is_err(),
            "{}",
            item["name"]
        );
    }
    for item in fixture["versions"].as_array().unwrap() {
        assert_eq!(
            request_v2::version_at_least(
                item["actual"].as_str().unwrap(),
                item["minimum"].as_str().unwrap()
            ),
            item["expected"].as_bool().unwrap(),
            "{item}"
        );
    }
    assert!(request.supports("1.2.3+other", 1, 1));
    assert!(!request.supports("1.2.4", 1, 1));
    assert!(!request.supports("1.2.3", 2, 1));
    assert!(!request.supports("1.2.3", 1, 2));
    let mut legacy = fixture["request"].clone();
    legacy["schema"] = "aster.license-request.v1".into();
    for key in [
        "license_schema",
        "capability_catalog_version",
        "quota_policy_version",
    ] {
        legacy.as_object_mut().unwrap().remove(key);
    }
    let legacy = serde_json::to_vec(&legacy).unwrap();
    assert!(parse_request(&legacy).is_ok());
    assert!(request_v2::parse_request(&legacy).is_err());
}
