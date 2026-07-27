use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use harness::prompt::SystemPromptStrategy;
use harness::types::model::ThinkingLevel;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::EvalError;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct E2eSubjectV1 {
    pub schema_version: String,
    pub subject_id: String,
    pub model: String,
    pub provider: String,
    pub system_prompt_path: PathBuf,
    pub system_prompt_strategy: SystemPromptStrategy,
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(default)]
    pub provider_options: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone)]
pub struct ResolvedE2eSubjectV1 {
    pub subject_id: String,
    pub subject_sha256: String,
    pub system_prompt_sha256: String,
    pub model: String,
    pub provider: String,
    pub system_prompt: String,
    pub system_prompt_strategy: SystemPromptStrategy,
    pub thinking_level: Option<ThinkingLevel>,
    pub provider_options: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubjectArtifactV1 {
    pub schema_version: &'static str,
    pub subject_id: String,
    pub subject_sha256: String,
    pub system_prompt_sha256: String,
    pub model: String,
    pub provider: String,
    pub system_prompt_strategy: SystemPromptStrategy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<BTreeMap<String, Value>>,
}

impl ResolvedE2eSubjectV1 {
    pub fn artifact(&self) -> SubjectArtifactV1 {
        SubjectArtifactV1 {
            schema_version: "1",
            subject_id: self.subject_id.clone(),
            subject_sha256: self.subject_sha256.clone(),
            system_prompt_sha256: self.system_prompt_sha256.clone(),
            model: self.model.clone(),
            provider: self.provider.clone(),
            system_prompt_strategy: self.system_prompt_strategy,
            thinking_level: self.thinking_level,
            provider_options: self.provider_options.clone(),
        }
    }
}

pub fn load(path: &Path) -> Result<ResolvedE2eSubjectV1, EvalError> {
    let subject_bytes = std::fs::read(path)
        .map_err(|error| EvalError::setup(format!("read subject {}: {error}", path.display())))?;
    let subject: E2eSubjectV1 = serde_json::from_slice(&subject_bytes)
        .map_err(|error| EvalError::setup(format!("parse subject {}: {error}", path.display())))?;
    if subject.schema_version != "1" {
        return Err(EvalError::setup(format!(
            "unsupported subject schema_version {:?}; expected \"1\"",
            subject.schema_version
        )));
    }
    for (name, value) in [
        ("subject_id", subject.subject_id.as_str()),
        ("model", subject.model.as_str()),
        ("provider", subject.provider.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(EvalError::setup(format!("subject {name} cannot be empty")));
        }
    }

    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let prompt_path = if subject.system_prompt_path.is_absolute() {
        subject.system_prompt_path.clone()
    } else {
        base.join(&subject.system_prompt_path)
    };
    let prompt_bytes = std::fs::read(&prompt_path).map_err(|error| {
        EvalError::setup(format!(
            "read system prompt {}: {error}",
            prompt_path.display()
        ))
    })?;
    let system_prompt = String::from_utf8(prompt_bytes.clone()).map_err(|error| {
        EvalError::setup(format!(
            "system prompt {} is not UTF-8: {error}",
            prompt_path.display()
        ))
    })?;

    Ok(ResolvedE2eSubjectV1 {
        subject_id: subject.subject_id,
        subject_sha256: sha256(&subject_bytes),
        system_prompt_sha256: sha256(&prompt_bytes),
        model: subject.model,
        provider: subject.provider,
        system_prompt,
        system_prompt_strategy: subject.system_prompt_strategy,
        thinking_level: subject.thinking_level,
        provider_options: subject.provider_options,
    })
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn resolves_prompt_relative_to_subject_and_hashes_exact_bytes() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("prompt.md"), b"exact prompt\n").unwrap();
        fs::write(
            dir.path().join("subject.json"),
            br#"{
              "schema_version":"1",
              "subject_id":"baseline",
              "model":"model-v1",
              "provider":"provider",
              "system_prompt_path":"prompt.md",
              "system_prompt_strategy":"override"
            }"#,
        )
        .unwrap();

        let resolved = load(&dir.path().join("subject.json")).unwrap();
        assert_eq!(resolved.system_prompt, "exact prompt\n");
        assert_eq!(resolved.system_prompt_sha256, sha256(b"exact prompt\n"));
    }

    #[test]
    fn rejects_execution_limits_and_unknown_subject_fields() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("prompt.md"), "prompt").unwrap();
        fs::write(
            dir.path().join("subject.json"),
            r#"{
              "schema_version":"1",
              "subject_id":"baseline",
              "model":"model-v1",
              "provider":"provider",
              "system_prompt_path":"prompt.md",
              "system_prompt_strategy":"override",
              "max_output_tokens":42,
              "unexpected":true
            }"#,
        )
        .unwrap();

        let error = load(&dir.path().join("subject.json")).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }
}
