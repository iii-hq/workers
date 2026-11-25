use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug, Clone)]
pub struct ResizeConfig {
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default)]
    pub strategy: ResizeStrategy,
    #[serde(default)]
    pub quality: QualityConfig,
}

fn default_width() -> u32 {
    200
}
fn default_height() -> u32 {
    200
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ResizeStrategy {
    #[default]
    ScaleToFit,
    CropToFit,
}

#[derive(Deserialize, Debug, Clone)]
pub struct QualityConfig {
    #[serde(default = "default_jpeg_quality")]
    pub jpeg: u8,
    #[serde(default = "default_webp_quality")]
    pub webp: u8,
}

fn default_jpeg_quality() -> u8 {
    85
}
fn default_webp_quality() -> u8 {
    80
}

impl Default for QualityConfig {
    fn default() -> Self {
        QualityConfig {
            jpeg: default_jpeg_quality(),
            webp: default_webp_quality(),
        }
    }
}

impl Default for ResizeConfig {
    fn default() -> Self {
        ResizeConfig {
            width: default_width(),
            height: default_height(),
            strategy: ResizeStrategy::default(),
            quality: QualityConfig::default(),
        }
    }
}

pub fn check_quality(q: u8) -> Result<u8> {
    anyhow::ensure!((1..=100).contains(&q), "quality must be 1-100, got {q}");
    Ok(q)
}

pub fn load_config(path: &str) -> Result<ResizeConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: ResizeConfig = serde_yaml_ng::from_str(&contents)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config: ResizeConfig = serde_yaml_ng::from_str("{}").unwrap();
        assert_eq!(config.width, 200);
        assert_eq!(config.height, 200);
        assert_eq!(config.strategy, ResizeStrategy::ScaleToFit);
        assert_eq!(config.quality.jpeg, 85);
        assert_eq!(config.quality.webp, 80);
    }

    #[test]
    fn test_config_custom() {
        let yaml = r#"
width: 300
height: 150
strategy: crop-to-fit
quality:
  jpeg: 70
  webp: 60
"#;
        let config: ResizeConfig = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.width, 300);
        assert_eq!(config.height, 150);
        assert_eq!(config.strategy, ResizeStrategy::CropToFit);
        assert_eq!(config.quality.jpeg, 70);
        assert_eq!(config.quality.webp, 60);
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

    #[test]
    fn test_resize_config_default() {
        let config = ResizeConfig::default();
        assert_eq!(config.width, 200);
        assert_eq!(config.height, 200);
        assert_eq!(config.strategy, ResizeStrategy::ScaleToFit);
        assert_eq!(config.quality.jpeg, 85);
        assert_eq!(config.quality.webp, 80);
    }
}
