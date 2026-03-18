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

fn default_width() -> u32 { 200 }
fn default_height() -> u32 { 200 }

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ResizeStrategy {
    ScaleToFit,
    CropToFit,
}

impl Default for ResizeStrategy {
    fn default() -> Self { ResizeStrategy::ScaleToFit }
}

#[derive(Deserialize, Debug, Clone)]
pub struct QualityConfig {
    #[serde(default = "default_jpeg_quality")]
    pub jpeg: u8,
    #[serde(default = "default_webp_quality")]
    pub webp: u8,
}

fn default_jpeg_quality() -> u8 { 85 }
fn default_webp_quality() -> u8 { 80 }

impl Default for QualityConfig {
    fn default() -> Self {
        QualityConfig { jpeg: default_jpeg_quality(), webp: default_webp_quality() }
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

pub fn load_config(path: &str) -> Result<ResizeConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: ResizeConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config: ResizeConfig = serde_yaml::from_str("{}").unwrap();
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
        let config: ResizeConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.width, 300);
        assert_eq!(config.height, 150);
        assert_eq!(config.strategy, ResizeStrategy::CropToFit);
        assert_eq!(config.quality.jpeg, 70);
        assert_eq!(config.quality.webp, 60);
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
