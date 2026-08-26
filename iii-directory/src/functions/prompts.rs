//! Filesystem-backed system-prompt reader.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{SharedConfig, SkillsConfig};
use crate::fs_source;
use crate::functions::error::{not_found_message, NextAction};

const NAME_MAX_LEN: usize = 64;
const SYSTEM_PROMPT_NOT_FOUND_NEXT: &[NextAction] = &[NextAction::new(
    "directory::system-prompts::list",
    "browse system prompt names",
)];

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct ListSystemPromptsInput {}

#[derive(Debug, Serialize, JsonSchema)]
struct SystemPromptEntry {
    name: String,
    description: String,
    modified_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
struct ListSystemPromptsOutput {
    prompts: Vec<SystemPromptEntry>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SystemPromptGetInput {
    pub name: String,
    /// When `true`, the response includes the full on-disk file content
    /// (frontmatter included) as `raw`, ready for
    /// `directory::system-prompts::update`.
    #[serde(default)]
    pub raw: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SystemPromptGetOutput {
    pub name: String,
    pub description: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    pub modified_at: String,
}

pub fn register(iii: &Arc<IIIClient>, cfg: &SharedConfig) {
    let cfg_inner = cfg.clone();
    iii.register_function(
        "directory::system-prompts::list",
        RegisterFunction::new_async(move |_input: ListSystemPromptsInput| {
            let cfg = cfg_inner.load_full();
            async move {
                let (prompts, _skipped) = fs_source::scan_system_prompts_merged(
                    &cfg.resolved_skills_folder(),
                    &cfg.local_skills_folder(),
                );
                Ok::<_, Error>(ListSystemPromptsOutput {
                    prompts: prompts
                        .into_iter()
                        .map(|p| SystemPromptEntry {
                            modified_at: fs_modified_at(&p.abs_path),
                            name: p.name,
                            description: p.description,
                        })
                        .collect(),
                })
            }
        })
        .description("List filesystem-backed system prompts (name, description, modified_at) from skills_folder (`system-prompts/` path segment)."),
    );

    let cfg_inner = cfg.clone();
    iii.register_function(
        "directory::system-prompts::get",
        RegisterFunction::new_async(move |req: SystemPromptGetInput| {
            let cfg = cfg_inner.load_full();
            async move { get_system_prompt(&cfg, req).await.map_err(Error::Handler) }
        })
        .description("Fetch one filesystem-backed system prompt by name. Returns the raw markdown body plus name, description, and modified_at — no envelope, no templating."),
    );
}

pub async fn get_system_prompt(
    cfg: &SkillsConfig,
    req: SystemPromptGetInput,
) -> Result<SystemPromptGetOutput, String> {
    validate_name(&req.name)?;
    let (prompts, _skipped) = fs_source::scan_system_prompts_merged(
        &cfg.resolved_skills_folder(),
        &cfg.local_skills_folder(),
    );
    let Some(fs) = prompts.iter().find(|p| p.name == req.name).cloned() else {
        let names: Vec<String> = prompts.into_iter().map(|p| p.name).collect();
        return Err(not_found_message(
            "D210",
            "system prompt",
            &req.name,
            &rank_prompt_names(&names, &req.name, 3),
            SYSTEM_PROMPT_NOT_FOUND_NEXT,
        ));
    };
    Ok(SystemPromptGetOutput {
        name: fs.name,
        description: fs.description,
        body: fs_source::read_body(&fs.abs_path)?,
        raw: req
            .raw
            .filter(|raw| *raw)
            .map(|_| fs_source::read_raw(&fs.abs_path))
            .transpose()?,
        modified_at: fs_modified_at(&fs.abs_path),
    })
}

fn rank_prompt_names(names: &[String], missed: &str, limit: usize) -> Vec<String> {
    let missed_lc = missed.to_lowercase();
    let mut scored: Vec<(usize, &String)> = names
        .iter()
        .map(|name| {
            (
                crate::functions::skills::levenshtein(&missed_lc, &name.to_lowercase()),
                name,
            )
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, name)| name.clone())
        .collect()
}

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name must be non-empty".into());
    }
    if name.len() > NAME_MAX_LEN {
        return Err(format!(
            "name too long ({} chars; max {NAME_MAX_LEN})",
            name.len()
        ));
    }
    if name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
    {
        Ok(())
    } else {
        Err(format!(
            "name may only contain lowercase ASCII letters, digits, '-' and '_': {name:?}"
        ))
    }
}

fn fs_modified_at(path: &std::path::Path) -> String {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation_rejects_bad_chars() {
        assert!(validate_name("send-email").is_ok());
        assert!(validate_name("Send-Email").is_err());
    }

    #[tokio::test]
    async fn get_system_prompt_reads_system_prompt() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("system-prompts")).unwrap();
        std::fs::write(
            tmp.path().join("system-prompts/hello.md"),
            "---\ndescription: sys\n---\nSystem body.\n",
        )
        .unwrap();
        let cfg = SkillsConfig {
            skills_folder: tmp.path().to_string_lossy().into_owned(),
            local_skills_folder: tmp.path().join("local").to_string_lossy().into_owned(),
            ..SkillsConfig::default()
        };
        assert_eq!(
            get_system_prompt(
                &cfg,
                SystemPromptGetInput {
                    name: "hello".into(),
                    raw: None
                }
            )
            .await
            .unwrap()
            .body
            .trim(),
            "System body."
        );
    }
}
