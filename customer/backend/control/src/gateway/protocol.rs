use serde_json::{Map, Value, json};

use crate::ControlError;

#[derive(Clone, Copy)]
pub(crate) enum CanonicalResponseContract {
    Conversational,
    Image,
}

impl CanonicalResponseContract {
    pub(crate) fn validate(self, response: &Value) -> Result<(), ControlError> {
        match self {
            Self::Conversational => canonical_output(response).map(|_| ()),
            Self::Image => {
                crate::gateway::images::ImagesProtocolAdapter::generated_image(response).map(|_| ())
            }
        }
    }
}

pub(crate) fn decode_json(body: &[u8]) -> Result<Value, ControlError> {
    const MAX_GATEWAY_REQUEST_BYTES: usize = 16 * 1024 * 1024;
    if body.is_empty() || body.len() > MAX_GATEWAY_REQUEST_BYTES {
        return Err(ControlError::GatewayRequestInvalid);
    }
    serde_json::from_slice(body).map_err(|_| ControlError::GatewayRequestInvalid)
}

pub(crate) fn required_string(
    object: &Map<String, Value>,
    key: &str,
    maximum_length: usize,
) -> Result<String, ControlError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= maximum_length)
        .map(str::to_owned)
        .ok_or(ControlError::GatewayRequestInvalid)
}

pub(crate) fn text(value: &Value) -> Option<String> {
    if let Some(value) = value.as_str() {
        return Some(value.to_owned());
    }
    let items = value.as_array()?;
    let mut parts = Vec::new();
    for item in items {
        let item = item.as_object()?;
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("text");
        if !matches!(kind, "text" | "input_text" | "output_text") {
            return None;
        }
        parts.push(item.get("text")?.as_str()?.to_owned());
    }
    Some(parts.join("\n"))
}

pub(crate) fn canonical_output(response: &Value) -> Result<(String, Vec<Value>), ControlError> {
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .ok_or(ControlError::InvalidUpstreamResponse)?;
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for item in output {
        let item = item
            .as_object()
            .ok_or(ControlError::InvalidUpstreamResponse)?;
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                let content = item
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or(ControlError::InvalidUpstreamResponse)?;
                for part in content {
                    if matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("output_text" | "text")
                    ) {
                        text.push_str(
                            part.get("text")
                                .and_then(Value::as_str)
                                .ok_or(ControlError::InvalidUpstreamResponse)?,
                        );
                    }
                }
            }
            Some("function_call") => {
                let id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or(ControlError::InvalidUpstreamResponse)?;
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or(ControlError::InvalidUpstreamResponse)?;
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .ok_or(ControlError::InvalidUpstreamResponse)?;
                tool_calls.push(json!({
                    "id":id,
                    "type":"function",
                    "function":{"name":name,"arguments":arguments},
                }));
            }
            _ => {}
        }
    }
    Ok((text, tool_calls))
}
