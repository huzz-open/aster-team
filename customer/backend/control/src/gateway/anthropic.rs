use axum::{
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use crate::{
    ControlError, ModelUsage,
    gateway::{execution_options, protocol},
};

pub(crate) struct DecodedAnthropicRequest {
    pub(crate) model: String,
    pub(crate) canonical: Value,
    pub(crate) stream: bool,
}

pub(crate) struct AnthropicProtocolAdapter;

impl AnthropicProtocolAdapter {
    pub(crate) fn decode(body: &[u8]) -> Result<DecodedAnthropicRequest, ControlError> {
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
        let (input, mut instructions) = messages_to_responses(messages)?;
        if let Some(system) = source.get("system") {
            let system = protocol::text(system).ok_or(ControlError::GatewayRequestInvalid)?;
            if !system.is_empty() {
                instructions = if instructions.is_empty() {
                    system
                } else {
                    format!("{system}\n\n{instructions}")
                };
            }
        }
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
        execution_options::copy_anthropic_options(source, &mut canonical)?;
        Ok(DecodedAnthropicRequest {
            model,
            canonical,
            stream: source
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    pub(crate) fn usage(usage: &ModelUsage) -> Value {
        json!({
            "input_tokens":usage.uncached_input.saturating_add(usage.cached_input),
            "cache_creation_input_tokens":usage.cache_write,
            "cache_read_input_tokens":usage.cached_input,
            "output_tokens":usage.output_tokens,
        })
    }

    pub(crate) fn sse_response(message: &Value) -> Response {
        let mut events = vec![json!({
            "type":"message_start",
            "message":{
                "id":message["id"],"type":"message","role":"assistant","model":message["model"],
                "content":[],"stop_reason":null,"stop_sequence":null,
                "usage":{"input_tokens":message["usage"]["input_tokens"],"cache_creation_input_tokens":message["usage"]["cache_creation_input_tokens"],"cache_read_input_tokens":message["usage"]["cache_read_input_tokens"],"output_tokens":0},
            }
        })];
        if let Some(content) = message.get("content").and_then(Value::as_array) {
            for (index, block) in content.iter().enumerate() {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        events.push(json!({"type":"content_block_start","index":index,"content_block":{"type":"text","text":""}}));
                        events.push(json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":block["text"]}}));
                    }
                    Some("tool_use") => {
                        events.push(json!({"type":"content_block_start","index":index,"content_block":{"type":"tool_use","id":block["id"],"name":block["name"],"input":{}}}));
                        events.push(json!({"type":"content_block_delta","index":index,"delta":{"type":"input_json_delta","partial_json":block["input"].to_string()}}));
                    }
                    _ => continue,
                }
                events.push(json!({"type":"content_block_stop","index":index}));
            }
        }
        events.push(json!({
            "type":"message_delta",
            "delta":{"stop_reason":message["stop_reason"],"stop_sequence":null},
            "usage":{"output_tokens":message["usage"]["output_tokens"]},
        }));
        events.push(json!({"type":"message_stop"}));
        let mut body = String::new();
        for event in events {
            let kind = event
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("message");
            body.push_str("event: ");
            body.push_str(kind);
            body.push_str("\ndata: ");
            body.push_str(&event.to_string());
            body.push_str("\n\n");
        }
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
        let content = message
            .get("content")
            .ok_or(ControlError::GatewayRequestInvalid)?;
        if role == "system" {
            let text = protocol::text(content).ok_or(ControlError::GatewayRequestInvalid)?;
            if !text.is_empty() {
                instructions.push(text);
            }
            continue;
        }
        if !matches!(role.as_str(), "user" | "assistant") {
            return Err(ControlError::GatewayRequestInvalid);
        }
        if let Some(text) = content.as_str() {
            input.push(json!({
                "role":role,
                "content":[{"type":if role == "assistant" { "output_text" } else { "input_text" },"text":text}],
            }));
            continue;
        }
        let blocks = content
            .as_array()
            .ok_or(ControlError::GatewayRequestInvalid)?;
        let mut message_content = Vec::new();
        for block in blocks {
            let block = block
                .as_object()
                .ok_or(ControlError::GatewayRequestInvalid)?;
            match block.get("type").and_then(Value::as_str) {
                Some("text") => message_content.push(json!({
                    "type":if role == "assistant" { "output_text" } else { "input_text" },
                    "text":protocol::required_string(block,"text",8 * 1024 * 1024)?,
                })),
                Some("tool_use") if role == "assistant" => {
                    if !message_content.is_empty() {
                        input.push(
                            json!({"role":role,"content":std::mem::take(&mut message_content)}),
                        );
                    }
                    input.push(json!({
                        "type":"function_call",
                        "call_id":protocol::required_string(block,"id",256)?,
                        "name":protocol::required_string(block,"name",256)?,
                        "arguments":serde_json::to_string(block.get("input").unwrap_or(&json!({}))).map_err(|_| ControlError::GatewayRequestInvalid)?,
                    }));
                }
                Some("tool_result") if role == "user" => {
                    if !message_content.is_empty() {
                        input.push(
                            json!({"role":role,"content":std::mem::take(&mut message_content)}),
                        );
                    }
                    let output = block
                        .get("content")
                        .and_then(protocol::text)
                        .ok_or(ControlError::GatewayRequestInvalid)?;
                    input.push(json!({
                        "type":"function_call_output",
                        "call_id":protocol::required_string(block,"tool_use_id",256)?,
                        "output":output,
                    }));
                }
                Some("image") if role == "user" => {
                    let source = block
                        .get("source")
                        .and_then(Value::as_object)
                        .ok_or(ControlError::GatewayRequestInvalid)?;
                    let image_url = match source.get("type").and_then(Value::as_str) {
                        Some("base64") => format!(
                            "data:{};base64,{}",
                            protocol::required_string(source, "media_type", 128)?,
                            protocol::required_string(source, "data", 12 * 1024 * 1024)?,
                        ),
                        Some("url") => protocol::required_string(source, "url", 12 * 1024 * 1024)?,
                        _ => return Err(ControlError::GatewayRequestInvalid),
                    };
                    message_content
                        .push(json!({"type":"input_image","image_url":image_url,"detail":"auto"}));
                }
                _ => return Err(ControlError::GatewayRequestInvalid),
            }
        }
        if !message_content.is_empty() {
            input.push(json!({"role":role,"content":message_content}));
        }
    }
    if input.is_empty() {
        return Err(ControlError::GatewayRequestInvalid);
    }
    Ok((input, instructions.join("\n\n")))
}

fn tools_to_responses(value: &Value) -> Result<Value, ControlError> {
    let tools = value
        .as_array()
        .filter(|tools| tools.len() <= 128)
        .ok_or(ControlError::GatewayRequestInvalid)?;
    tools
        .iter()
        .map(|tool| {
            let tool = tool
                .as_object()
                .ok_or(ControlError::GatewayRequestInvalid)?;
            Ok(json!({
                "type":"function",
                "name":protocol::required_string(tool,"name",256)?,
                "description":tool.get("description").and_then(Value::as_str).unwrap_or(""),
                "parameters":tool.get("input_schema").cloned().ok_or(ControlError::GatewayRequestInvalid)?,
                "strict":false,
            }))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_tools_use_canonical_function_items() {
        let decoded = AnthropicProtocolAdapter::decode(
            br#"{"model":"gpt","max_tokens":100,"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"lookup","input":{"id":1}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"done"}]}]}"#,
        )
        .expect("decode anthropic");
        assert_eq!(decoded.canonical["input"][0]["type"], "function_call");
        assert_eq!(
            decoded.canonical["input"][1]["type"],
            "function_call_output"
        );
    }

    #[test]
    fn anthropic_output_token_limit_is_accepted_but_not_forwarded() {
        let decoded = AnthropicProtocolAdapter::decode(
            br#"{"model":"gpt","max_tokens":128,"messages":[{"role":"user","content":"hello"}]}"#,
        )
        .expect("decode anthropic request with an unsupported output limit");
        assert!(decoded.canonical.get("max_output_tokens").is_none());
        assert!(decoded.canonical.get("max_tokens").is_none());
    }

    #[test]
    fn claude_cli_shape_uses_the_mapped_aster_model() {
        let decoded = AnthropicProtocolAdapter::decode(
            br#"{
                "model":"gpt-5.4",
                "max_tokens":32000,
                "system":[{"type":"text","text":"You are Claude Code.","cache_control":{"type":"ephemeral"}}],
                "messages":[{"role":"user","content":[{"type":"text","text":"Inspect this repository.","cache_control":{"type":"ephemeral"}}]}],
                "thinking":{"type":"adaptive"},
                "metadata":{"user_id":"claude-code"},
                "tools":[{"name":"Read","description":"Read a file","input_schema":{"type":"object","properties":{"path":{"type":"string"}}},"strict":true}]
            }"#,
        )
        .expect("decode a Claude CLI gateway request");

        assert_eq!(decoded.model, "gpt-5.4");
        assert_eq!(decoded.canonical["model"], "gpt-5.4");
        assert_eq!(decoded.canonical["instructions"], "You are Claude Code.");
        assert_eq!(decoded.canonical["tools"][0]["name"], "Read");
    }

    #[test]
    fn claude_cli_system_messages_become_response_instructions() {
        let decoded = AnthropicProtocolAdapter::decode(
            br#"{
                "model":"gpt-5.6-sol",
                "max_tokens":32000,
                "system":[{"type":"text","text":"You are Claude Code.","cache_control":{"type":"ephemeral"}}],
                "messages":[
                    {"role":"user","content":[{"type":"text","text":"Who are you?"}]},
                    {"role":"system","content":[{"type":"text","text":"Additional runtime context."}]}
                ],
                "thinking":{"type":"adaptive","display":"omitted"},
                "context_management":{"edits":[]},
                "output_config":{"effort":"high"}
            }"#,
        )
        .expect("decode a Claude Code request containing a system message");

        assert_eq!(decoded.canonical["input"].as_array().unwrap().len(), 1);
        assert_eq!(
            decoded.canonical["instructions"],
            "You are Claude Code.\n\nAdditional runtime context."
        );
        assert_eq!(decoded.canonical["reasoning"]["effort"], "high");
    }

    #[test]
    fn anthropic_execution_options_use_native_request_fields() {
        let decoded = AnthropicProtocolAdapter::decode(
            br#"{"model":"gpt","max_tokens":100,"speed":"fast","output_config":{"effort":"xhigh"},"messages":[{"role":"user","content":"hello"}]}"#,
        )
        .expect("decode Anthropic execution options");
        assert_eq!(decoded.canonical["service_tier"], "fast");
        assert_eq!(decoded.canonical["reasoning"]["effort"], "xhigh");
    }
}
