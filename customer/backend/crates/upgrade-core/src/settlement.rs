//! Online peers must use the same durable result and request ownership formats.
use serde::Deserialize;

pub const INTENT_SCHEMA: &str = "aster.gateway-settlement.v1";
pub const REQUEST_LOCK_SCHEMA: &str = "aster.gateway-request-lock.v1";
pub const CAPABILITY_FILE: &str = "systemd/gateway-settlement.json";
pub const CAPABILITY_JSON: &str =
    include_str!("../../../../deploy/systemd/gateway-settlement.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capability {
    schema: String,
    intent_schema: String,
    request_lock_schema: String,
}

/// Missing, unknown, newer and partially compatible documents fail closed.
/// The caller must independently verify the containing Release and file tree.
#[must_use]
pub fn compatible(bytes: &[u8]) -> bool {
    if bytes.len() > 4096 {
        return false;
    }
    serde_json::from_slice::<Capability>(bytes).is_ok_and(|value| {
        value.schema == "aster.gateway-settlement-capability.v1"
            && value.intent_schema == INTENT_SCHEMA
            && value.request_lock_schema == REQUEST_LOCK_SCHEMA
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shipped_contract_matches_the_runtime_formats() {
        assert!(compatible(CAPABILITY_JSON.as_bytes()));
    }
    #[test]
    fn absent_partial_unknown_and_duplicate_protocols_are_rejected() {
        for bytes in [b"".as_slice(), b"null", b"{}", b"[]"] {
            assert!(!compatible(bytes));
        }
        for field in ["schema", "intent_schema", "request_lock_schema"] {
            let mut value: serde_json::Value = serde_json::from_str(CAPABILITY_JSON).unwrap();
            value.as_object_mut().unwrap().remove(field);
            assert!(!compatible(&serde_json::to_vec(&value).unwrap()));
            value[field] = "future-or-unrelated".into();
            assert!(!compatible(&serde_json::to_vec(&value).unwrap()));
        }
        assert!(!compatible(
            CAPABILITY_JSON
                .replace("{", "{\"automatic\":true,")
                .as_bytes()
        ));
        assert!(!compatible(
            CAPABILITY_JSON
                .replace(
                    "{",
                    "{\"schema\":\"aster.gateway-settlement-capability.v1\","
                )
                .as_bytes()
        ));
        assert!(!compatible(format!("{CAPABILITY_JSON} {{}}").as_bytes()));
    }
}
