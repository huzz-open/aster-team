#![forbid(unsafe_code)]

use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher as _, PasswordVerifier as _, Version,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroizing;

const PASSWORD_MEMORY_KIB: u32 = 65_536;
const PASSWORD_ITERATIONS: u32 = 3;
const PASSWORD_PARALLELISM: u32 = 1;
const PASSWORD_OUTPUT_BYTES: usize = 32;
const TOKEN_RANDOM_BYTES: usize = 32;

#[derive(Clone)]
pub struct AuthCore {
    installation_id: String,
    password_pepper: Zeroizing<[u8; 32]>,
    session_token_key: Zeroizing<[u8; 32]>,
    api_key_token_key: Zeroizing<[u8; 32]>,
    identity_integrity_key: Zeroizing<[u8; 32]>,
    session_integrity_key: Zeroizing<[u8; 32]>,
    api_key_integrity_key: Zeroizing<[u8; 32]>,
    security_state_integrity_key: Zeroizing<[u8; 32]>,
    audit_event_integrity_key: Zeroizing<[u8; 32]>,
}

pub struct IssuedToken {
    plaintext: Zeroizing<String>,
    digest: String,
    prefix: String,
}

impl IssuedToken {
    pub fn plaintext(&self) -> &str {
        self.plaintext.as_str()
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub fn into_plaintext(self) -> Zeroizing<String> {
        self.plaintext
    }
}

#[derive(Clone, Copy)]
pub struct IdentityIntegrityInput<'a> {
    pub id: &'a str,
    pub email: &'a str,
    pub display_name: &'a str,
    pub password_hash: &'a str,
    pub role: &'a str,
    pub status: &'a str,
    pub can_consume_model: bool,
    pub password_change_required: bool,
    pub revision: u32,
}

#[derive(Clone, Copy)]
pub struct SessionIntegrityInput<'a> {
    pub id: &'a str,
    pub identity_id: &'a str,
    pub token_digest: &'a str,
    pub expires_at: &'a str,
    pub created_at: &'a str,
}

#[derive(Clone, Copy)]
pub struct ApiKeyIntegrityInput<'a> {
    pub id: &'a str,
    pub identity_id: &'a str,
    pub name: &'a str,
    pub key_digest: &'a str,
    pub key_prefix: &'a str,
    pub status: &'a str,
    pub revision: u32,
    pub created_at: &'a str,
}

#[derive(Clone, Copy)]
pub struct SecurityStateIntegrityInput<'a> {
    pub key: &'a str,
    pub value: &'a [u8],
    pub revision: u64,
}

#[derive(Clone, Copy)]
pub struct AuditEventIntegrityInput<'a> {
    pub id: &'a str,
    pub sequence: u64,
    pub actor_identity_id: Option<&'a str>,
    pub actor_role: &'a str,
    pub action: &'a str,
    pub target_type: &'a str,
    pub target_id: Option<&'a str>,
    pub outcome: &'a str,
    pub previous_event_hmac: &'a str,
    pub created_at: &'a str,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AuthError {
    #[error("installation key must contain exactly 32 bytes")]
    InvalidInstallationKey,
    #[error("authentication context is invalid")]
    InvalidContext,
    #[error("password must contain between 12 and 1024 bytes")]
    InvalidPassword,
    #[error("password hash is invalid")]
    InvalidPasswordHash,
    #[error("operating-system randomness is unavailable")]
    Randomness,
    #[error("authentication key derivation failed")]
    KeyDerivation,
}

impl AuthCore {
    #[must_use]
    pub fn installation_id(&self) -> &str {
        &self.installation_id
    }

    pub fn new(installation_key: &[u8], installation_id: &str) -> Result<Self, AuthError> {
        if installation_key.len() != 32 {
            return Err(AuthError::InvalidInstallationKey);
        }
        if !valid_identifier(installation_id) {
            return Err(AuthError::InvalidContext);
        }
        let hkdf = Hkdf::<Sha256>::new(Some(b"aster-team-auth-core-v1"), installation_key);
        Ok(Self {
            installation_id: installation_id.to_owned(),
            password_pepper: derive_key(&hkdf, installation_id, "password-pepper")?,
            session_token_key: derive_key(&hkdf, installation_id, "session-token-hmac")?,
            api_key_token_key: derive_key(&hkdf, installation_id, "api-key-token-hmac")?,
            identity_integrity_key: derive_key(&hkdf, installation_id, "identity-integrity-hmac")?,
            session_integrity_key: derive_key(&hkdf, installation_id, "session-integrity-hmac")?,
            api_key_integrity_key: derive_key(&hkdf, installation_id, "api-key-integrity-hmac")?,
            security_state_integrity_key: derive_key(
                &hkdf,
                installation_id,
                "security-state-integrity-hmac",
            )?,
            audit_event_integrity_key: derive_key(
                &hkdf,
                installation_id,
                "audit-event-integrity-hmac",
            )?,
        })
    }

    pub fn hash_password(&self, password: &[u8]) -> Result<String, AuthError> {
        validate_password(password)?;
        password_hasher(&self.password_pepper[..])?
            .hash_password(password)
            .map(|value| value.to_string())
            .map_err(|_| AuthError::InvalidPasswordHash)
    }

    pub fn verify_password(&self, encoded: &str, password: &[u8]) -> Result<bool, AuthError> {
        validate_password(password)?;
        if !encoded.starts_with("$argon2id$") || encoded.len() > 512 {
            return Err(AuthError::InvalidPasswordHash);
        }
        let parsed = PasswordHash::new(encoded).map_err(|_| AuthError::InvalidPasswordHash)?;
        Ok(password_hasher(&self.password_pepper[..])?
            .verify_password(password, &parsed)
            .is_ok())
    }

    pub fn issue_session_token(&self) -> Result<IssuedToken, AuthError> {
        issue_token("asts_", &self.session_token_key[..])
    }

    pub fn issue_api_key(&self) -> Result<IssuedToken, AuthError> {
        issue_token("ask_", &self.api_key_token_key[..])
    }

    pub fn session_token_digest(&self, token: &str) -> Result<String, AuthError> {
        token_digest("asts_", token, &self.session_token_key[..])
    }

    pub fn api_key_digest(&self, token: &str) -> Result<String, AuthError> {
        token_digest("ask_", token, &self.api_key_token_key[..])
    }

    pub fn identity_integrity_hmac(
        &self,
        input: IdentityIntegrityInput<'_>,
    ) -> Result<String, AuthError> {
        Ok(URL_SAFE_NO_PAD.encode(self.identity_integrity_mac(input)?.finalize().into_bytes()))
    }

    pub fn verify_identity_integrity_hmac(
        &self,
        input: IdentityIntegrityInput<'_>,
        expected: &str,
    ) -> Result<bool, AuthError> {
        let expected = decode_tag(expected)?;
        Ok(self
            .identity_integrity_mac(input)?
            .verify_slice(&expected)
            .is_ok())
    }

    pub fn session_integrity_hmac(
        &self,
        input: SessionIntegrityInput<'_>,
    ) -> Result<String, AuthError> {
        Ok(URL_SAFE_NO_PAD.encode(self.session_integrity_mac(input)?.finalize().into_bytes()))
    }

    pub fn verify_session_integrity_hmac(
        &self,
        input: SessionIntegrityInput<'_>,
        expected: &str,
    ) -> Result<bool, AuthError> {
        let expected = decode_tag(expected)?;
        Ok(self
            .session_integrity_mac(input)?
            .verify_slice(&expected)
            .is_ok())
    }

    pub fn api_key_integrity_hmac(
        &self,
        input: ApiKeyIntegrityInput<'_>,
    ) -> Result<String, AuthError> {
        Ok(URL_SAFE_NO_PAD.encode(self.api_key_integrity_mac(input)?.finalize().into_bytes()))
    }

    pub fn verify_api_key_integrity_hmac(
        &self,
        input: ApiKeyIntegrityInput<'_>,
        expected: &str,
    ) -> Result<bool, AuthError> {
        let expected = decode_tag(expected)?;
        Ok(self
            .api_key_integrity_mac(input)?
            .verify_slice(&expected)
            .is_ok())
    }

    pub fn security_state_integrity_hmac(
        &self,
        input: SecurityStateIntegrityInput<'_>,
    ) -> Result<String, AuthError> {
        Ok(URL_SAFE_NO_PAD.encode(
            self.security_state_integrity_mac(input)?
                .finalize()
                .into_bytes(),
        ))
    }

    pub fn verify_security_state_integrity_hmac(
        &self,
        input: SecurityStateIntegrityInput<'_>,
        expected: &str,
    ) -> Result<bool, AuthError> {
        let expected = decode_tag(expected)?;
        Ok(self
            .security_state_integrity_mac(input)?
            .verify_slice(&expected)
            .is_ok())
    }

    pub fn audit_event_integrity_hmac(
        &self,
        input: AuditEventIntegrityInput<'_>,
    ) -> Result<String, AuthError> {
        Ok(URL_SAFE_NO_PAD.encode(
            self.audit_event_integrity_mac(input)?
                .finalize()
                .into_bytes(),
        ))
    }

    pub fn verify_audit_event_integrity_hmac(
        &self,
        input: AuditEventIntegrityInput<'_>,
        expected: &str,
    ) -> Result<bool, AuthError> {
        let expected = decode_tag(expected)?;
        Ok(self
            .audit_event_integrity_mac(input)?
            .verify_slice(&expected)
            .is_ok())
    }

    fn identity_integrity_mac(
        &self,
        input: IdentityIntegrityInput<'_>,
    ) -> Result<Hmac<Sha256>, AuthError> {
        validate_identity_input(input)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.identity_integrity_key.as_ref())
            .map_err(|_| AuthError::KeyDerivation)?;
        mac.update(b"aster.identity-integrity.v1");
        for field in [
            input.id.as_bytes(),
            input.email.as_bytes(),
            input.display_name.as_bytes(),
            input.password_hash.as_bytes(),
            input.role.as_bytes(),
            input.status.as_bytes(),
        ] {
            update_length_prefixed(&mut mac, field);
        }
        mac.update(&[u8::from(input.can_consume_model)]);
        mac.update(&[u8::from(input.password_change_required)]);
        mac.update(&input.revision.to_be_bytes());
        Ok(mac)
    }

    fn session_integrity_mac(
        &self,
        input: SessionIntegrityInput<'_>,
    ) -> Result<Hmac<Sha256>, AuthError> {
        if !valid_identifier(input.id)
            || !valid_identifier(input.identity_id)
            || input.token_digest.len() != 43
            || input.expires_at.len() != 24
            || input.created_at.len() != 24
        {
            return Err(AuthError::InvalidContext);
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(self.session_integrity_key.as_ref())
            .map_err(|_| AuthError::KeyDerivation)?;
        mac.update(b"aster.session-integrity.v1");
        for field in [
            input.id.as_bytes(),
            input.identity_id.as_bytes(),
            input.token_digest.as_bytes(),
            input.expires_at.as_bytes(),
            input.created_at.as_bytes(),
        ] {
            update_length_prefixed(&mut mac, field);
        }
        Ok(mac)
    }

    fn api_key_integrity_mac(
        &self,
        input: ApiKeyIntegrityInput<'_>,
    ) -> Result<Hmac<Sha256>, AuthError> {
        if !valid_identifier(input.id)
            || !valid_identifier(input.identity_id)
            || !(1..=160).contains(&input.name.len())
            || input.key_digest.len() != 43
            || !(4..=32).contains(&input.key_prefix.len())
            || !matches!(input.status, "active" | "suspended" | "revoked")
            || input.created_at.len() != 24
        {
            return Err(AuthError::InvalidContext);
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(self.api_key_integrity_key.as_ref())
            .map_err(|_| AuthError::KeyDerivation)?;
        mac.update(b"aster.api-key-integrity.v1");
        for field in [
            input.id.as_bytes(),
            input.identity_id.as_bytes(),
            input.name.as_bytes(),
            input.key_digest.as_bytes(),
            input.key_prefix.as_bytes(),
            input.status.as_bytes(),
            input.created_at.as_bytes(),
        ] {
            update_length_prefixed(&mut mac, field);
        }
        mac.update(&input.revision.to_be_bytes());
        Ok(mac)
    }

    fn security_state_integrity_mac(
        &self,
        input: SecurityStateIntegrityInput<'_>,
    ) -> Result<Hmac<Sha256>, AuthError> {
        if !valid_identifier(input.key) || input.value.len() > 1_048_576 {
            return Err(AuthError::InvalidContext);
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(self.security_state_integrity_key.as_ref())
            .map_err(|_| AuthError::KeyDerivation)?;
        mac.update(b"aster.security-state-integrity.v1");
        update_length_prefixed(&mut mac, input.key.as_bytes());
        update_length_prefixed(&mut mac, input.value);
        mac.update(&input.revision.to_be_bytes());
        Ok(mac)
    }

    fn audit_event_integrity_mac(
        &self,
        input: AuditEventIntegrityInput<'_>,
    ) -> Result<Hmac<Sha256>, AuthError> {
        if !valid_identifier(input.id)
            || input.sequence == 0
            || input
                .actor_identity_id
                .is_some_and(|value| !valid_identifier(value))
            || !matches!(
                input.actor_role,
                "owner" | "admin" | "member" | "system" | "runner"
            )
            || !valid_identifier(input.action)
            || !valid_identifier(input.target_type)
            || input
                .target_id
                .is_some_and(|value| !valid_identifier(value))
            || !matches!(input.outcome, "succeeded" | "failed")
            || (!input.previous_event_hmac.is_empty() && input.previous_event_hmac.len() != 43)
            || input.created_at.len() != 24
        {
            return Err(AuthError::InvalidContext);
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(self.audit_event_integrity_key.as_ref())
            .map_err(|_| AuthError::KeyDerivation)?;
        mac.update(b"aster.audit-event-integrity.v1");
        mac.update(&input.sequence.to_be_bytes());
        update_length_prefixed(&mut mac, input.id.as_bytes());
        update_optional_field(&mut mac, input.actor_identity_id);
        update_length_prefixed(&mut mac, input.actor_role.as_bytes());
        update_length_prefixed(&mut mac, input.action.as_bytes());
        update_length_prefixed(&mut mac, input.target_type.as_bytes());
        update_optional_field(&mut mac, input.target_id);
        update_length_prefixed(&mut mac, input.outcome.as_bytes());
        update_length_prefixed(&mut mac, input.previous_event_hmac.as_bytes());
        update_length_prefixed(&mut mac, input.created_at.as_bytes());
        Ok(mac)
    }
}

fn validate_identity_input(input: IdentityIntegrityInput<'_>) -> Result<(), AuthError> {
    if !valid_identifier(input.id)
        || !(3..=320).contains(&input.email.len())
        || !(1..=256).contains(&input.display_name.len())
        || input.password_hash.is_empty()
        || !matches!(input.role, "owner" | "admin" | "member")
        || !matches!(input.status, "active" | "disabled" | "deleted")
    {
        return Err(AuthError::InvalidContext);
    }
    Ok(())
}

fn decode_tag(value: &str) -> Result<Vec<u8>, AuthError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| AuthError::InvalidContext)?;
    if decoded.len() != 32 {
        return Err(AuthError::InvalidContext);
    }
    Ok(decoded)
}

fn password_hasher(pepper: &[u8]) -> Result<Argon2<'_>, AuthError> {
    let params = Params::new(
        PASSWORD_MEMORY_KIB,
        PASSWORD_ITERATIONS,
        PASSWORD_PARALLELISM,
        Some(PASSWORD_OUTPUT_BYTES),
    )
    .map_err(|_| AuthError::InvalidPasswordHash)?;
    Argon2::new_with_secret(pepper, Algorithm::Argon2id, Version::V0x13, params)
        .map_err(|_| AuthError::InvalidPasswordHash)
}

fn derive_key(
    hkdf: &Hkdf<Sha256>,
    installation_id: &str,
    purpose: &str,
) -> Result<Zeroizing<[u8; 32]>, AuthError> {
    let mut output = Zeroizing::new([0_u8; 32]);
    let info = format!("{purpose}\n{installation_id}");
    hkdf.expand(info.as_bytes(), output.as_mut())
        .map_err(|_| AuthError::KeyDerivation)?;
    Ok(output)
}

fn validate_password(password: &[u8]) -> Result<(), AuthError> {
    if !(12..=1024).contains(&password.len()) {
        return Err(AuthError::InvalidPassword);
    }
    Ok(())
}

fn issue_token(prefix: &str, key: &[u8]) -> Result<IssuedToken, AuthError> {
    let mut random = Zeroizing::new([0_u8; TOKEN_RANDOM_BYTES]);
    getrandom::fill(random.as_mut()).map_err(|_| AuthError::Randomness)?;
    let plaintext = Zeroizing::new(format!("{prefix}{}", URL_SAFE_NO_PAD.encode(&random[..])));
    let digest = token_digest(prefix, &plaintext, key)?;
    let visible_prefix = plaintext.chars().take(12).collect();
    Ok(IssuedToken {
        plaintext,
        digest,
        prefix: visible_prefix,
    })
}

fn token_digest(prefix: &str, token: &str, key: &[u8]) -> Result<String, AuthError> {
    if !token.starts_with(prefix) || token.len() != prefix.len() + 43 {
        return Err(AuthError::InvalidContext);
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| AuthError::KeyDerivation)?;
    mac.update(prefix.as_bytes());
    mac.update(&[0]);
    mac.update(token.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn update_length_prefixed(mac: &mut Hmac<Sha256>, value: &[u8]) {
    mac.update(&(value.len() as u64).to_be_bytes());
    mac.update(value);
}

fn update_optional_field(mac: &mut Hmac<Sha256>, value: Option<&str>) {
    match value {
        Some(value) => {
            mac.update(&[1]);
            update_length_prefixed(mac, value.as_bytes());
        }
        None => mac.update(&[0]),
    }
}

fn valid_identifier(value: &str) -> bool {
    (3..=128).contains(&value.len())
        && value.bytes().all(|value| {
            value.is_ascii_alphanumeric() || matches!(value, b'_' | b'-' | b'.' | b':')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> AuthCore {
        AuthCore::new(&[41_u8; 32], "installation-test-001").expect("auth core")
    }

    fn identity<'a>(password_hash: &'a str, status: &'a str) -> IdentityIntegrityInput<'a> {
        IdentityIntegrityInput {
            id: "identity-test-001",
            email: "member@example.com",
            display_name: "Member",
            password_hash,
            role: "member",
            status,
            can_consume_model: true,
            password_change_required: false,
            revision: 0,
        }
    }

    #[test]
    fn argon2id_password_hashes_are_randomized_and_installation_peppered() {
        let auth = auth();
        let first = auth
            .hash_password(b"correct horse battery staple")
            .expect("first hash");
        let second = auth
            .hash_password(b"correct horse battery staple")
            .expect("second hash");
        assert_ne!(first, second);
        assert!(
            auth.verify_password(&first, b"correct horse battery staple")
                .expect("verify")
        );
        assert!(
            !auth
                .verify_password(&first, b"wrong-password-value")
                .expect("reject wrong password")
        );
        let other = AuthCore::new(&[42_u8; 32], "installation-test-001").expect("other auth");
        assert!(
            !other
                .verify_password(&first, b"correct horse battery staple")
                .expect("reject other installation")
        );
    }

    #[test]
    fn session_and_api_tokens_use_separate_keyed_digests() {
        let auth = auth();
        let session = auth.issue_session_token().expect("session token");
        let api_key = auth.issue_api_key().expect("api key");
        assert_eq!(
            auth.session_token_digest(session.plaintext())
                .expect("session digest"),
            session.digest()
        );
        assert_eq!(
            auth.api_key_digest(api_key.plaintext())
                .expect("api key digest"),
            api_key.digest()
        );
        assert!(auth.api_key_digest(session.plaintext()).is_err());
        assert_eq!(session.prefix().len(), 12);
    }

    #[test]
    fn identity_integrity_covers_security_relevant_fields() {
        let auth = auth();
        let original = auth
            .identity_integrity_hmac(identity("$argon2id$example", "active"))
            .expect("original mac");
        let changed = auth
            .identity_integrity_hmac(identity("$argon2id$example", "disabled"))
            .expect("changed mac");
        assert_ne!(original, changed);
        let other = AuthCore::new(&[41_u8; 32], "installation-test-002").expect("other auth");
        assert_ne!(
            original,
            other
                .identity_integrity_hmac(identity("$argon2id$example", "active"))
                .expect("other mac")
        );
    }

    #[test]
    fn session_integrity_rejects_database_field_edits() {
        let auth = auth();
        let token_digest = "A".repeat(43);
        let input = SessionIntegrityInput {
            id: "session-test-001",
            identity_id: "identity-test-001",
            token_digest: &token_digest,
            expires_at: "2026-09-04T00:00:00.000Z",
            created_at: "2026-08-28T00:00:00.000Z",
        };
        let tag = auth.session_integrity_hmac(input).expect("session tag");
        assert!(
            auth.verify_session_integrity_hmac(input, &tag)
                .expect("verify session tag")
        );
        let edited = SessionIntegrityInput {
            expires_at: "2027-09-04T00:00:00.000Z",
            ..input
        };
        assert!(
            !auth
                .verify_session_integrity_hmac(edited, &tag)
                .expect("reject edited session")
        );
    }

    #[test]
    fn api_key_integrity_covers_status_and_revision() {
        let auth = auth();
        let digest = "B".repeat(43);
        let input = ApiKeyIntegrityInput {
            id: "api-key-test-001",
            identity_id: "identity-test-001",
            name: "Automation",
            key_digest: &digest,
            key_prefix: "ask_example1",
            status: "active",
            revision: 0,
            created_at: "2026-08-28T00:00:00.000Z",
        };
        let tag = auth.api_key_integrity_hmac(input).expect("api key tag");
        assert!(
            auth.verify_api_key_integrity_hmac(input, &tag)
                .expect("verify api key tag")
        );
        assert!(
            !auth
                .verify_api_key_integrity_hmac(
                    ApiKeyIntegrityInput {
                        status: "revoked",
                        revision: 1,
                        ..input
                    },
                    &tag,
                )
                .expect("reject edited api key")
        );
    }

    #[test]
    fn security_state_integrity_is_installation_bound_and_revisioned() {
        let auth = auth();
        let input = SecurityStateIntegrityInput {
            key: "member-seat-registry-v1",
            value: b"identity-a\nidentity-b",
            revision: 7,
        };
        let tag = auth
            .security_state_integrity_hmac(input)
            .expect("security state tag");
        assert!(
            auth.verify_security_state_integrity_hmac(input, &tag)
                .expect("verify security state")
        );
        assert!(
            !auth
                .verify_security_state_integrity_hmac(
                    SecurityStateIntegrityInput {
                        revision: 8,
                        ..input
                    },
                    &tag,
                )
                .expect("reject rollback or edit")
        );
        let other = AuthCore::new(&[42_u8; 32], "installation-test-001").expect("other auth");
        assert!(
            !other
                .verify_security_state_integrity_hmac(input, &tag)
                .expect("reject other installation")
        );
    }

    #[test]
    fn audit_event_integrity_chains_actor_action_target_and_outcome() {
        let auth = auth();
        let previous = "C".repeat(43);
        let input = AuditEventIntegrityInput {
            id: "audit-test-001",
            sequence: 8,
            actor_identity_id: Some("identity-test-001"),
            actor_role: "admin",
            action: "member.disable",
            target_type: "identity",
            target_id: Some("identity-target-001"),
            outcome: "succeeded",
            previous_event_hmac: &previous,
            created_at: "2026-08-28T00:00:00.000Z",
        };
        let tag = auth
            .audit_event_integrity_hmac(input)
            .expect("audit event tag");
        assert!(
            auth.verify_audit_event_integrity_hmac(input, &tag)
                .expect("verify audit event")
        );
        assert!(
            !auth
                .verify_audit_event_integrity_hmac(
                    AuditEventIntegrityInput {
                        action: "member.delete",
                        ..input
                    },
                    &tag,
                )
                .expect("reject edited audit event")
        );
        assert!(
            !auth
                .verify_audit_event_integrity_hmac(
                    AuditEventIntegrityInput {
                        previous_event_hmac: "",
                        ..input
                    },
                    &tag,
                )
                .expect("reject detached audit event")
        );
    }
}
