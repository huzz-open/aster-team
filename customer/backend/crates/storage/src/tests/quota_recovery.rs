use super::*;

#[test]
fn active_quota_discovery_pages_distinct_identities_without_trusting_row_expiry() {
    let directory = tempdir().unwrap();
    let store = initialize(&directory.path().join("quota.db"));
    for (id, identity, status) in [
        ("a1", "identity_a", "active"),
        ("a2", "identity_a", "active"),
        ("b", "identity_b", "released"),
        ("c", "identity_c", "active"),
        ("d", "identity_d", "active"),
    ] {
        store.connection.execute(
            "INSERT OR IGNORE INTO identities(id,email,display_name,password_hash,role,status,can_consume_model,password_change_required,revision,integrity_hmac,created_at,updated_at)
             VALUES(?,?,'Recovery','hash','member','active',1,0,0,'untrusted',?,?)",
            params![identity, format!("{identity}@example.test"), NOW, NOW],
        ).unwrap();
        store.connection.execute(
            "INSERT OR IGNORE INTO user_balances(identity_id,balance_tokens,reserved_tokens,granted_tokens,integrity_hmac,updated_at)
             VALUES(?,10,?,10,'untrusted',?)",
            params![identity, i64::from(status == "active"), NOW],
        ).unwrap();
        store.connection.execute(
            "INSERT INTO api_keys(id,identity_id,name,key_hash,key_prefix,status,revision,integrity_hmac,created_at)
             VALUES(?,?,'Recovery',?,'ask_test','active',0,'untrusted',?)",
            params![id, identity, format!("hash_{id}"), NOW],
        ).unwrap();
        store.connection.execute(
            "INSERT INTO quota_reservations(id,identity_id,api_key_id,request_id,reserved_tokens,status,revision,integrity_hmac,created_at,expires_at)
             VALUES(?,?,?,?,1,?,0,'untrusted',?,'untrusted-expiry')",
            params![id, identity, id, id, status, NOW],
        ).unwrap();
    }
    assert_eq!(
        store.active_quota_identity_page("", 2).unwrap(),
        ["identity_a", "identity_c"]
    );
    assert_eq!(
        store.active_quota_identity_page("identity_c", 2).unwrap(),
        ["identity_d"]
    );
    assert!(
        store
            .active_quota_identity_page("identity_d", 2)
            .unwrap()
            .is_empty()
    );
    store
        .connection
        .execute(
            "UPDATE user_balances SET reserved_tokens=0 WHERE identity_id='identity_a'",
            [],
        )
        .unwrap();
    // Zero-reserved balances are no longer recovery candidates even when old
    // reservation rows remain; discovery must not scan that history instead.
    assert_eq!(
        store.active_quota_identity_page("", 2).unwrap(),
        ["identity_c", "identity_d"]
    );
    assert_eq!(
        store.active_quota_identity_page("identity_a", 2).unwrap(),
        ["identity_c", "identity_d"]
    );
}
