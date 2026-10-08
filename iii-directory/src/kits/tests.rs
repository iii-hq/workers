//! End-to-end kit flows against an in-memory registry and Compose: plan
//! building, collision classification, three-way merges, apply ordering,
//! idempotent retries, stale plans, removal and update checks.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::apply::{ApplyTiming, DecisionInput};
use super::compose::ComposeControl;
use super::lock::{sha256_hex, KitsLock};
use super::origin::SourceKind;
use super::plan::{Change, Decision, KitsEnv, LocalState, PlanKind, WorkerAction};
use super::registry::{
    AgentEntry, Author, Deprecation, KitDetail, KitFile, KitFiles, KitFunction, KitRegistry,
    KitVersion, KitVersions, KitWorker, SkillEntry, UpdateAnswer, UpdateQuery,
};
use super::service::{self, PlanStatus};
use super::store;

const KIT: &str = "acme/team";

// ───────────────────────── fixtures ──────────────────────────────────

struct Version {
    files: Vec<(String, String)>,
    workers: Vec<(String, String, String)>,
    deprecated: bool,
}

#[derive(Default)]
struct MockRegistry {
    versions: Mutex<BTreeMap<String, Version>>,
    log: Arc<Mutex<Vec<String>>>,
    updates: Mutex<Vec<UpdateAnswer>>,
}

fn frontmatter_field(content: &str, key: &str) -> Option<String> {
    let (fm, _) = crate::fs_source::split_frontmatter(content);
    let value: serde_yaml::Value = serde_yaml::from_str(fm?).ok()?;
    value.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn frontmatter_list(content: &str, key: &str) -> Vec<String> {
    let (fm, _) = crate::fs_source::split_frontmatter(content);
    let Some(fm) = fm else { return vec![] };
    let value: serde_yaml::Value = serde_yaml::from_str(fm).unwrap_or_default();
    value
        .get(key)
        .and_then(|v| v.as_sequence())
        .map(|s| {
            s.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

impl MockRegistry {
    fn publish(&self, version: &str, files: &[(&str, &str)], workers: &[(&str, &str, &str)]) {
        self.versions.lock().unwrap().insert(
            version.to_string(),
            Version {
                files: files
                    .iter()
                    .map(|(p, c)| (p.to_string(), c.to_string()))
                    .collect(),
                workers: workers
                    .iter()
                    .map(|(n, r, v)| (n.to_string(), r.to_string(), v.to_string()))
                    .collect(),
                deprecated: false,
            },
        );
    }

    fn deprecate(&self, version: &str) {
        self.versions
            .lock()
            .unwrap()
            .get_mut(version)
            .unwrap()
            .deprecated = true;
    }

    fn latest(&self) -> Option<String> {
        self.versions
            .lock()
            .unwrap()
            .keys()
            .filter_map(|v| semver::Version::parse(v).ok())
            .max()
            .map(|v| v.to_string())
    }

    fn build_detail(&self, version: &str) -> Option<KitDetail> {
        let versions = self.versions.lock().unwrap();
        let v = versions.get(version)?;
        let agents = v
            .files
            .iter()
            .filter(|(p, _)| p.starts_with("agents/"))
            .map(|(p, c)| AgentEntry {
                id: p
                    .trim_start_matches("agents/")
                    .trim_end_matches(".md")
                    .to_string(),
                path: p.clone(),
                sha256: sha256_hex(c.as_bytes()),
                name: frontmatter_field(c, "name"),
                model: frontmatter_field(c, "model"),
                functions: frontmatter_list(c, "functions"),
                skills: frontmatter_list(c, "skills"),
                ..Default::default()
            })
            .collect();
        let skills = v
            .files
            .iter()
            .filter(|(p, _)| p.starts_with("skills/"))
            .map(|(p, c)| SkillEntry {
                id: super::paths::skill_id_for_source(KIT, p).unwrap(),
                path: p.clone(),
                sha256: sha256_hex(c.as_bytes()),
                ..Default::default()
            })
            .collect();
        Some(KitDetail {
            id: KIT.into(),
            author: Author {
                handle: "acme".into(),
                name: Some("Acme".into()),
                verified: true,
            },
            name: "team".into(),
            version: version.to_string(),
            notes: Some(format!("notes for {version}")),
            deprecation: v.deprecated.then(|| Deprecation {
                message: Some("use acme/team2".into()),
                replaced_by: Some("acme/team2".into()),
            }),
            workers: v
                .workers
                .iter()
                .map(|(n, r, res)| KitWorker {
                    name: n.clone(),
                    range: r.clone(),
                    resolved: Some(res.clone()),
                    kind: Some("binary".into()),
                    description: None,
                })
                .collect(),
            agents,
            skills,
            ..Default::default()
        })
    }

    fn build_files(&self, version: &str) -> Option<KitFiles> {
        let versions = self.versions.lock().unwrap();
        let v = versions.get(version)?;
        Some(KitFiles {
            version: version.to_string(),
            files: v
                .files
                .iter()
                .map(|(p, c)| KitFile {
                    path: p.clone(),
                    kind: None,
                    sha256: sha256_hex(c.as_bytes()),
                    size_bytes: None,
                    content: c.clone(),
                })
                .collect(),
            workers: vec![],
        })
    }
}

#[async_trait]
impl KitRegistry for MockRegistry {
    async fn detail(&self, kit: &str, version: &str) -> Result<Option<KitDetail>, String> {
        if kit != KIT {
            return Ok(None);
        }
        let version = if version == "latest" {
            match self.latest() {
                Some(v) => v,
                None => return Ok(None),
            }
        } else {
            version.to_string()
        };
        Ok(self.build_detail(&version))
    }
    async fn versions(&self, _kit: &str) -> Result<Option<KitVersions>, String> {
        Ok(Some(KitVersions {
            release_tags: BTreeMap::new(),
            versions: self
                .versions
                .lock()
                .unwrap()
                .keys()
                .map(|v| KitVersion {
                    version: v.clone(),
                    ..Default::default()
                })
                .collect(),
        }))
    }
    async fn files(&self, _kit: &str, version: &str) -> Result<KitFiles, String> {
        self.build_files(version)
            .ok_or_else(|| "no such version".into())
    }
    async fn download(&self, _kit: &str, version: &str, ci: bool) -> Result<KitFiles, String> {
        self.log
            .lock()
            .unwrap()
            .push(format!("download {version} ci={ci}"));
        self.build_files(version)
            .ok_or_else(|| "no such version".into())
    }
    async fn compare(
        &self,
        _kit: &str,
        _from: &str,
        _to: &str,
    ) -> Result<Option<super::registry::KitCompare>, String> {
        Ok(None)
    }
    async fn functions(&self, _kit: &str, _version: &str) -> Result<Vec<KitFunction>, String> {
        Ok(vec![KitFunction {
            id: "kanban::ticket::create".into(),
            used_by: vec!["planner".into()],
            status: Some("ok".into()),
            worker: Some("kanban".into()),
            ..Default::default()
        }])
    }
    async fn readme(&self, _kit: &str, _version: &str) -> Result<Option<String>, String> {
        Ok(Some("# team\n".into()))
    }
    async fn updates(&self, _queries: &[UpdateQuery]) -> Result<Vec<UpdateAnswer>, String> {
        Ok(self.updates.lock().unwrap().clone())
    }
    async fn blob(&self, sha: &str) -> Result<Option<String>, String> {
        let versions = self.versions.lock().unwrap();
        Ok(versions
            .values()
            .flat_map(|v| v.files.iter())
            .find(|(_, c)| sha256_hex(c.as_bytes()) == sha)
            .map(|(_, c)| c.clone()))
    }
}

struct MockCompose {
    available: bool,
    fail_add: Mutex<bool>,
    log: Arc<Mutex<Vec<String>>>,
    /// Checked when `compose::add` runs: nothing may be written yet.
    watch: Vec<PathBuf>,
    present_at_add: Mutex<Vec<bool>>,
    ops: Mutex<BTreeMap<String, &'static str>>,
}

impl MockCompose {
    fn new(log: Arc<Mutex<Vec<String>>>, watch: Vec<PathBuf>) -> Self {
        Self {
            available: true,
            fail_add: Mutex::new(false),
            log,
            watch,
            present_at_add: Mutex::new(vec![]),
            ops: Mutex::new(BTreeMap::new()),
        }
    }
}

#[async_trait]
impl ComposeControl for MockCompose {
    async fn available(&self) -> Result<(), String> {
        if self.available {
            Ok(())
        } else {
            Err("function_not_found".into())
        }
    }
    async fn add(&self, workers: Vec<Value>, operation_id: &str) -> Result<String, String> {
        self.log.lock().unwrap().push(format!(
            "compose::add {}",
            serde_json::to_string(&workers).unwrap()
        ));
        let present = self.watch.iter().any(|p| p.exists());
        self.present_at_add.lock().unwrap().push(present);
        let status = if *self.fail_add.lock().unwrap() {
            "failed"
        } else {
            "succeeded"
        };
        self.ops
            .lock()
            .unwrap()
            .insert(operation_id.to_string(), status);
        Ok(operation_id.to_string())
    }
    async fn remove(&self, containers: Vec<String>, operation_id: &str) -> Result<String, String> {
        self.log
            .lock()
            .unwrap()
            .push(format!("compose::remove {}", containers.join(",")));
        self.ops
            .lock()
            .unwrap()
            .insert(operation_id.to_string(), "succeeded");
        Ok(operation_id.to_string())
    }
    async fn operation(&self, operation_id: &str) -> Result<(String, String), String> {
        let status = self
            .ops
            .lock()
            .unwrap()
            .get(operation_id)
            .copied()
            .unwrap_or("running");
        let detail = if status == "failed" {
            "kanban failed to start"
        } else {
            "all requested workers are ready"
        };
        Ok((status.to_string(), detail.to_string()))
    }
}

struct Project {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    env: KitsEnv,
    registry: MockRegistry,
    compose: MockCompose,
    log: Arc<Mutex<Vec<String>>>,
}

fn project() -> Project {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let env = KitsEnv {
        agents_folder: root.join("agents"),
        global_agent_roots: vec![root.join("home-agents")],
        skills_folder: root.join("skills"),
        lock_path: root.join("kits.lock"),
        compose_file: root.join("worker-compose.yaml"),
        plans_dir: root.join(".iii/directory/kit-plans"),
        updates_cache: root.join(".iii/directory/kit-updates.json"),
        registry_base: "https://api.workers.iii.dev".into(),
        registry_web: None,
        ci: false,
    };
    std::fs::create_dir_all(&env.agents_folder).unwrap();
    std::fs::create_dir_all(&env.skills_folder).unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let registry = MockRegistry {
        log: log.clone(),
        ..Default::default()
    };
    let compose = MockCompose::new(
        log.clone(),
        vec![root.join("kits.lock"), root.join("agents/planner.md")],
    );
    Project {
        _tmp: tmp,
        root,
        env,
        registry,
        compose,
        log,
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

const PLANNER_V1: &str =
    "---\nname: Planner\nmodel: sonnet\nfunctions: [kanban::ticket::create]\n---\nPlan features.\n";
const PLANNER_V2: &str = "---\nname: Planner\nmodel: opus\nfunctions: [kanban::ticket::create, github::pr::create]\n---\nPlan features.\nOpen PRs.\n";
const REVIEWER: &str = "---\nname: Reviewer\n---\nReview the kit way.\n";
const FLOW_V1: &str = "# Flow\n\nStep one.\nStep two.\nStep three.\n";
const FLOW_V2: &str = "# Flow\n\nStep one.\nStep two (kit).\nStep three.\n";
const TRIAGE_V2: &str = "# Triage\n\nSort it.\n";
const LEGACY: &str = "# Legacy\n";

fn publish_v1(p: &Project) {
    p.registry.publish(
        "1.0.0",
        &[
            ("agents/planner.md", PLANNER_V1),
            ("agents/reviewer.md", REVIEWER),
            ("skills/tickets/flow.md", FLOW_V1),
            ("skills/legacy/old.md", LEGACY),
        ],
        &[("kanban", "^1.4", "1.6.1")],
    );
}

fn publish_v2(p: &Project) {
    p.registry.publish(
        "1.1.0",
        &[
            ("agents/planner.md", PLANNER_V2),
            ("agents/reviewer.md", REVIEWER),
            ("skills/tickets/flow.md", FLOW_V2),
            ("skills/tickets/triage.md", TRIAGE_V2),
        ],
        &[("kanban", "^1.6", "1.6.1"), ("github", "^2.0", "2.0.3")],
    );
}

/// A kanban worker whose bundle wrote `agents/reviewer.md` (marker records it).
fn kanban_shipped_reviewer(p: &Project, content: &str) {
    write(&p.env.agents_folder.join("reviewer.md"), content);
    write(
        &p.env.skills_folder.join("kanban/.iii-skill-complete"),
        &json!({
            "worker": "kanban", "source": "registry", "tag_or_version": "latest",
            "schema": 2, "version": "1.6.1",
            "agents": { "reviewer": sha256_hex(content.as_bytes()) }
        })
        .to_string(),
    );
}

fn compose_declares(p: &Project, kanban_selector: &str, kanban_resolved: &str) {
    write(
        &p.env.compose_file,
        &format!("containers:\n  kanban:\n    worker: package://api.workers.iii.dev/kanban\n    version: \"{kanban_selector}\"\n"),
    );
    write(
        &p.root.join("worker-compose.lock"),
        &format!("version: 1\ncontainers:\n  kanban:\n    worker: package://api.workers.iii.dev/kanban\n    requested: \"{kanban_selector}\"\n    resolved:\n      name: kanban\n      version: {kanban_resolved}\n"),
    );
}

fn timing() -> ApplyTiming {
    ApplyTiming {
        poll: Duration::from_millis(1),
        deadline: Duration::from_secs(5),
    }
}

fn noop(_: &super::store::ApplyProgress) {}

async fn plan_download(p: &Project, kit: &str) -> super::service::KitPlanResponse {
    service::download_kit(&p.env, &p.registry, &p.compose, kit, false, timing(), &noop)
        .await
        .unwrap()
}

async fn apply(
    p: &Project,
    plan_id: &str,
    decisions: BTreeMap<String, DecisionInput>,
) -> Result<super::service::KitPlanResponse, String> {
    service::run_apply(
        &p.env,
        &p.registry,
        &p.compose,
        plan_id,
        &decisions,
        &[],
        timing(),
        &noop,
    )
    .await
}

// ───────────────────────── install ───────────────────────────────────

#[tokio::test]
async fn install_plan_classifies_collisions_and_workers() {
    let p = project();
    publish_v1(&p);
    kanban_shipped_reviewer(&p, "---\nname: Reviewer\n---\nReview the kanban way.\n");
    compose_declares(&p, "1.5.2", "1.5.2");

    let out = plan_download(&p, KIT).await;
    assert_eq!(out.status, PlanStatus::Planned);
    let plan = out.plan.unwrap();
    assert_eq!(plan.kind, PlanKind::Install);
    assert_eq!(plan.to.as_deref(), Some("1.0.0"));
    assert_eq!(plan.requested, "latest");
    assert_eq!(plan.registry_url, "https://workers.iii.dev/kits/acme/team");

    // Profiles first, then skills, all added.
    let paths: Vec<&str> = plan.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "agents/planner.md",
            "agents/reviewer.md",
            "skills/acme/team/legacy/old.md",
            "skills/acme/team/tickets/flow.md"
        ]
    );
    assert!(plan.files.iter().all(|f| f.change == Change::Added));

    let reviewer = plan.files.iter().find(|f| f.id == "reviewer").unwrap();
    let c = reviewer.collision.as_ref().unwrap();
    assert_eq!(c.owner, SourceKind::Worker);
    assert_eq!(c.worker.as_deref(), Some("kanban"));
    assert_eq!(c.version.as_deref(), Some("1.6.1"));
    assert_eq!(reviewer.local, LocalState::Occupied);
    assert_eq!(reviewer.default, Some(Decision::Overwrite));
    assert_eq!(reviewer.options, vec![Decision::Overwrite, Decision::Keep]);
    assert_eq!(plan.warnings.len(), 1);
    assert_eq!(plan.warnings[0].code, "agent_overwrite");
    assert!(plan.warnings[0].message.contains("worker kanban 1.6.1"));

    let flow = plan
        .files
        .iter()
        .find(|f| f.path.ends_with("flow.md"))
        .unwrap();
    assert_eq!(flow.id, "acme/team/tickets/flow");
    assert_eq!(flow.local, LocalState::Absent);
    assert!(flow.collision.is_none());

    // kanban 1.5.2 is outside ^1.4? No: 1.5.2 satisfies ^1.4 → nothing to do.
    assert_eq!(plan.workers.len(), 1);
    assert_eq!(plan.workers[0].action, WorkerAction::None);
    assert_eq!(plan.workers[0].installed.as_deref(), Some("1.5.2"));
    assert!(plan.blocking.is_empty());
    assert_eq!(plan.functions.len(), 1);
    assert_eq!(plan.capabilities.functions_added.len(), 1);
    assert_eq!(plan.counts.agents, 2);
    assert_eq!(plan.counts.skills, 2);
    assert_eq!(plan.counts.collisions, 1);

    // The plan was stored with the bodies it refers to.
    let record = store::load(&p.env.plans_dir, &plan.plan_id)
        .unwrap()
        .unwrap();
    assert!(record
        .contents
        .blobs
        .contains_key(&sha256_hex(PLANNER_V1.as_bytes())));
    assert!(record.contents.ours.contains_key("agents/reviewer.md"));
}

#[tokio::test]
async fn install_plan_warns_local_global_and_builtin_collisions() {
    let p = project();
    p.registry.publish(
        "1.0.0",
        &[
            ("agents/mine.md", "---\nname: Mine (kit)\n---\nx\n"),
            ("agents/glob.md", "---\nname: Glob\n---\nx\n"),
            ("agents/default.md", "---\nname: Default (kit)\n---\nx\n"),
            ("agents/same.md", "---\nname: Same\n---\nx\n"),
        ],
        &[],
    );
    write(
        &p.env.agents_folder.join("mine.md"),
        "---\nname: Mine\n---\nhand-made\n",
    );
    write(
        &p.root.join("home-agents/glob.md"),
        "---\nname: Glob\n---\nglobal\n",
    );
    write(
        &p.env.agents_folder.join("same.md"),
        "---\nname: Same\n---\nx\n",
    );

    let plan = plan_download(&p, KIT).await.plan.unwrap();
    let by_id = |id: &str| plan.files.iter().find(|f| f.id == id).unwrap();
    assert_eq!(
        by_id("mine").collision.as_ref().unwrap().owner,
        SourceKind::Local
    );
    assert_eq!(
        by_id("glob").collision.as_ref().unwrap().owner,
        SourceKind::Global
    );
    assert_eq!(
        by_id("default").collision.as_ref().unwrap().owner,
        SourceKind::Builtin
    );
    // Identical content is no collision at all.
    assert!(by_id("same").collision.is_none());
    assert_eq!(by_id("same").local, LocalState::Intact);
    let codes: Vec<&str> = plan.warnings.iter().map(|w| w.code.as_str()).collect();
    assert_eq!(
        codes,
        vec![
            "builtin_override",
            "agent_shadows_global",
            "agent_overwrite_local"
        ]
    );
    assert!(plan.workers.is_empty());
}

#[tokio::test]
async fn install_plan_blocks_without_compose_and_when_deprecated() {
    let mut p = project();
    publish_v1(&p);
    p.compose.available = false;
    let plan = plan_download(&p, KIT).await.plan.unwrap();
    // kanban is not declared: adding it needs Compose.
    assert_eq!(plan.workers[0].action, WorkerAction::Add);
    assert_eq!(plan.blocking[0].code, "compose_not_running");
    assert!(!super::plan::auto_applicable(&plan));
    let err = apply(&p, &plan.plan_id, BTreeMap::new()).await.unwrap_err();
    assert!(err.starts_with("D514"), "{err}");

    p.compose.available = true;
    p.registry.deprecate("1.0.0");
    let plan = plan_download(&p, KIT).await.plan.unwrap();
    assert_eq!(plan.blocking.len(), 1);
    assert_eq!(plan.blocking[0].code, "kit_deprecated");
    assert!(plan.blocking[0].message.contains("acme/team2"));
}

#[tokio::test]
async fn apply_install_runs_workers_then_files_then_lock() {
    let p = project();
    publish_v1(&p);
    kanban_shipped_reviewer(&p, "---\nname: Reviewer\n---\nReview the kanban way.\n");
    let plan = plan_download(&p, KIT).await.plan.unwrap();
    assert_eq!(plan.workers[0].action, WorkerAction::Add);

    // Keep the worker's reviewer, take everything else.
    let decisions = BTreeMap::from([(
        "agents/reviewer.md".to_string(),
        DecisionInput::Choice(Decision::Keep),
    )]);
    let out = apply(&p, &plan.plan_id, decisions).await.unwrap();
    assert_eq!(out.status, PlanStatus::Applied);
    let report = out.report.unwrap();

    // Download (counted) → compose::add → files → lock.
    let log = p.log.lock().unwrap().clone();
    assert_eq!(log[0], "download 1.0.0 ci=false");
    assert_eq!(
        log[1],
        r#"compose::add [{"version":"^1.4","worker":"kanban"}]"#
    );
    assert_eq!(*p.compose.present_at_add.lock().unwrap(), vec![false]);
    assert_eq!(report.workers_added, vec!["kanban"]);
    assert_eq!(
        report.steps.iter().map(|s| s.state).collect::<Vec<_>>(),
        vec![super::store::StepState::Done; 3]
    );

    assert_eq!(read(&p.env.agents_folder.join("planner.md")), PLANNER_V1);
    assert_eq!(
        read(&p.env.agents_folder.join("reviewer.md")),
        "---\nname: Reviewer\n---\nReview the kanban way.\n"
    );
    assert_eq!(
        read(&p.env.skills_folder.join("acme/team/tickets/flow.md")),
        FLOW_V1
    );

    let lock = KitsLock::read(&p.env.lock_path).unwrap();
    let kit = &lock.kits[KIT];
    assert_eq!(kit.version, "1.0.0");
    assert_eq!(kit.requested, "latest");
    assert_eq!(kit.workers["kanban"], "^1.4");
    assert!(kit.files["agents/reviewer.md"].skipped);
    assert!(!kit.files["agents/planner.md"].skipped);
    assert_eq!(
        kit.files["skills/acme/team/tickets/flow.md"].sha256,
        sha256_hex(FLOW_V1.as_bytes())
    );
    // The plan is gone once applied; agents the kit now provides are listed.
    assert!(store::load(&p.env.plans_dir, &plan.plan_id)
        .unwrap()
        .is_none());
    assert_eq!(report.agents, vec!["planner"]);
    assert!(report
        .files_kept
        .contains(&"agents/reviewer.md".to_string()));

    // A second download of the same version is a no-op.
    let again = plan_download(&p, KIT).await;
    assert_eq!(again.status, PlanStatus::UpToDate);
}

#[tokio::test]
async fn failed_compose_aborts_before_files_and_retry_converges() {
    let p = project();
    publish_v1(&p);
    let plan = plan_download(&p, KIT).await.plan.unwrap();
    *p.compose.fail_add.lock().unwrap() = true;
    let err = apply(&p, &plan.plan_id, BTreeMap::new()).await.unwrap_err();
    assert!(err.starts_with("D515"), "{err}");
    assert!(!p.env.agents_folder.join("planner.md").exists());
    assert!(!p.env.lock_path.exists());
    // The journal recorded the failed step for the UI.
    let record = store::load(&p.env.plans_dir, &plan.plan_id)
        .unwrap()
        .unwrap();
    let progress = record.progress.unwrap();
    assert_eq!(progress.steps[0].state, super::store::StepState::Failed);

    *p.compose.fail_add.lock().unwrap() = false;
    let out = apply(&p, &plan.plan_id, BTreeMap::new()).await.unwrap();
    assert_eq!(out.status, PlanStatus::Applied);
    assert!(p.env.lock_path.exists());
}

#[tokio::test]
async fn journaled_writes_do_not_make_a_retried_plan_stale() {
    let p = project();
    publish_v1(&p);
    compose_declares(&p, "^1.4", "1.6.1");
    let plan = plan_download(&p, KIT).await.plan.unwrap();
    // Simulate an attempt that wrote planner.md and then died.
    let mut record = store::load(&p.env.plans_dir, &plan.plan_id)
        .unwrap()
        .unwrap();
    write(&p.env.agents_folder.join("planner.md"), PLANNER_V1);
    let mut progress = super::store::ApplyProgress::default();
    progress.written.insert(
        "agents/planner.md".into(),
        sha256_hex(PLANNER_V1.as_bytes()),
    );
    record.progress = Some(progress);
    store::save(&p.env.plans_dir, &record).unwrap();

    let out = apply(&p, &plan.plan_id, BTreeMap::new()).await.unwrap();
    assert_eq!(out.status, PlanStatus::Applied);
}

#[tokio::test]
async fn a_local_change_after_planning_replans_instead_of_applying() {
    let p = project();
    publish_v1(&p);
    compose_declares(&p, "^1.4", "1.6.1");
    let plan = plan_download(&p, KIT).await.plan.unwrap();
    write(
        &p.env.agents_folder.join("reviewer.md"),
        "---\nname: R\n---\nnew local file\n",
    );
    let out = apply(&p, &plan.plan_id, BTreeMap::new()).await.unwrap();
    assert_eq!(out.status, PlanStatus::Replanned);
    assert!(out.message.contains("agents/reviewer.md changed on disk"));
    let fresh = out.plan.unwrap();
    assert_ne!(fresh.plan_id, plan.plan_id);
    let reviewer = fresh.files.iter().find(|f| f.id == "reviewer").unwrap();
    assert_eq!(
        reviewer.collision.as_ref().unwrap().owner,
        SourceKind::Local
    );
    assert!(!p.env.lock_path.exists());
}

#[tokio::test]
async fn apply_true_installs_only_a_plan_without_warnings() {
    let p = project();
    publish_v1(&p);
    compose_declares(&p, "^1.4", "1.6.1");
    let out = service::download_kit(
        &p.env,
        &p.registry,
        &p.compose,
        "acme/team@1.0.0",
        true,
        timing(),
        &noop,
    )
    .await
    .unwrap();
    assert_eq!(out.status, PlanStatus::Applied);
    assert_eq!(
        KitsLock::read(&p.env.lock_path).unwrap().kits[KIT].requested,
        "1.0.0"
    );

    // A collision keeps the plan for review even with apply=true.
    let q = project();
    publish_v1(&q);
    compose_declares(&q, "^1.4", "1.6.1");
    kanban_shipped_reviewer(&q, "---\nname: Reviewer\n---\nworker\n");
    let out = service::download_kit(&q.env, &q.registry, &q.compose, KIT, true, timing(), &noop)
        .await
        .unwrap();
    assert_eq!(out.status, PlanStatus::Planned);
    assert!(out.message.starts_with("Not applied automatically"));
}

#[tokio::test]
async fn unknown_kits_and_versions_are_d510() {
    let p = project();
    publish_v1(&p);
    let err = service::download_kit(
        &p.env,
        &p.registry,
        &p.compose,
        "acme/nope",
        false,
        timing(),
        &noop,
    )
    .await
    .unwrap_err();
    assert!(err.starts_with("D510"), "{err}");
    let err = service::download_kit(
        &p.env,
        &p.registry,
        &p.compose,
        "acme/team@9.0.0",
        false,
        timing(),
        &noop,
    )
    .await
    .unwrap_err();
    assert!(err.contains("no version or tag"), "{err}");
    let err = service::download_kit(
        &p.env,
        &p.registry,
        &p.compose,
        "acme/team@^3",
        false,
        timing(),
        &noop,
    )
    .await
    .unwrap_err();
    assert!(err.contains("satisfies"), "{err}");
    let err = service::download_kit(
        &p.env,
        &p.registry,
        &p.compose,
        "Acme",
        false,
        timing(),
        &noop,
    )
    .await
    .unwrap_err();
    assert!(err.starts_with("D511"), "{err}");
}

// ───────────────────────── update ────────────────────────────────────

async fn installed_v1(p: &Project) {
    publish_v1(p);
    compose_declares(p, "^1.4", "1.6.1");
    let plan = plan_download(p, "acme/team@^1").await.plan.unwrap();
    apply(p, &plan.plan_id, BTreeMap::new()).await.unwrap();
    p.log.lock().unwrap().clear();
}

#[tokio::test]
async fn update_plan_merges_edits_and_flags_conflicts() {
    let p = project();
    installed_v1(&p).await;
    publish_v2(&p);
    // Edit flow.md where the kit also changed it (conflict), and legacy/old.md
    // which the kit removes (kept as local).
    let flow = p.env.skills_folder.join("acme/team/tickets/flow.md");
    write(
        &flow,
        "# Flow\n\nStep one.\nStep two (mine).\nStep three.\n",
    );
    let old = p.env.skills_folder.join("acme/team/legacy/old.md");
    write(&old, "# Legacy\n\nmy notes\n");

    let out = plan_download(&p, KIT).await;
    let plan = out.plan.unwrap();
    assert_eq!(plan.kind, PlanKind::Update);
    assert_eq!(plan.from.as_deref(), Some("1.0.0"));
    assert_eq!(plan.to.as_deref(), Some("1.1.0"));
    assert_eq!(plan.bump.as_deref(), Some("minor"));
    assert!(!plan.major);
    // `requested` is carried from the lock.
    assert_eq!(plan.requested, "^1");
    assert_eq!(plan.notes.as_deref(), Some("notes for 1.1.0"));

    let by_path = |p_: &str| plan.files.iter().find(|f| f.path == p_).unwrap();
    let planner = by_path("agents/planner.md");
    assert_eq!(planner.change, Change::Modified);
    assert_eq!(planner.local, LocalState::Intact);
    assert_eq!(planner.default, Some(Decision::Kit));
    let changes = planner.agent_changes.as_ref().unwrap();
    assert_eq!(
        changes.fields.get("model"),
        Some(&(Some("sonnet".to_string()), Some("opus".to_string())))
    );
    assert_eq!(changes.functions_added, vec!["github::pr::create"]);
    assert_eq!(planner.body_changed, Some(true));

    let flow_f = by_path("skills/acme/team/tickets/flow.md");
    assert_eq!(flow_f.local, LocalState::Edited);
    assert_eq!(flow_f.merge, Some(super::merge::MergeStatus::Conflicts));
    assert_eq!(flow_f.default, None);
    assert_eq!(
        flow_f.options,
        vec![Decision::Kit, Decision::Mine, Decision::Merged]
    );

    let triage = by_path("skills/acme/team/tickets/triage.md");
    assert_eq!(triage.change, Change::Added);

    let legacy = by_path("skills/acme/team/legacy/old.md");
    assert_eq!(legacy.change, Change::Removed);
    assert_eq!(legacy.default, Some(Decision::Keep));

    // reviewer.md did not change: not in the plan.
    assert!(plan.files.iter().all(|f| f.id != "reviewer"));
    assert_eq!(plan.counts.unchanged, 1);
    assert_eq!(plan.counts.conflicts, 1);
    assert_eq!(plan.counts.decisions_required, 1);

    // kanban ^1.4 → ^1.6: 1.6.1 already satisfies, the declaration moves.
    let kanban = plan.workers.iter().find(|w| w.name == "kanban").unwrap();
    assert_eq!(kanban.action, WorkerAction::Redeclare);
    assert_eq!(kanban.range_from.as_deref(), Some("^1.4"));
    let github = plan.workers.iter().find(|w| w.name == "github").unwrap();
    assert_eq!(github.action, WorkerAction::Add);
    assert_eq!(plan.capabilities.workers_added, vec!["github"]);
    assert_eq!(
        plan.capabilities.models_changed[0].to.as_deref(),
        Some("opus")
    );

    // Without a decision for the conflict the apply refuses.
    let err = apply(&p, &plan.plan_id, BTreeMap::new()).await.unwrap_err();
    assert!(err.starts_with("D513"), "{err}");
    assert!(err.contains("needs a decision"), "{err}");
    // Markers left in the resolved text are refused too.
    let record = store::load(&p.env.plans_dir, &plan.plan_id)
        .unwrap()
        .unwrap();
    let with_markers = record.contents.merged[&flow_f.path].clone();
    let err = apply(
        &p,
        &plan.plan_id,
        BTreeMap::from([(
            flow_f.path.clone(),
            DecisionInput::Detailed {
                choice: Decision::Merged,
                content: Some(with_markers),
            },
        )]),
    )
    .await
    .unwrap_err();
    assert!(err.contains("conflict markers"), "{err}");

    let resolved = "# Flow\n\nStep one.\nStep two (mine + kit).\nStep three.\n";
    let out = apply(
        &p,
        &plan.plan_id,
        BTreeMap::from([(
            flow_f.path.clone(),
            DecisionInput::Detailed {
                choice: Decision::Merged,
                content: Some(resolved.into()),
            },
        )]),
    )
    .await
    .unwrap();
    assert_eq!(out.status, PlanStatus::Applied);
    let report = out.report.unwrap();
    assert_eq!(report.files_merged, vec![flow_f.path.clone()]);
    assert_eq!(read(&flow), resolved);
    assert_eq!(read(&p.env.agents_folder.join("planner.md")), PLANNER_V2);
    assert_eq!(read(&old), "# Legacy\n\nmy notes\n");
    assert!(p
        .env
        .skills_folder
        .join("acme/team/tickets/triage.md")
        .exists());

    // Rejected applies counted nothing; one compose::add for the new worker
    // and the moved declaration.
    let log = p.log.lock().unwrap().clone();
    assert_eq!(log.len(), 2, "{log:?}");
    assert_eq!(log[0], "download 1.1.0 ci=false");
    assert!(
        log[1].contains(r#""worker":"kanban""#) && log[1].contains(r#""worker":"github""#),
        "{log:?}"
    );

    let lock = KitsLock::read(&p.env.lock_path).unwrap();
    let kit = &lock.kits[KIT];
    assert_eq!(kit.version, "1.1.0");
    assert_eq!(kit.workers["kanban"], "^1.6");
    // The merge base moves to the kit's new version; the file reads as edited.
    assert_eq!(
        kit.files[&flow_f.path].sha256,
        sha256_hex(FLOW_V2.as_bytes())
    );
    assert!(!kit.files.contains_key("skills/acme/team/legacy/old.md"));
    let listing = service::list(&p.env).unwrap();
    assert_eq!(listing.kits[0].files.edited, 1);
}

#[tokio::test]
async fn clean_merges_are_the_default_and_removed_intact_files_go() {
    let p = project();
    installed_v1(&p).await;
    publish_v2(&p);
    let flow = p.env.skills_folder.join("acme/team/tickets/flow.md");
    // Edit a different line than the kit did.
    write(
        &flow,
        "# Flow (mine)\n\nStep one.\nStep two.\nStep three.\n",
    );
    let plan = plan_download(&p, KIT).await.plan.unwrap();
    let f = plan
        .files
        .iter()
        .find(|f| f.path.ends_with("flow.md"))
        .unwrap();
    assert_eq!(f.merge, Some(super::merge::MergeStatus::Clean));
    assert_eq!(f.default, Some(Decision::Merged));
    assert!(plan.warnings.iter().any(|w| w.code == "local_edits_merged"));
    let legacy = plan
        .files
        .iter()
        .find(|f| f.path.ends_with("old.md"))
        .unwrap();
    assert_eq!(legacy.default, Some(Decision::Remove));

    apply(&p, &plan.plan_id, BTreeMap::new()).await.unwrap();
    assert_eq!(
        read(&flow),
        "# Flow (mine)\n\nStep one.\nStep two (kit).\nStep three.\n"
    );
    assert!(!p.env.skills_folder.join("acme/team/legacy/old.md").exists());
    // The emptied directory went with it.
    assert!(!p.env.skills_folder.join("acme/team/legacy").exists());
}

#[tokio::test]
async fn skipped_files_stay_skipped_across_updates() {
    let p = project();
    publish_v1(&p);
    compose_declares(&p, "^1.4", "1.6.1");
    kanban_shipped_reviewer(&p, "---\nname: Reviewer\n---\nworker\n");
    let plan = plan_download(&p, KIT).await.plan.unwrap();
    apply(
        &p,
        &plan.plan_id,
        BTreeMap::from([(
            "agents/reviewer.md".into(),
            DecisionInput::Choice(Decision::Keep),
        )]),
    )
    .await
    .unwrap();
    // 1.1.0 changes the reviewer.
    p.registry.publish(
        "1.1.0",
        &[
            ("agents/planner.md", PLANNER_V1),
            (
                "agents/reviewer.md",
                "---\nname: Reviewer\n---\nReview v2.\n",
            ),
            ("skills/tickets/flow.md", FLOW_V1),
            ("skills/legacy/old.md", LEGACY),
        ],
        &[("kanban", "^1.4", "1.6.1")],
    );
    let plan = plan_download(&p, KIT).await.plan.unwrap();
    let reviewer = plan.files.iter().find(|f| f.id == "reviewer").unwrap();
    assert_eq!(reviewer.change, Change::Kept);
    assert!(reviewer.skipped);
    assert_eq!(reviewer.default, Some(Decision::Keep));
    apply(&p, &plan.plan_id, BTreeMap::new()).await.unwrap();
    assert_eq!(
        read(&p.env.agents_folder.join("reviewer.md")),
        "---\nname: Reviewer\n---\nworker\n"
    );
    assert!(
        KitsLock::read(&p.env.lock_path).unwrap().kits[KIT].files["agents/reviewer.md"].skipped
    );
}

#[tokio::test]
async fn taking_a_profile_from_another_kit_marks_it_skipped_there() {
    let p = project();
    installed_v1(&p).await;
    // Pretend a second kit is being installed that ships planner too: reuse
    // the registry by renaming the lock entry of the first kit.
    let mut lock = KitsLock::read(&p.env.lock_path).unwrap();
    let entry = lock.kits.remove(KIT).unwrap();
    lock.kits.insert("other/kit".into(), entry);
    lock.write(&p.env.lock_path).unwrap();
    write(
        &p.env.agents_folder.join("planner.md"),
        "---\nname: Planner (other)\n---\nx\n",
    );
    let mut lock = KitsLock::read(&p.env.lock_path).unwrap();
    lock.kits.get_mut("other/kit").unwrap().files.insert(
        "agents/planner.md".into(),
        super::lock::LockedFile {
            sha256: sha256_hex(b"---\nname: Planner (other)\n---\nx\n"),
            skipped: false,
        },
    );
    lock.write(&p.env.lock_path).unwrap();

    let plan = plan_download(&p, KIT).await.plan.unwrap();
    let planner = plan.files.iter().find(|f| f.id == "planner").unwrap();
    let c = planner.collision.as_ref().unwrap();
    assert_eq!(c.owner, SourceKind::Kit);
    assert_eq!(c.kit.as_deref(), Some("other/kit"));
    assert!(plan
        .warnings
        .iter()
        .any(|w| w.code == "agent_overwrite_kit"));
    apply(&p, &plan.plan_id, BTreeMap::new()).await.unwrap();
    let lock = KitsLock::read(&p.env.lock_path).unwrap();
    assert!(lock.kits["other/kit"].files["agents/planner.md"].skipped);
    assert!(!lock.kits[KIT].files["agents/planner.md"].skipped);
}

#[tokio::test]
async fn major_jumps_are_flagged_and_out_of_range_workers_update() {
    let p = project();
    installed_v1(&p).await;
    p.registry.publish(
        "2.0.0",
        &[("agents/planner.md", PLANNER_V2)],
        &[("kanban", "^2.0", "2.1.0")],
    );
    let out = service::plan_update(&p.env, &p.registry, &p.compose, KIT, Some("2.0.0"))
        .await
        .unwrap();
    let plan = out.plan.unwrap();
    assert!(plan.major);
    assert_eq!(plan.warnings[0].code, "major_update");
    let kanban = &plan.workers[0];
    assert_eq!(kanban.action, WorkerAction::Update);
    assert_eq!(kanban.to.as_deref(), Some("2.1.0"));
    // Same-version plan-update is "up to date".
    let out = service::plan_update(&p.env, &p.registry, &p.compose, KIT, Some("1.0.0"))
        .await
        .unwrap();
    assert_eq!(out.status, PlanStatus::UpToDate);
}

// ───────────────────────── removal ───────────────────────────────────

#[tokio::test]
async fn removal_deletes_intact_files_keeps_edited_and_drops_the_lock() {
    let p = project();
    installed_v1(&p).await;
    let flow = p.env.skills_folder.join("acme/team/tickets/flow.md");
    write(&flow, "# Flow\n\nmine\n");
    let out = service::plan_remove(&p.env, &p.compose, KIT).await.unwrap();
    let plan = out.plan.unwrap();
    assert_eq!(plan.kind, PlanKind::Remove);
    let flow_f = plan
        .files
        .iter()
        .find(|f| f.path.ends_with("flow.md"))
        .unwrap();
    assert_eq!(flow_f.default, Some(Decision::Keep));
    assert_eq!(plan.warnings[0].code, "edited_file_kept");
    let kanban = &plan.workers[0];
    assert_eq!(kanban.action, WorkerAction::Remove);

    // Untick nothing: workers stay.
    let out = service::run_apply(
        &p.env,
        &p.registry,
        &p.compose,
        &plan.plan_id,
        &BTreeMap::new(),
        &["kanban".to_string()],
        timing(),
        &noop,
    )
    .await
    .unwrap();
    let report = out.report.unwrap();
    assert_eq!(report.workers_removed, vec!["kanban"]);
    assert!(p
        .log
        .lock()
        .unwrap()
        .contains(&"compose::remove kanban".to_string()));
    assert!(!p.env.agents_folder.join("planner.md").exists());
    assert_eq!(read(&flow), "# Flow\n\nmine\n");
    // Last kit removed: the lock file goes away.
    assert!(!p.env.lock_path.exists());
    assert!(!p.env.skills_folder.join("acme/team/legacy").exists());
}

// ───────────────────────── updates and listing ───────────────────────

#[tokio::test]
async fn check_updates_honours_ignored_versions_and_list_counts_attention() {
    let p = project();
    installed_v1(&p).await;
    *p.registry.updates.lock().unwrap() = vec![UpdateAnswer {
        kit: KIT.into(),
        current: Some("1.0.0".into()),
        available: Some("1.1.0".into()),
        latest: Some("1.1.0".into()),
        major: false,
        deprecation: None,
        not_found: false,
    }];
    let (cache, changed) = service::check_updates(&p.env, &p.registry, None)
        .await
        .unwrap();
    assert!(changed);
    assert_eq!(cache.kits[KIT].available.as_deref(), Some("1.1.0"));
    let listing = service::list(&p.env).unwrap();
    assert_eq!(listing.attention, 1);
    assert_eq!(listing.kits[0].files.agents, 2);

    let mut lock = KitsLock::read(&p.env.lock_path).unwrap();
    super::apply::set_ignored(&mut lock, KIT, "1.1.0", true).unwrap();
    lock.write(&p.env.lock_path).unwrap();
    let (cache, changed) = service::check_updates(&p.env, &p.registry, None)
        .await
        .unwrap();
    assert!(changed);
    assert_eq!(cache.kits[KIT].available, None);
    assert_eq!(cache.kits[KIT].ignored.as_deref(), Some("1.1.0"));
    assert_eq!(service::list(&p.env).unwrap().attention, 0);
    assert!(KitsLock::read(&p.env.lock_path)
        .unwrap()
        .render()
        .unwrap()
        .contains("ignored_versions"));
}

#[tokio::test]
async fn get_reports_file_states_and_diff_serves_both_sides() {
    let p = project();
    installed_v1(&p).await;
    let flow = p.env.skills_folder.join("acme/team/tickets/flow.md");
    write(&flow, "# Flow edited\n");
    std::fs::remove_file(p.env.agents_folder.join("reviewer.md")).unwrap();
    let info = service::get(&p.env, &p.registry, KIT, true).await.unwrap();
    assert_eq!(info.installed.files.edited, 1);
    assert_eq!(info.installed.files.missing, 1);
    assert_eq!(info.readme.as_deref(), Some("# team\n"));
    let flow_f = info
        .file_list
        .iter()
        .find(|f| f.path.ends_with("flow.md"))
        .unwrap();
    assert_eq!(flow_f.id, "acme/team/tickets/flow");
    let sides = service::file_sides(&p.env, &p.registry, KIT, &flow_f.path)
        .await
        .unwrap();
    assert_eq!(sides["base"], FLOW_V1);
    assert_eq!(sides["local"], "# Flow edited\n");
    assert_eq!(sides["state"], "edited");
}

#[tokio::test]
async fn origins_from_the_lock_feed_agent_sources() {
    let p = project();
    installed_v1(&p).await;
    let index = super::origin::OriginIndex::load(
        KitsLock::read(&p.env.lock_path).unwrap(),
        &p.env.skills_folder,
    );
    let planner = index.classify_project_agent("planner", &p.env.agents_folder.join("planner.md"));
    assert_eq!(planner.kind, SourceKind::Kit);
    assert_eq!(planner.version.as_deref(), Some("1.0.0"));
    write(&p.env.agents_folder.join("planner.md"), "edited");
    let planner = index.classify_project_agent("planner", &p.env.agents_folder.join("planner.md"));
    assert_eq!(planner.modified, Some(true));
}
