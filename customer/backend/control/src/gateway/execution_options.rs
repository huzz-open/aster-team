use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::ControlError;

const SPEED_VARIANTS: [(&str, ProcessingTier); 2] = [
    ("standard", ProcessingTier::Standard),
    ("fast", ProcessingTier::Fast),
];

const REASONING_VARIANTS: [(&str, ReasoningEffort); 6] = [
    ("none", ReasoningEffort::None),
    ("low", ReasoningEffort::Low),
    ("medium", ReasoningEffort::Medium),
    ("high", ReasoningEffort::High),
    ("xhigh", ReasoningEffort::XHigh),
    ("max", ReasoningEffort::Max),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessingTier {
    Auto,
    Standard,
    Flex,
    Fast,
    UltraFast,
}

impl ProcessingTier {
    fn from_openai(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "default" => Some(Self::Standard),
            "flex" => Some(Self::Flex),
            "fast" | "priority" => Some(Self::Fast),
            "ultrafast" => Some(Self::UltraFast),
            _ => None,
        }
    }

    const fn as_openai(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Standard => "default",
            Self::Flex => "flex",
            Self::Fast => "fast",
            Self::UltraFast => "ultrafast",
        }
    }

    const fn as_record(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Standard => "standard",
            Self::Flex => "flex",
            Self::Fast => "fast",
            Self::UltraFast => "ultrafast",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReasoningEffort {
    None,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl ReasoningEffort {
    fn from_wire(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    const fn as_wire(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    fn from_anthropic(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExecutionOptions {
    processing_tier: Option<ProcessingTier>,
    reasoning_effort: Option<ReasoningEffort>,
}

impl ExecutionOptions {
    pub(crate) fn processing_tier(self) -> Option<&'static str> {
        self.processing_tier.map(ProcessingTier::as_record)
    }

    pub(crate) fn reasoning_effort(self) -> Option<&'static str> {
        self.reasoning_effort.map(ReasoningEffort::as_wire)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ModelVariant {
    base_model: String,
    options: ExecutionOptions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedModelVariant {
    pub(crate) requested_model: String,
    pub(crate) base_model: String,
    options: ExecutionOptions,
}

impl ResolvedModelVariant {
    pub(crate) fn apply_to_canonical(
        &self,
        request: &mut Value,
    ) -> Result<ExecutionOptions, ControlError> {
        let object = request
            .as_object_mut()
            .ok_or(ControlError::GatewayRequestInvalid)?;
        let explicit = openai_options(object)?;
        let merged = explicit.combine_model_options(self.options)?;
        write_openai_options(object, merged)?;
        object.insert("model".to_owned(), Value::String(self.base_model.clone()));
        Ok(merged)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ModelVariantCatalog {
    entries: HashMap<String, ModelVariant>,
}

impl ModelVariantCatalog {
    pub(crate) fn from_base_models<'a>(models: impl IntoIterator<Item = &'a str>) -> Self {
        let mut base_models = models
            .into_iter()
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        base_models.sort();
        base_models.dedup();

        let mut entries = HashMap::with_capacity(base_models.len().saturating_mul(21));
        for base_model in &base_models {
            entries.insert(
                base_model.clone(),
                ModelVariant {
                    base_model: base_model.clone(),
                    options: ExecutionOptions::default(),
                },
            );
        }
        for base_model in base_models {
            for (speed_name, processing_tier) in SPEED_VARIANTS {
                insert_variant(
                    &mut entries,
                    format!("{base_model}-{speed_name}"),
                    &base_model,
                    ExecutionOptions {
                        processing_tier: Some(processing_tier),
                        reasoning_effort: None,
                    },
                );
            }
            for (effort_name, reasoning_effort) in REASONING_VARIANTS {
                insert_variant(
                    &mut entries,
                    format!("{base_model}-{effort_name}"),
                    &base_model,
                    ExecutionOptions {
                        processing_tier: None,
                        reasoning_effort: Some(reasoning_effort),
                    },
                );
                for (speed_name, processing_tier) in SPEED_VARIANTS {
                    insert_variant(
                        &mut entries,
                        format!("{base_model}-{speed_name}-{effort_name}"),
                        &base_model,
                        ExecutionOptions {
                            processing_tier: Some(processing_tier),
                            reasoning_effort: Some(reasoning_effort),
                        },
                    );
                }
            }
        }
        Self { entries }
    }

    pub(crate) fn resolve(&self, requested_model: &str) -> Option<ResolvedModelVariant> {
        self.entries
            .get(requested_model)
            .map(|variant| ResolvedModelVariant {
                requested_model: requested_model.to_owned(),
                base_model: variant.base_model.clone(),
                options: variant.options,
            })
    }
}

fn insert_variant(
    entries: &mut HashMap<String, ModelVariant>,
    public_name: String,
    base_model: &str,
    options: ExecutionOptions,
) {
    if public_name.len() > 160 {
        return;
    }
    entries.entry(public_name).or_insert_with(|| ModelVariant {
        base_model: base_model.to_owned(),
        options,
    });
}

impl ExecutionOptions {
    fn combine_model_options(self, model: Self) -> Result<Self, ControlError> {
        Ok(Self {
            processing_tier: merge_option(self.processing_tier, model.processing_tier)?,
            reasoning_effort: merge_option(self.reasoning_effort, model.reasoning_effort)?,
        })
    }
}

fn merge_option<T: Copy + Eq>(
    explicit: Option<T>,
    from_model: Option<T>,
) -> Result<Option<T>, ControlError> {
    match (explicit, from_model) {
        (Some(left), Some(right)) if left != right => Err(ControlError::GatewayRequestInvalid),
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

fn openai_options(source: &Map<String, Value>) -> Result<ExecutionOptions, ControlError> {
    let processing_tier = optional_string(source.get("service_tier"))?
        .map(|value| ProcessingTier::from_openai(value).ok_or(ControlError::GatewayRequestInvalid))
        .transpose()?;
    let reasoning_effort = match source.get("reasoning") {
        None | Some(Value::Null) => None,
        Some(Value::Object(reasoning)) => optional_string(reasoning.get("effort"))?
            .map(|value| {
                ReasoningEffort::from_wire(value).ok_or(ControlError::GatewayRequestInvalid)
            })
            .transpose()?,
        Some(_) => return Err(ControlError::GatewayRequestInvalid),
    };
    Ok(ExecutionOptions {
        processing_tier,
        reasoning_effort,
    })
}

fn write_openai_options(
    target: &mut Map<String, Value>,
    options: ExecutionOptions,
) -> Result<(), ControlError> {
    if let Some(processing_tier) = options.processing_tier {
        target.insert(
            "service_tier".to_owned(),
            Value::String(processing_tier.as_openai().to_owned()),
        );
    }
    if let Some(reasoning_effort) = options.reasoning_effort {
        let reasoning = target
            .entry("reasoning")
            .or_insert_with(|| Value::Object(Map::new()));
        if reasoning.is_null() {
            *reasoning = Value::Object(Map::new());
        }
        reasoning
            .as_object_mut()
            .ok_or(ControlError::GatewayRequestInvalid)?
            .insert(
                "effort".to_owned(),
                Value::String(reasoning_effort.as_wire().to_owned()),
            );
    }
    Ok(())
}

pub(crate) fn copy_chat_options(
    source: &Map<String, Value>,
    canonical: &mut Value,
) -> Result<(), ControlError> {
    let processing_tier = optional_string(source.get("service_tier"))?
        .map(|value| ProcessingTier::from_openai(value).ok_or(ControlError::GatewayRequestInvalid))
        .transpose()?;
    let reasoning_effort = optional_string(source.get("reasoning_effort"))?
        .map(|value| ReasoningEffort::from_wire(value).ok_or(ControlError::GatewayRequestInvalid))
        .transpose()?;
    write_openai_options(
        canonical
            .as_object_mut()
            .ok_or(ControlError::GatewayRequestInvalid)?,
        ExecutionOptions {
            processing_tier,
            reasoning_effort,
        },
    )
}

pub(crate) fn copy_anthropic_options(
    source: &Map<String, Value>,
    canonical: &mut Value,
) -> Result<(), ControlError> {
    let processing_tier = match optional_string(source.get("speed"))? {
        None => None,
        Some("fast") => Some(ProcessingTier::Fast),
        Some(_) => return Err(ControlError::GatewayRequestInvalid),
    };
    let reasoning_effort = match source.get("output_config") {
        None | Some(Value::Null) => None,
        Some(Value::Object(output_config)) => optional_string(output_config.get("effort"))?
            .map(|value| {
                ReasoningEffort::from_anthropic(value).ok_or(ControlError::GatewayRequestInvalid)
            })
            .transpose()?,
        Some(_) => return Err(ControlError::GatewayRequestInvalid),
    };
    write_openai_options(
        canonical
            .as_object_mut()
            .ok_or(ControlError::GatewayRequestInvalid)?,
        ExecutionOptions {
            processing_tier,
            reasoning_effort,
        },
    )
}

fn optional_string(value: Option<&Value>) -> Result<Option<&str>, ControlError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(ControlError::GatewayRequestInvalid),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn catalog_resolves_only_precomputed_exact_variants() {
        let catalog = ModelVariantCatalog::from_base_models(["gpt-5.6-terra"]);
        let resolved = catalog
            .resolve("gpt-5.6-terra-fast-high")
            .expect("resolve exact variant");
        assert_eq!(resolved.base_model, "gpt-5.6-terra");
        assert!(catalog.resolve("gpt-5.6-terra-high-fast").is_none());
        assert!(catalog.resolve("Gpt-5.6-Terra-fast-high").is_none());
    }

    #[test]
    fn base_model_keeps_upstream_defaults_unset() {
        let catalog = ModelVariantCatalog::from_base_models(["gpt-5.6-terra"]);
        let resolved = catalog
            .resolve("gpt-5.6-terra")
            .expect("resolve base model");
        let mut request = json!({"model":"gpt-5.6-terra","input":"hello"});
        resolved
            .apply_to_canonical(&mut request)
            .expect("apply base model");
        assert!(request.get("service_tier").is_none());
        assert!(request.get("reasoning").is_none());
    }

    #[test]
    fn model_variant_writes_fast_and_reasoning_options() {
        let catalog = ModelVariantCatalog::from_base_models(["gpt-5.6-terra"]);
        let resolved = catalog
            .resolve("gpt-5.6-terra-fast-low")
            .expect("resolve fast low variant");
        let mut request = json!({"model":"gpt-5.6-terra-fast-low","input":"hello"});
        resolved
            .apply_to_canonical(&mut request)
            .expect("apply variant");
        assert_eq!(request["model"], "gpt-5.6-terra");
        assert_eq!(request["service_tier"], "fast");
        assert_eq!(request["reasoning"]["effort"], "low");
    }

    #[test]
    fn equivalent_explicit_options_are_normalized() {
        let catalog = ModelVariantCatalog::from_base_models(["gpt-5.6-terra"]);
        let resolved = catalog
            .resolve("gpt-5.6-terra-fast-high")
            .expect("resolve variant");
        let mut request = json!({
            "model":"gpt-5.6-terra-fast-high",
            "input":"hello",
            "service_tier":"priority",
            "reasoning":{"effort":"high","summary":"auto"}
        });
        resolved
            .apply_to_canonical(&mut request)
            .expect("merge equivalent options");
        assert_eq!(request["service_tier"], "fast");
        assert_eq!(request["reasoning"]["summary"], "auto");
    }

    #[test]
    fn conflicting_explicit_options_are_rejected() {
        let catalog = ModelVariantCatalog::from_base_models(["gpt-5.6-terra"]);
        let resolved = catalog
            .resolve("gpt-5.6-terra-fast-high")
            .expect("resolve variant");
        let mut request = json!({
            "model":"gpt-5.6-terra-fast-high",
            "input":"hello",
            "service_tier":"default"
        });
        assert!(matches!(
            resolved.apply_to_canonical(&mut request),
            Err(ControlError::GatewayRequestInvalid)
        ));
    }

    #[test]
    fn anthropic_native_options_become_canonical_openai_options() {
        let source = json!({
            "speed":"fast",
            "output_config":{"effort":"medium"}
        });
        let mut canonical = json!({"model":"gpt-5.6-terra","input":[]});
        copy_anthropic_options(source.as_object().unwrap(), &mut canonical)
            .expect("copy Anthropic options");
        assert_eq!(canonical["service_tier"], "fast");
        assert_eq!(canonical["reasoning"]["effort"], "medium");
    }
}
