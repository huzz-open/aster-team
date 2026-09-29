use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::{ControlError, ModelUsage};

pub(crate) struct DecodedResponsesRequest {
    pub(crate) public_model: String,
    pub(crate) client_wants_stream: bool,
    pub(crate) body: Value,
}

pub(crate) struct ResponsesProtocolAdapter;

impl ResponsesProtocolAdapter {
    pub(crate) fn decode(body: &[u8]) -> Result<DecodedResponsesRequest, ControlError> {
        const MAX_GATEWAY_REQUEST_BYTES: usize = 16 * 1024 * 1024;
        if body.is_empty() || body.len() > MAX_GATEWAY_REQUEST_BYTES {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let mut body: Value =
            serde_json::from_slice(body).map_err(|_| ControlError::GatewayRequestInvalid)?;
        let object = body
            .as_object_mut()
            .ok_or(ControlError::GatewayRequestInvalid)?;
        let public_model = object
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty() && value.len() <= 160)
            .ok_or(ControlError::GatewayRequestInvalid)?
            .to_owned();
        let input = object
            .get_mut("input")
            .ok_or(ControlError::GatewayRequestInvalid)?;
        normalize_input(input)?;
        let client_wants_stream = object
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(DecodedResponsesRequest {
            public_model,
            client_wants_stream,
            body,
        })
    }
}

fn normalize_input(input: &mut Value) -> Result<(), ControlError> {
    match input {
        Value::String(text) => {
            *input = json!([{
                "role": "user",
                "content": [{"type": "input_text", "text": text}],
            }]);
        }
        Value::Object(_) => {
            *input = Value::Array(vec![std::mem::take(input)]);
        }
        Value::Array(_) => {}
        _ => return Err(ControlError::GatewayRequestInvalid),
    }
    Ok(())
}

/// Provider adapter for the ChatGPT Codex Responses transport.
pub(crate) struct CodexUpstreamAdapter;

impl CodexUpstreamAdapter {
    pub(crate) fn encode(body: &Value, upstream_model: &str) -> Result<Value, ControlError> {
        let mut body = body.clone();
        let object = body
            .as_object_mut()
            .ok_or(ControlError::GatewayRequestInvalid)?;
        let input = object
            .get_mut("input")
            .ok_or(ControlError::GatewayRequestInvalid)?;
        normalize_input(input)?;
        // The public Responses API accepts this field, but the ChatGPT Codex
        // transport used by Aster rejects it. Keep accepting it at the public
        // boundary and omit it only from this provider-specific request.
        object.remove("max_output_tokens");
        // Aster's canonical speed name is `fast`, while the ChatGPT Codex
        // transport names the same tier `priority` and rejects `fast`.
        if object.get("service_tier").and_then(Value::as_str) == Some("fast") {
            object.insert(
                "service_tier".to_owned(),
                Value::String("priority".to_owned()),
            );
        }
        object.insert("model".to_owned(), Value::String(upstream_model.to_owned()));
        object.insert("stream".to_owned(), Value::Bool(true));
        object.insert("store".to_owned(), Value::Bool(false));
        Ok(body)
    }
}

/// Reduces the Responses event stream into the canonical completed response used
/// by every buffered public protocol.
pub(crate) struct ResponseEventReducer {
    completed: Option<Value>,
    output: BTreeMap<usize, Value>,
    failed: bool,
}

impl ResponseEventReducer {
    pub(crate) fn new() -> Self {
        Self {
            completed: None,
            output: BTreeMap::new(),
            failed: false,
        }
    }

    pub(crate) fn push(&mut self, event: Value) {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "response.output_item.added" | "response.output_item.done" => {
                if let (Some(index), Some(item)) =
                    (output_index(&event), event.get("item").cloned())
                {
                    self.output.insert(index, item);
                }
            }
            "response.content_part.added" | "response.content_part.done" => {
                if let (Some(output_index), Some(content_index), Some(part)) = (
                    output_index(&event),
                    content_index(&event),
                    event.get("part").cloned(),
                ) {
                    self.set_content_part(output_index, content_index, part, &event);
                }
            }
            "response.output_text.delta" => {
                if let (Some(index), Some(delta)) = (
                    output_index(&event),
                    event.get("delta").and_then(Value::as_str),
                ) {
                    self.append_output_text(
                        index,
                        content_index(&event).unwrap_or(0),
                        delta,
                        &event,
                    );
                }
            }
            "response.output_text.done" => {
                if let (Some(index), Some(text)) = (
                    output_index(&event),
                    event.get("text").and_then(Value::as_str),
                ) {
                    self.set_output_text(index, content_index(&event).unwrap_or(0), text, &event);
                }
            }
            "response.function_call_arguments.delta" => {
                if let (Some(index), Some(delta)) = (
                    output_index(&event),
                    event.get("delta").and_then(Value::as_str),
                ) {
                    let item = self.output.entry(index).or_insert_with(|| {
                        json!({
                            "id": event.get("item_id").cloned().unwrap_or(Value::Null),
                            "type": "function_call",
                            "arguments": "",
                        })
                    });
                    let arguments = item
                        .as_object_mut()
                        .and_then(|item| {
                            item.entry("arguments")
                                .or_insert_with(|| json!(""))
                                .as_str()
                                .map(str::to_owned)
                        })
                        .unwrap_or_default();
                    item["arguments"] = Value::String(format!("{arguments}{delta}"));
                }
            }
            "response.function_call_arguments.done" => {
                if let (Some(index), Some(arguments)) = (
                    output_index(&event),
                    event.get("arguments").and_then(Value::as_str),
                ) && let Some(item) = self.output.get_mut(&index)
                {
                    item["arguments"] = Value::String(arguments.to_owned());
                }
            }
            "response.completed" => self.completed = event.get("response").cloned(),
            "error" | "response.failed" | "response.incomplete" => self.failed = true,
            _ => {}
        }
    }

    fn output_message<'a>(&'a mut self, index: usize, event: &Value) -> &'a mut Value {
        self.output.entry(index).or_insert_with(|| {
            json!({
                "id": event.get("item_id").cloned().unwrap_or(Value::Null),
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [],
            })
        })
    }

    fn set_content_part(
        &mut self,
        output_index: usize,
        content_index: usize,
        part: Value,
        event: &Value,
    ) {
        let item = self.output_message(output_index, event);
        let content = ensure_content(item);
        resize_with_null(content, content_index);
        content[content_index] = part;
    }

    fn append_output_text(
        &mut self,
        output_index: usize,
        content_index: usize,
        delta: &str,
        event: &Value,
    ) {
        let item = self.output_message(output_index, event);
        let content = ensure_content(item);
        resize_with_null(content, content_index);
        if !content[content_index].is_object() {
            content[content_index] = json!({"type": "output_text", "text": ""});
        }
        let current = content[content_index]
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        content[content_index]["text"] = Value::String(format!("{current}{delta}"));
    }

    fn set_output_text(
        &mut self,
        output_index: usize,
        content_index: usize,
        text: &str,
        event: &Value,
    ) {
        let item = self.output_message(output_index, event);
        let content = ensure_content(item);
        resize_with_null(content, content_index);
        content[content_index] = json!({"type": "output_text", "text": text});
    }

    pub(crate) fn finish(self) -> Result<(Value, ModelUsage), ControlError> {
        if self.failed {
            return Err(ControlError::UpstreamRequestFailed);
        }
        let mut completed = self.completed.ok_or(ControlError::UpstreamUsageInvalid)?;
        let should_rebuild = completed
            .get("output")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty);
        if should_rebuild && !self.output.is_empty() {
            completed["output"] = Value::Array(self.output.into_values().collect());
        }
        let usage = usage_from_response(&completed)?;
        Ok((completed, usage))
    }
}

fn ensure_content(item: &mut Value) -> &mut Vec<Value> {
    let object = item
        .as_object_mut()
        .expect("canonical output item is an object");
    object
        .entry("content")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .expect("canonical message content is an array")
}

fn resize_with_null(values: &mut Vec<Value>, index: usize) {
    if values.len() <= index {
        values.resize(index + 1, Value::Null);
    }
}

fn output_index(event: &Value) -> Option<usize> {
    event
        .get("output_index")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn content_index(event: &Value) -> Option<usize> {
    event
        .get("content_index")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn usage_from_response(completed: &Value) -> Result<ModelUsage, ControlError> {
    let usage = completed
        .get("usage")
        .and_then(Value::as_object)
        .ok_or(ControlError::UpstreamUsageInvalid)?;
    let mut input_tokens = usage
        .get("input_tokens")
        .and_then(Value::as_i64)
        .ok_or(ControlError::UpstreamUsageInvalid)?;
    let mut output_tokens = usage
        .get("output_tokens")
        .and_then(Value::as_i64)
        .ok_or(ControlError::UpstreamUsageInvalid)?;
    let details = usage.get("input_tokens_details").and_then(Value::as_object);
    let cached_input = token_field(details, "cached_tokens");
    let cache_write = token_field(details, "cache_write_tokens");
    if let Some(image_usage) = completed
        .get("tool_usage")
        .and_then(|value| value.get("image_gen"))
        .and_then(Value::as_object)
    {
        input_tokens = input_tokens
            .checked_add(token_field(Some(image_usage), "input_tokens"))
            .ok_or(ControlError::UpstreamUsageInvalid)?;
        output_tokens = output_tokens
            .checked_add(token_field(Some(image_usage), "output_tokens"))
            .ok_or(ControlError::UpstreamUsageInvalid)?;
    }
    if input_tokens < 0
        || output_tokens < 0
        || cached_input < 0
        || cache_write < 0
        || cached_input
            .checked_add(cache_write)
            .is_none_or(|total| total > input_tokens)
    {
        return Err(ControlError::UpstreamUsageInvalid);
    }
    Ok(ModelUsage {
        uncached_input: input_tokens - cached_input - cache_write,
        cached_input,
        cache_write,
        output_tokens,
        multiplier_micros: 1_000_000,
        protocol: "openai_responses".to_owned(),
        model: String::new(),
        requested_model: None,
        processing_tier: None,
        reasoning_effort: None,
        runner_id: String::new(),
    })
}

fn token_field(object: Option<&Map<String, Value>>, field: &str) -> i64 {
    object
        .and_then(|value| value.get(field))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

pub(crate) fn reduce_sse(body: &[u8]) -> Result<(Value, ModelUsage), ControlError> {
    let text = std::str::from_utf8(body).map_err(|_| ControlError::InvalidUpstreamResponse)?;
    let mut reducer = ResponseEventReducer::new();
    for line in text.lines() {
        let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<Value>(data) {
            reducer.push(event);
        }
    }
    reducer.finish()
}

pub(crate) fn rewrite_sse_frame_model(frame: &[u8], public_model: &str) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(frame) else {
        return frame.to_vec();
    };
    let mut rewritten = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        let Some(data) = line_without_newline
            .trim()
            .strip_prefix("data:")
            .map(str::trim)
        else {
            rewritten.push_str(line);
            continue;
        };
        let Ok(mut event) = serde_json::from_str::<Value>(data) else {
            rewritten.push_str(line);
            continue;
        };
        rewrite_model_fields(&mut event, public_model);
        let prefix = line_without_newline
            .find("data:")
            .map(|index| &line_without_newline[..index])
            .unwrap_or_default();
        rewritten.push_str(prefix);
        rewritten.push_str("data: ");
        rewritten.push_str(&event.to_string());
        if line.ends_with('\n') {
            rewritten.push('\n');
        }
    }
    rewritten.into_bytes()
}

fn rewrite_model_fields(event: &mut Value, public_model: &str) {
    if event.get("model").is_some() {
        event["model"] = Value::String(public_model.to_owned());
    }
    if let Some(response) = event.get_mut("response")
        && response.get("model").is_some()
    {
        response["model"] = Value::String(public_model.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_input_becomes_a_standard_responses_message() {
        let decoded = ResponsesProtocolAdapter::decode(
            br#"{"model":"gpt-5.6-sol","input":"hello","stream":false}"#,
        )
        .expect("decode request");
        assert_eq!(decoded.body["input"][0]["role"], "user");
        assert_eq!(decoded.body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(decoded.body["input"][0]["content"][0]["text"], "hello");
    }

    #[test]
    fn codex_upstream_uses_priority_for_the_fast_tier() {
        let canonical = json!({
            "model": "gpt-5.5",
            "input": "hello",
            "service_tier": "fast"
        });
        let encoded =
            CodexUpstreamAdapter::encode(&canonical, "gpt-5.5").expect("encode Codex request");
        assert_eq!(encoded["service_tier"], "priority");
        assert_eq!(canonical["service_tier"], "fast");
    }

    #[test]
    fn codex_adapter_omits_the_unsupported_output_token_limit() {
        let public = json!({
            "model":"public-model",
            "input":"hello",
            "max_output_tokens":128,
            "stream":false,
        });
        let upstream =
            CodexUpstreamAdapter::encode(&public, "upstream-model").expect("encode Codex request");
        assert!(upstream.get("max_output_tokens").is_none());
        assert_eq!(upstream["model"], "upstream-model");
        assert_eq!(upstream["stream"], true);
        assert_eq!(public["max_output_tokens"], 128);
    }

    #[test]
    fn reducer_rebuilds_output_and_includes_image_tool_usage() {
        let body = concat!(
            "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"hel\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"lo\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"output\":[],\"usage\":{\"input_tokens\":10,\"output_tokens\":2},\"tool_usage\":{\"image_gen\":{\"input_tokens\":3,\"output_tokens\":5}}}}\n\n",
        );
        let (response, usage) = reduce_sse(body.as_bytes()).expect("reduce response");
        assert_eq!(response["output"][0]["content"][0]["text"], "hello");
        assert_eq!(usage.uncached_input, 13);
        assert_eq!(usage.output_tokens, 7);
    }

    #[test]
    fn cache_write_tokens_are_not_counted_twice() {
        let response = json!({"usage": {"input_tokens": 300, "output_tokens": 20,
            "input_tokens_details": {"cached_tokens": 40, "cache_write_tokens": 10}}});
        let usage = usage_from_response(&response).unwrap();
        assert_eq!(
            (usage.uncached_input, usage.cached_input, usage.cache_write),
            (250, 40, 10)
        );
        let invalid = json!({"usage": {"input_tokens": 30, "output_tokens": 0,
            "input_tokens_details": {"cached_tokens": 20, "cache_write_tokens": 20}}});
        assert!(usage_from_response(&invalid).is_err());
    }

    #[test]
    fn streaming_events_expose_only_the_public_model() {
        let frame = b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"model\":\"upstream\"}}\n\n";
        let rewritten = rewrite_sse_frame_model(frame, "public");
        let text = String::from_utf8(rewritten).expect("utf8 frame");
        assert!(text.contains(r#""model":"public""#));
        assert!(!text.contains("upstream"));
    }
}
