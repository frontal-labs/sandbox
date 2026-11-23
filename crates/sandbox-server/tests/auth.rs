//! Unit tests moved out of src/auth.rs.

use sandbox_server::auth::{AuthManager, KeyId};

#[test]
fn valid_key_is_accepted() {
    let auth = AuthManager::new();
    let key = auth.create_key();
    assert!(auth.validate_api_key(&key).is_ok());
}

#[test]
fn wrong_secret_is_rejected() {
    let auth = AuthManager::new();
    let key = auth.create_key();
    let (prefix, _) = key.rsplit_once('_').unwrap();
    let forged = format!("{prefix}_{}", "0".repeat(64));
    assert!(auth.validate_api_key(&forged).is_err());
}

#[test]
fn unknown_or_malformed_keys_are_rejected() {
    let auth = AuthManager::new();
    let _ = auth.create_key();
    assert!(auth.validate_api_key("").is_err());
    assert!(auth.validate_api_key("sbx_nounderscore").is_err());
    assert!(auth.validate_api_key("sbx_deadbeef_secret").is_err());
    assert!(auth.validate_api_key("Bearer sbx_x_y").is_err());
}

#[test]
fn keys_have_full_length_and_do_not_repeat() {
    let auth = AuthManager::new();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..500 {
        let key = auth.create_key();
        let body = key.strip_prefix("sbx_").unwrap();
        let (id, secret) = body.split_once('_').unwrap();
        assert_eq!(id.len(), 16, "8-byte id as hex");
        assert_eq!(secret.len(), 64, "32-byte secret as hex");
        assert!(seen.insert(key), "a key was generated twice");
    }
}

#[test]
fn revoked_key_is_rejected() {
    let auth = AuthManager::new();
    let key = auth.create_key();
    let (id, _) = auth.validate_api_key(&key).unwrap();
    assert!(auth.revoke(&id));
    assert!(auth.validate_api_key(&key).is_err());
    assert!(!auth.revoke(&id));
}

#[test]
fn digests_roundtrip_through_export_and_import() {
    let auth = AuthManager::new();
    let key = auth.create_key();
    let records = auth.export_digests();
    assert_eq!(records.len(), 1);

    let other = AuthManager::new();
    other.import_digests(&records).unwrap();
    let id = KeyId::from_stored(records[0].0.clone());
    assert!(other
        .validate_api_key(&key)
        .is_ok_and(|validated| validated.0 == id));
}
