use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Core harness loop workers — shown first in the dashboard and started by `workers-dev up`.
pub const HARNESS_STACK: &[&str] = &[
    "session-manager",
    "llm-router",
    "context-manager",
    "provider-anthropic",
    "provider-openai",
    "approval-gate",
    "harness",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkerGroup {
    HarnessStack,
    Other,
}

impl WorkerGroup {
    pub fn label(self) -> &'static str {
        match self {
            Self::HarnessStack => "harness stack",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone)]
pub enum SpawnKind {
    CargoRun,
    Unsupported { reason: String },
}

#[derive(Debug, Clone)]
pub struct WorkerSpec {
    pub name: String,
    pub dir: PathBuf,
    pub group: WorkerGroup,
    pub spawn: SpawnKind,
}

#[derive(Debug, Deserialize)]
struct WorkerYaml {
    name: Option<String>,
    language: Option<String>,
    deploy: Option<String>,
}

pub fn discover_repo_workers(repo_root: &Path) -> Result<Vec<WorkerSpec>> {
    let mut specs = Vec::new();
    for entry in fs::read_dir(repo_root)
        .with_context(|| format!("read repo root {}", repo_root.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path();
        let folder = entry.file_name().to_string_lossy().into_owned();
        if folder.starts_with('.') {
            continue;
        }
        let yaml_path = dir.join("iii.worker.yaml");
        if !yaml_path.is_file() {
            continue;
        }

        let raw = fs::read_to_string(&yaml_path)
            .with_context(|| format!("read {}", yaml_path.display()))?;
        let parsed: WorkerYaml = serde_yaml::from_str(&raw)
            .with_context(|| format!("parse {}", yaml_path.display()))?;
        let name = parsed.name.clone().unwrap_or(folder.clone());
        if name != folder {
            eprintln!(
                "warning: skipping worker folder {folder}: iii.worker.yaml name={name} (mismatch)"
            );
            continue;
        }

        let group = if HARNESS_STACK.contains(&name.as_str()) {
            WorkerGroup::HarnessStack
        } else {
            WorkerGroup::Other
        };

        let spawn = classify_spawn(&dir, &parsed);
        specs.push(WorkerSpec {
            name,
            dir,
            group,
            spawn,
        });
    }

    specs.sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.name.cmp(&b.name)));
    Ok(specs)
}

fn classify_spawn(dir: &Path, yaml: &WorkerYaml) -> SpawnKind {
    let language = yaml.language.as_deref().unwrap_or("");
    let deploy = yaml.deploy.as_deref().unwrap_or("");
    if language == "rust" && deploy == "binary" && dir.join("Cargo.toml").is_file() {
        SpawnKind::CargoRun
    } else {
        SpawnKind::Unsupported {
            reason: format!("{language}/{deploy} (use iii worker add for non-Rust workers)"),
        }
    }
}

pub fn order_worker_names(specs: &[WorkerSpec]) -> Vec<String> {
    specs.iter().map(|s| s.name.clone()).collect()
}

pub fn harness_stack_names(specs: &[WorkerSpec]) -> Vec<String> {
    HARNESS_STACK
        .iter()
        .filter_map(|name| {
            specs
                .iter()
                .find(|s| s.name == *name)
                .map(|s| s.name.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_worker(tmp: &TempDir, name: &str, language: &str, deploy: &str, with_cargo: bool) {
        let dir = tmp.path().join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("iii.worker.yaml"),
            format!(
                "iii: v1\nname: {name}\nlanguage: {language}\ndeploy: {deploy}\nmanifest: Cargo.toml\nbin: {name}\ndescription: test\n"
            ),
        )
        .unwrap();
        if with_cargo {
            fs::write(dir.join("Cargo.toml"), "[workspace]\n").unwrap();
        }
    }

    #[test]
    fn discovers_and_groups_workers() {
        let tmp = TempDir::new().unwrap();
        write_worker(&tmp, "harness", "rust", "binary", true);
        write_worker(&tmp, "telegram-bot", "rust", "binary", true);
        write_worker(&tmp, "claude-code", "javascript", "bundle", false);

        let specs = discover_repo_workers(tmp.path()).unwrap();
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].name, "harness");
        assert_eq!(specs[0].group, WorkerGroup::HarnessStack);
        assert!(matches!(specs[0].spawn, SpawnKind::CargoRun));
        assert_eq!(specs[2].name, "telegram-bot");
        assert_eq!(specs[2].group, WorkerGroup::Other);
        assert!(matches!(
            specs.iter().find(|s| s.name == "claude-code").unwrap().spawn,
            SpawnKind::Unsupported { .. }
        ));
    }
}
