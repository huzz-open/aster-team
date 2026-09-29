use std::{
    cell::RefCell,
    collections::HashSet,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use mlua::{Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, Value, VmState};
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
use thiserror::Error;

use crate::{Bundle, PluginResult};

#[derive(Clone, Copy, Debug)]
pub struct ExecutionLimits {
    pub memory_bytes: usize,
    pub instructions: u64,
    pub synchronous_deadline: Duration,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 * 1024 * 1024,
            instructions: 10_000_000,
            synchronous_deadline: Duration::from_millis(100),
        }
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("plugin entrypoint or operation is missing")]
    MissingOperation,
    #[error("plugin JSON value exceeds the canonical boundary")]
    InvalidValue,
    #[error("plugin execution exceeded its budget")]
    Budget,
    #[error("plugin execution failed: {0}")]
    Lua(#[from] mlua::Error),
    #[error("plugin returned an invalid contract: {0}")]
    Contract(#[from] serde_json::Error),
}

/// A new instance is created for each workflow. No mutable globals are shared
/// between requests, while all modules come from the same verified snapshot.
pub struct LuaRuntime {
    bundle: Arc<Bundle>,
    limits: ExecutionLimits,
}

impl LuaRuntime {
    pub fn new(bundle: Arc<Bundle>, limits: ExecutionLimits) -> Self {
        Self { bundle, limits }
    }

    pub fn bundle_digest(&self) -> &str {
        &self.bundle.digest
    }

    pub fn invoke(
        &self,
        entrypoint: &str,
        operation: &str,
        input: &JsonValue,
    ) -> Result<JsonValue, RuntimeError> {
        validate_json(input, 0)?;
        let source_path = self
            .bundle
            .manifest
            .entrypoints
            .get(entrypoint)
            .ok_or(RuntimeError::MissingOperation)?;
        let source = self
            .bundle
            .files
            .get(source_path)
            .ok_or(RuntimeError::MissingOperation)?;
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
            LuaOptions::default(),
        )?;
        lua.set_memory_limit(self.limits.memory_bytes)?;
        install_budget(&lua, self.limits)?;
        let globals = lua.globals();
        for name in ["dofile", "loadfile", "load", "collectgarbage"] {
            globals.set(name, Value::Nil)?;
        }
        let aster = lua.create_table()?;
        aster.set("null", lua.null())?;
        aster.set(
            "array",
            lua.create_function(|lua, table: Table| {
                table.set_metatable(Some(lua.array_metatable()))?;
                Ok(table)
            })?,
        )?;
        aster.set(
            "is_array",
            lua.create_function(|lua, table: Table| {
                Ok(table
                    .metatable()
                    .is_some_and(|meta| meta.to_pointer() == lua.array_metatable().to_pointer()))
            })?,
        )?;
        aster.set(
            "parse_json",
            lua.create_function(|lua, source: String| {
                if source.len() > 1024 * 1024 {
                    return Err(mlua::Error::runtime("JSON input exceeds plugin limit"));
                }
                let value: JsonValue = serde_json::from_str(&source)
                    .map_err(|_| mlua::Error::runtime("invalid JSON value"))?;
                validate_json(&value, 0)
                    .map_err(|_| mlua::Error::runtime("JSON value exceeds plugin limit"))?;
                lua.to_value(&value)
            })?,
        )?;
        aster.set(
            "stringify_json",
            lua.create_function(|lua, value: Value| {
                let value: JsonValue = lua.from_value(value)?;
                validate_json(&value, 0)
                    .map_err(|_| mlua::Error::runtime("JSON value exceeds plugin limit"))?;
                let encoded = serde_json::to_string(&value)
                    .map_err(|_| mlua::Error::runtime("invalid JSON value"))?;
                if encoded.len() > 1024 * 1024 {
                    return Err(mlua::Error::runtime("JSON output exceeds plugin limit"));
                }
                Ok(encoded)
            })?,
        )?;
        let data_files = self.bundle.files.clone();
        aster.set(
            "data",
            lua.create_function(move |lua, path: String| {
                if !path.ends_with(".json")
                    || path.contains("..")
                    || path.contains('\\')
                    || path.starts_with('/')
                    || path.len() > 160
                {
                    return Err(mlua::Error::runtime("invalid bundle data path"));
                }
                let source = data_files
                    .get(&path)
                    .ok_or_else(|| mlua::Error::runtime("data is not in the signed bundle"))?;
                if source.len() > 1024 * 1024 {
                    return Err(mlua::Error::runtime("bundle data exceeds plugin limit"));
                }
                let value: JsonValue = serde_json::from_slice(source)
                    .map_err(|_| mlua::Error::runtime("invalid bundle data"))?;
                validate_json(&value, 0)
                    .map_err(|_| mlua::Error::runtime("bundle data exceeds plugin limit"))?;
                lua.to_value(&value)
            })?,
        )?;
        globals.set("aster", aster)?;

        let modules = self.bundle.files.clone();
        let cache = lua.create_table()?;
        let loading = Rc::new(RefCell::new(HashSet::<String>::new()));
        let require = lua.create_function(move |lua, module: String| {
            if module.is_empty()
                || module.split('.').any(|part| {
                    part.is_empty() || !part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                })
            {
                return Err(mlua::Error::runtime("invalid module name"));
            }
            let cached: Value = cache.get(module.as_str())?;
            if !matches!(cached, Value::Nil) {
                return Ok(cached);
            }
            let path = format!("{}.lua", module.replace('.', "/"));
            let source = modules
                .get(&path)
                .ok_or_else(|| mlua::Error::runtime("module is not in the signed bundle"))?;
            if !loading.borrow_mut().insert(module.clone()) {
                return Err(mlua::Error::runtime("cyclic module dependency"));
            }
            let result = lua.load(source.as_slice()).set_name(&path).eval::<Value>();
            loading.borrow_mut().remove(&module);
            let value = result?;
            if matches!(value, Value::Nil) {
                return Err(mlua::Error::runtime("module returned nil"));
            }
            cache.set(module, value.clone())?;
            Ok(value)
        })?;
        globals.set("require", require)?;

        let exports: Table = lua.load(source.as_slice()).set_name(source_path).eval()?;
        let function: Function = exports
            .get(operation)
            .map_err(|_| RuntimeError::MissingOperation)?;
        let argument = lua.to_value(input)?;
        let result: Value = function.call(argument).map_err(|error| {
            if error.to_string().contains("plugin budget exceeded") {
                RuntimeError::Budget
            } else {
                RuntimeError::Lua(error)
            }
        })?;
        let output = lua.from_value(result).map_err(RuntimeError::Lua)?;
        validate_json(&output, 0)?;
        Ok(output)
    }

    pub fn self_test(&self) -> Result<(), RuntimeError> {
        for name in self.bundle.manifest.entrypoints.keys() {
            let result = self.invoke(name, "self_test", &JsonValue::Null)?;
            if result != JsonValue::Bool(true) {
                return Err(RuntimeError::Lua(mlua::Error::runtime(
                    "plugin self_test must return true",
                )));
            }
        }
        Ok(())
    }

    pub fn invoke_typed<T: DeserializeOwned>(
        &self,
        entrypoint: &str,
        operation: &str,
        input: &JsonValue,
    ) -> Result<PluginResult<T>, RuntimeError> {
        let result = self.invoke(entrypoint, operation, input)?;
        serde_json::from_value(result).map_err(RuntimeError::Contract)
    }
}

fn validate_json(value: &JsonValue, depth: usize) -> Result<(), RuntimeError> {
    if depth > 64 {
        return Err(RuntimeError::InvalidValue);
    }
    match value {
        JsonValue::Number(number) => {
            if number.as_i64().is_some() {
                // Lua 5.4 retains signed 64-bit integers without float coercion.
            } else if number.as_u64().is_some()
                || number.as_f64().is_some_and(|float| {
                    !float.is_finite()
                        || float.fract() == 0.0 && float.abs() > 9_007_199_254_740_991.0
                })
            {
                return Err(RuntimeError::InvalidValue);
            }
        }
        JsonValue::Array(items) => {
            if items.len() > 100_000 {
                return Err(RuntimeError::InvalidValue);
            }
            for item in items {
                validate_json(item, depth + 1)?;
            }
        }
        JsonValue::Object(fields) => {
            if fields.len() > 100_000 {
                return Err(RuntimeError::InvalidValue);
            }
            for value in fields.values() {
                validate_json(value, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn install_budget(lua: &Lua, limits: ExecutionLimits) -> mlua::Result<()> {
    let started = Instant::now();
    let used = Arc::new(AtomicU64::new(0));
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(10_000),
        move |_, _| {
            let instructions = used.fetch_add(10_000, Ordering::Relaxed) + 10_000;
            if instructions > limits.instructions || started.elapsed() > limits.synchronous_deadline
            {
                return Err(mlua::Error::runtime("plugin budget exceeded"));
            }
            Ok(VmState::Continue)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{signed_fixture, signed_fixture_with_files, verify_bundle};
    use serde_json::json;

    #[test]
    fn signed_lua_runs_without_filesystem_or_network() {
        let (archive, key) = signed_fixture(
            b"return { self_test = function() return true end, echo = function(v) return {value=v.value, no_io=io==nil, no_os=os==nil, no_debug=debug==nil} end }",
        );
        let bundle = Arc::new(verify_bundle(&archive, &[key], "2.1.1", 2).unwrap());
        let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
        runtime.self_test().unwrap();
        assert_eq!(
            runtime
                .invoke("public", "echo", &json!({"value": 0}))
                .unwrap(),
            json!({"value": 0, "no_io": true, "no_os": true, "no_debug": true}),
        );
    }

    #[test]
    fn infinite_loop_is_stopped_by_instruction_budget() {
        let (archive, key) = signed_fixture(
            b"return { self_test = function() return true end, spin = function() while true do end end }",
        );
        let bundle = Arc::new(verify_bundle(&archive, &[key], "2.1.1", 2).unwrap());
        let runtime = LuaRuntime::new(
            bundle,
            ExecutionLimits {
                instructions: 20_000,
                ..ExecutionLimits::default()
            },
        );
        assert!(matches!(
            runtime.invoke("public", "spin", &JsonValue::Null),
            Err(RuntimeError::Budget)
        ));
    }

    #[test]
    fn provider_bundles_expose_only_their_own_channels() {
        let source = include_bytes!("../../../../plugins/shared/gateway.lua");
        let rules = include_bytes!("../../../../plugins/shared/public-rules.json");
        let all_channels: JsonValue =
            serde_json::from_slice(include_bytes!("../../../../plugins/shared/channels.json"))
                .unwrap();
        for provider in ["openai", "deepseek", "glm"] {
            let provider_json = format!(r#"{{"provider":"{provider}"}}"#);
            let channels = all_channels
                .as_object()
                .unwrap()
                .iter()
                .filter(|(_, channel)| channel["provider"] == provider)
                .map(|(id, channel)| (id.clone(), channel.clone()))
                .collect::<serde_json::Map<_, _>>();
            let channels_json = serde_json::to_vec(&channels).unwrap();
            let (archive, key) = signed_fixture_with_files(
                source,
                &[
                    ("public-rules.json", rules),
                    ("provider.json", provider_json.as_bytes()),
                    ("channels.json", &channels_json),
                ],
            );
            let bundle = Arc::new(verify_bundle(&archive, &[key], "2.1.1", 2).unwrap());
            let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
            runtime.self_test().unwrap();
            let description = runtime
                .invoke("public", "describe", &JsonValue::Null)
                .unwrap();
            let channels = description["value"]["channels"].as_object().unwrap();
            assert!(!channels.is_empty());
            assert!(
                channels
                    .keys()
                    .all(|id| id.starts_with(&format!("{provider}.")))
            );
        }
    }

    #[test]
    fn official_text_rules_decode_assess_and_prepare_without_credentials() {
        let source = include_bytes!("../../../../plugins/shared/gateway.lua");
        let rules = include_bytes!("../../../../plugins/shared/public-rules.json");
        let (archive, key) = signed_fixture_with_files(
            source,
            &[
                ("public-rules.json", rules),
                ("provider.json", br#"{"provider":"all"}"#),
                (
                    "channels.json",
                    include_bytes!("../../../../plugins/shared/channels.json"),
                ),
            ],
        );
        let bundle = Arc::new(verify_bundle(&archive, &[key], "2.1.1", 2).unwrap());
        let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
        runtime.self_test().unwrap();
        let operation = runtime
            .invoke(
                "public",
                "decode_request",
                &json!({
                    "protocol": "chat_completions",
                    "body": {"model": "public-model", "messages": [{"role": "user", "content": "hi"}]}
                }),
            )
            .unwrap();
        assert_eq!(operation["status"], "ok");
        let target = json!({
            "channel_id": "glm.zai.coding",
            "connection_id": "fixture",
            "connection_revision": 1,
            "upstream_model": "glm-5.1"
        });
        let plan = runtime
            .invoke(
                "public",
                "assess",
                &json!({
                    "operation": operation["value"],
                    "target": target,
                    "compatibility_mode": "compatible"
                }),
            )
            .unwrap();
        assert_eq!(plan["value"]["compatible"], true);
        let intent = runtime
            .invoke_typed::<crate::HttpIntent>(
                "public",
                "prepare",
                &json!({
                    "operation": operation["value"],
                    "target": target,
                    "plan": plan["value"]
                }),
            )
            .unwrap();
        let crate::PluginResult::Ok { value: intent } = intent else {
            panic!("official plugin must prepare the simple request");
        };
        crate::validate_http_intent(&intent).unwrap();
        assert_eq!(intent.endpoint_id, "glm.zai.coding");
        assert_eq!(intent.relative_path, "/chat/completions");
    }

    #[test]
    fn official_image_rules_map_glm_and_openai_without_asset_bytes() {
        let source = include_bytes!("../../../../plugins/shared/gateway.lua");
        let rules = include_bytes!("../../../../plugins/shared/public-rules.json");
        let (archive, key) = signed_fixture_with_files(
            source,
            &[
                ("public-rules.json", rules),
                ("provider.json", br#"{"provider":"all"}"#),
                (
                    "channels.json",
                    include_bytes!("../../../../plugins/shared/channels.json"),
                ),
            ],
        );
        let bundle = Arc::new(verify_bundle(&archive, &[key], "2.1.1", 2).unwrap());
        let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
        let glm = runtime
            .invoke(
                "public",
                "prepare_image",
                &json!({
                    "body":{"model":"public-glm","prompt":"a blue bird","n":2,
                        "size":"1280x1280","quality":"high"},
                    "edit":false,"sources":[],"mask":null,
                    "target":{"channel_id":"glm.zai.general","upstream_model":"glm-image"}
                }),
            )
            .unwrap();
        assert_eq!(glm["status"], "ok");
        assert_eq!(glm["value"]["relative_path"], "/images/generations");
        assert_eq!(glm["value"]["body"]["quality"], "hd");
        assert_eq!(glm["value"]["body"].get("n"), None);
        let edit = runtime
            .invoke(
                "public",
                "prepare_image",
                &json!({
                    "body":{"model":"public-image","prompt":"remove the cloud","n":1},
                    "edit":true,"sources":["asset_0"],"mask":"asset_mask",
                    "target":{"channel_id":"openai.api","upstream_model":"gpt-image-1"}
                }),
            )
            .unwrap();
        assert_eq!(edit["status"], "ok");
        assert_eq!(edit["value"]["relative_path"], "/images/edits");
        assert_eq!(edit["value"]["body"]["__image_sources"][0], "asset_0");
        let parsed = runtime
            .invoke(
                "public",
                "parse_image",
                &json!({
                    "provider":"glm","body":{"data":[{"url":"https://cdn.example/image.png"}]}
                }),
            )
            .unwrap();
        assert_eq!(parsed["value"]["kind"], "url");
        let unsupported_quality = runtime
            .invoke(
                "public",
                "prepare_image",
                &json!({
                    "body":{"model":"public-image","prompt":"a bird","quality":"max"},
                    "edit":false,"sources":[],"mask":null,
                    "target":{"channel_id":"openai.api","upstream_model":"gpt-image-1"}
                }),
            )
            .unwrap();
        assert_eq!(unsupported_quality["status"], "error");
        assert_eq!(unsupported_quality["error"]["source_path"], "quality");
        let unsupported_glm_edit = runtime
            .invoke(
                "public",
                "prepare_image",
                &json!({
                    "body":{"model":"public-glm","prompt":"edit"},
                    "edit":true,"sources":["asset_0"],"mask":null,
                    "target":{"channel_id":"glm.zai.general","upstream_model":"glm-image"}
                }),
            )
            .unwrap();
        assert_eq!(unsupported_glm_edit["status"], "error");
    }

    #[test]
    fn official_codex_request_body_uses_signed_lua_rules() {
        let source = include_bytes!("../../../../plugins/shared/gateway.lua");
        let rules = include_bytes!("../../../../plugins/shared/public-rules.json");
        let (archive, key) = signed_fixture_with_files(
            source,
            &[
                ("public-rules.json", rules),
                ("provider.json", br#"{"provider":"all"}"#),
                (
                    "channels.json",
                    include_bytes!("../../../../plugins/shared/channels.json"),
                ),
            ],
        );
        let bundle = Arc::new(verify_bundle(&archive, &[key], "2.1.1", 2).unwrap());
        let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
        let prepared = runtime
            .invoke(
                "public",
                "prepare_codex",
                &json!({
                    "body":{"input":"hello","model":"public-model","service_tier":"fast"},
                    "upstream_model":"gpt-5.5"
                }),
            )
            .unwrap();
        assert_eq!(prepared["status"], "ok");
        assert_eq!(prepared["value"]["model"], "gpt-5.5");
        assert_eq!(prepared["value"]["service_tier"], "priority");
        assert_eq!(prepared["value"]["input"][0]["content"][0]["text"], "hello");
        assert_eq!(prepared["value"]["stream"], true);
        assert_eq!(prepared["value"]["store"], false);
        let refused = runtime
            .invoke(
                "public",
                "prepare_codex",
                &json!({
                    "body":{"input":"hello","max_output_tokens":100},
                    "upstream_model":"gpt-5.5"
                }),
            )
            .unwrap();
        assert_eq!(refused["status"], "error");
        assert_eq!(refused["error"]["source_path"], "max_output_tokens");
    }

    #[test]
    fn official_reasoning_mapping_is_visible_and_strict_mode_rejects_approximation() {
        let source = include_bytes!("../../../../plugins/shared/gateway.lua");
        let rules = include_bytes!("../../../../plugins/shared/public-rules.json");
        let (archive, key) = signed_fixture_with_files(
            source,
            &[
                ("public-rules.json", rules),
                ("provider.json", br#"{"provider":"all"}"#),
                (
                    "channels.json",
                    include_bytes!("../../../../plugins/shared/channels.json"),
                ),
            ],
        );
        let bundle = Arc::new(verify_bundle(&archive, &[key], "2.1.1", 2).unwrap());
        let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
        let decoded = runtime
            .invoke(
                "public",
                "decode_request",
                &json!({
                    "protocol": "responses",
                    "body": {"model":"public-glm", "input":"hi", "reasoning":{"effort":"xhigh"}}
                }),
            )
            .unwrap();
        assert_eq!(decoded["status"], "ok");
        let target = json!({
            "channel_id":"glm.zai.general", "connection_id":"connection-fixture",
            "connection_revision":1, "upstream_model":"glm-5.2"
        });
        let compatible = runtime
            .invoke(
                "public",
                "assess",
                &json!({
                    "operation":decoded["value"], "target":target, "compatibility_mode":"compatible"
                }),
            )
            .unwrap();
        assert_eq!(compatible["value"]["compatible"], true);
        assert_eq!(compatible["value"]["changes"][0]["effective"], "max");
        let intent = runtime
            .invoke(
                "public",
                "prepare",
                &json!({
                    "operation":decoded["value"], "target":target, "plan":compatible["value"]
                }),
            )
            .unwrap();
        assert_eq!(intent["value"]["body"]["reasoning_effort"], "max");
        assert_eq!(intent["value"]["body"]["thinking"]["type"], "enabled");
        let strict = runtime
            .invoke(
                "public",
                "assess",
                &json!({
                    "operation":decoded["value"], "target":target, "compatibility_mode":"strict"
                }),
            )
            .unwrap();
        assert_eq!(strict["value"]["compatible"], false);
    }

    #[test]
    fn deepseek_thinking_sampling_rule_is_enforced_before_dispatch() {
        let source = include_bytes!("../../../../plugins/shared/gateway.lua");
        let rules = include_bytes!("../../../../plugins/shared/public-rules.json");
        let (archive, key) = signed_fixture_with_files(
            source,
            &[
                ("public-rules.json", rules),
                ("provider.json", br#"{"provider":"all"}"#),
                (
                    "channels.json",
                    include_bytes!("../../../../plugins/shared/channels.json"),
                ),
            ],
        );
        let bundle = Arc::new(verify_bundle(&archive, &[key], "2.1.1", 2).unwrap());
        let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
        let decoded = runtime
            .invoke(
                "public",
                "decode_request",
                &json!({
                    "protocol":"chat_completions",
                    "body":{"model":"deepseek-flash","messages":[{"role":"user","content":"hi"}],
                        "reasoning_effort":"medium","temperature":0.4,"top_p":0.7}
                }),
            )
            .unwrap();
        assert_eq!(decoded["status"], "ok");
        let target = json!({"channel_id":"deepseek.api","connection_id":"fixture",
            "connection_revision":1,"upstream_model":"deepseek-flash"});
        let rejected = runtime
            .invoke(
                "public",
                "assess",
                &json!({
                    "operation":decoded["value"],"target":target,"compatibility_mode":"compatible"
                }),
            )
            .unwrap();
        assert_eq!(rejected["value"]["compatible"], false);
        assert!(
            rejected["value"]["rejected"]
                .as_array()
                .unwrap()
                .iter()
                .any(|error| error["source_path"] == "parameters.temperature")
        );

        let decoded = runtime
            .invoke(
                "public",
                "decode_request",
                &json!({
                    "protocol":"chat_completions",
                    "body":{"model":"deepseek-flash","messages":[{"role":"user","content":"hi"}],
                        "reasoning_effort":"medium","top_p":0.7}
                }),
            )
            .unwrap();
        let plan = runtime
            .invoke(
                "public",
                "assess",
                &json!({
                    "operation":decoded["value"],"target":target,"compatibility_mode":"compatible"
                }),
            )
            .unwrap();
        assert_eq!(plan["value"]["compatible"], true);
        let intent = runtime
            .invoke(
                "public",
                "prepare",
                &json!({
                    "operation":decoded["value"],"target":target,"plan":plan["value"]
                }),
            )
            .unwrap();
        assert_eq!(intent["value"]["body"]["reasoning_effort"], "high");
        assert_eq!(intent["value"]["body"]["top_p"], 0.95);
    }

    #[test]
    fn official_tool_history_and_result_cross_public_protocols() {
        let source = include_bytes!("../../../../plugins/shared/gateway.lua");
        let rules = include_bytes!("../../../../plugins/shared/public-rules.json");
        let (archive, key) = signed_fixture_with_files(
            source,
            &[
                ("public-rules.json", rules),
                ("provider.json", br#"{"provider":"all"}"#),
                (
                    "channels.json",
                    include_bytes!("../../../../plugins/shared/channels.json"),
                ),
            ],
        );
        let bundle = Arc::new(verify_bundle(&archive, &[key], "2.1.1", 2).unwrap());
        let runtime = LuaRuntime::new(bundle, ExecutionLimits::default());
        let decoded = runtime
            .invoke_typed::<crate::CanonicalOperation>(
                "public",
                "decode_request",
                &json!({
                    "protocol": "chat_completions",
                    "body": {
                        "model": "public-model",
                        "messages": [
                            {"role":"user", "content":"Weather?"},
                            {"role":"assistant", "tool_calls":[{
                                "id":"call_1", "type":"function",
                                "function":{"name":"weather", "arguments":"{\"city\":\"Paris\"}"}
                            }]},
                            {"role":"tool", "tool_call_id":"call_1", "content":"Sunny"}
                        ],
                        "tools":[{"type":"function", "function":{
                            "name":"weather", "description":"Get weather",
                            "parameters":{"type":"object", "properties":{"city":{"type":"string"}}}
                        }}]
                    }
                }),
            )
            .unwrap();
        let crate::PluginResult::Ok { value: operation } = decoded else {
            panic!("tool history should decode");
        };
        assert_eq!(operation.conversation.len(), 3);
        assert_eq!(operation.conversation[1].call_id.as_deref(), Some("call_1"));
        let operation = serde_json::to_value(operation).unwrap();
        let target = json!({
            "channel_id": "glm.zai.general", "connection_id": "fixture",
            "connection_revision": 1, "upstream_model": "glm-fixture"
        });
        let plan = runtime
            .invoke_typed::<crate::AdaptationPlan>(
                "public",
                "assess",
                &json!({"operation":operation, "target":target, "compatibility_mode":"compatible"}),
            )
            .unwrap();
        let crate::PluginResult::Ok { value: plan } = plan else {
            panic!("tool call should be assessed");
        };
        assert!(plan.compatible);
        assert!(
            plan.required_features
                .iter()
                .any(|feature| feature == "tools")
        );
        let intent = runtime
            .invoke_typed::<crate::HttpIntent>(
                "public",
                "prepare",
                &json!({"operation":operation, "target":target, "plan":plan}),
            )
            .unwrap();
        let crate::PluginResult::Ok { value: intent } = intent else {
            panic!("tool call should prepare");
        };
        assert_eq!(intent.body["messages"][1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(intent.body["messages"][2]["tool_call_id"], "call_1");

        let result = runtime
            .invoke_typed::<crate::CanonicalResultV2>(
                "public", "parse_buffered",
                &json!({
                    "wire_protocol":"chat", "public_model":"public-model",
                    "body":{
                        "id":"upstream-1", "choices":[{"finish_reason":"tool_calls",
                            "message":{"role":"assistant", "content":null,
                              "tool_calls":[{"id":"call_2", "type":"function",
                                "function":{"name":"weather", "arguments":"{\"city\":\"Rome\"}"}}]}}],
                        "usage":{"prompt_tokens":12,"completion_tokens":4,
                            "prompt_tokens_details":{"cached_tokens":3}}
                    }
                }),
            )
            .unwrap();
        let crate::PluginResult::Ok { value: result } = result else {
            panic!("provider tool result should parse");
        };
        assert_eq!(result.finish_reason, "tool_calls");
        assert_eq!(result.usage.unwrap().cached_input_tokens, Some(3));
        let public = runtime
            .invoke(
                "public",
                "encode_public",
                &json!({
                    "protocol":"anthropic_messages", "public_id":"msg_fixture",
                    "result":{
                        "schema_version":2, "public_model":"public-model",
                        "upstream_id":"upstream-1", "finish_reason":"tool_calls",
                        "output":[{"kind":"tool_call","call_id":"call_2",
                            "name":"weather","arguments":{"city":"Rome"}}],
                        "usage":null
                    }
                }),
            )
            .unwrap();
        assert_eq!(public["value"]["content"][0]["type"], "tool_use");
        assert_eq!(public["value"]["stop_reason"], "tool_use");

        let empty_object_arguments = runtime
            .invoke(
                "public",
                "decode_request",
                &json!({
                    "protocol":"chat_completions",
                    "body":{"model":"public-model","messages":[
                        {"role":"assistant","tool_calls":[{"id":"call_empty",
                          "type":"function","function":{"name":"weather","arguments":"{}"}}]}
                    ]}
                }),
            )
            .unwrap();
        assert_eq!(empty_object_arguments["status"], "ok");
        let array_arguments = runtime
            .invoke(
                "public",
                "decode_request",
                &json!({
                    "protocol":"chat_completions",
                    "body":{"model":"public-model","messages":[
                        {"role":"assistant","tool_calls":[{"id":"call_bad",
                          "type":"function","function":{"name":"weather","arguments":"[]"}}]}
                    ]}
                }),
            )
            .unwrap();
        assert_eq!(array_arguments["status"], "error");
    }

    #[test]
    fn official_chat_stream_maps_text_and_terminal_usage_to_responses() {
        let source = include_bytes!("../../../../plugins/shared/gateway.lua");
        let rules = include_bytes!("../../../../plugins/shared/public-rules.json");
        let (archive, key) = signed_fixture_with_files(
            source,
            &[
                ("public-rules.json", rules),
                ("provider.json", br#"{"provider":"all"}"#),
                (
                    "channels.json",
                    include_bytes!("../../../../plugins/shared/channels.json"),
                ),
            ],
        );
        let bundle = verify_bundle(&archive, &[key], "2.1.1", 2).unwrap();
        let runtime = LuaRuntime::new(Arc::new(bundle), ExecutionLimits::default());
        let first = runtime.invoke("public", "map_chat_to_responses_stream", &serde_json::json!({
            "state":null,"public_id":"resp_fixture","public_model":"glm-fixture","created":12,
            "done":false,"chunk":{"choices":[{"index":0,"delta":{"role":"assistant","content":"hi"},"finish_reason":null}]}
        })).unwrap();
        assert_eq!(first["status"], "ok");
        let first = &first["value"];
        assert!(
            first["events"]
                .as_array()
                .unwrap()
                .iter()
                .any(
                    |event| event["type"] == "response.output_text.delta" && event["delta"] == "hi"
                )
        );
        let second = runtime.invoke("public", "map_chat_to_responses_stream", &serde_json::json!({
            "state":first["state"],"public_id":"resp_fixture","public_model":"glm-fixture","created":12,
            "done":false,"chunk":{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}
        })).unwrap();
        assert_eq!(second["status"], "ok");
        let finished = runtime.invoke("public", "map_chat_to_responses_stream", &serde_json::json!({
            "state":second["value"]["state"],"public_id":"resp_fixture","public_model":"glm-fixture","created":12,
            "done":true,"usage":{"prompt_tokens":3,"completion_tokens":1}
        })).unwrap();
        assert_eq!(finished["status"], "ok");
        assert_eq!(finished["value"]["terminal"], true);
        let events = finished["value"]["events"].as_array().unwrap();
        assert_eq!(events.last().unwrap()["type"], "response.completed");
        assert_eq!(
            events.last().unwrap()["response"]["usage"]["total_tokens"],
            4
        );

        let tool_start = runtime
            .invoke(
                "public",
                "map_chat_to_responses_stream",
                &serde_json::json!({
                    "state":null,"public_id":"resp_tool","public_model":"glm-fixture","created":12,
                    "done":false,"chunk":{"choices":[{"index":0,"delta":{"tool_calls":[
                        {"index":0,"id":"call_1","type":"function",
                         "function":{"name":"weather","arguments":"{\"city\":"}}
                    ]},"finish_reason":null}]}
                }),
            )
            .unwrap();
        assert_eq!(tool_start["status"], "ok");
        let tool_end = runtime.invoke("public", "map_chat_to_responses_stream", &serde_json::json!({
            "state":tool_start["value"]["state"],"public_id":"resp_tool","public_model":"glm-fixture","created":12,
            "done":false,"chunk":{"choices":[{"index":0,"delta":{"tool_calls":[
                {"index":0,"function":{"arguments":"\"Paris\"}"}}
            ]},"finish_reason":"tool_calls"}]}
        })).unwrap();
        assert_eq!(tool_end["status"], "ok");
        let tool_done = runtime.invoke("public", "map_chat_to_responses_stream", &serde_json::json!({
            "state":tool_end["value"]["state"],"public_id":"resp_tool","public_model":"glm-fixture","created":12,
            "done":true,"usage":{"prompt_tokens":7,"completion_tokens":3}
        })).unwrap();
        assert_eq!(tool_done["status"], "ok");
        let output = &tool_done["value"]["events"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["response"]["output"];
        assert_eq!(output[0]["type"], "function_call");
        assert_eq!(output[0]["arguments"], "{\"city\":\"Paris\"}");
    }

    #[test]
    fn official_chat_stream_maps_tool_closure_to_messages() {
        let source = include_bytes!("../../../../plugins/shared/gateway.lua");
        let rules = include_bytes!("../../../../plugins/shared/public-rules.json");
        let (archive, key) = signed_fixture_with_files(
            source,
            &[
                ("public-rules.json", rules),
                ("provider.json", br#"{"provider":"all"}"#),
                (
                    "channels.json",
                    include_bytes!("../../../../plugins/shared/channels.json"),
                ),
            ],
        );
        let bundle = verify_bundle(&archive, &[key], "2.1.1", 2).unwrap();
        let runtime = LuaRuntime::new(Arc::new(bundle), ExecutionLimits::default());
        let first = runtime.invoke("public", "map_chat_to_messages_stream", &serde_json::json!({
            "state":null,"public_id":"msg_fixture","public_model":"glm-fixture",
            "done":false,"chunk":{"choices":[{"index":0,"delta":{"content":"Checking"},"finish_reason":null}]}
        })).unwrap();
        assert_eq!(first["status"], "ok");
        let second = runtime.invoke("public", "map_chat_to_messages_stream", &serde_json::json!({
            "state":first["value"]["state"],"public_id":"msg_fixture","public_model":"glm-fixture",
            "done":false,"chunk":{"choices":[{"index":0,"delta":{"tool_calls":[{
                "index":0,"id":"call_1","function":{"name":"weather","arguments":"{}"}
            }]},"finish_reason":"tool_calls"}]}
        })).unwrap();
        assert_eq!(second["status"], "ok");
        let done = runtime.invoke("public", "map_chat_to_messages_stream", &serde_json::json!({
            "state":second["value"]["state"],"public_id":"msg_fixture","public_model":"glm-fixture",
            "done":true,"usage":{"prompt_tokens":8,"completion_tokens":2}
        })).unwrap();
        assert_eq!(done["status"], "ok");
        let events = done["value"]["events"].as_array().unwrap();
        assert_eq!(events[0]["type"], "content_block_stop");
        assert!(
            events
                .iter()
                .any(|event| event["type"] == "content_block_start"
                    && event["content_block"]["type"] == "tool_use")
        );
        assert_eq!(events[events.len() - 2]["delta"]["stop_reason"], "tool_use");
        assert_eq!(events.last().unwrap()["type"], "message_stop");
    }

    #[test]
    fn official_responses_stream_maps_tool_arguments_to_chat() {
        let source = include_bytes!("../../../../plugins/shared/gateway.lua");
        let rules = include_bytes!("../../../../plugins/shared/public-rules.json");
        let (archive, key) = signed_fixture_with_files(
            source,
            &[
                ("public-rules.json", rules),
                ("provider.json", br#"{"provider":"all"}"#),
                (
                    "channels.json",
                    include_bytes!("../../../../plugins/shared/channels.json"),
                ),
            ],
        );
        let bundle = verify_bundle(&archive, &[key], "2.1.1", 2).unwrap();
        let runtime = LuaRuntime::new(Arc::new(bundle), ExecutionLimits::default());
        let first = runtime.invoke("public", "map_responses_to_chat_stream", &serde_json::json!({
            "state":null,"public_id":"chat_fixture","public_model":"deepseek-fixture","created":12,
            "event":{"type":"response.output_item.added",
              "item":{"id":"fc_1","type":"function_call","call_id":"call_1","name":"weather"}}
        })).unwrap();
        assert_eq!(first["status"], "ok");
        assert_eq!(
            first["value"]["events"][0]["chunk"]["choices"][0]["delta"]["tool_calls"][0]["id"],
            "call_1"
        );
        let second = runtime.invoke("public", "map_responses_to_chat_stream", &serde_json::json!({
            "state":first["value"]["state"],"public_id":"chat_fixture","public_model":"deepseek-fixture","created":12,
            "event":{"type":"response.function_call_arguments.delta", "item_id":"fc_1", "delta":"{}"}
        })).unwrap();
        assert_eq!(second["status"], "ok");
        let done = runtime.invoke("public", "map_responses_to_chat_stream", &serde_json::json!({
            "state":second["value"]["state"],"public_id":"chat_fixture","public_model":"deepseek-fixture","created":12,
            "event":{"type":"response.completed", "response":{"status":"completed",
              "output":[{"type":"function_call"}],"usage":{"input_tokens":5,"output_tokens":2}}}
        })).unwrap();
        assert_eq!(done["status"], "ok");
        assert_eq!(
            done["value"]["events"][0]["chunk"]["choices"][0]["finish_reason"],
            "tool_calls"
        );
        assert_eq!(
            done["value"]["events"][0]["chunk"]["usage"]["total_tokens"],
            7
        );
    }
}
