//! `coder::scaffold-worker` — create a new iii worker from a template (see
//! `coder::list-templates`). The files under the template's `worker.dir`
//! are written into a missing or empty folder named after the worker, with
//! the name token replaced in paths and contents, through
//! `coder::create-file`'s journalled write path (executable bit kept), all
//! or nothing. The result carries the `compose::add` payload and its container
//! object; registering the worker is the caller's call.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::code::change_journal::ChangeJournal;
use crate::code::config::CoderConfig;
use crate::code::error::{err_to_string, CoderError};
use crate::code::functions::create_file::{self, CreateFileInput, CreateFileSpec};
use crate::code::path::PathResolver;
use crate::code::templates::{
    compose_entry, load_worker_templates, replace_token, resolve_source, validate_worker_name,
};

// examples are wire-contract; goldens pin them.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(example = "example_scaffold_worker_input")]
pub struct ScaffoldWorkerInput {
    /// Template id from coder::list-templates.
    pub template: String,
    /// The new worker's name: lowercase kebab-case, 1-63 chars. It replaces
    /// the template's name token, so it names the folder, the compose
    /// container and the function prefix (`<name>::hello`).
    pub name: String,
    /// Folder to create the worker in: missing or empty, and its last
    /// folder must be `name` (compose names the container after it).
    /// Defaults to the template's worker folder for this name
    /// (`workers/<name>`).
    #[serde(default)]
    pub directory: Option<String>,
    /// Internal harness filesystem scope; omitted from published schema.
    #[serde(default)]
    #[schemars(skip)]
    pub fs_scope: Option<crate::fs::FsScope>,
}

// examples are wire-contract; goldens pin them.
fn example_scaffold_worker_input() -> serde_json::Value {
    serde_json::json!({
        "template": "worker-node-ade",
        "name": "orders",
        "directory": "workers/orders"
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScaffoldWorkerOutput {
    pub template: String,
    pub name: String,
    /// Canonical absolute path of the new worker folder.
    pub directory: String,
    pub files: Vec<ScaffoldedFile>,
    /// Container object to pass whole in `compose::add { workers: [compose] }`
    /// (the bare `worker` string form drops its scripts): `worker` is the
    /// absolute folder path; `start_after` is left to the caller.
    pub compose: serde_json::Value,
    /// The compose::add payload: send it whole (do not move its entry's
    /// fields to the top level, where scripts are ignored), adding
    /// start_after to its workers entry and, for each requires that
    /// compose::status does not list, one more entry in workers.
    pub compose_add: serde_json::Value,
    /// Compose containers the worker needs (e.g. `http`); add the ones
    /// `compose::status` does not list in the same `compose::add`.
    pub requires: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScaffoldedFile {
    /// Canonical absolute path.
    pub path: String,
    pub bytes: u64,
    /// Revision of the bytes written, as coder::read-file reports it.
    pub revision: String,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    journal: ChangeJournal,
    req: ScaffoldWorkerInput,
) -> Result<ScaffoldWorkerOutput, String> {
    // Before the template source: a bad name never costs a clone.
    validate_worker_name(&req.name).map_err(err_to_string)?;
    // III_TEMPLATE_DIR / III_TEMPLATE_URL win over the config (spec §3.2).
    let source = resolve_source(&cfg.templates.clone().with_env(), false)
        .await
        .map_err(err_to_string)?;
    scaffold(resolver, cfg, journal, source.root, req).await
}

/// Everything after the name check (spec §3.3 steps 2-7). `root` is the
/// folder holding the root `template.yaml`. Errors are wire strings
/// (`{code, message}`), as `handle` returns them.
async fn scaffold(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    journal: ChangeJournal,
    root: PathBuf,
    req: ScaffoldWorkerInput,
) -> Result<ScaffoldWorkerOutput, String> {
    let fs_scope = req.fs_scope.clone();
    let planner = resolver.clone();
    // Reading the template is blocking I/O.
    let (mut out, files) = tokio::task::spawn_blocking(move || plan(&planner, &root, &req))
        .await
        .map_err(|e| format!("coder::scaffold-worker blocking task failed: {e}"))?
        .map_err(err_to_string)?;

    let target = PathBuf::from(&out.directory);
    // The deepest folder that exists before the first write: a rollback
    // never removes it or anything above it.
    let floor = target
        .ancestors()
        .find(|d| d.exists())
        .unwrap_or(target.as_path())
        .to_path_buf();
    let written = create_file::handle_with_journal(
        resolver,
        cfg,
        journal,
        CreateFileInput { files, fs_scope },
    )
    .await?;
    // All or nothing: one failed entry undoes every entry that was written.
    if let Some(failed) = written.results.iter().find(|r| !r.success) {
        let done: Vec<PathBuf> = written
            .results
            .iter()
            .filter(|r| r.success)
            .map(|r| PathBuf::from(&r.path))
            .collect();
        rollback(&done, &target, &floor);
        return Err(serde_json::json!(failed.error).to_string());
    }
    out.files = written
        .results
        .into_iter()
        .map(|r| ScaffoldedFile {
            path: r.path,
            bytes: r.bytes_written,
            revision: r.revision.unwrap_or_default(),
        })
        .collect();
    Ok(out)
}

/// Spec §3.3 steps 2-4 and 6, before any write: pick the template, resolve
/// and check the target, read every file and build the compose object. A
/// bad template, a binary file or a protected path fails here with nothing
/// on disk. Returns the output (`files` still empty) and one
/// `coder::create-file` entry per template file.
fn plan(
    resolver: &PathResolver,
    root: &Path,
    req: &ScaffoldWorkerInput,
) -> Result<(ScaffoldWorkerOutput, Vec<CreateFileSpec>), CoderError> {
    let (templates, warnings) = load_worker_templates(root)?;
    let Some(template) = templates.iter().find(|t| t.id == req.template) else {
        // Left out by the loader as invalid: say why, not "unknown".
        let rejected = format!("C234: template {}:", req.template);
        if let Some(warning) = warnings.iter().find(|w| w.starts_with(&rejected)) {
            return Err(CoderError::InvalidTemplate(
                warning["C234: ".len()..].to_string(),
            ));
        }
        let ids: Vec<&str> = templates.iter().map(|t| t.id.as_str()).collect();
        return Err(CoderError::UnknownTemplate(format!(
            "no worker template {:?}; available: {}. Pass one of these as \
             `template` (coder::list-templates describes them).",
            req.template,
            ids.join(", ")
        )));
    };
    let token = template.worker.name.as_str();
    let name = req.name.as_str();
    let fs_scope = req.fs_scope.as_ref();

    let directory = req
        .directory
        .clone()
        .unwrap_or_else(|| replace_token(&template.worker.dir, token, name));
    let target = resolver.require_writable_scope(fs_scope, &directory)?;
    // Compose keys a container by the folder's last segment: another name
    // would register the worker as, or over, a different container.
    if target.file_name().is_none_or(|last| last != name) {
        return Err(CoderError::InvalidWorkerName(format!(
            "directory must end with the worker name ({name}): {directory} resolves \
             to {}, and compose names the container after the last folder. Pass a \
             directory whose last folder is {name}, e.g. workers/{name}.",
            target.display()
        )));
    }
    let occupied = match std::fs::read_dir(&target) {
        Ok(mut entries) => entries.next().is_some(),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    };
    if occupied {
        return Err(CoderError::TargetNotEmpty(format!(
            "{directory} ({}) exists and is not an empty folder; \
             coder::scaffold-worker never writes into one. Pass a \
             `directory` that is missing or empty, or another `name`.",
            target.display()
        )));
    }

    // `files` is relative to worker.dir (Part B1). Checked again here: a
    // refreshed clone may have changed since the load, and a symlink must
    // not pull a host file into the workspace.
    let worker_dir =
        std::fs::canonicalize(template.dir.join(&template.worker.dir)).map_err(|e| {
            invalid(
                &template.id,
                &format!("has a worker.dir that cannot be read ({e})"),
            )
        })?;
    let mut files = Vec::with_capacity(template.files.len());
    for rel in &template.files {
        let src = std::fs::canonicalize(worker_dir.join(rel))
            .ok()
            .filter(|p| p.starts_with(&worker_dir) && p.is_file())
            .ok_or_else(|| {
                invalid(
                    &template.id,
                    &format!(
                        "lists {rel}, which is missing, not a regular file, or links \
                         outside {}",
                        template.worker.dir
                    ),
                )
            })?;
        let src_mode = std::fs::metadata(&src)
            .map_err(|e| CoderError::io_for_path(e, rel))?
            .permissions()
            .mode();
        let raw = std::fs::read(&src).map_err(|e| CoderError::io_for_path(e, rel))?;
        // coder::create-file carries text; no template ships binaries.
        let text = String::from_utf8(raw).map_err(|_| {
            CoderError::InvalidTemplate(format!(
                "binary template files are not supported: {}/{}/{rel}. \
                 coder::scaffold-worker writes UTF-8 text only; drop the file \
                 from the template's files:, or pick another template.",
                template.id, template.worker.dir
            ))
        })?;
        let dest = target.join(replace_token(rel, token, name));
        // C211 for a protected path, before the first write.
        let path = resolver.require_writable_scope(fs_scope, &dest.to_string_lossy())?;
        let mode = if src_mode & 0o111 == 0 {
            "0644"
        } else {
            "0755"
        };
        files.push(CreateFileSpec {
            path: path.display().to_string(),
            content: replace_token(&text, token, name),
            mode: mode.to_string(),
            parents: true,
            overwrite: false,
            expected_revision: None,
        });
    }

    let entry = replace_token(&compose_entry(template)?.to_string(), token, name);
    let mut compose: serde_json::Value = serde_json::from_str(&entry).map_err(|e| {
        invalid(
            &template.id,
            &format!("has a compose entry that breaks with this name ({e})"),
        )
    })?;
    // compose_entry returns a mapping (Part B1).
    if let Some(container) = compose.as_object_mut() {
        container.insert("worker".into(), target.display().to_string().into());
        container.remove("start_after");
    }

    let out = ScaffoldWorkerOutput {
        template: template.id.clone(),
        name: req.name.clone(),
        directory: target.display().to_string(),
        // Filled in once the files are written.
        files: Vec::new(),
        compose_add: serde_json::json!({ "workers": [&compose] }),
        compose,
        requires: template.worker.requires.clone(),
        next_steps: template
            .next_steps
            .iter()
            .map(|step| replace_token(step, token, name))
            .collect(),
    };
    Ok((out, files))
}

/// Undo a failed scaffold: delete the files this call wrote, then every
/// folder between them and `floor` (the deepest folder that existed before
/// the call), deepest first. `remove_dir` only removes empty folders, so
/// nothing this call did not create is lost, even to a concurrent writer.
fn rollback(written: &[PathBuf], target: &Path, floor: &Path) {
    let mut dirs: Vec<&Path> = target
        .ancestors()
        .take_while(|d| *d != floor && d.starts_with(floor))
        .collect();
    for file in written {
        let _ = std::fs::remove_file(file);
        dirs.extend(
            file.ancestors()
                .skip(1)
                .take_while(|d| *d != floor && d.starts_with(floor)),
        );
    }
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for dir in dirs {
        let _ = std::fs::remove_dir(dir);
    }
}

fn invalid(id: &str, problem: &str) -> CoderError {
    CoderError::InvalidTemplate(format!(
        "template {id} {problem}. Fix the template, or pick another one from \
         coder::list-templates."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::config::TemplatesConfig;
    use crate::code::functions::list_templates::fixture;

    struct Env {
        work: tempfile::TempDir,
        templates: tempfile::TempDir,
        resolver: Arc<PathResolver>,
        cfg: Arc<CoderConfig>,
    }

    impl Env {
        fn new(non_accessible_globs: &[&str]) -> Self {
            let work = tempfile::tempdir().unwrap();
            let templates = tempfile::tempdir().unwrap();
            fixture::write(templates.path());
            let cfg = Arc::new(CoderConfig {
                base_paths: vec![work.path().to_path_buf()],
                non_accessible_globs: non_accessible_globs.iter().map(|g| g.to_string()).collect(),
                ..CoderConfig::default()
            });
            let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
            Self {
                work,
                templates,
                resolver,
                cfg,
            }
        }

        /// Canonical workspace root, the way the resolver reports paths.
        fn root(&self) -> PathBuf {
            std::fs::canonicalize(self.work.path()).unwrap()
        }

        /// The node template's folder in this env's templates root.
        fn template(&self) -> PathBuf {
            self.templates.path().join("worker-node-ade")
        }

        async fn run(
            &self,
            template: &str,
            name: &str,
            directory: Option<&str>,
        ) -> Result<ScaffoldWorkerOutput, String> {
            scaffold(
                self.resolver.clone(),
                self.cfg.clone(),
                ChangeJournal::default(),
                self.templates.path().to_path_buf(),
                input(template, name, directory),
            )
            .await
        }
    }

    fn input(template: &str, name: &str, directory: Option<&str>) -> ScaffoldWorkerInput {
        ScaffoldWorkerInput {
            template: template.into(),
            name: name.into(),
            directory: directory.map(Into::into),
            fs_scope: None,
        }
    }

    /// The `code` of a wire error string (`{"code": …, "message": …}`).
    fn code(err: &str) -> String {
        let wire: serde_json::Value =
            serde_json::from_str(err).unwrap_or_else(|e| panic!("not a wire error ({e}): {err}"));
        wire["code"].as_str().unwrap_or_default().to_string()
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[tokio::test]
    async fn scaffolds_the_worker_folder_with_the_name_replaced() {
        let env = Env::new(&[]);
        let out = env.run("worker-node-ade", "orders", None).await.unwrap();
        let dir = env.root().join("workers/orders");
        assert_eq!(out.template, "worker-node-ade");
        assert_eq!(out.name, "orders");
        assert_eq!(out.directory, dir.display().to_string());

        let mut paths: Vec<&str> = out.files.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        let expected: Vec<String> = ["package.json", "scripts/dev.sh", "src/index.ts"]
            .iter()
            .map(|rel| dir.join(rel).display().to_string())
            .collect();
        assert_eq!(paths, expected);
        // The project's README.md and worker-compose.yaml stay in the template.
        assert!(!env.root().join("README.md").exists());
        assert!(!dir.join("worker-compose.yaml").exists());

        for file in &out.files {
            let text = std::fs::read_to_string(&file.path).unwrap();
            assert_eq!(file.bytes, text.len() as u64);
            assert_eq!(
                file.revision,
                create_file::content_revision(text.as_bytes())
            );
            assert!(!text.contains("my-worker"), "{}: {text}", file.path);
        }
        assert_eq!(
            std::fs::read_to_string(dir.join("src/index.ts")).unwrap(),
            "export const id = 'orders::hello'\n"
        );

        assert_eq!(
            out.compose,
            serde_json::json!({
                "worker": dir.display().to_string(),
                "scripts": { "pre_run": "pnpm install", "run": "pnpm dev" },
                "environment": { "WORKER_NAME": "orders" }
            })
        );
        assert_eq!(
            out.compose_add,
            serde_json::json!({ "workers": [out.compose] })
        );
        assert_eq!(out.requires, ["http"]);
        assert_eq!(out.next_steps, ["Call: iii trigger orders::hello"]);
    }

    #[tokio::test]
    async fn keeps_the_executable_bit_of_template_scripts() {
        let env = Env::new(&[]);
        env.run("worker-node-ade", "orders", None).await.unwrap();
        let dir = env.root().join("workers/orders");
        assert_eq!(mode(&dir.join("scripts/dev.sh")), 0o755);
        assert_eq!(mode(&dir.join("package.json")), 0o644);
    }

    #[tokio::test]
    async fn writes_into_an_existing_empty_directory() {
        let env = Env::new(&[]);
        let dir = env.root().join("svc/orders");
        std::fs::create_dir_all(&dir).unwrap();
        let out = env
            .run("worker-node-ade", "orders", Some("svc/orders"))
            .await
            .unwrap();
        assert_eq!(out.directory, dir.display().to_string());
        assert_eq!(out.compose["worker"], dir.display().to_string());
        assert!(dir.join("src/index.ts").is_file());
    }

    #[tokio::test]
    async fn refuses_a_directory_that_is_not_empty_and_touches_nothing() {
        let env = Env::new(&[]);
        let dir = env.root().join("workers/orders");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("keep.txt"), "mine").unwrap();
        let err = env
            .run("worker-node-ade", "orders", None)
            .await
            .unwrap_err();
        assert_eq!(code(&err), "C233", "{err}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        assert_eq!(
            std::fs::read_to_string(dir.join("keep.txt")).unwrap(),
            "mine"
        );
    }

    #[tokio::test]
    async fn an_unknown_or_non_scaffoldable_template_is_c231() {
        let env = Env::new(&[]);
        for id in ["nope", "bare"] {
            let err = env.run(id, "orders", None).await.unwrap_err();
            assert_eq!(code(&err), "C231", "{id}: {err}");
            assert!(err.contains("worker-node-ade"), "{err}");
        }
    }

    #[tokio::test]
    async fn an_invalid_name_is_c232_before_the_templates_are_read() {
        let env = Env::new(&[]);
        let err = handle(
            env.resolver.clone(),
            env.cfg.clone(),
            ChangeJournal::default(),
            input("worker-node-ade", "Orders_1", None),
        )
        .await
        .unwrap_err();
        assert_eq!(code(&err), "C232", "{err}");
    }

    #[tokio::test]
    async fn a_directory_not_named_after_the_worker_is_c232() {
        let env = Env::new(&[]);
        let dir = env.root().join("svc");
        std::fs::create_dir(&dir).unwrap();
        let err = env
            .run("worker-node-ade", "orders", Some("svc"))
            .await
            .unwrap_err();
        assert_eq!(code(&err), "C232", "{err}");
        assert!(
            err.contains("directory must end with the worker name (orders)"),
            "{err}"
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_directory_outside_the_roots_is_c215_and_writes_nothing() {
        let env = Env::new(&[]);
        let err = env
            .run("worker-node-ade", "orders", Some("../escape"))
            .await
            .unwrap_err();
        assert_eq!(code(&err), "C215", "{err}");
        assert!(!env.root().parent().unwrap().join("escape").exists());
    }

    #[tokio::test]
    async fn a_protected_file_fails_c211_before_any_write() {
        let env = Env::new(&["**/*.sh"]);
        let err = env
            .run("worker-node-ade", "orders", None)
            .await
            .unwrap_err();
        assert_eq!(code(&err), "C211", "{err}");
        assert!(!env.root().join("workers").exists());
    }

    #[tokio::test]
    async fn a_template_file_linking_outside_the_template_is_c234() {
        let env = Env::new(&[]);
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "token").unwrap();
        let link = env.template().join("workers/my-worker/src/index.ts");
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), &link).unwrap();
        let err = env
            .run("worker-node-ade", "orders", None)
            .await
            .unwrap_err();
        // The loader's own warning for this template, not "unknown" (C231).
        assert_eq!(code(&err), "C234", "{err}");
        assert!(err.contains("template worker-node-ade:"), "{err}");
        assert!(!env.root().join("workers").exists());
    }

    #[tokio::test]
    async fn a_binary_template_file_is_c234_and_writes_nothing() {
        let env = Env::new(&[]);
        let template = env.template();
        std::fs::write(
            template.join("workers/my-worker/logo.bin"),
            b"\xffmy-worker\xfe",
        )
        .unwrap();
        let manifest = std::fs::read_to_string(template.join("template.yaml"))
            .unwrap()
            .replace(
                "  - workers/my-worker/scripts/dev.sh\n",
                "  - workers/my-worker/scripts/dev.sh\n  - workers/my-worker/logo.bin\n",
            );
        std::fs::write(template.join("template.yaml"), manifest).unwrap();
        let err = env
            .run("worker-node-ade", "orders", None)
            .await
            .unwrap_err();
        assert_eq!(code(&err), "C234", "{err}");
        assert!(
            err.contains(
                "binary template files are not supported: \
                 worker-node-ade/workers/my-worker/logo.bin"
            ),
            "{err}"
        );
        assert!(!env.root().join("workers").exists());
    }

    #[tokio::test]
    async fn a_failed_write_removes_only_what_this_call_created() {
        let env = Env::new(&[]);
        // `collide` ships a.txt and my-worker.txt: named "a", both land on
        // workers/a/a.txt, so the second write fails C213 after the first
        // one succeeded.
        let err = env.run("collide", "a", None).await.unwrap_err();
        assert_eq!(code(&err), "C213", "{err}");
        assert!(!env.root().join("workers").exists());

        // A folder that existed before the call survives the rollback.
        std::fs::create_dir(env.root().join("workers")).unwrap();
        let err = env.run("collide", "a", None).await.unwrap_err();
        assert_eq!(code(&err), "C213", "{err}");
        assert!(env.root().join("workers").is_dir());
        assert!(!env.root().join("workers/a").exists());
    }

    /// Opt-in (spec §6): with `III_TEMPLATE_DIR` set to a templates
    /// checkout, both `-ade` templates scaffold with no name token left in
    /// a path, a file, the compose object or the next steps.
    #[tokio::test]
    #[ignore = "needs III_TEMPLATE_DIR"]
    async fn real_ade_templates_leave_no_token() {
        let templates = TemplatesConfig::default().with_env();
        assert!(
            templates.dir.is_some(),
            "set III_TEMPLATE_DIR to a templates checkout"
        );
        let source = resolve_source(&templates, false).await.unwrap();
        let env = Env::new(&[]);
        for (template, parent) in [("worker-node-ade", "node"), ("worker-python-ade", "python")] {
            let directory = format!("{parent}/orders");
            let out = scaffold(
                env.resolver.clone(),
                env.cfg.clone(),
                ChangeJournal::default(),
                source.root.clone(),
                input(template, "orders", Some(directory.as_str())),
            )
            .await
            .unwrap_or_else(|e| panic!("{template}: {e}"));
            assert!(!out.files.is_empty(), "{template}");
            for file in &out.files {
                assert!(
                    !file.path.contains("my-worker"),
                    "{template}: {}",
                    file.path
                );
                let text = std::fs::read_to_string(&file.path).unwrap();
                assert!(!text.contains("my-worker"), "{template}: {}", file.path);
            }
            let rest = format!("{} {:?}", out.compose, out.next_steps);
            assert!(!rest.contains("my-worker"), "{template}: {rest}");
        }
    }
}
