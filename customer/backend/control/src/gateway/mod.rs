//! Public protocol boundaries for the model gateway.
//!
//! HTTP handlers authenticate, reserve quota, and select a route. Protocol-specific
//! validation and translation live here so the provider transport never needs to
//! understand OpenAI Chat, Anthropic, or Images payloads.

pub(crate) mod anthropic;
pub(crate) mod capabilities;
pub(crate) mod chat;
pub(crate) mod durable_settlement;
pub(crate) mod execution_options;
pub(crate) mod images;
pub(crate) mod orchestrator;
pub(crate) mod protocol;
pub(crate) mod responses;
pub(crate) mod settlement_outbox;
