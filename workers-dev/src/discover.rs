use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Harness-stack *roots*: what `workers-dev up` / `Ctrl+u` start by name.
/// The dashboard's "harness stack" group is these plus everything they
/// transitively depend on — see `assign_groups`.
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
    /// Direct dependencies declared in iii.worker.yaml, filtered to workers
    /// that actually exist in this repo, sorted.
    pub deps: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct WorkerYaml {
    name: Option<String>,
    language: Option<String>,
    deploy: Option<String>,
    dependencies: Option<HashMap<String, String>>,
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
        let parsed: WorkerYaml =
            serde_yaml::from_str(&raw).with_context(|| format!("parse {}", yaml_path.display()))?;
        let name = parsed.name.clone().unwrap_or(folder.clone());
        if name != folder {
            eprintln!(
                "warning: skipping worker folder {folder}: iii.worker.yaml name={name} (mismatch)"
            );
            continue;
        }

        let spawn = classify_spawn(&dir, &parsed);
        let deps: Vec<String> = parsed
            .dependencies
            .unwrap_or_default()
            .into_keys()
            .collect();
        specs.push(WorkerSpec {
            name,
            dir,
            group: WorkerGroup::Other, // assigned below once all names are known
            spawn,
            deps,
        });
    }

    // Deps can name registry workers that aren't in this repo — keep only the
    // ones we discovered, so the graph and grouping stay within managed workers.
    let names: HashSet<String> = specs.iter().map(|s| s.name.clone()).collect();
    for spec in &mut specs {
        spec.deps.retain(|d| names.contains(d));
        spec.deps.sort_unstable();
    }

    let default_roots: Vec<String> = HARNESS_STACK
        .iter()
        .filter(|n| names.contains(**n))
        .map(|n| n.to_string())
        .collect();
    assign_groups(&mut specs, &default_roots);
    Ok(specs)
}

/// Group workers by deriving the harness stack from the dependency graph:
/// the stack is `roots` plus everything they transitively depend on, so a
/// newly declared dependency joins the stack group automatically instead of
/// waiting for someone to extend a hardcoded list. Re-sorts specs into
/// display order (stack first, alphabetical within a group).
pub fn assign_groups(specs: &mut [WorkerSpec], roots: &[String]) {
    let deps_by_name: HashMap<&str, &[String]> = specs
        .iter()
        .map(|s| (s.name.as_str(), s.deps.as_slice()))
        .collect();
    let mut stack: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<&str> = roots.iter().map(String::as_str).collect();
    while let Some(name) = queue.pop_front() {
        if !stack.insert(name) {
            continue;
        }
        if let Some(deps) = deps_by_name.get(name) {
            queue.extend(deps.iter().map(String::as_str));
        }
    }
    let stack: HashSet<String> = stack.into_iter().map(str::to_string).collect();

    for spec in specs.iter_mut() {
        spec.group = if stack.contains(&spec.name) {
            WorkerGroup::HarnessStack
        } else {
            WorkerGroup::Other
        };
    }
    specs.sort_by(|a, b| a.group.cmp(&b.group).then_with(|| a.name.cmp(&b.name)));
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

    fn write_worker_with_deps(tmp: &TempDir, name: &str, deps: &[&str]) {
        let dir = tmp.path().join(name);
        fs::create_dir_all(&dir).unwrap();
        let mut yaml =
            format!("iii: v1\nname: {name}\nlanguage: rust\ndeploy: binary\ndescription: test\n");
        if !deps.is_empty() {
            yaml.push_str("dependencies:\n");
            for dep in deps {
                yaml.push_str(&format!("  {dep}: \"^1.0.0\"\n"));
            }
        }
        fs::write(dir.join("iii.worker.yaml"), yaml).unwrap();
        fs::write(dir.join("Cargo.toml"), "[workspace]\n").unwrap();
    }

    /// The stack group is derived from the graph: roots plus transitive deps.
    /// A dependency two hops from `harness` lands in the stack group without
    /// appearing in any hardcoded list; unmanaged dep names are dropped.
    #[test]
    fn harness_stack_group_follows_dependencies() {
        let tmp = TempDir::new().unwrap();
        write_worker_with_deps(&tmp, "harness", &["state", "configuration"]);
        write_worker_with_deps(&tmp, "state", &["iii-directory"]);
        write_worker_with_deps(&tmp, "iii-directory", &[]);
        write_worker_with_deps(&tmp, "telegram-bot", &[]);

        let specs = discover_repo_workers(tmp.path()).unwrap();
        let group = |n: &str| specs.iter().find(|s| s.name == n).unwrap().group;
        assert_eq!(group("harness"), WorkerGroup::HarnessStack);
        assert_eq!(group("state"), WorkerGroup::HarnessStack);
        assert_eq!(group("iii-directory"), WorkerGroup::HarnessStack);
        assert_eq!(group("telegram-bot"), WorkerGroup::Other);
        // `configuration` isn't a repo worker — dropped from the spec's deps.
        assert_eq!(
            specs.iter().find(|s| s.name == "harness").unwrap().deps,
            vec!["state".to_string()]
        );
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
            specs
                .iter()
                .find(|s| s.name == "claude-code")
                .unwrap()
                .spawn,
            SpawnKind::Unsupported { .. }
        ));
    }
}
