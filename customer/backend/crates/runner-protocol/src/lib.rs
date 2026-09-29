#![forbid(unsafe_code)]

use std::{collections::BTreeMap, fmt};

use aster_error_catalog::{ErrorDescriptor, runner as runner_errors};
use aster_license_core::canonicalize;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{
    Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey, pkcs8::DecodePublicKey as _,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use zeroize::Zeroize as _;

mod authorization;
pub use authorization::{
    AdminSubject, ControlService, ImageModelSubject, MaintenanceActor, ModelResource, ModelSubject,
    SignedExpiry, TaskAuthorization, TaskLicense,
};

pub const RUNNER_PROTOCOL_VERSION: u32 = 4;
pub const TASK_TICKET_SCHEMA: &str = "aster.runner-task.v4";
pub const TASK_TICKET_LIFETIME_SECONDS: i64 = 120;
pub const MAX_TASK_EXECUTION_MILLISECONDS: u32 = 600_000;
pub const PROBE_PROVIDER: &str = "aster";
pub const PROBE_HOST: &str = "runner.internal";
pub const PROBE_TIMEOUT_MS: u32 = 3000;
const FUTURE_CLOCK_SKEW_SECONDS: i64 = 30;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCommand {
    Execute,
    FetchAsset,
    DiscoverModels,
    RefreshCredential,
    AuthorizeCredential,
    Probe,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskTicketClaims {
    pub schema: String,
    pub key_id: String,
    pub task_id: String,
    pub runner_id: String,
    pub provider_id: String,
    #[serde(deserialize_with = "required_nullable")]
    pub credential_instance_id: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub credential_revision: Option<u32>,
    pub upstream_host: String,
    pub command: TaskCommand,
    pub authorization: TaskAuthorization,
    pub payload_sha256: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub execution_deadline_ms: i64,
    pub nonce: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskTicketDocument {
    #[serde(flatten)]
    pub claims: TaskTicketClaims,
    pub signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskTicketWire {
    schema: String,
    key_id: String,
    task_id: String,
    runner_id: String,
    provider_id: String,
    #[serde(deserialize_with = "required_nullable")]
    credential_instance_id: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    credential_revision: Option<u32>,
    upstream_host: String,
    command: TaskCommand,
    authorization: TaskAuthorization,
    payload_sha256: String,
    issued_at: i64,
    expires_at: i64,
    execution_deadline_ms: i64,
    nonce: String,
    signature: String,
}

impl TaskTicketWire {
    fn into_document(self) -> TaskTicketDocument {
        TaskTicketDocument {
            claims: TaskTicketClaims {
                schema: self.schema,
                key_id: self.key_id,
                task_id: self.task_id,
                runner_id: self.runner_id,
                provider_id: self.provider_id,
                credential_instance_id: self.credential_instance_id,
                credential_revision: self.credential_revision,
                upstream_host: self.upstream_host,
                command: self.command,
                authorization: self.authorization,
                payload_sha256: self.payload_sha256,
                issued_at: self.issued_at,
                expires_at: self.expires_at,
                execution_deadline_ms: self.execution_deadline_ms,
                nonce: self.nonce,
            },
            signature: self.signature,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTaskTicket(TaskTicketDocument);

impl VerifiedTaskTicket {
    pub fn claims(&self) -> &TaskTicketClaims {
        &self.0.claims
    }

    /// A verified signature is not a perpetual execution grant. Call again at
    /// delayed consumption boundaries without reconstructing an unverified claim.
    pub fn validate_execution_time(&self, now: OffsetDateTime) -> Result<(), RunnerProtocolError> {
        validate_ticket_time(&self.0.claims, now)
    }
}

#[derive(Default)]
pub struct TrustedTaskKeys {
    keys: BTreeMap<String, VerifyingKey>,
}

impl TrustedTaskKeys {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &mut self,
        key_id: impl Into<String>,
        key: VerifyingKey,
    ) -> Result<(), RunnerProtocolError> {
        let key_id = key_id.into();
        validate_identifier(&key_id)?;
        if self.keys.insert(key_id, key).is_some() {
            return Err(RunnerProtocolError::DuplicateKey);
        }
        Ok(())
    }

    pub fn insert_spki_base64url(
        &mut self,
        key_id: impl Into<String>,
        encoded: &str,
    ) -> Result<(), RunnerProtocolError> {
        let der = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| RunnerProtocolError::InvalidPublicKey)?;
        let key = VerifyingKey::from_public_key_der(&der)
            .map_err(|_| RunnerProtocolError::InvalidPublicKey)?;
        self.insert(key_id, key)
    }

    fn get(&self, key_id: &str) -> Result<&VerifyingKey, RunnerProtocolError> {
        self.keys
            .get(key_id)
            .ok_or(RunnerProtocolError::UntrustedKey)
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum RunnerProtocolError {
    #[error("Runner task ticket JSON is invalid")]
    InvalidJson,
    #[error("Runner task ticket contains trailing JSON")]
    TrailingJson,
    #[error("Runner task ticket field is invalid")]
    InvalidField,
    #[error("Runner task ticket signature is invalid")]
    InvalidSignature,
    #[error("Runner task ticket key is not trusted")]
    UntrustedKey,
    #[error("Runner task ticket public key is invalid")]
    InvalidPublicKey,
    #[error("Runner task ticket key is duplicated")]
    DuplicateKey,
    #[error("Runner task ticket is for a different Runner")]
    RunnerMismatch,
    #[error("Runner task payload digest does not match the ticket")]
    PayloadMismatch,
    #[error("Runner task ticket is outside its valid time range")]
    InvalidTime,
    #[error("authenticated Runner task ticket has expired")]
    ExpiredTask(String),
    #[error("Runner task ticket was already consumed")]
    Replay,
    #[error("Runner protocol version is incompatible")]
    ProtocolIncompatible,
    #[error("canonical Runner task ticket encoding failed")]
    CanonicalEncoding,
}

impl RunnerProtocolError {
    pub const fn descriptor(&self) -> ErrorDescriptor {
        match self {
            Self::Replay => runner_errors::TASK_REPLAYED,
            Self::ProtocolIncompatible => runner_errors::PROTOCOL_INCOMPATIBLE,
            _ => runner_errors::TASK_TICKET_INVALID,
        }
    }
}

pub struct TaskTicketIssue<'a> {
    pub key_id: &'a str,
    pub signing_key: &'a SigningKey,
    pub task_id: &'a str,
    pub runner_id: &'a str,
    pub provider_id: &'a str,
    pub credential_instance_id: Option<&'a str>,
    pub credential_revision: Option<u32>,
    pub upstream_host: &'a str,
    pub command: TaskCommand,
    pub authorization: TaskAuthorization,
    pub payload: &'a [u8],
    pub nonce: &'a str,
    pub now: OffsetDateTime,
    pub execution_timeout_ms: u32,
}

pub fn issue_task_ticket(
    request: TaskTicketIssue<'_>,
) -> Result<TaskTicketDocument, RunnerProtocolError> {
    if !(1..=MAX_TASK_EXECUTION_MILLISECONDS).contains(&request.execution_timeout_ms)
        || (request.command == TaskCommand::Probe
            && request.execution_timeout_ms > PROBE_TIMEOUT_MS)
    {
        return Err(RunnerProtocolError::InvalidTime);
    }
    let issued_at = request.now.unix_timestamp();
    let issued_ms = i64::try_from(request.now.unix_timestamp_nanos() / 1_000_000)
        .map_err(|_| RunnerProtocolError::InvalidTime)?;
    let maximum_expiry = issued_at
        .checked_add(TASK_TICKET_LIFETIME_SECONDS)
        .ok_or(RunnerProtocolError::InvalidTime)?;
    let expires_at = request
        .authorization
        .deadline()
        .map_or(maximum_expiry, |expiry| expiry.min(maximum_expiry));
    let claims = TaskTicketClaims {
        schema: TASK_TICKET_SCHEMA.to_owned(),
        key_id: request.key_id.to_owned(),
        task_id: request.task_id.to_owned(),
        runner_id: request.runner_id.to_owned(),
        provider_id: request.provider_id.to_owned(),
        credential_instance_id: request.credential_instance_id.map(str::to_owned),
        credential_revision: request.credential_revision,
        upstream_host: request.upstream_host.to_owned(),
        command: request.command,
        authorization: request.authorization,
        payload_sha256: payload_digest(request.payload),
        issued_at,
        expires_at,
        execution_deadline_ms: issued_ms
            .checked_add(i64::from(request.execution_timeout_ms))
            .ok_or(RunnerProtocolError::InvalidTime)?,
        nonce: request.nonce.to_owned(),
    };
    validate_claims(&claims)?;
    let signature = request.signing_key.sign(&canonical_claims(&claims)?);
    Ok(TaskTicketDocument {
        claims,
        signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    })
}

pub fn verify_task_ticket(
    encoded_ticket: &[u8],
    trusted_keys: &TrustedTaskKeys,
    expected_runner_id: &str,
    payload: &[u8],
    now: OffsetDateTime,
) -> Result<VerifiedTaskTicket, RunnerProtocolError> {
    let wire: TaskTicketWire = decode_exact(encoded_ticket)?;
    let document = wire.into_document();
    validate_claims(&document.claims)?;
    if document.claims.runner_id != expected_runner_id {
        return Err(RunnerProtocolError::RunnerMismatch);
    }
    if document.claims.payload_sha256 != payload_digest(payload) {
        return Err(RunnerProtocolError::PayloadMismatch);
    }
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(&document.signature)
        .map_err(|_| RunnerProtocolError::InvalidSignature)?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| RunnerProtocolError::InvalidSignature)?;
    trusted_keys
        .get(&document.claims.key_id)?
        .verify(&canonical_claims(&document.claims)?, &signature)
        .map_err(|_| RunnerProtocolError::InvalidSignature)?;
    let now_ms = now.unix_timestamp_nanos() / 1_000_000;
    let now = now.unix_timestamp();
    if document.claims.issued_at
        > now
            .checked_add(FUTURE_CLOCK_SKEW_SECONDS)
            .ok_or(RunnerProtocolError::InvalidTime)?
    {
        return Err(RunnerProtocolError::InvalidTime);
    }
    if now_ms >= i128::from(document.claims.execution_deadline_ms)
        || now >= document.claims.expires_at
    {
        return Err(RunnerProtocolError::ExpiredTask(document.claims.task_id));
    }
    Ok(VerifiedTaskTicket(document))
}

fn validate_ticket_time(
    claims: &TaskTicketClaims,
    now: OffsetDateTime,
) -> Result<(), RunnerProtocolError> {
    let now_ms = now.unix_timestamp_nanos() / 1_000_000;
    let now = now.unix_timestamp();
    let latest_issuance = now
        .checked_add(FUTURE_CLOCK_SKEW_SECONDS)
        .ok_or(RunnerProtocolError::InvalidTime)?;
    if now >= claims.expires_at
        || now_ms >= i128::from(claims.execution_deadline_ms)
        || claims.issued_at > latest_issuance
    {
        return Err(RunnerProtocolError::InvalidTime);
    }
    Ok(())
}

#[derive(Default)]
pub struct ReplayCache {
    consumed: BTreeMap<String, i64>,
}

impl ReplayCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn consume(
        &mut self,
        ticket: &VerifiedTaskTicket,
        now: OffsetDateTime,
    ) -> Result<(), RunnerProtocolError> {
        ticket.validate_execution_time(now)?;
        let now = now.unix_timestamp();
        self.consumed.retain(|_, expires_at| *expires_at > now);
        if self.consumed.contains_key(&ticket.claims().task_id) {
            return Err(RunnerProtocolError::Replay);
        }
        self.consumed
            .insert(ticket.claims().task_id.clone(), ticket.claims().expires_at);
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerHello {
    pub runner_id: String,
    pub protocol_version: u32,
    pub runner_version: String,
    pub platform: String,
    pub architecture: String,
    pub max_inflight: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerHeartbeat {
    pub inflight: u32,
    pub recent_request_count: u32,
    pub recent_error_count: u32,
    pub latency_ms: u32,
    pub observed_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskLifecycle {
    pub task_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskResult {
    pub task_id: String,
    pub status: u16,
    pub usage_json: Option<String>,
}

#[derive(Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialRefreshResult {
    pub task_id: String,
    pub credential_id: String,
    pub expected_revision: u32,
    pub status: u16,
    pub credential_payload_base64url: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamRequestHeader {
    pub name: String,
    pub value: String,
}

impl fmt::Debug for UpstreamRequestHeader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpstreamRequestHeader")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

impl Drop for UpstreamRequestHeader {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<UpstreamRequestHeader>,
    pub body_base64url: String,
}

impl fmt::Debug for UpstreamHttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpstreamHttpRequest")
            .field("method", &self.method)
            .field("url", &"[REDACTED]")
            .field("headers", &self.headers)
            .field("body_base64url", &"[REDACTED]")
            .finish()
    }
}

impl Drop for UpstreamHttpRequest {
    fn drop(&mut self) {
        self.url.zeroize();
        self.body_base64url.zeroize();
    }
}

#[derive(Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialRefreshTask {
    pub credential_id: String,
    pub expected_revision: u32,
    pub request: UpstreamHttpRequest,
}

impl fmt::Debug for CredentialRefreshTask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialRefreshTask")
            .field("credential_id", &self.credential_id)
            .field("expected_revision", &self.expected_revision)
            .field("request", &self.request)
            .finish()
    }
}

impl fmt::Debug for CredentialRefreshResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialRefreshResult")
            .field("task_id", &self.task_id)
            .field("credential_id", &self.credential_id)
            .field("expected_revision", &self.expected_revision)
            .field("status", &self.status)
            .field("credential_payload_base64url", &"[REDACTED]")
            .finish()
    }
}

impl Drop for CredentialRefreshResult {
    fn drop(&mut self) {
        self.credential_payload_base64url.zeroize();
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseHeader {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskResponseStarted {
    pub task_id: String,
    pub status: u16,
    pub headers: Vec<ResponseHeader>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskFailure {
    pub task_id: String,
    pub category: String,
    pub retryable_before_upstream: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskChunk {
    pub task_id: String,
    pub sequence: u32,
    pub data_base64url: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskFrame {
    pub ticket_json_base64url: String,
    pub payload_base64url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PingFrame {
    pub nonce: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum ControlToRunner {
    Task(TaskFrame),
    Ping(PingFrame),
    Drain,
    CancelTask(TaskLifecycle),
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum RunnerToControl {
    Hello(RunnerHello),
    Heartbeat(RunnerHeartbeat),
    Pong(PingFrame),
    TaskAccepted(TaskLifecycle),
    UpstreamStarted(TaskLifecycle),
    TaskResponseStarted(TaskResponseStarted),
    TaskChunk(TaskChunk),
    TaskFinished(TaskResult),
    CredentialRefreshed(CredentialRefreshResult),
    TaskFailed(TaskFailure),
}

pub fn decode_control_frame(data: &[u8]) -> Result<ControlToRunner, RunnerProtocolError> {
    let frame = decode_exact(data)?;
    if let ControlToRunner::CancelTask(task) = &frame {
        validate_identifier(&task.task_id)?;
    }
    Ok(frame)
}

pub fn encode_control_frame(frame: &ControlToRunner) -> Result<Vec<u8>, RunnerProtocolError> {
    serde_json::to_vec(frame).map_err(|_| RunnerProtocolError::InvalidJson)
}

pub fn encode_runner_frame(frame: &RunnerToControl) -> Result<Vec<u8>, RunnerProtocolError> {
    serde_json::to_vec(frame).map_err(|_| RunnerProtocolError::InvalidJson)
}

pub fn decode_runner_frame(data: &[u8]) -> Result<RunnerToControl, RunnerProtocolError> {
    decode_exact(data)
}

pub fn decode_base64url(value: &str) -> Result<Vec<u8>, RunnerProtocolError> {
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| RunnerProtocolError::InvalidField)
}

pub fn encode_base64url(value: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(value)
}

fn payload_digest(payload: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(payload))
}

fn canonical_claims(claims: &TaskTicketClaims) -> Result<Vec<u8>, RunnerProtocolError> {
    let value = serde_json::to_value(claims).map_err(|_| RunnerProtocolError::CanonicalEncoding)?;
    canonicalize(&value).map_err(|_| RunnerProtocolError::CanonicalEncoding)
}

fn decode_exact<'de, T: Deserialize<'de>>(data: &'de [u8]) -> Result<T, RunnerProtocolError> {
    let mut deserializer = serde_json::Deserializer::from_slice(data);
    let decoded =
        T::deserialize(&mut deserializer).map_err(|_| RunnerProtocolError::InvalidJson)?;
    deserializer
        .end()
        .map_err(|_| RunnerProtocolError::TrailingJson)?;
    Ok(decoded)
}

// An explicit null is valid for enrollment; an omitted field is not. A custom
// deserializer prevents serde's implicit missing-Option default in both wires.
fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

fn validate_claims(claims: &TaskTicketClaims) -> Result<(), RunnerProtocolError> {
    if claims.schema != TASK_TICKET_SCHEMA
        || !claims
            .expires_at
            .checked_sub(claims.issued_at)
            .is_some_and(|duration| (1..=TASK_TICKET_LIFETIME_SECONDS).contains(&duration))
        || !(1..=i128::from(MAX_TASK_EXECUTION_MILLISECONDS) + 999).contains(
            &(i128::from(claims.execution_deadline_ms) - i128::from(claims.issued_at) * 1000),
        )
        || URL_SAFE_NO_PAD
            .decode(&claims.payload_sha256)
            .map_or(true, |value| value.len() != 32)
    {
        return Err(RunnerProtocolError::InvalidField);
    }
    claims
        .authorization
        .validate(claims.command, claims.expires_at)?;
    for value in [
        &claims.key_id,
        &claims.task_id,
        &claims.runner_id,
        &claims.provider_id,
        &claims.nonce,
    ] {
        validate_identifier(value)?;
    }
    validate_upstream_host(&claims.upstream_host)?;
    match (
        claims.command,
        claims.credential_instance_id.as_deref(),
        claims.credential_revision,
    ) {
        (TaskCommand::Probe, None, None)
            if claims.provider_id == PROBE_PROVIDER
                && claims.upstream_host == PROBE_HOST
                && i128::from(claims.execution_deadline_ms)
                    - i128::from(claims.issued_at) * 1000
                    <= i128::from(PROBE_TIMEOUT_MS) + 999 => {}
        (TaskCommand::Probe, _, _) => return Err(RunnerProtocolError::InvalidField),
        (TaskCommand::AuthorizeCredential, None, None) => {}
        (TaskCommand::AuthorizeCredential, _, _) => return Err(RunnerProtocolError::InvalidField),
        (_, Some(credential_instance_id), Some(_)) => {
            validate_identifier(credential_instance_id)?;
        }
        _ => return Err(RunnerProtocolError::InvalidField),
    }
    Ok(())
}

fn validate_upstream_host(value: &str) -> Result<(), RunnerProtocolError> {
    if !(1..=253).contains(&value.len())
        || value.ends_with('.')
        || value.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(RunnerProtocolError::InvalidField);
    }
    Ok(())
}

fn validate_identifier(value: &str) -> Result<(), RunnerProtocolError> {
    if !(3..=128).contains(&value.len())
        || !value.bytes().all(|value| {
            value.is_ascii_alphanumeric() || matches!(value, b'_' | b'-' | b'.' | b':')
        })
    {
        return Err(RunnerProtocolError::InvalidField);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use time::macros::datetime;

    use super::*;

    mod authorization;

    #[test]
    fn published_cross_language_vector_matches_the_protocol_contract() {
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../contracts/test-vectors/runner-task.v4.json"
        ))
        .expect("parse Runner task vector");
        let ticket = serde_json::to_vec(&fixture["ticket"]).expect("serialize vector ticket");
        let payload = fixture["payload_json"]
            .as_str()
            .expect("vector payload must be a string");
        let public_key = fixture["public_key_spki"]
            .as_str()
            .expect("vector public key must be a string");
        let mut keys = TrustedTaskKeys::new();
        keys.insert_spki_base64url("runner-task-vector-v4", public_key)
            .expect("load vector public key");
        let verified = verify_task_ticket(
            &ticket,
            &keys,
            "runner-vector-123",
            payload.as_bytes(),
            OffsetDateTime::from_unix_timestamp(1_787_565_660).expect("valid vector time"),
        )
        .expect("verify vector ticket");
        assert_eq!(verified.claims().key_id, "runner-task-vector-v4");
        assert_eq!(
            canonical_claims(verified.claims()).unwrap(),
            fixture["canonical_claims"].as_str().unwrap().as_bytes()
        );
        for case in fixture["cases"].as_array().unwrap() {
            let verified = verify_task_ticket(
                &serde_json::to_vec(&case["ticket"]).unwrap(),
                &keys,
                "runner-vector-123",
                payload.as_bytes(),
                OffsetDateTime::from_unix_timestamp(1_787_565_601).unwrap(),
            )
            .unwrap();
            assert_eq!(
                canonical_claims(verified.claims()).unwrap(),
                case["canonical_claims"].as_str().unwrap().as_bytes()
            );
        }
    }

    fn signed(payload: &[u8]) -> (Vec<u8>, TrustedTaskKeys) {
        signed_with_timeout(payload, MAX_TASK_EXECUTION_MILLISECONDS)
    }

    fn signed_with_timeout(
        payload: &[u8],
        execution_timeout_ms: u32,
    ) -> (Vec<u8>, TrustedTaskKeys) {
        let signing_key = SigningKey::from_bytes(&[71_u8; 32]);
        let document = issue_task_ticket(TaskTicketIssue {
            key_id: "runner-task-test-01",
            signing_key: &signing_key,
            task_id: "task-test-001",
            runner_id: "runner-test-001",
            provider_id: "openai",
            credential_instance_id: Some("credential_00000000000000000000000000000001"),
            credential_revision: Some(7),
            upstream_host: "api.openai.com",
            command: TaskCommand::Execute,
            authorization: authorization::model_authorization(),
            payload,
            nonce: "nonce-test-001",
            now: datetime!(2026-08-28 0:00 UTC),
            execution_timeout_ms,
        })
        .expect("issue ticket");
        let mut keys = TrustedTaskKeys::new();
        keys.insert("runner-task-test-01", signing_key.verifying_key())
            .expect("insert key");
        (
            serde_json::to_vec(&document).expect("serialize ticket"),
            keys,
        )
    }

    #[test]
    fn ticket_binds_runner_payload_command_and_two_minute_lifetime() {
        let payload = br#"{"model":"gpt-test"}"#;
        let (ticket, keys) = signed(payload);
        let verified = verify_task_ticket(
            &ticket,
            &keys,
            "runner-test-001",
            payload,
            datetime!(2026-08-28 0:01 UTC),
        )
        .expect("verify ticket");
        assert_eq!(verified.claims().command, TaskCommand::Execute);
        assert_eq!(verified.claims().provider_id, "openai");
        assert_eq!(
            verified.claims().credential_instance_id.as_deref(),
            Some("credential_00000000000000000000000000000001")
        );
        assert_eq!(verified.claims().credential_revision, Some(7));
        assert_eq!(verified.claims().upstream_host, "api.openai.com");
        assert_eq!(
            verified.claims().expires_at - verified.claims().issued_at,
            TASK_TICKET_LIFETIME_SECONDS
        );
        assert_eq!(
            verify_task_ticket(
                &ticket,
                &keys,
                "another-runner",
                payload,
                datetime!(2026-08-28 0:01 UTC),
            ),
            Err(RunnerProtocolError::RunnerMismatch)
        );
        assert_eq!(
            verify_task_ticket(
                &ticket,
                &keys,
                "runner-test-001",
                b"tampered",
                datetime!(2026-08-28 0:01 UTC),
            ),
            Err(RunnerProtocolError::PayloadMismatch)
        );
    }

    #[test]
    fn expired_and_replayed_tickets_are_rejected() {
        let payload = b"payload";
        let (ticket, keys) = signed(payload);
        assert_eq!(
            verify_task_ticket(
                &ticket,
                &keys,
                "runner-test-001",
                payload,
                datetime!(2026-08-28 0:02 UTC),
            ),
            Err(RunnerProtocolError::ExpiredTask("task-test-001".into()))
        );
        let verified = verify_task_ticket(
            &ticket,
            &keys,
            "runner-test-001",
            payload,
            datetime!(2026-08-28 0:01 UTC),
        )
        .expect("verify ticket");
        let mut replay = ReplayCache::new();
        replay
            .consume(&verified, datetime!(2026-08-28 0:01 UTC))
            .expect("first consumption");
        assert_eq!(
            replay.consume(&verified, datetime!(2026-08-28 0:01 UTC)),
            Err(RunnerProtocolError::Replay)
        );
        assert_eq!(
            replay.consume(&verified, datetime!(2026-08-28 0:02 UTC)),
            Err(RunnerProtocolError::InvalidTime)
        );
        assert_eq!(
            verified.validate_execution_time(datetime!(2026-08-28 0:02 UTC)),
            Err(RunnerProtocolError::InvalidTime)
        );
        assert_eq!(
            verified.validate_execution_time(datetime!(2026-08-27 23:59:29 UTC)),
            Err(RunnerProtocolError::InvalidTime)
        );
    }

    #[test]
    fn extreme_untrusted_timestamps_are_rejected_without_arithmetic_overflow() {
        let payload = b"payload";
        let (ticket, keys) = signed(payload);
        let original: serde_json::Value = serde_json::from_slice(&ticket).unwrap();
        for (issued, expiry) in [(i64::MIN, i64::MAX), (i64::MAX, i64::MIN)] {
            let mut changed = original.clone();
            changed["issued_at"] = serde_json::json!(issued);
            changed["expires_at"] = serde_json::json!(expiry);
            assert_eq!(
                verify_task_ticket(
                    &serde_json::to_vec(&changed).unwrap(),
                    &keys,
                    "runner-test-001",
                    payload,
                    datetime!(2026-08-28 0:01 UTC)
                ),
                Err(RunnerProtocolError::InvalidField)
            );
        }
    }

    #[test]
    fn strict_decoder_rejects_unknown_and_trailing_fields() {
        let payload = b"payload";
        let (ticket, keys) = signed(payload);
        let mut value: serde_json::Value =
            serde_json::from_slice(&ticket).expect("parse ticket JSON");
        value["runner_limit"] = serde_json::json!(999);
        assert_eq!(
            verify_task_ticket(
                &serde_json::to_vec(&value).expect("serialize modified ticket"),
                &keys,
                "runner-test-001",
                payload,
                datetime!(2026-08-28 0:01 UTC),
            ),
            Err(RunnerProtocolError::InvalidJson)
        );
        let mut trailing = ticket;
        trailing.extend_from_slice(b" true");
        assert_eq!(
            verify_task_ticket(
                &trailing,
                &keys,
                "runner-test-001",
                payload,
                datetime!(2026-08-28 0:01 UTC),
            ),
            Err(RunnerProtocolError::TrailingJson)
        );
    }

    #[test]
    fn credential_refresh_result_round_trips_without_exposing_secret_in_debug() {
        let frame = RunnerToControl::CredentialRefreshed(CredentialRefreshResult {
            task_id: "task_refresh_001".to_owned(),
            credential_id: "credential_00000000000000000000000000000001".to_owned(),
            expected_revision: 7,
            status: 200,
            credential_payload_base64url: "c2VjcmV0LXJlZnJlc2gtdG9rZW4".to_owned(),
        });
        let debug = format!("{frame:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("c2VjcmV0LXJlZnJlc2gtdG9rZW4"));
        let encoded = encode_runner_frame(&frame).expect("encode refresh result");
        let decoded = decode_runner_frame(&encoded).expect("decode refresh result");
        assert_eq!(decoded, frame);
    }
    #[test]
    fn execution_deadline_is_signed_and_can_expire_before_admission_ticket() {
        let (ticket, keys) = signed_with_timeout(b"payload", 10);
        let now = datetime!(2026-08-28 0:00 UTC);
        assert!(
            verify_task_ticket(
                &ticket,
                &keys,
                "runner-test-001",
                b"payload",
                now + time::Duration::milliseconds(9)
            )
            .is_ok()
        );
        assert_eq!(
            verify_task_ticket(
                &ticket,
                &keys,
                "runner-test-001",
                b"payload",
                now + time::Duration::milliseconds(10)
            ),
            Err(RunnerProtocolError::ExpiredTask("task-test-001".into()))
        );
        let mut altered: serde_json::Value = serde_json::from_slice(&ticket).unwrap();
        altered["execution_deadline_ms"] = serde_json::json!(now.unix_timestamp() * 1000 + 20);
        assert_eq!(
            verify_task_ticket(
                &serde_json::to_vec(&altered).unwrap(),
                &keys,
                "runner-test-001",
                b"payload",
                now
            ),
            Err(RunnerProtocolError::InvalidSignature)
        );
        altered["issued_at"] = serde_json::json!(i64::MAX);
        altered["expires_at"] = serde_json::json!(i64::MIN);
        assert!(
            verify_task_ticket(
                &serde_json::to_vec(&altered).unwrap(),
                &keys,
                "runner-test-001",
                b"payload",
                now
            )
            .is_err()
        );
    }

    #[test]
    fn legacy_ticket_cannot_claim_deadline_support_and_cancel_frames_are_strict() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../contracts/test-vectors/runner-task.v2.json"
        ))
        .unwrap();
        assert!(
            verify_task_ticket(
                &serde_json::to_vec(&fixture["ticket"]).unwrap(),
                &TrustedTaskKeys::new(),
                "runner-vector-123",
                b"payload",
                datetime!(2026-08-28 0:00 UTC)
            )
            .is_err()
        );
        let frame = ControlToRunner::CancelTask(TaskLifecycle {
            task_id: "task-a".into(),
        });
        assert!(
            matches!(decode_control_frame(&encode_control_frame(&frame).unwrap()).unwrap(), ControlToRunner::CancelTask(TaskLifecycle {task_id}) if task_id=="task-a")
        );
        assert!(
            decode_control_frame(br#"{"type":"cancel_task","task_id":"task-a","all":true}"#)
                .is_err()
        );
    }
    #[test]
    fn probe_ticket_cannot_bind_credentials_or_an_external_host_or_long_budget() {
        let key = SigningKey::from_bytes(&[81; 32]);
        let issue = |host, credential, budget| {
            issue_task_ticket(TaskTicketIssue {
                key_id: "probe-key",
                signing_key: &key,
                task_id: "probe-test",
                runner_id: "runner-test",
                provider_id: PROBE_PROVIDER,
                credential_instance_id: credential,
                credential_revision: credential.map(|_| 0),
                upstream_host: host,
                command: TaskCommand::Probe,
                authorization: TaskAuthorization::Probe {},
                payload: &[3; 32],
                nonce: "nonce-test",
                now: OffsetDateTime::now_utc(),
                execution_timeout_ms: budget,
            })
        };
        assert!(issue(PROBE_HOST, None, PROBE_TIMEOUT_MS).is_ok());
        assert!(issue("api.openai.com", None, PROBE_TIMEOUT_MS).is_err());
        assert!(issue(PROBE_HOST, Some("credential-test"), PROBE_TIMEOUT_MS).is_err());
        assert!(issue(PROBE_HOST, None, PROBE_TIMEOUT_MS + 1).is_err());
    }
}
