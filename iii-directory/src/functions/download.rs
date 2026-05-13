//! `skills::download` — pull markdown into `skills_folder` from either
//! the workers registry (`worker=` source) or a GitHub repo (`repo=`
//! source).
//!
//! The function is the only write path in the worker. It validates the
//! incoming arguments, dispatches to the matching source module under
//! [`crate::sources`], and fires the `skills::on-change` /
//! `prompts::on-change` triggers on success so that subscribers (the
//! `mcp` worker today) can forward MCP `notifications/*_list_changed`
//! to their clients.

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::SkillsConfig;
use crate::sources::{self, registry::VersionSpec, DownloadResult};
use crate::trigger_types::{self, SubscriberSet};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct DownloadInput {
    /// Source A: GitHub repo URL. Pair with `skill`.
    #[serde(default)]
    pub repo: Option<String>,
    /// Source A: subfolder under `skills/` inside the repo. Doubles as
    /// the destination namespace inside `skills_folder`.
    #[serde(default)]
    pub skill: Option<String>,

    /// Source B: workers registry name. Pair with exactly one of
    /// `version` / `tag`.
    #[serde(default)]
    pub worker: Option<String>,
    /// Source B: explicit semver to pull. Mutually exclusive with `tag`.
    #[serde(default)]
    pub version: Option<String>,
    /// Source B: registry tag to pull (e.g. `latest`). Mutually
    /// exclusive with `version`.
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct DownloadOutput {
    namespace: String,
    skills_written: Vec<String>,
    prompts_written: Vec<String>,
    source: Value,
}

/// Disambiguated input shape produced by [`classify_input`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifiedInput {
    Repo { repo: String, skill: String },
    Registry { worker: String, spec: VersionSpec },
}

pub fn register(iii: &Arc<III>, cfg: &Arc<SkillsConfig>, subscribers: &super::Subscribers) {
    let iii_inner = iii.clone();
    let cfg_inner = cfg.clone();
    let skills_subs = subscribers.skills.clone();
    let prompts_subs = subscribers.prompts.clone();
    iii.register_function(
        RegisterFunction::new_async("skills::download", move |req: DownloadInput| {
            let iii = iii_inner.clone();
            let cfg = cfg_inner.clone();
            let skills_subs = skills_subs.clone();
            let prompts_subs = prompts_subs.clone();
            async move {
                let classified = classify_input(req).map_err(IIIError::Handler)?;
                let result = run_download(&cfg, &classified)
                    .await
                    .map_err(IIIError::Handler)?;
                fan_out(&iii, &skills_subs, &prompts_subs, &classified, &result).await;
                Ok::<_, IIIError>(build_output(&classified, result))
            }
        })
        .description(
            "Download skills + prompts into skills_folder. \
             Pass {repo, skill} to clone a single skill folder from a GitHub repo \
             (git clone --depth 1), or {worker, version|tag} to pull from the workers registry. \
             Files in the destination namespace are overwritten file-by-file.",
        )
        .metadata(json!({"tool": {"label": "Download skills"}})),
    );
}

/// Validate the incoming arguments and pick exactly one source. Public
/// so the pure validation can be unit-tested without the engine.
pub fn classify_input(input: DownloadInput) -> Result<ClassifiedInput, String> {
    let DownloadInput {
        repo,
        skill,
        worker,
        version,
        tag,
    } = input;

    let repo = repo.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let skill = skill
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let worker = worker
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let version = version
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let tag = tag.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

    let has_repo = repo.is_some() || skill.is_some();
    let has_worker = worker.is_some() || version.is_some() || tag.is_some();

    if has_repo && has_worker {
        return Err("specify either {repo, skill} OR {worker, version|tag}, not both".into());
    }
    if !has_repo && !has_worker {
        return Err("specify either {repo, skill} OR {worker, version|tag}".into());
    }

    if has_repo {
        let repo = repo.ok_or_else(|| "repo is required when skill is set".to_string())?;
        let skill = skill.ok_or_else(|| "skill is required when repo is set".to_string())?;
        return Ok(ClassifiedInput::Repo { repo, skill });
    }

    let worker =
        worker.ok_or_else(|| "worker is required when version or tag is set".to_string())?;
    let spec = match (version, tag) {
        (Some(v), None) => VersionSpec::Version(v),
        (None, Some(t)) => VersionSpec::Tag(t),
        (Some(_), Some(_)) => return Err("specify either version OR tag, not both".into()),
        (None, None) => return Err("worker requires either version or tag".into()),
    };
    Ok(ClassifiedInput::Registry { worker, spec })
}

async fn run_download(
    cfg: &SkillsConfig,
    classified: &ClassifiedInput,
) -> Result<DownloadResult, String> {
    let folder = cfg.resolved_skills_folder();
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("create_dir_all {}: {e}", folder.display()))?;

    match classified {
        ClassifiedInput::Repo { repo, skill } => {
            sources::git::download(repo, skill, &folder, cfg.download_timeout_ms).await
        }
        ClassifiedInput::Registry { worker, spec } => {
            sources::registry::download(
                cfg.registry_base(),
                worker,
                spec,
                &folder,
                cfg.download_timeout_ms,
            )
            .await
        }
    }
}

fn build_output(classified: &ClassifiedInput, result: DownloadResult) -> DownloadOutput {
    let source = match classified {
        ClassifiedInput::Repo { repo, skill } => json!({
            "kind": "repo",
            "repo": repo,
            "skill": skill,
        }),
        ClassifiedInput::Registry { worker, spec } => match spec {
            VersionSpec::Version(v) => json!({
                "kind": "registry",
                "worker": worker,
                "version": v,
            }),
            VersionSpec::Tag(t) => json!({
                "kind": "registry",
                "worker": worker,
                "tag": t,
            }),
        },
    };
    DownloadOutput {
        namespace: result.namespace,
        skills_written: result.skills_written,
        prompts_written: result.prompts_written,
        source,
    }
}

/// Fan out to subscribers. We fire `skills::on-change` only when at
/// least one skill was written (and likewise for prompts) so noisy
/// no-op downloads don't churn MCP `notifications/list_changed`.
async fn fan_out(
    iii: &III,
    skills_subs: &SubscriberSet,
    prompts_subs: &SubscriberSet,
    classified: &ClassifiedInput,
    result: &DownloadResult,
) {
    let payload = json!({
        "op": "download",
        "namespace": result.namespace,
        "source": match classified {
            ClassifiedInput::Repo { .. } => "repo",
            ClassifiedInput::Registry { .. } => "registry",
        },
    });
    if !result.skills_written.is_empty() {
        trigger_types::dispatch(iii, skills_subs, payload.clone()).await;
    }
    if !result.prompts_written.is_empty() {
        trigger_types::dispatch(iii, prompts_subs, payload).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_rejects_empty_input() {
        let err = classify_input(DownloadInput::default()).unwrap_err();
        assert!(err.contains("specify either"), "got: {err}");
    }

    #[test]
    fn classify_rejects_both_sources() {
        let err = classify_input(DownloadInput {
            repo: Some("https://github.com/x/y".into()),
            skill: Some("foo".into()),
            worker: Some("z".into()),
            tag: Some("latest".into()),
            ..DownloadInput::default()
        })
        .unwrap_err();
        assert!(err.contains("not both"), "got: {err}");
    }

    #[test]
    fn classify_rejects_repo_without_skill() {
        let err = classify_input(DownloadInput {
            repo: Some("https://github.com/x/y".into()),
            ..DownloadInput::default()
        })
        .unwrap_err();
        assert!(err.contains("skill"), "got: {err}");
    }

    #[test]
    fn classify_rejects_skill_without_repo() {
        let err = classify_input(DownloadInput {
            skill: Some("foo".into()),
            ..DownloadInput::default()
        })
        .unwrap_err();
        assert!(err.contains("repo"), "got: {err}");
    }

    #[test]
    fn classify_rejects_worker_without_version_or_tag() {
        let err = classify_input(DownloadInput {
            worker: Some("resend".into()),
            ..DownloadInput::default()
        })
        .unwrap_err();
        assert!(err.contains("version or tag"), "got: {err}");
    }

    #[test]
    fn classify_rejects_both_version_and_tag() {
        let err = classify_input(DownloadInput {
            worker: Some("resend".into()),
            version: Some("1.2.3".into()),
            tag: Some("latest".into()),
            ..DownloadInput::default()
        })
        .unwrap_err();
        assert!(err.contains("either version OR tag"), "got: {err}");
    }

    #[test]
    fn classify_accepts_repo_form() {
        let c = classify_input(DownloadInput {
            repo: Some("https://github.com/anthropics/skills".into()),
            skill: Some("frontend-design".into()),
            ..DownloadInput::default()
        })
        .unwrap();
        assert_eq!(
            c,
            ClassifiedInput::Repo {
                repo: "https://github.com/anthropics/skills".into(),
                skill: "frontend-design".into(),
            }
        );
    }

    #[test]
    fn classify_accepts_registry_with_version() {
        let c = classify_input(DownloadInput {
            worker: Some("resend".into()),
            version: Some("1.2.3".into()),
            ..DownloadInput::default()
        })
        .unwrap();
        assert_eq!(
            c,
            ClassifiedInput::Registry {
                worker: "resend".into(),
                spec: VersionSpec::Version("1.2.3".into()),
            }
        );
    }

    #[test]
    fn classify_accepts_registry_with_tag() {
        let c = classify_input(DownloadInput {
            worker: Some("agent-memory".into()),
            tag: Some("latest".into()),
            ..DownloadInput::default()
        })
        .unwrap();
        assert_eq!(
            c,
            ClassifiedInput::Registry {
                worker: "agent-memory".into(),
                spec: VersionSpec::Tag("latest".into()),
            }
        );
    }

    #[test]
    fn classify_trims_whitespace() {
        let c = classify_input(DownloadInput {
            worker: Some("  resend  ".into()),
            tag: Some("latest\n".into()),
            ..DownloadInput::default()
        })
        .unwrap();
        assert_eq!(
            c,
            ClassifiedInput::Registry {
                worker: "resend".into(),
                spec: VersionSpec::Tag("latest".into()),
            }
        );
    }

    #[test]
    fn build_output_includes_repo_source() {
        let mut result = DownloadResult::new("foo");
        result.skills_written.push("foo/bar.md".into());
        let classified = ClassifiedInput::Repo {
            repo: "https://github.com/x/y".into(),
            skill: "foo".into(),
        };
        let out = build_output(&classified, result);
        assert_eq!(out.namespace, "foo");
        assert_eq!(out.source["kind"], "repo");
        assert_eq!(out.source["repo"], "https://github.com/x/y");
        assert_eq!(out.source["skill"], "foo");
    }

    #[test]
    fn build_output_includes_registry_source_with_tag() {
        let result = DownloadResult::new("resend");
        let classified = ClassifiedInput::Registry {
            worker: "resend".into(),
            spec: VersionSpec::Tag("latest".into()),
        };
        let out = build_output(&classified, result);
        assert_eq!(out.source["kind"], "registry");
        assert_eq!(out.source["worker"], "resend");
        assert_eq!(out.source["tag"], "latest");
        assert!(out.source.get("version").is_none());
    }

    #[test]
    fn build_output_includes_registry_source_with_version() {
        let result = DownloadResult::new("resend");
        let classified = ClassifiedInput::Registry {
            worker: "resend".into(),
            spec: VersionSpec::Version("1.2.3".into()),
        };
        let out = build_output(&classified, result);
        assert_eq!(out.source["version"], "1.2.3");
        assert!(out.source.get("tag").is_none());
    }
}
