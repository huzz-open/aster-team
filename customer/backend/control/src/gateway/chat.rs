use axum::{
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use crate::{
    ControlError, ModelUsage,
    gateway::{execution_options, protocol},
};

pub(crate) struct DecodedChatRequest {
    pub(crate) model: String,
    pub(crate) canonical: Value,
    pub(crate) stream: bool,
}

pub(crate) struct ChatProtocolAdapter;

impl ChatProtocolAdapter {
    pub(crate) fn decode(body: &[u8]) -> Result<DecodedChatRequest, ControlError> {
        let source = protocol::decode_json(body)?;
        let source = source
            .as_object()
            .ok_or(ControlError::GatewayRequestInvalid)?;
        let model = protocol::required_string(source, "model", 160)?;
        let messages = source
            .get("messages")
            .and_then(Value::as_array)
            .filter(|items| !items.is_empty())
            .ok_or(ControlError::GatewayRequestInvalid)?;
        let (input, instructions) = messages_to_responses(messages)?;
        let mut canonical = json!({
            "model": model,
            "instructions": instructions,
            "input": input,
            "stream": true,
            "store": false,
        });
        if let Some(tools) = source.get("tools") {
            canonical["tools"] = tools_to_responses(tools)?;
        }
        if let Some(tool_choice) = source.get("tool_choice") {
            canonical["tool_choice"] = tool_choice_to_responses(tool_choice)?;
        }
        execution_options::copy_chat_options(source, &mut canonical)?;
        Ok(DecodedChatRequest {
            model,
            canonical,
            stream: source
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    pub(crate) fn usage(usage: &ModelUsage) -> Value {
        let prompt_tokens = usage
            .uncached_input
            .saturating_add(usage.cached_input)
            .saturating_add(usage.cache_write);
        json!({
            "prompt_tokens":prompt_tokens,
            "completion_tokens":usage.output_tokens,
            "total_tokens":prompt_tokens.saturating_add(usage.output_tokens),
            "prompt_tokens_details":{"cached_tokens":usage.cached_input},
            "completion_tokens_details":{},
        })
    }

    pub(crate) fn sse_response(chunks: Vec<Value>) -> Response {
        let mut body = String::new();
        for chunk in chunks {
            body.push_str("data: ");
            body.push_str(&chunk.to_string());
            body.push_str("\n\n");
        }
        body.push_str("data: [DONE]\n\n");
        let mut response = (
            StatusCode::OK,
            [("content-type", "text/event-stream; charset=utf-8")],
            body,
        )
            .into_response();
        response.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        );
        response
    }
}

fn messages_to_responses(messages: &[Value]) -> Result<(Vec<Value>, String), ControlError> {
    let mut input = Vec::new();
    let mut instructions = Vec::new();
    for message in messages {
        let message = message
            .as_object()
            .ok_or(ControlError::GatewayRequestInvalid)?;
        let role = protocol::required_string(message, "role", 32)?;
        match role.as_str() {
            "system" | "developer" => {
                let content = message
                    .get("content")
                    .and_then(protocol::text)
                    .ok_or(ControlError::GatewayRequestInvalid)?;
                if !content.is_empty() {
                    instructions.push(content);
                }
            }
            "tool" => {
                let call_id = protocol::required_string(message, "tool_call_id", 256)?;
                let output = message
                    .get("content")
                    .and_then(protocol::text)
                    .ok_or(ControlError::GatewayRequestInvalid)?;
                input.push(json!({
                    "type":"function_call_output",
                    "call_id":call_id,
                    "output":output,
                }));
            }
            "user" | "assistant" => {
                if let Some(content) = message.get("content")
                    && !content.is_null()
                {
                    let content = content_to_responses(content, &role)?;
                    if !content.is_empty() {
                        input.push(json!({"role":role,"content":content}));
                    }
                }
                if role == "assistant"
                    && let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array)
                {
                    for tool_call in tool_calls {
                        let tool_call = tool_call
                            .as_object()
                            .ok_or(ControlError::GatewayRequestInvalid)?;
                        let call_id = protocol::required_string(tool_call, "id", 256)?;
                        let function = tool_call
                            .get("function")
                            .and_then(Value::as_object)
                            .ok_or(ControlError::GatewayRequestInvalid)?;
                        input.push(json!({
                            "type":"function_call",
                            "call_id":call_id,
                            "name":protocol::required_string(function,"name",256)?,
                            "arguments":protocol::required_string(function,"arguments",1024 * 1024)?,
                        }));
                    }
                }
            }
            _ => return Err(ControlError::GatewayRequestInvalid),
        }
    }
    if input.is_empty() {
        return Err(ControlError::GatewayRequestInvalid);
    }
    Ok((input, instructions.join("\n\n")))
}

fn content_to_responses(content: &Value, role: &str) -> Result<Vec<Value>, ControlError> {
    if let Some(text) = content.as_str() {
        return Ok(vec![json!({
            "type": if role == "assistant" { "output_text" } else { "input_text" },
            "text":text,
        })]);
    }
    let items = content
        .as_array()
        .ok_or(ControlError::GatewayRequestInvalid)?;
    let mut converted = Vec::new();
    for item in items {
        let item = item
            .as_object()
            .ok_or(ControlError::GatewayRequestInvalid)?;
        match item.get("type").and_then(Value::as_str).unwrap_or("text") {
            "text" | "input_text" | "output_text" => {
                let text = protocol::required_string(item, "text", 8 * 1024 * 1024)?;
                converted.push(json!({
                    "type":if role == "assistant" { "output_text" } else { "input_text" },
                    "text":text,
                }));
            }
            "image_url" if role == "user" => {
                let image = item
                    .get("image_url")
                    .ok_or(ControlError::GatewayRequestInvalid)?;
                let url = image
                    .as_str()
                    .or_else(|| image.get("url").and_then(Value::as_str))
                    .filter(|value| !value.is_empty() && value.len() <= 12 * 1024 * 1024)
                    .ok_or(ControlError::GatewayRequestInvalid)?;
                let detail = image
                    .get("detail")
                    .and_then(Value::as_str)
                    .unwrap_or("auto");
                converted.push(json!({"type":"input_image","image_url":url,"detail":detail}));
            }
            _ => return Err(ControlError::GatewayRequestInvalid),
        }
    }
    Ok(converted)
}

fn tools_to_responses(value: &Value) -> Result<Value, ControlError> {
    let tools = value
        .as_array()
        .filter(|tools| tools.len() <= 128)
        .ok_or(ControlError::GatewayRequestInvalid)?;
    let mut converted = Vec::new();
    for tool in tools {
        let tool = tool
            .as_object()
            .ok_or(ControlError::GatewayRequestInvalid)?;
        if tool.get("type").and_then(Value::as_str) != Some("function") {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let function = tool
            .get("function")
            .and_then(Value::as_object)
            .ok_or(ControlError::GatewayRequestInvalid)?;
        converted.push(json!({
            "type":"function",
            "name":protocol::required_string(function,"name",256)?,
            "description":function.get("description").and_then(Value::as_str).unwrap_or(""),
            "parameters":function.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}})),
            "strict":function.get("strict").and_then(Value::as_bool).unwrap_or(false),
        }));
    }
    Ok(Value::Array(converted))
}

fn tool_choice_to_responses(value: &Value) -> Result<Value, ControlError> {
    if value
        .as_str()
        .is_some_and(|value| matches!(value, "auto" | "none" | "required"))
    {
        return Ok(value.clone());
    }
    let value = value
        .as_object()
        .ok_or(ControlError::GatewayRequestInvalid)?;
    let function = value
        .get("function")
        .and_then(Value::as_object)
        .ok_or(ControlError::GatewayRequestInvalid)?;
    Ok(json!({
        "type":"function",
        "name":protocol::required_string(function,"name",256)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_tool_round_trip_uses_canonical_function_items() {
        let decoded = ChatProtocolAdapter::decode(
            br#"{"model":"gpt","messages":[{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{\"id\":1}"}}]},{"role":"tool","tool_call_id":"call_1","content":"done"}]}"#,
        )
        .expect("decode chat");
        assert_eq!(decoded.canonical["input"][0]["type"], "function_call");
        assert_eq!(
            decoded.canonical["input"][1]["type"],
            "function_call_output"
        );
    }

    #[test]
    fn chat_output_token_limits_are_accepted_but_not_forwarded() {
        let decoded = ChatProtocolAdapter::decode(
            br#"{"model":"gpt","max_completion_tokens":128,"max_tokens":64,"messages":[{"role":"user","content":"hello"}]}"#,
        )
        .expect("decode chat with an unsupported output limit");
        assert!(decoded.canonical.get("max_output_tokens").is_none());
        assert!(decoded.canonical.get("max_completion_tokens").is_none());
        assert!(decoded.canonical.get("max_tokens").is_none());
    }

    #[test]
    fn chat_execution_options_use_the_canonical_responses_shape() {
        let decoded = ChatProtocolAdapter::decode(
            br#"{"model":"gpt","service_tier":"fast","reasoning_effort":"low","messages":[{"role":"user","content":"hello"}]}"#,
        )
        .expect("decode chat execution options");
        assert_eq!(decoded.canonical["service_tier"], "fast");
        assert_eq!(decoded.canonical["reasoning"]["effort"], "low");
    }
}
