use std::num::NonZeroU32;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CONFIG_ID: &str = "image-resize";
pub const CONFIG_NAME: &str = "Image resize";
pub const CONFIG_DESCRIPTION: &str = "Default box, strategy and JPEG/WebP quality for image_resize::resize calls that leave them out.";

const DEFAULT_SIDE: NonZeroU32 = NonZeroU32::new(200).unwrap();

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ResizeConfig {
    /// Box width in pixels for a call without `target_width`.
    pub width: NonZeroU32,
    /// Box height in pixels for a call without `target_height`.
    pub height: NonZeroU32,
    /// Strategy for a call without `strategy`.
    pub strategy: ResizeStrategy,
    /// Quality for a call without `quality`.
    pub quality: QualityConfig,
}

impl Default for ResizeConfig {
    fn default() -> Self {
        ResizeConfig {
            width: DEFAULT_SIDE,
            height: DEFAULT_SIDE,
            strategy: ResizeStrategy::default(),
            quality: QualityConfig::default(),
        }
    }
}

#[derive(Deserialize, Serialize, JsonSchema, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ResizeStrategy {
    #[default]
    ScaleToFit,
    CropToFit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct QualityConfig {
    /// JPEG quality, 1-100.
    #[schemars(range(min = 1, max = 100))]
    pub jpeg: u8,
    /// WebP quality, 1-100.
    #[schemars(range(min = 1, max = 100))]
    pub webp: u8,
}

impl Default for QualityConfig {
    fn default() -> Self {
        QualityConfig { jpeg: 85, webp: 80 }
    }
}

impl ResizeConfig {
    /// A value from the configuration worker or a seed file; missing keys take the defaults.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let config =
            Self::deserialize(value).map_err(|e| format!("invalid image-resize config: {e}"))?;
        for quality in [config.quality.jpeg, config.quality.webp] {
            check_quality(quality).map_err(|e| format!("invalid image-resize config: {e}"))?;
        }
        Ok(config)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("ResizeConfig serializes")
    }

    /// The registered schema, with the defaults as its top-level `example`.
    pub fn json_schema() -> Value {
        let mut schema = serde_json::to_value(schemars::schema_for!(ResizeConfig))
            .expect("ResizeConfig schema serializes");
        schema["example"] = Self::default().to_json();
        schema
    }
}

pub fn check_quality(q: u8) -> anyhow::Result<u8> {
    anyhow::ensure!((1..=100).contains(&q), "quality must be 1-100, got {q}");
    Ok(q)
}

/// The `--config` seed. It only applies while nothing is stored, so a file
/// that cannot be used is skipped with a warning instead of stopping the worker.
pub fn load_seed(path: &str) -> Option<ResizeConfig> {
    std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|yaml| serde_yaml_ng::from_str::<Value>(&yaml).map_err(|e| e.to_string()))
        .and_then(|value| ResizeConfig::from_json(&value))
        .map_err(|error| tracing::warn!(path, %error, "ignoring the --config seed"))
        .ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn empty_value_is_the_default() {
        assert_eq!(
            ResizeConfig::from_json(&json!({})).unwrap(),
            ResizeConfig::default()
        );
    }

    #[test]
    fn partial_values_take_defaults() {
        let config = ResizeConfig::from_json(&json!({"width": 512})).unwrap();
        assert_eq!(config.width.get(), 512);
        assert_eq!(config.height.get(), 200);
        assert_eq!(config.quality.webp, 80);

        let config = ResizeConfig::from_json(&json!({"quality": {"jpeg": 70}})).unwrap();
        assert_eq!(config.quality.jpeg, 70);
        assert_eq!(config.quality.webp, 80);
    }

    #[test]
    fn json_round_trips() {
        let custom = ResizeConfig {
            width: NonZeroU32::new(300).unwrap(),
            height: NonZeroU32::new(150).unwrap(),
            strategy: ResizeStrategy::CropToFit,
            quality: QualityConfig { jpeg: 70, webp: 60 },
        };
        for config in [ResizeConfig::default(), custom] {
            assert_eq!(ResizeConfig::from_json(&config.to_json()).unwrap(), config);
        }
    }

    #[test]
    fn default_json_is_the_documented_shape() {
        assert_eq!(
            ResizeConfig::default().to_json(),
            json!({"width":200,"height":200,"strategy":"scale-to-fit","quality":{"jpeg":85,"webp":80}})
        );
    }

    #[test]
    fn invalid_values_are_rejected() {
        for value in [
            json!({"stratgy": "crop-to-fit"}),
            json!({"strategy": "crop"}),
            json!({"width": 0}),
            json!({"quality": {"webp": 101}}),
            json!({"quality": {"jpeg": 0}}),
        ] {
            let err = ResizeConfig::from_json(&value).unwrap_err();
            assert!(
                err.starts_with("invalid image-resize config"),
                "{value}: {err}"
            );
        }
    }

    #[test]
    fn schema_allows_partial_values_and_rejects_unknown_keys() {
        let schema = ResizeConfig::json_schema();
        assert!(schema.get("required").is_none(), "{schema:#}");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["width"]["minimum"], 1.0);

        let quality = &schema["definitions"]["QualityConfig"];
        assert!(quality.get("required").is_none(), "{quality:#}");
        assert_eq!(quality["additionalProperties"], false);
        assert_eq!(quality["properties"]["jpeg"]["minimum"], 1.0);
        assert_eq!(quality["properties"]["jpeg"]["maximum"], 100.0);

        assert_eq!(
            schema["definitions"]["ResizeStrategy"]["enum"],
            json!(["scale-to-fit", "crop-to-fit"])
        );
        assert_eq!(schema["example"], ResizeConfig::default().to_json());
    }

    #[test]
    fn seed_file_is_read_and_a_bad_one_is_ignored() {
        // CARGO_TARGET_TMPDIR exists only for integration tests.
        let dir = std::env::temp_dir().join(format!("image-resize-seed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, yaml: &str| {
            let path = dir.join(name);
            std::fs::write(&path, yaml).unwrap();
            path.to_str().unwrap().to_owned()
        };

        let seed = load_seed(&write("seed-ok.yaml", "width: 320\n")).unwrap();
        assert_eq!(seed.width.get(), 320);
        assert_eq!(seed.height.get(), 200);

        assert!(load_seed(dir.join("seed-missing.yaml").to_str().unwrap()).is_none());
        assert!(load_seed(&write("seed-strategy.yaml", "strategy: crop\n")).is_none());
        assert!(load_seed(&write("seed-zero.yaml", "width: 0\n")).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn check_quality_bounds() {
        for bad in [0, 101, 255] {
            assert!(check_quality(bad).is_err(), "{bad}");
        }
        for good in [1, 50, 100] {
            assert_eq!(check_quality(good).unwrap(), good);
        }
    }
}
