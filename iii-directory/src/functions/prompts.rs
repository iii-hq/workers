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
    /// Bundled with the worker, no file behind it: editing it creates the
    /// local file (which then shadows this entry); there is nothing to
    /// delete.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    builtin: bool,
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
    /// Served from the copy bundled with the worker (no local file yet):
    /// an update creates the local file.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub builtin: bool,
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
                let mut entries: Vec<SystemPromptEntry> = prompts
                    .into_iter()
                    .map(|p| SystemPromptEntry {
                        modified_at: fs_modified_at(&p.abs_path),
                        name: p.name,
                        description: p.description,
                        builtin: false,
                    })
                    .collect();
                // Bundled prompts are always visible; a local file with the
                // same name shadows its bundled copy.
                for bundled in crate::bundled::bundled_system_prompts() {
                    if !entries.iter().any(|entry| entry.name == bundled.name) {
                        entries.push(SystemPromptEntry {
                            name: bundled.name.to_string(),
                            description: bundled.description,
                            modified_at: String::new(),
                            builtin: true,
                        });
                    }
                }
                entries.sort_by(|a, b| a.name.cmp(&b.name));
                Ok::<_, Error>(ListSystemPromptsOutput { prompts: entries })
            }
        })
        .description("List system prompts (name, description, modified_at): filesystem-backed entries from skills_folder (`system-prompts/` path segment) plus the prompts bundled with this worker. A bundled prompt with no local file carries `builtin: true` — editing it via directory::system-prompts::update creates the local file, which then shadows the bundled copy."),
    );

    let cfg_inner = cfg.clone();
    iii.register_function(
        "directory::system-prompts::get",
        RegisterFunction::new_async(move |req: SystemPromptGetInput| {
            let cfg = cfg_inner.load_full();
            async move { get_system_prompt(&cfg, req).await.map_err(Error::Handler) }
        })
        .description("Fetch one system prompt by name — a filesystem-backed entry, or a worker-bundled one (`builtin: true`) when no local file shadows it. Returns the raw markdown body plus name, description, and modified_at — no envelope, no templating."),
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
        // No local file — a bundled copy still serves (and `raw` round-trips
        // the full file form an update would copy-on-write to disk).
        if let Some(bundled) = crate::bundled::bundled_system_prompt(&req.name) {
            return Ok(SystemPromptGetOutput {
                name: bundled.name.to_string(),
                description: bundled.description,
                body: bundled.body,
                raw: req.raw.filter(|raw| *raw).map(|_| bundled.raw.to_string()),
                modified_at: String::new(),
                builtin: true,
            });
        }
        let mut names: Vec<String> = prompts.into_iter().map(|p| p.name).collect();
        names.extend(crate::bundled::bundled_system_prompts().map(|b| b.name.to_string()));
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
        builtin: false,
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
    async fn get_serves_the_bundled_prompt_until_a_local_file_shadows_it() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = SkillsConfig {
            skills_folder: tmp.path().to_string_lossy().into_owned(),
            local_skills_folder: tmp.path().join("local").to_string_lossy().into_owned(),
            ..SkillsConfig::default()
        };
        // No file: the bundled copy serves, raw round-trips the full form.
        let out = get_system_prompt(
            &cfg,
            SystemPromptGetInput {
                name: "iii-minimal".into(),
                raw: Some(true),
            },
        )
        .await
        .unwrap();
        assert!(out.builtin);
        assert!(out.body.starts_with("You are an iii agent."));
        assert!(out.raw.unwrap().starts_with("---\n"));

        // A local file with the same name shadows the bundled copy.
        std::fs::create_dir_all(tmp.path().join("system-prompts")).unwrap();
        std::fs::write(
            tmp.path().join("system-prompts/iii-minimal.md"),
            "---\ndescription: mine\n---\nShadowed.\n",
        )
        .unwrap();
        let out = get_system_prompt(
            &cfg,
            SystemPromptGetInput {
                name: "iii-minimal".into(),
                raw: None,
            },
        )
        .await
        .unwrap();
        assert!(!out.builtin);
        assert_eq!(out.body.trim(), "Shadowed.");
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
