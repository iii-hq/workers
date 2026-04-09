use anyhow::Result;
use regex::Regex;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct PiiPatternDef {
    pub name: String,
    pub pattern: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GuardrailsConfig {
    #[serde(default)]
    pub pii_patterns: Vec<PiiPatternDef>,
    #[serde(default)]
    pub injection_keywords: Vec<String>,
    #[serde(default = "default_max_input_length")]
    pub max_input_length: usize,
    #[serde(default = "default_max_output_length")]
    pub max_output_length: usize,
}

fn default_max_input_length() -> usize {
    50000
}

fn default_max_output_length() -> usize {
    100000
}

fn default_pii_patterns() -> Vec<PiiPatternDef> {
    vec![
        PiiPatternDef {
            name: "email".into(),
            pattern: r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}".into(),
        },
        PiiPatternDef {
            name: "phone".into(),
            pattern: r"\b\d{3}[-.]?\d{3}[-.]?\d{4}\b".into(),
        },
        PiiPatternDef {
            name: "ssn".into(),
            pattern: r"\b\d{3}-\d{2}-\d{4}\b".into(),
        },
        PiiPatternDef {
            name: "credit_card".into(),
            pattern: r"\b\d{4}[- ]?\d{4}[- ]?\d{4}[- ]?\d{4}\b".into(),
        },
        PiiPatternDef {
            name: "ip_address".into(),
            pattern: r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b".into(),
        },
    ]
}

fn default_injection_keywords() -> Vec<String> {
    vec![
        "ignore previous instructions".into(),
        "ignore all instructions".into(),
        "disregard the above".into(),
        "you are now".into(),
        "pretend you are".into(),
        "act as if".into(),
        "system prompt".into(),
        "reveal your instructions".into(),
        "what are your rules".into(),
    ]
}

impl Default for GuardrailsConfig {
    fn default() -> Self {
        GuardrailsConfig {
            pii_patterns: default_pii_patterns(),
            injection_keywords: default_injection_keywords(),
            max_input_length: default_max_input_length(),
            max_output_length: default_max_output_length(),
        }
    }
}

impl GuardrailsConfig {
    pub fn compile_pii_patterns(&self) -> Vec<(String, Regex)> {
        self.pii_patterns
            .iter()
            .filter_map(|p| {
                Regex::new(&p.pattern)
                    .ok()
                    .map(|re| (p.name.clone(), re))
            })
            .collect()
    }
}

pub fn load_config(path: &str) -> Result<GuardrailsConfig> {
    let contents = std::fs::read_to_string(path)?;
    let config: GuardrailsConfig = serde_yaml::from_str(&contents)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = GuardrailsConfig::default();
        assert_eq!(config.max_input_length, 50000);
        assert_eq!(config.max_output_length, 100000);
        assert_eq!(config.pii_patterns.len(), 5);
        assert_eq!(config.injection_keywords.len(), 9);
    }

    #[test]
    fn test_config_custom() {
        let yaml = r#"
pii_patterns:
  - name: "email"
    pattern: "[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}"
injection_keywords:
  - "ignore previous instructions"
max_input_length: 10000
max_output_length: 20000
"#;
        let config: GuardrailsConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.pii_patterns.len(), 1);
        assert_eq!(config.pii_patterns[0].name, "email");
        assert_eq!(config.injection_keywords.len(), 1);
        assert_eq!(config.max_input_length, 10000);
        assert_eq!(config.max_output_length, 20000);
    }

    #[test]
    fn test_compile_pii_patterns() {
        let config = GuardrailsConfig {
            pii_patterns: vec![
                PiiPatternDef {
                    name: "email".to_string(),
                    pattern: r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}".to_string(),
                },
                PiiPatternDef {
                    name: "bad_regex".to_string(),
                    pattern: r"[invalid".to_string(),
                },
            ],
            injection_keywords: vec![],
            max_input_length: 50000,
            max_output_length: 100000,
        };
        let compiled = config.compile_pii_patterns();
        assert_eq!(compiled.len(), 1);
        assert_eq!(compiled[0].0, "email");
    }

    #[test]
    fn test_default_impl() {
        let config = GuardrailsConfig::default();
        assert_eq!(config.max_input_length, 50000);
        assert_eq!(config.max_output_length, 100000);
        assert_eq!(config.pii_patterns.len(), 5);
        assert_eq!(config.pii_patterns[0].name, "email");
    }
}
