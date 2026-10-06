//! Scaffoldable worker templates. A template is scaffoldable when its
//! `template.yaml` carries a `worker:` block; this module reads and checks
//! those templates under a templates root (the directory holding the root
//! `template.yaml`, see `super::source`).

use std::path::{Component, Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::error::CoderError;

/// The `worker:` block of a template's `template.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerBlock {
    /// Subtree copied into the target directory, relative to the template
    /// dir. `WorkerTemplate.files` entries are relative to this.
    pub dir: String,
    /// Token replaced by the new worker name, in paths and text contents.
    pub name: String,
    /// Key under `containers:` in the template's `worker-compose.yaml`.
    pub compose: String,
    /// Compose containers the worker needs, e.g. `http`.
    #[serde(default)]
    pub requires: Vec<String>,
}

/// Runtime of a worker template, from the template's own `requires`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Node,
    Python,
}

/// A checked scaffoldable template.
#[derive(Debug, Clone)]
pub struct WorkerTemplate {
    /// Folder name under the templates root, e.g. `worker-node-ade`.
    pub id: String,
    pub name: String,
    pub description: String,
    pub language: Language,
    /// The `files:` entries under `worker.dir`, relative to `worker.dir`
    /// (prefix already stripped; not relative to `dir`), in manifest order,
    /// token not replaced. Source: `dir.join(&worker.dir).join(rel)`. Each
    /// one is a regular file that resolves (symlinks included) inside
    /// `worker.dir`. Contents are not read here: the scaffold rejects
    /// non-UTF-8 files (C234).
    pub files: Vec<String>,
    pub next_steps: Vec<String>,
    pub worker: WorkerBlock,
    /// Canonical template directory, `<root>/<id>`.
    pub dir: PathBuf,
}

/// The parts of a template's `template.yaml` the scaffold reads.
#[derive(Deserialize)]
struct TemplateManifest {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    next_steps: Vec<String>,
    worker: WorkerBlock,
}

/// Every scaffoldable template listed by `<root>/template.yaml`, in manifest
/// order, plus one `C234: template <id>: …` message per template that has a
/// `worker:` block but is invalid (logged and left out). Templates without
/// `worker:`, and listed ids without a `template.yaml` (upstream lists
/// `worker-bare` with no folder), are skipped silently.
pub fn load_worker_templates(
    root: &Path,
) -> Result<(Vec<WorkerTemplate>, Vec<String>), CoderError> {
    #[derive(Deserialize)]
    struct RootManifest {
        #[serde(default)]
        templates: Vec<String>,
    }
    let path = root.join("template.yaml");
    let manifest: RootManifest = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|raw| serde_yaml::from_str(&raw).map_err(|e| e.to_string()))
        .map_err(|e| {
            CoderError::InvalidTemplate(format!(
                "{}: {e}. Point code.templates.dir at a templates checkout (its root or its iii/ dir).",
                path.display()
            ))
        })?;
    let mut valid = Vec::new();
    let mut warnings = Vec::new();
    for id in &manifest.templates {
        match load_one(root, id) {
            Ok(Some(template)) => valid.push(template),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(template = %id, error = %e, "skipping invalid worker template");
                warnings.push(e.to_string());
            }
        }
    }
    Ok((valid, warnings))
}

fn load_one(root: &Path, id: &str) -> Result<Option<WorkerTemplate>, CoderError> {
    let invalid = |why: String| CoderError::InvalidTemplate(format!("template {id}: {why}"));
    if safe_rel(id).is_none_or(|p| p.components().count() != 1) {
        return Err(invalid(
            "the root manifest must list a plain folder name".into(),
        ));
    }
    let raw = match std::fs::read_to_string(root.join(id).join("template.yaml")) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(invalid(format!("template.yaml: {e}"))),
    };
    let value: serde_yaml::Value =
        serde_yaml::from_str(&raw).map_err(|e| invalid(format!("template.yaml: {e}")))?;
    if value.get("worker").is_none() {
        return Ok(None);
    }
    let m: TemplateManifest =
        serde_yaml::from_value(value).map_err(|e| invalid(format!("template.yaml: {e}")))?;

    let language = m
        .requires
        .iter()
        .find_map(|r| match r.as_str() {
            "typescript" | "javascript" => Some(Language::Node),
            "python" => Some(Language::Python),
            _ => None,
        })
        .ok_or_else(|| {
            invalid("requires names no language (typescript, javascript or python)".into())
        })?;
    validate_worker_name(&m.worker.name).map_err(|_| {
        invalid(format!(
            "worker.name {:?} is not a kebab-case name",
            m.worker.name
        ))
    })?;
    let dir = std::fs::canonicalize(root.join(id)).map_err(|e| invalid(e.to_string()))?;
    let worker_root = safe_rel(&m.worker.dir)
        .and_then(|rel| std::fs::canonicalize(dir.join(rel)).ok())
        .filter(|p| p.starts_with(&dir) && p.is_dir())
        .ok_or_else(|| {
            invalid(format!(
                "worker.dir {:?} must be a relative directory inside the template",
                m.worker.dir
            ))
        })?;

    let mut files = Vec::new();
    for f in &m.files {
        let path = safe_rel(f).ok_or_else(|| {
            invalid(format!(
                "files entry {f:?} must be a relative path without `..`"
            ))
        })?;
        // Entries outside worker.dir (README.md, worker-compose.yaml) belong
        // to the template's project, not to the worker.
        let Ok(rel) = path.strip_prefix(&m.worker.dir) else {
            continue;
        };
        let inside = std::fs::canonicalize(worker_root.join(rel))
            .is_ok_and(|p| p.starts_with(&worker_root) && p.is_file());
        if !inside {
            return Err(invalid(format!(
                "files entry {f:?} is missing, not a file, or a symlink leaving {}",
                m.worker.dir
            )));
        }
        files.push(rel.to_string_lossy().into_owned());
    }
    if files.is_empty() {
        return Err(invalid(format!(
            "no files: entries under worker.dir {:?}",
            m.worker.dir
        )));
    }
    let template = WorkerTemplate {
        id: id.to_string(),
        name: m.name,
        description: m.description,
        language,
        files,
        next_steps: m.next_steps,
        worker: m.worker,
        dir,
    };
    compose_entry(&template)?;
    Ok(Some(template))
}

/// `containers.<worker.compose>` from the template's `worker-compose.yaml`,
/// as written: the token is not replaced and `worker` is the template's own
/// `path://` value.
pub fn compose_entry(t: &WorkerTemplate) -> Result<serde_json::Value, CoderError> {
    let invalid = |why: String| CoderError::InvalidTemplate(format!("template {}: {why}", t.id));
    let raw = std::fs::read_to_string(t.dir.join("worker-compose.yaml"))
        .map_err(|e| invalid(format!("worker-compose.yaml: {e}")))?;
    let doc: serde_json::Value =
        serde_yaml::from_str(&raw).map_err(|e| invalid(format!("worker-compose.yaml: {e}")))?;
    doc.get("containers")
        .and_then(|containers| containers.get(&t.worker.compose))
        .filter(|entry| entry.is_object())
        .cloned()
        .ok_or_else(|| {
            invalid(format!(
                "worker-compose.yaml has no containers.{} entry (worker.compose)",
                t.worker.compose
            ))
        })
}

/// Enforce `^[a-z][a-z0-9]*(-[a-z0-9]+)*$`, 1–63 chars: the name is used as
/// is for the compose key, the function-id prefix, the CSS scope and the
/// HTTP prefix.
pub fn validate_worker_name(name: &str) -> Result<(), CoderError> {
    let ok = name.len() <= 63
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        return Ok(());
    }
    Err(CoderError::InvalidWorkerName(format!(
        "worker name {name:?} is invalid: use 1-63 lowercase letters, digits and single \
         hyphens, starting with a letter and not ending with a hyphen (e.g. \"my-worker\"). \
         Retry with a corrected name."
    )))
}

/// Replace every occurrence of a template's name token (kebab form only).
pub fn replace_token(text: &str, token: &str, name: &str) -> String {
    text.replace(token, name)
}

/// `p` as a path when it is non-empty and made only of plain components
/// (no `..`, no leading `.`, no root).
fn safe_rel(p: &str) -> Option<&Path> {
    let path = Path::new(p);
    let plain = path.components().all(|c| matches!(c, Component::Normal(_)));
    (!p.is_empty() && plain).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/templates/iii")
    }

    /// A one-template root: template `t`, worker dir `workers/w` holding
    /// `a.txt`, compose container `w`; `files` is written verbatim into the
    /// `files:` list.
    fn scratch(files: &[&str]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path().join("t");
        std::fs::create_dir_all(t.join("workers/w")).unwrap();
        std::fs::write(tmp.path().join("template.yaml"), "templates: [t]\n").unwrap();
        std::fs::write(t.join("workers/w/a.txt"), "w\n").unwrap();
        std::fs::write(
            t.join("worker-compose.yaml"),
            "containers:\n  w:\n    worker: path://./workers/w\n",
        )
        .unwrap();
        let list: String = files.iter().map(|f| format!("  - '{f}'\n")).collect();
        std::fs::write(
            t.join("template.yaml"),
            format!(
                "name: T\nrequires: [python]\nfiles:\n{list}worker:\n  dir: workers/w\n  name: w\n  compose: w\n"
            ),
        )
        .unwrap();
        tmp
    }

    /// The single C234 warning a scratch root produced; the template is left out.
    fn rejection(tmp: &tempfile::TempDir) -> String {
        let (valid, warnings) = load_worker_templates(tmp.path()).unwrap();
        assert!(valid.is_empty(), "the template should be rejected");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].starts_with("C234: template t:"),
            "{}",
            warnings[0]
        );
        warnings[0].clone()
    }

    #[test]
    fn loads_scaffoldable_templates_and_reports_invalid_ones() {
        let (valid, warnings) = load_worker_templates(&fixture_root()).unwrap();
        let ids: Vec<&str> = valid.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            ["worker-mini"],
            "plain (no worker:) and absent are skipped silently"
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].starts_with("C234: template broken:")
                && warnings[0].contains("containers.nope"),
            "{}",
            warnings[0]
        );

        let t = &valid[0];
        assert_eq!(t.name, "Mini worker");
        assert_eq!(
            t.description,
            "Fixture worker template for the ide template tests."
        );
        assert_eq!(t.language, Language::Node);
        assert_eq!(
            t.worker,
            WorkerBlock {
                dir: "workers/my-worker".into(),
                name: "my-worker".into(),
                compose: "my-worker".into(),
                requires: vec!["http".into()],
            }
        );
        assert_eq!(
            t.files,
            ["package.json", "src/index.ts", "scripts/dev.sh"],
            "only entries under worker.dir, relative to it"
        );
        for rel in &t.files {
            assert!(t.dir.join(&t.worker.dir).join(rel).is_file(), "{rel}");
        }
        assert_eq!(t.next_steps, ["Call it: iii trigger my-worker::hello"]);
        assert!(t.dir.is_absolute() && t.dir.ends_with("worker-mini"));
    }

    #[test]
    fn compose_entry_is_the_named_container_as_written() {
        let (valid, _) = load_worker_templates(&fixture_root()).unwrap();
        assert_eq!(
            compose_entry(&valid[0]).unwrap(),
            serde_json::json!({
                "worker": "path://./workers/my-worker",
                "scripts": { "pre_run": "pnpm install", "pre_run_timeout": "5m", "run": "pnpm dev" },
                "environment": { "MY_WORKER_GREETING": "hello from my-worker" }
            })
        );
    }

    #[test]
    fn listed_ids_without_a_template_yaml_are_skipped_silently() {
        // Upstream lists `worker-bare` with no folder; a folder without a
        // template.yaml is skipped the same way.
        let tmp = scratch(&["workers/w/a.txt"]);
        std::fs::write(
            tmp.path().join("template.yaml"),
            "templates: [worker-bare, t, empty]\n",
        )
        .unwrap();
        std::fs::create_dir(tmp.path().join("empty")).unwrap();
        let (valid, warnings) = load_worker_templates(tmp.path()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let ids: Vec<&str> = valid.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["t"]);
    }

    #[test]
    fn entries_outside_worker_dir_are_skipped_and_python_is_detected() {
        let tmp = scratch(&["README.md", "workers/w/a.txt"]);
        let (valid, warnings) = load_worker_templates(tmp.path()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(valid[0].language, Language::Python);
        assert_eq!(valid[0].files, ["a.txt"]);
    }

    #[test]
    fn files_entries_with_dotdot_absolute_or_missing_are_c234() {
        for bad in [
            "workers/w/../w/a.txt",
            "../outside.txt",
            "/etc/passwd",
            "workers/w/missing.txt",
        ] {
            let msg = rejection(&scratch(&["workers/w/a.txt", bad]));
            assert!(msg.contains(bad), "{msg}");
        }
    }

    #[test]
    fn symlinks_leaving_worker_dir_are_c234() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret\n").unwrap();

        // A file symlink pointing outside.
        let tmp = scratch(&["workers/w/a.txt", "workers/w/link.txt"]);
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            tmp.path().join("t/workers/w/link.txt"),
        )
        .unwrap();
        assert!(rejection(&tmp).contains("link.txt"));

        // A directory symlink pointing outside.
        let tmp = scratch(&["workers/w/a.txt", "workers/w/sub/secret.txt"]);
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("t/workers/w/sub")).unwrap();
        assert!(rejection(&tmp).contains("sub/secret.txt"));

        // A symlink that stays inside worker.dir is fine.
        let tmp = scratch(&["workers/w/a.txt", "workers/w/inner.txt"]);
        std::os::unix::fs::symlink(
            tmp.path().join("t/workers/w/a.txt"),
            tmp.path().join("t/workers/w/inner.txt"),
        )
        .unwrap();
        let (valid, warnings) = load_worker_templates(tmp.path()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(valid[0].files, ["a.txt", "inner.txt"]);
    }

    #[test]
    fn worker_names_follow_the_kebab_rule() {
        let max = format!("a{}", "b".repeat(62));
        assert_eq!(max.len(), 63);
        for ok in ["a", "my-worker", "a1-b2", "w2", max.as_str()] {
            assert!(validate_worker_name(ok).is_ok(), "{ok:?} should be valid");
        }
        let too_long = format!("{max}c");
        for bad in [
            "",
            too_long.as_str(),
            "a--b",
            "-a",
            "a-",
            "A",
            "1a",
            "my_worker",
            "my worker",
            "é",
        ] {
            let err = validate_worker_name(bad).expect_err(bad);
            assert_eq!(err.code(), "C232", "{bad:?}");
        }
    }

    #[test]
    fn replace_token_swaps_every_occurrence() {
        assert_eq!(
            replace_token(
                "workers/my-worker: my-worker::hello",
                "my-worker",
                "billing"
            ),
            "workers/billing: billing::hello"
        );
    }
}
