use serde_json::{Value, json};

pub(crate) const GPT_IMAGE_2_5_FLARE: &str = "gpt-image-2.5-flare";
pub(crate) const GPT_IMAGE_2_5_SUNBURST: &str = "gpt-image-2.5-sunburst";
pub(crate) const GPT_IMAGE_2: &str = "gpt-image-2";
pub(crate) const GPT_IMAGE_1: &str = "gpt-image-1";

pub(crate) const IMAGE_MODELS: [&str; 4] = [
    GPT_IMAGE_2_5_FLARE,
    GPT_IMAGE_2_5_SUNBURST,
    GPT_IMAGE_2,
    GPT_IMAGE_1,
];
pub(crate) const DEFAULT_IMAGE_MODEL: &str = IMAGE_MODELS[0];

const IMAGE_HOST_MODEL_PREFERENCE: [&str; 6] = [
    "gpt-5.4-mini",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
];

pub(crate) fn is_image_model(model: &str) -> bool {
    IMAGE_MODELS.contains(&model)
}

pub(crate) fn supports_dynamic_image_size(model: &str) -> bool {
    matches!(
        model,
        GPT_IMAGE_2_5_FLARE | GPT_IMAGE_2_5_SUNBURST | GPT_IMAGE_2
    )
}

pub(crate) fn supports_extended_image_quality(model: &str) -> bool {
    matches!(model, GPT_IMAGE_2_5_FLARE | GPT_IMAGE_2_5_SUNBURST)
}

pub(crate) fn supports_transparent_image_background(model: &str) -> bool {
    matches!(
        model,
        GPT_IMAGE_2_5_FLARE | GPT_IMAGE_2_5_SUNBURST | GPT_IMAGE_1
    )
}

pub(crate) fn supports_configurable_image_fidelity(model: &str) -> bool {
    model == GPT_IMAGE_1
}

pub(crate) fn openai_image_model(model: &str) -> Value {
    json!({
        "id": model,
        "object": "model",
        "created": 0,
        "owned_by": "openai",
    })
}

/// Strategy for selecting the text-capable Codex route which hosts image tools.
/// Public image capabilities deliberately do not depend on the model-sync response.
pub(crate) fn select_image_host_model<'a>(
    available: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let available = available.into_iter().collect::<Vec<_>>();
    IMAGE_HOST_MODEL_PREFERENCE
        .iter()
        .find(|preferred| available.contains(preferred))
        .map(|model| (*model).to_owned())
        .or_else(|| {
            available
                .into_iter()
                .find(|model| model.starts_with("gpt-") && !is_image_model(model))
                .map(str::to_owned)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_capability_is_synthetic_and_host_selection_is_deterministic() {
        assert_eq!(DEFAULT_IMAGE_MODEL, "gpt-image-2.5-flare");
        assert!(is_image_model("gpt-image-2.5-flare"));
        assert!(is_image_model("gpt-image-2.5-sunburst"));
        assert!(is_image_model("gpt-image-2"));
        assert!(!is_image_model("gpt-5.6-sol"));
        assert!(supports_dynamic_image_size("gpt-image-2.5-flare"));
        assert!(supports_dynamic_image_size("gpt-image-2.5-sunburst"));
        assert!(supports_dynamic_image_size("gpt-image-2"));
        assert!(!supports_dynamic_image_size("gpt-image-1"));
        assert!(supports_extended_image_quality("gpt-image-2.5-flare"));
        assert!(supports_extended_image_quality("gpt-image-2.5-sunburst"));
        assert!(!supports_extended_image_quality("gpt-image-2"));
        assert!(supports_transparent_image_background("gpt-image-2.5-flare"));
        assert!(!supports_transparent_image_background("gpt-image-2"));
        assert!(supports_configurable_image_fidelity("gpt-image-1"));
        assert!(!supports_configurable_image_fidelity("gpt-image-2.5-flare"));
        assert_eq!(
            select_image_host_model(["gpt-5.6-sol", "gpt-5.5"]),
            Some("gpt-5.5".to_owned())
        );
        assert_eq!(
            select_image_host_model(["o4-mini", "gpt-custom"]),
            Some("gpt-custom".to_owned())
        );
    }
}
