#![forbid(unsafe_code)]

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead as _, KeyInit as _, Payload, array::Array},
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroizing;

const ENCRYPTION_SCHEMA: &str = "aster.credential-envelope.v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialContext<'a> {
    pub credential_id: &'a str,
    pub account_id: &'a str,
    pub revision: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptedCredentialMaterial {
    pub encrypted_payload: Vec<u8>,
    pub payload_nonce: Vec<u8>,
    pub wrapped_data_key: Vec<u8>,
    pub wrap_nonce: Vec<u8>,
}

#[derive(Clone)]
pub struct CredentialVault {
    wrapping_key: Zeroizing<[u8; 32]>,
    identity_key: Zeroizing<[u8; 32]>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CredentialVaultError {
    #[error("installation key must contain exactly 32 bytes")]
    InvalidInstallationKey,
    #[error("credential context is invalid")]
    InvalidContext,
    #[error("operating-system randomness is unavailable")]
    Randomness,
    #[error("credential key derivation failed")]
    KeyDerivation,
    #[error("credential encryption failed")]
    Encryption,
    #[error("credential envelope is invalid or belongs to another installation or instance")]
    InvalidEnvelope,
}

impl CredentialVault {
    pub fn new(
        installation_key: &[u8],
        installation_id: &str,
    ) -> Result<Self, CredentialVaultError> {
        if installation_key.len() != 32 {
            return Err(CredentialVaultError::InvalidInstallationKey);
        }
        if !valid_identifier(installation_id) {
            return Err(CredentialVaultError::InvalidContext);
        }
        let hkdf = Hkdf::<Sha256>::new(Some(b"aster-team-credential-vault-v1"), installation_key);
        let mut wrapping_key = Zeroizing::new([0_u8; 32]);
        let mut identity_key = Zeroizing::new([0_u8; 32]);
        let wrap_info = format!("wrapping-key\n{installation_id}");
        let identity_info = format!("identity-hmac\n{installation_id}");
        hkdf.expand(wrap_info.as_bytes(), wrapping_key.as_mut())
            .map_err(|_| CredentialVaultError::KeyDerivation)?;
        hkdf.expand(identity_info.as_bytes(), identity_key.as_mut())
            .map_err(|_| CredentialVaultError::KeyDerivation)?;
        Ok(Self {
            wrapping_key,
            identity_key,
        })
    }

    pub fn encrypt(
        &self,
        context: &CredentialContext<'_>,
        plaintext: &[u8],
    ) -> Result<EncryptedCredentialMaterial, CredentialVaultError> {
        validate_context(context)?;
        let mut data_key = Zeroizing::new([0_u8; 32]);
        let mut payload_nonce = [0_u8; 24];
        let mut wrap_nonce = [0_u8; 24];
        getrandom::fill(data_key.as_mut()).map_err(|_| CredentialVaultError::Randomness)?;
        getrandom::fill(&mut payload_nonce).map_err(|_| CredentialVaultError::Randomness)?;
        getrandom::fill(&mut wrap_nonce).map_err(|_| CredentialVaultError::Randomness)?;

        let data_cipher = XChaCha20Poly1305::new(&Array(*data_key));
        let encrypted_payload = data_cipher
            .encrypt(
                &XNonce::from(payload_nonce),
                Payload {
                    msg: plaintext,
                    aad: &payload_aad(context),
                },
            )
            .map_err(|_| CredentialVaultError::Encryption)?;
        let wrapping_cipher = XChaCha20Poly1305::new(&Array(*self.wrapping_key));
        let wrapped_data_key = wrapping_cipher
            .encrypt(
                &XNonce::from(wrap_nonce),
                Payload {
                    msg: data_key.as_ref(),
                    aad: &wrap_aad(context),
                },
            )
            .map_err(|_| CredentialVaultError::Encryption)?;
        Ok(EncryptedCredentialMaterial {
            encrypted_payload,
            payload_nonce: payload_nonce.to_vec(),
            wrapped_data_key,
            wrap_nonce: wrap_nonce.to_vec(),
        })
    }

    pub fn decrypt(
        &self,
        context: &CredentialContext<'_>,
        material: &EncryptedCredentialMaterial,
    ) -> Result<Zeroizing<Vec<u8>>, CredentialVaultError> {
        validate_context(context)?;
        let payload_nonce: [u8; 24] = material
            .payload_nonce
            .as_slice()
            .try_into()
            .map_err(|_| CredentialVaultError::InvalidEnvelope)?;
        let wrap_nonce: [u8; 24] = material
            .wrap_nonce
            .as_slice()
            .try_into()
            .map_err(|_| CredentialVaultError::InvalidEnvelope)?;
        let wrapping_cipher = XChaCha20Poly1305::new(&Array(*self.wrapping_key));
        let data_key = Zeroizing::new(
            wrapping_cipher
                .decrypt(
                    &XNonce::from(wrap_nonce),
                    Payload {
                        msg: &material.wrapped_data_key,
                        aad: &wrap_aad(context),
                    },
                )
                .map_err(|_| CredentialVaultError::InvalidEnvelope)?,
        );
        let data_key: &[u8; 32] = data_key
            .as_slice()
            .try_into()
            .map_err(|_| CredentialVaultError::InvalidEnvelope)?;
        let cipher = XChaCha20Poly1305::new(&Array(*data_key));
        let plaintext = cipher
            .decrypt(
                &XNonce::from(payload_nonce),
                Payload {
                    msg: &material.encrypted_payload,
                    aad: &payload_aad(context),
                },
            )
            .map_err(|_| CredentialVaultError::InvalidEnvelope)?;
        Ok(Zeroizing::new(plaintext))
    }

    pub fn credential_identity_hmac(
        &self,
        provider: &str,
        stable_secret: &[u8],
    ) -> Result<String, CredentialVaultError> {
        if !valid_identifier(provider) || stable_secret.is_empty() {
            return Err(CredentialVaultError::InvalidContext);
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(self.identity_key.as_ref())
            .map_err(|_| CredentialVaultError::KeyDerivation)?;
        mac.update(provider.as_bytes());
        mac.update(&[0]);
        mac.update(stable_secret);
        Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
    }
}

fn validate_context(context: &CredentialContext<'_>) -> Result<(), CredentialVaultError> {
    if !valid_identifier(context.credential_id) || !valid_identifier(context.account_id) {
        return Err(CredentialVaultError::InvalidContext);
    }
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    (3..=128).contains(&value.len())
        && value.bytes().all(|value| {
            value.is_ascii_alphanumeric() || matches!(value, b'_' | b'-' | b'.' | b':')
        })
}

fn wrap_aad(context: &CredentialContext<'_>) -> Vec<u8> {
    format!(
        "{ENCRYPTION_SCHEMA}\nwrap\n{}\n{}",
        context.credential_id, context.account_id
    )
    .into_bytes()
}

fn payload_aad(context: &CredentialContext<'_>) -> Vec<u8> {
    format!(
        "{ENCRYPTION_SCHEMA}\npayload\n{}\n{}\n{}",
        context.credential_id, context.account_id, context.revision
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(credential_id: &'a str, revision: u32) -> CredentialContext<'a> {
        CredentialContext {
            credential_id,
            account_id: "account-test-001",
            revision,
        }
    }

    #[test]
    fn one_credential_instance_round_trips_with_independent_envelope_keys() {
        let vault =
            CredentialVault::new(&[91_u8; 32], "installation-test-001").expect("create vault");
        let first = vault
            .encrypt(&context("credential-test-001", 0), b"refresh-token-a")
            .expect("encrypt first");
        let second = vault
            .encrypt(&context("credential-test-001", 0), b"refresh-token-a")
            .expect("encrypt second");
        assert_ne!(first.encrypted_payload, second.encrypted_payload);
        assert_ne!(first.wrapped_data_key, second.wrapped_data_key);
        assert_eq!(
            vault
                .decrypt(&context("credential-test-001", 0), &first)
                .expect("decrypt first")
                .as_slice(),
            b"refresh-token-a"
        );
    }

    #[test]
    fn envelope_cannot_move_between_instances_revisions_or_installations() {
        let vault =
            CredentialVault::new(&[92_u8; 32], "installation-test-001").expect("create vault");
        let encrypted = vault
            .encrypt(&context("credential-test-001", 0), b"secret")
            .expect("encrypt");
        assert_eq!(
            vault.decrypt(&context("credential-test-002", 0), &encrypted),
            Err(CredentialVaultError::InvalidEnvelope)
        );
        assert_eq!(
            vault.decrypt(&context("credential-test-001", 1), &encrypted),
            Err(CredentialVaultError::InvalidEnvelope)
        );
        let other = CredentialVault::new(&[92_u8; 32], "installation-test-002")
            .expect("create other vault");
        assert_eq!(
            other.decrypt(&context("credential-test-001", 0), &encrypted),
            Err(CredentialVaultError::InvalidEnvelope)
        );
    }

    #[test]
    fn duplicate_stable_tokens_have_the_same_private_fingerprint() {
        let vault =
            CredentialVault::new(&[93_u8; 32], "installation-test-001").expect("create vault");
        let first = vault
            .credential_identity_hmac("openai", b"refresh-token-a")
            .expect("first identity");
        let duplicate = vault
            .credential_identity_hmac("openai", b"refresh-token-a")
            .expect("duplicate identity");
        let independent = vault
            .credential_identity_hmac("openai", b"refresh-token-b")
            .expect("independent identity");
        assert_eq!(first, duplicate);
        assert_ne!(first, independent);
    }
}
