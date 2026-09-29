use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};
use url::Url;

use crate::{ControlError, ModelUsage, gateway::capabilities};

pub(crate) const MAX_IMAGE_EDIT_BODY_BYTES: usize = 18 * 1024 * 1024;
pub(crate) const MAX_IMAGE_EDIT_SOURCE_BYTES: usize = 12 * 1024 * 1024;
pub(crate) const MAX_IMAGE_FILE_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const MAX_IMAGE_EDIT_FILES: usize = 16;

const BASE_QUALITIES: [&str; 4] = ["auto", "low", "medium", "high"];
const EXTENDED_QUALITIES: [&str; 2] = ["xhigh", "max"];
const BACKGROUNDS: [&str; 3] = ["auto", "opaque", "transparent"];
const MODERATION_LEVELS: [&str; 2] = ["auto", "low"];
const OUTPUT_FORMATS: [&str; 3] = ["png", "jpeg", "webp"];
const INPUT_FIDELITIES: [&str; 2] = ["high", "low"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImageControls {
    pub(crate) model: String,
    pub(crate) prompt: String,
    pub(crate) size: String,
    pub(crate) quality: String,
    pub(crate) background: String,
    pub(crate) moderation: String,
    pub(crate) output_format: String,
    pub(crate) output_compression: Option<i64>,
    pub(crate) input_fidelity: String,
    pub(crate) n: usize,
}

pub(crate) struct ImagesProtocolAdapter;

impl ImagesProtocolAdapter {
    pub(crate) fn decode_generation(body: &[u8]) -> Result<ImageControls, ControlError> {
        const MAX_IMAGE_JSON_BYTES: usize = 18 * 1024 * 1024;
        if body.is_empty() || body.len() > MAX_IMAGE_JSON_BYTES {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let body: Value =
            serde_json::from_slice(body).map_err(|_| ControlError::GatewayRequestInvalid)?;
        Self::decode_fields(
            body.as_object()
                .ok_or(ControlError::GatewayRequestInvalid)?,
            false,
        )
    }

    pub(crate) fn decode_fields(
        body: &Map<String, Value>,
        edit: bool,
    ) -> Result<ImageControls, ControlError> {
        let model = optional_string(body, "model").unwrap_or(capabilities::DEFAULT_IMAGE_MODEL);
        let prompt = optional_string(body, "prompt")
            .filter(|value| !value.is_empty() && value.chars().count() <= 32_000)
            .ok_or(ControlError::GatewayRequestInvalid)?;
        if !capabilities::is_image_model(model) {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let size = optional_string(body, "size").unwrap_or("auto");
        let quality = optional_string(body, "quality").unwrap_or("auto");
        let background = optional_string(body, "background").unwrap_or("auto");
        let moderation = optional_string(body, "moderation").unwrap_or("auto");
        let output_format = optional_string(body, "output_format").unwrap_or("png");
        let input_fidelity = optional_string(body, "input_fidelity").unwrap_or_default();
        let response_format = optional_string(body, "response_format").unwrap_or("b64_json");
        let n = optional_integer(body, "n")?.unwrap_or(1);
        let output_compression = optional_integer(body, "output_compression")?;

        if !supported_size(model, size)
            || !supported_quality(model, quality)
            || !BACKGROUNDS.contains(&background)
            || (background == "transparent"
                && (!capabilities::supports_transparent_image_background(model)
                    || output_format == "jpeg"))
            || !MODERATION_LEVELS.contains(&moderation)
            || !OUTPUT_FORMATS.contains(&output_format)
            || response_format != "b64_json"
            || !(1..=10).contains(&n)
            || output_compression.is_some_and(|value| !(0..=100).contains(&value))
            || (output_compression.is_some() && output_format == "png")
            || (!edit && !input_fidelity.is_empty())
            || (!input_fidelity.is_empty()
                && (!INPUT_FIDELITIES.contains(&input_fidelity)
                    || !capabilities::supports_configurable_image_fidelity(model)))
        {
            return Err(ControlError::GatewayRequestInvalid);
        }
        Ok(ImageControls {
            model: model.to_owned(),
            prompt: prompt.to_owned(),
            size: size.to_owned(),
            quality: quality.to_owned(),
            background: background.to_owned(),
            moderation: moderation.to_owned(),
            output_format: output_format.to_owned(),
            output_compression,
            input_fidelity: input_fidelity.to_owned(),
            n: usize::try_from(n).map_err(|_| ControlError::GatewayRequestInvalid)?,
        })
    }

    pub(crate) fn decode_json_edit(
        body: &[u8],
    ) -> Result<(ImageControls, Vec<String>), ControlError> {
        if body.is_empty() || body.len() > MAX_IMAGE_EDIT_BODY_BYTES {
            return Err(ControlError::GatewayRequestInvalid);
        }
        let mut body: Map<String, Value> =
            serde_json::from_slice(body).map_err(|_| ControlError::GatewayRequestInvalid)?;
        let images = body
            .remove("images")
            .and_then(|value| value.as_array().cloned())
            .filter(|images| !images.is_empty() && images.len() <= MAX_IMAGE_EDIT_FILES)
            .ok_or(ControlError::GatewayRequestInvalid)?;
        let mut sources = Vec::with_capacity(images.len());
        let mut source_bytes = 0_usize;
        for image in images {
            let source = image
                .as_object()
                .and_then(|image| image.get("image_url"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|source| !source.is_empty())
                .ok_or(ControlError::GatewayRequestInvalid)?;
            if let Some(bytes) = validated_json_image_source_bytes(source)? {
                source_bytes = source_bytes
                    .checked_add(bytes)
                    .filter(|total| *total <= MAX_IMAGE_EDIT_SOURCE_BYTES)
                    .ok_or(ControlError::GatewayRequestInvalid)?;
            }
            sources.push(source.to_owned());
        }
        let controls = Self::decode_fields(&body, true)?;
        Ok((controls, sources))
    }

    pub(crate) fn canonical_request(
        controls: &ImageControls,
        host_model: &str,
        sources: &[String],
        mask: Option<&str>,
    ) -> Value {
        let action = if sources.is_empty() {
            "generate"
        } else {
            "edit"
        };
        let mut tool = json!({
            "type": "image_generation",
            "action": action,
            "model": controls.model,
            "output_format": controls.output_format,
        });
        insert_non_auto(&mut tool, "size", &controls.size);
        insert_non_auto(&mut tool, "quality", &controls.quality);
        insert_non_auto(&mut tool, "background", &controls.background);
        insert_non_auto(&mut tool, "moderation", &controls.moderation);
        if let Some(value) = controls.output_compression {
            tool["output_compression"] = json!(value);
        }
        if !controls.input_fidelity.is_empty() {
            tool["input_fidelity"] = Value::String(controls.input_fidelity.clone());
        }
        if let Some(mask) = mask {
            tool["input_image_mask"] = json!({"image_url": mask});
        }
        let mut content = vec![json!({"type": "input_text", "text": controls.prompt})];
        content.extend(
            sources.iter().map(
                |source| json!({"type": "input_image", "image_url": source, "detail": "auto"}),
            ),
        );
        json!({
            "model": host_model,
            "instructions": "",
            "input": [{"role": "user", "content": content}],
            "tools": [tool],
            "stream": true,
            "store": false,
        })
    }

    pub(crate) fn generated_image(response: &Value) -> Result<Value, ControlError> {
        let item = response
            .get("output")
            .and_then(Value::as_array)
            .and_then(|output| {
                output.iter().find(|item| {
                    item.get("type").and_then(Value::as_str) == Some("image_generation_call")
                        && item.get("result").and_then(Value::as_str).is_some()
                })
            })
            .ok_or(ControlError::InvalidUpstreamResponse)?;
        Ok(json!({
            "b64_json": item["result"],
            "revised_prompt": item.get("revised_prompt").cloned().unwrap_or(Value::Null),
        }))
    }
}

fn validated_json_image_source_bytes(source: &str) -> Result<Option<usize>, ControlError> {
    if let Some((metadata, encoded)) = source.split_once(',')
        && matches!(
            metadata.to_ascii_lowercase().as_str(),
            "data:image/png;base64" | "data:image/jpeg;base64" | "data:image/webp;base64"
        )
    {
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| ControlError::GatewayRequestInvalid)?;
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_FILE_BYTES {
            return Err(ControlError::GatewayRequestInvalid);
        }
        return Ok(Some(bytes.len()));
    }
    let url = Url::parse(source).map_err(|_| ControlError::GatewayRequestInvalid)?;
    if !matches!(url.scheme(), "http" | "https") || source.len() > 8_192 {
        return Err(ControlError::GatewayRequestInvalid);
    }
    Ok(None)
}

fn optional_string<'a>(body: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn optional_integer(body: &Map<String, Value>, key: &str) -> Result<Option<i64>, ControlError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_i64()
            .map(Some)
            .ok_or(ControlError::GatewayRequestInvalid),
        Some(Value::String(value)) if !value.trim().is_empty() => value
            .trim()
            .parse::<i64>()
            .map(Some)
            .map_err(|_| ControlError::GatewayRequestInvalid),
        Some(Value::String(_)) => Ok(None),
        _ => Err(ControlError::GatewayRequestInvalid),
    }
}

fn supported_size(model: &str, size: &str) -> bool {
    if !capabilities::supports_dynamic_image_size(model) {
        return matches!(size, "auto" | "1024x1024" | "1536x1024" | "1024x1536");
    }
    if size == "auto" {
        return true;
    }
    let Some((width, height)) = size.split_once('x') else {
        return false;
    };
    let (Ok(width), Ok(height)) = (width.parse::<u64>(), height.parse::<u64>()) else {
        return false;
    };
    let long_edge = width.max(height);
    let short_edge = width.min(height);
    let Some(pixels) = width.checked_mul(height) else {
        return false;
    };
    width > 0
        && height > 0
        && width % 16 == 0
        && height % 16 == 0
        && long_edge <= 3840
        && short_edge > 0
        && long_edge <= short_edge.saturating_mul(3)
        && (655_360..=8_294_400).contains(&pixels)
}

fn supported_quality(model: &str, quality: &str) -> bool {
    BASE_QUALITIES.contains(&quality)
        || (capabilities::supports_extended_image_quality(model)
            && EXTENDED_QUALITIES.contains(&quality))
}

fn insert_non_auto(target: &mut Value, key: &str, value: &str) {
    if value != "auto" && !value.is_empty() {
        target[key] = Value::String(value.to_owned());
    }
}

pub(crate) fn openai_image_response(
    created: i64,
    controls: &ImageControls,
    data: Vec<Value>,
    usage: &ModelUsage,
) -> Value {
    let input = usage
        .uncached_input
        .saturating_add(usage.cached_input)
        .saturating_add(usage.cache_write);
    json!({
        "created": created,
        "background": controls.background,
        "data": data,
        "output_format": controls.output_format,
        "quality": controls.quality,
        "size": controls.size,
        "usage": {
            "input_tokens": input,
            "output_tokens": usage.output_tokens,
            "total_tokens": input.saturating_add(usage.output_tokens),
        },
    })
}

pub(crate) fn documentation(english: bool) -> Value {
    let parameters = if english {
        vec![
            parameter(
                "model",
                "string",
                false,
                "gpt-image-2.5-flare, gpt-image-2.5-sunburst, gpt-image-2, gpt-image-1",
                "gpt-image-2.5-flare",
                "Optional. GPT Image 2.5 Flare is the default; use Sunburst for precise editing.",
            ),
            parameter(
                "prompt",
                "string",
                true,
                "1–32,000 characters",
                "—",
                "Describe the desired subject, scene, style, composition, lighting, color, and text.",
            ),
            parameter(
                "size",
                "string",
                false,
                "auto or WIDTHxHEIGHT",
                "auto",
                "GPT Image 2.5 and GPT Image 2 support dynamic sizes within the documented constraints.",
            ),
            parameter(
                "quality",
                "string",
                false,
                "auto, low, medium, high, xhigh, max (xhigh/max require GPT Image 2.5)",
                "auto",
                "Higher quality generally takes longer.",
            ),
            parameter(
                "background",
                "string",
                false,
                "auto, opaque, transparent",
                "auto",
                "Availability depends on model capabilities.",
            ),
            parameter(
                "moderation",
                "string",
                false,
                "auto, low",
                "auto",
                "Content moderation strictness.",
            ),
            parameter(
                "output_format",
                "string",
                false,
                "png, jpeg, webp",
                "png",
                "The image format after Base64 decoding.",
            ),
            parameter(
                "output_compression",
                "integer",
                false,
                "0–100 (jpeg/webp only)",
                "100",
                "JPEG/WebP output quality.",
            ),
            parameter(
                "response_format",
                "string",
                false,
                "b64_json",
                "b64_json",
                "Images are returned in data[].b64_json.",
            ),
            parameter(
                "n",
                "integer",
                false,
                "1–10",
                "1",
                "Each returned image is metered separately.",
            ),
        ]
    } else {
        vec![
            parameter(
                "model",
                "string",
                false,
                "gpt-image-2.5-flare、gpt-image-2.5-sunburst、gpt-image-2、gpt-image-1",
                "gpt-image-2.5-flare",
                "可省略；默认使用 GPT Image 2.5 Flare，精确编辑可使用 Sunburst。",
            ),
            parameter(
                "prompt",
                "string",
                true,
                "1～32,000 个字符",
                "—",
                "描述主体、场景、风格、构图、光线、色彩和文字。",
            ),
            parameter(
                "size",
                "string",
                false,
                "auto 或 WIDTHxHEIGHT",
                "auto",
                "GPT Image 2.5 和 GPT Image 2 支持满足约束的动态尺寸。",
            ),
            parameter(
                "quality",
                "string",
                false,
                "auto、low、medium、high、xhigh、max（xhigh/max 仅 GPT Image 2.5）",
                "auto",
                "质量越高通常耗时越长。",
            ),
            parameter(
                "background",
                "string",
                false,
                "auto、opaque、transparent",
                "auto",
                "是否可用由模型能力决定。",
            ),
            parameter(
                "moderation",
                "string",
                false,
                "auto、low",
                "auto",
                "内容审核严格程度。",
            ),
            parameter(
                "output_format",
                "string",
                false,
                "png、jpeg、webp",
                "png",
                "Base64 解码后的真实图片格式。",
            ),
            parameter(
                "output_compression",
                "integer",
                false,
                "0～100（仅 jpeg/webp）",
                "100",
                "JPEG/WebP 输出质量。",
            ),
            parameter(
                "response_format",
                "string",
                false,
                "b64_json",
                "b64_json",
                "图片通过 data[].b64_json 返回。",
            ),
            parameter(
                "n",
                "integer",
                false,
                "1～10",
                "1",
                "每张返回图片分别计量。",
            ),
        ]
    };
    let mut edit_parameters = if english {
        vec![
            parameter(
                "image / image[]",
                "file",
                true,
                "1–16 PNG/JPEG/WebP files",
                "—",
                "Source images submitted as multipart/form-data.",
            ),
            parameter(
                "mask",
                "file",
                false,
                "Alpha-channel PNG, up to 4 MiB",
                "—",
                "Transparent areas identify what should be edited.",
            ),
            parameter(
                "input_fidelity",
                "string",
                false,
                "high, low (gpt-image-1)",
                "—",
                "Only GPT Image 1 allows this setting; newer models manage input fidelity automatically.",
            ),
        ]
    } else {
        vec![
            parameter(
                "image / image[]",
                "file",
                true,
                "1～16 个 PNG/JPEG/WebP",
                "—",
                "通过 multipart/form-data 提交原图。",
            ),
            parameter(
                "mask",
                "file",
                false,
                "带 alpha 通道的 PNG，最大 4 MiB",
                "—",
                "透明区域表示需要编辑的位置。",
            ),
            parameter(
                "input_fidelity",
                "string",
                false,
                "high、low（gpt-image-1）",
                "—",
                "仅 GPT Image 1 允许设置；较新的模型会自动管理输入保真度。",
            ),
        ]
    };
    edit_parameters.extend(parameters.clone());
    let constraints = if english {
        vec![
            "Width and height must be multiples of 16px",
            "The longest edge cannot exceed 3840px",
            "The aspect ratio cannot exceed 3:1",
            "Total pixels must be between 655,360 and 8,294,400",
        ]
    } else {
        vec![
            "宽和高都必须是 16px 的倍数",
            "最长边不得超过 3840px",
            "长边与短边的比例不得超过 3:1",
            "总像素不少于 655,360，不超过 8,294,400",
        ]
    };
    json!({
        "image_parameters": parameters,
        "image_edit_parameters": edit_parameters,
        "image_size": {
            "syntax": if english { "auto or WIDTHxHEIGHT" } else { "auto 或 WIDTHxHEIGHT" },
            "popular_examples": ["1024x1024", "1536x1024", "1024x1536", "2048x2048", "2048x1152", "3840x2160", "2160x3840"],
            "constraints": constraints,
        },
        "image_output": {
            "response_field": "data[0].b64_json",
            "response_encoding": "base64",
            "formats": [
                {"format":"png","media_type":"image/png","suggested_filename":"aster-image.png"},
                {"format":"jpeg","media_type":"image/jpeg","suggested_filename":"aster-image.jpg"},
                {"format":"webp","media_type":"image/webp","suggested_filename":"aster-image.webp"},
            ],
        },
    })
}

fn parameter(
    name: &str,
    kind: &str,
    required: bool,
    values: &str,
    default_value: &str,
    description: &str,
) -> Value {
    json!({
        "name": name,
        "type": kind,
        "required": required,
        "values": values,
        "default_value": default_value,
        "description": description,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_controls_validate_model_specific_constraints() {
        let flare = json!({
            "prompt":"cat",
            "size":"2048x1152",
            "quality":"max",
            "background":"transparent",
            "output_format":"webp"
        });
        assert!(ImagesProtocolAdapter::decode_fields(flare.as_object().unwrap(), false).is_ok());
        let sunburst = json!({
            "model":"gpt-image-2.5-sunburst",
            "prompt":"cat",
            "size":"2048x1152",
            "quality":"xhigh",
            "background":"transparent"
        });
        assert!(ImagesProtocolAdapter::decode_fields(sunburst.as_object().unwrap(), false).is_ok());
        let image_two = json!({"model":"gpt-image-2","prompt":"cat","size":"2048x1152"});
        assert!(
            ImagesProtocolAdapter::decode_fields(image_two.as_object().unwrap(), false).is_ok()
        );
        let legacy = json!({"model":"gpt-image-1","prompt":"cat","size":"2048x1152"});
        assert!(ImagesProtocolAdapter::decode_fields(legacy.as_object().unwrap(), false).is_err());
        let unsupported_quality = json!({"model":"gpt-image-2","prompt":"cat","quality":"xhigh"});
        assert!(
            ImagesProtocolAdapter::decode_fields(unsupported_quality.as_object().unwrap(), false)
                .is_err()
        );
        let unsupported_transparency =
            json!({"model":"gpt-image-2","prompt":"cat","background":"transparent"});
        assert!(
            ImagesProtocolAdapter::decode_fields(
                unsupported_transparency.as_object().unwrap(),
                false
            )
            .is_err()
        );
        let transparent_jpeg = json!({
            "prompt":"cat",
            "background":"transparent",
            "output_format":"jpeg"
        });
        assert!(
            ImagesProtocolAdapter::decode_fields(transparent_jpeg.as_object().unwrap(), false)
                .is_err()
        );
        let configurable_fidelity =
            json!({"model":"gpt-image-1","prompt":"cat","input_fidelity":"low"});
        assert!(
            ImagesProtocolAdapter::decode_fields(configurable_fidelity.as_object().unwrap(), true)
                .is_ok()
        );
        let automatic_fidelity =
            json!({"model":"gpt-image-2.5-flare","prompt":"cat","input_fidelity":"high"});
        assert!(
            ImagesProtocolAdapter::decode_fields(automatic_fidelity.as_object().unwrap(), true)
                .is_err()
        );
    }

    #[test]
    fn image_request_uses_the_image_tool_without_leaking_it_into_other_protocols() {
        let controls = ImagesProtocolAdapter::decode_generation(br#"{"prompt":"cat"}"#)
            .expect("decode image request");
        let request = ImagesProtocolAdapter::canonical_request(&controls, "gpt-5.6-sol", &[], None);
        assert_eq!(request["model"], "gpt-5.6-sol");
        assert_eq!(request["tools"][0]["type"], "image_generation");
        assert_eq!(request["tools"][0]["model"], "gpt-image-2.5-flare");
        assert_eq!(request["tools"][0]["action"], "generate");
        assert!(request.get("max_output_tokens").is_none());
    }

    #[test]
    fn codex_json_edit_request_is_normalized_to_the_existing_image_tool() {
        let source = format!(
            "data:image/png;base64,{}",
            STANDARD.encode(b"\x89PNG\r\n\x1a\n")
        );
        let body = serde_json::to_vec(&json!({
            "images": [{"image_url": source}],
            "prompt": "change only the background",
            "background": "auto",
            "model": "gpt-image-2.5-sunburst",
            "quality": "auto",
            "size": "auto"
        }))
        .expect("serialize Codex image edit");

        let (controls, sources) =
            ImagesProtocolAdapter::decode_json_edit(&body).expect("decode Codex image edit");
        assert_eq!(controls.model, "gpt-image-2.5-sunburst");
        assert_eq!(controls.prompt, "change only the background");
        assert_eq!(sources.len(), 1);

        let request =
            ImagesProtocolAdapter::canonical_request(&controls, "gpt-5.6-sol", &sources, None);
        assert_eq!(request["tools"][0]["action"], "edit");
        assert_eq!(request["input"][0]["content"][1]["type"], "input_image");
        assert_eq!(request["input"][0]["content"][1]["image_url"], sources[0]);
    }

    #[test]
    fn codex_json_edit_request_rejects_invalid_or_excessive_sources() {
        let missing = serde_json::to_vec(&json!({"prompt": "edit"})).expect("missing images");
        assert!(ImagesProtocolAdapter::decode_json_edit(&missing).is_err());

        let invalid = serde_json::to_vec(&json!({
            "images": [{"image_url": "data:image/gif;base64,R0lGODlh"}],
            "prompt": "edit"
        }))
        .expect("unsupported image type");
        assert!(ImagesProtocolAdapter::decode_json_edit(&invalid).is_err());

        let images = (0..=MAX_IMAGE_EDIT_FILES)
            .map(|index| json!({"image_url": format!("https://example.com/{index}.png")}))
            .collect::<Vec<_>>();
        let excessive =
            serde_json::to_vec(&json!({"images": images, "prompt": "edit"})).expect("many images");
        assert!(ImagesProtocolAdapter::decode_json_edit(&excessive).is_err());
    }
}
