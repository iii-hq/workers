//! `directory::kits::apply` — execute a reviewed plan.
//!
//! Order, so a failure never leaves files that disagree with the workers or
//! a lock that disagrees with the files:
//!
//! 1. Re-validate: the plan has not expired, the lock still holds the
//!    version the plan was made against, and every local file the plan
//!    looked at is unchanged (files an interrupted attempt already wrote
//!    count as expected). Anything else means the plan is stale and is built
//!    again instead of applied.
//! 2. `GET /k/…/download` — counts the download and returns the bodies,
//!    each checked against the sha the plan reviewed.
//! 3. Workers through Compose (`compose::add` with `{worker, version: range}`
//!    for added / moved / re-declared workers, `compose::remove` only for
//!    workers the caller ticked), followed through `compose::operation`. A
//!    failure stops here, before any file is written.
//! 4. Files, one atomic write (or delete) at a time, journaled in the plan
//!    record so a retry converges.
//! 5. `kits.lock`, last.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::compose::{wait_operation, ComposeControl, OperationOutcome};
use super::lock::{local_sha, sha256_hex, KitsLock, LockedFile, LockedKit};
use super::merge::has_conflict_markers;
use super::origin::SourceKind;
use super::plan::{Change, Decision, FileKind, KitsEnv, Plan, PlanFile, PlanKind, WorkerAction};
use super::registry::KitRegistry;
use super::store::{self, ApplyProgress, ApplyStep, PlanRecord, StepState};

/// A caller's choice for one file: a bare decision, or a decision with the
/// content to write (`merged` from the conflict editor).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum DecisionInput {
    Choice(Decision),
    Detailed {
        choice: Decision,
        #[serde(default)]
        content: Option<String>,
    },
}

impl DecisionInput {
    fn parts(&self) -> (Decision, Option<&str>) {
        match self {
            Self::Choice(d) => (*d, None),
            Self::Detailed { choice, content } => (*choice, content.as_deref()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApplyReport {
    pub plan_id: String,
    pub kind: Option<PlanKind>,
    pub kit: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub steps: Vec<ApplyStep>,
    pub files_written: Vec<String>,
    pub files_merged: Vec<String>,
    pub files_removed: Vec<String>,
    /// Files left as they are (kept, skipped, or mine).
    pub files_kept: Vec<String>,
    pub workers_added: Vec<String>,
    pub workers_updated: Vec<String>,
    pub workers_removed: Vec<String>,
    /// Agent profile ids this kit now provides.
    pub agents: Vec<String>,
    pub skills_changed: bool,
    pub agents_changed: bool,
    pub lock_path: String,
}

#[derive(Debug)]
pub enum ApplyError {
    /// The plan no longer matches the project; build it again.
    Stale(String),
    /// A handler error (D-coded prose), with the steps reached so far.
    Failed(String),
}

impl From<String> for ApplyError {
    fn from(value: String) -> Self {
        Self::Failed(value)
    }
}

/// Timing for Compose polling; tests shrink it.
#[derive(Debug, Clone, Copy)]
pub struct ApplyTiming {
    pub poll: Duration,
    pub deadline: Duration,
}

impl Default for ApplyTiming {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(750),
            deadline: Duration::from_secs(15 * 60),
        }
    }
}

fn initial_steps(plan: &Plan) -> Vec<ApplyStep> {
    let workers = plan
        .workers
        .iter()
        .filter(|w| {
            matches!(
                w.action,
                WorkerAction::Add | WorkerAction::Update | WorkerAction::Redeclare
            )
        })
        .count();
    vec![
        ApplyStep {
            id: "workers".into(),
            label: if workers == 0 {
                "Workers".into()
            } else {
                format!("Workers ({workers})")
            },
            state: StepState::Pending,
            detail: None,
        },
        ApplyStep {
            id: "files".into(),
            label: format!("Files ({})", plan.files.len()),
            state: StepState::Pending,
            detail: None,
        },
        ApplyStep {
            id: "lock".into(),
            label: "kits.lock".into(),
            state: StepState::Pending,
            detail: None,
        },
    ]
}

fn set_step(progress: &mut ApplyProgress, id: &str, state: StepState, detail: Option<String>) {
    if let Some(step) = progress.steps.iter_mut().find(|s| s.id == id) {
        step.state = state;
        if detail.is_some() {
            step.detail = detail;
        }
    }
}

/// Step 1: is the plan still what the project looks like?
pub fn revalidate(
    env: &KitsEnv,
    record: &PlanRecord,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), ApplyError> {
    let plan = &record.plan;
    if store::is_expired(plan, now) {
        return Err(ApplyError::Failed(format!(
            "D512 invalid_input: plan {} expired at {}. Next: call directory::download-kit kit={} to plan again.",
            plan.plan_id, plan.expires_at, plan.kit
        )));
    }
    let lock = env.read_lock()?;
    let installed = lock.kits.get(&plan.kit).map(|k| k.version.clone());
    match plan.kind {
        PlanKind::Install if installed.is_some() => {
            return Err(ApplyError::Stale(format!(
                "{} was installed after this plan was made",
                plan.kit
            )))
        }
        PlanKind::Update | PlanKind::Remove if installed != plan.from => {
            return Err(ApplyError::Stale(format!(
                "kits.lock now has {} {}, not {}",
                plan.kit,
                installed.as_deref().unwrap_or("not installed"),
                plan.from.as_deref().unwrap_or("?")
            )))
        }
        _ => {}
    }
    let written = record
        .progress
        .as_ref()
        .map(|p| p.written.clone())
        .unwrap_or_default();
    for file in &plan.files {
        let current = env.abs(&file.path).as_deref().and_then(local_sha);
        if current == file.ours {
            continue;
        }
        let journaled = written.get(&file.path).map(|sha| {
            if sha.is_empty() {
                current.is_none()
            } else {
                current.as_deref() == Some(sha.as_str())
            }
        });
        if journaled == Some(true) {
            continue;
        }
        return Err(ApplyError::Stale(format!(
            "{} changed on disk after the plan was made",
            file.path
        )));
    }
    Ok(())
}

/// What will be done to each file: the decision and the bytes to write.
#[derive(Debug, Clone)]
struct FileAction {
    decision: Decision,
    write: Option<String>,
    delete: bool,
}

/// Step 1b: resolve and validate the caller's decisions.
fn resolve_decisions(
    record: &PlanRecord,
    decisions: &BTreeMap<String, DecisionInput>,
    bodies: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, FileAction>, String> {
    let plan = &record.plan;
    let mut problems = Vec::new();
    for path in decisions.keys() {
        if !plan.files.iter().any(|f| &f.path == path) {
            problems.push(format!("{path} is not part of plan {}", plan.plan_id));
        }
    }
    let mut out = BTreeMap::new();
    for file in &plan.files {
        let input = decisions.get(&file.path).map(|d| d.parts());
        let (decision, content) = match input {
            Some((d, content)) => (d, content),
            None => match file.default {
                Some(d) => (d, None),
                None => {
                    problems.push(format!(
                        "{} needs a decision ({})",
                        file.path,
                        options_text(file)
                    ));
                    continue;
                }
            },
        };
        if !file.options.contains(&decision) {
            problems.push(format!(
                "{} cannot take {:?}; choose {}",
                file.path,
                decision_word(decision),
                options_text(file)
            ));
            continue;
        }
        let theirs = || {
            file.theirs
                .as_ref()
                .and_then(|sha| bodies.get(sha).cloned())
        };
        let action = match decision {
            Decision::Overwrite | Decision::Kit => FileAction {
                decision,
                write: theirs(),
                delete: false,
            },
            Decision::Merged => {
                let text = match content {
                    Some(text) => text.to_string(),
                    None => match record.contents.merged.get(&file.path) {
                        Some(text) if file.merge == Some(super::merge::MergeStatus::Clean) => {
                            text.clone()
                        }
                        _ => {
                            problems.push(format!(
                                "{} has conflicts: pass the resolved text as {{\"choice\": \"merged\", \"content\": …}}",
                                file.path
                            ));
                            continue;
                        }
                    },
                };
                if has_conflict_markers(&text) {
                    problems.push(format!("{} still has conflict markers", file.path));
                    continue;
                }
                FileAction {
                    decision,
                    write: Some(text),
                    delete: false,
                }
            }
            Decision::Remove => FileAction {
                decision,
                write: None,
                delete: true,
            },
            Decision::Keep | Decision::Mine => FileAction {
                decision,
                write: None,
                delete: false,
            },
        };
        if matches!(decision, Decision::Overwrite | Decision::Kit) && action.write.is_none() {
            problems.push(format!("{} has no downloaded content", file.path));
            continue;
        }
        out.insert(file.path.clone(), action);
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(format!(
            "D513 invalid_input: {}. Next: call directory::kits::plan plan_id={} to see each file's options.",
            problems.join("; "),
            plan.plan_id
        ))
    }
}

fn decision_word(d: Decision) -> &'static str {
    match d {
        Decision::Overwrite => "overwrite",
        Decision::Keep => "keep",
        Decision::Kit => "kit",
        Decision::Mine => "mine",
        Decision::Merged => "merged",
        Decision::Remove => "remove",
    }
}

fn options_text(file: &PlanFile) -> String {
    file.options
        .iter()
        .map(|d| decision_word(*d))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Run a validated plan. `on_progress` sees every step transition.
#[allow(clippy::too_many_arguments)]
pub async fn apply_plan(
    env: &KitsEnv,
    registry: &dyn KitRegistry,
    compose: &dyn ComposeControl,
    mut record: PlanRecord,
    decisions: &BTreeMap<String, DecisionInput>,
    remove_workers: &[String],
    timing: ApplyTiming,
    on_progress: &(dyn Fn(&ApplyProgress) + Send + Sync),
) -> Result<ApplyReport, ApplyError> {
    let now = chrono::Utc::now();
    revalidate(env, &record, now)?;
    let plan = record.plan.clone();
    if let Some(block) = plan
        .blocking
        .iter()
        .find(|b| b.code != "compose_not_running")
    {
        return Err(ApplyError::Failed(format!(
            "D514 invalid_input: the plan is blocked: {}",
            block.message
        )));
    }
    if plan
        .blocking
        .iter()
        .any(|b| b.code == "compose_not_running")
    {
        // Compose may have come up since; a fresh plan says so.
        return Err(match compose.available().await {
            Ok(()) => ApplyError::Stale("the Compose daemon is running now".into()),
            Err(_) => ApplyError::Failed(format!(
                "D514 invalid_input: the plan is blocked: {}",
                plan.blocking[0].message
            )),
        });
    }
    let removable: BTreeSet<&str> = plan
        .workers
        .iter()
        .filter(|w| w.action == WorkerAction::Remove)
        .map(|w| w.name.as_str())
        .collect();
    if let Some(bad) = remove_workers
        .iter()
        .find(|w| !removable.contains(w.as_str()))
    {
        return Err(ApplyError::Failed(format!(
            "D513 invalid_input: worker {bad:?} is not one this plan can remove (only workers the kit no longer declares, or the kit's own on removal)."
        )));
    }

    // Decisions are checked against the bodies the plan reviewed before the
    // download, so a rejected apply never counts one.
    let actions = resolve_decisions(&record, decisions, &record.contents.blobs)?;

    // Step 2: the counted download.
    let mut bodies: BTreeMap<String, String> = BTreeMap::new();
    let mut downloaded: BTreeMap<String, String> = BTreeMap::new();
    if let Some(to) = &plan.to {
        let files = registry.download(&plan.kit, to, env.ci).await?;
        for file in files.files {
            let sha = sha256_hex(file.content.as_bytes());
            if let Some(path) = super::paths::install_path_for_source(&plan.kit, &file.path) {
                downloaded.insert(path, sha.clone());
            }
            bodies.insert(sha, file.content);
        }
        for file in &plan.files {
            if let Some(theirs) = &file.theirs {
                if downloaded.get(&file.path) != Some(theirs) {
                    return Err(ApplyError::Failed(format!(
                        "D516 integrity: the registry served different content for {} than the plan reviewed. \
                         Next: call directory::download-kit kit={} to review the current version.",
                        file.path, plan.kit
                    )));
                }
            }
        }
    }
    // The served bodies are the reviewed ones (checked above); write those.
    let actions: BTreeMap<String, FileAction> = actions
        .into_iter()
        .map(|(path, mut action)| {
            if matches!(action.decision, Decision::Overwrite | Decision::Kit) {
                let served = plan
                    .files
                    .iter()
                    .find(|f| f.path == path)
                    .and_then(|f| f.theirs.as_ref())
                    .and_then(|sha| bodies.get(sha));
                if let Some(text) = served {
                    action.write = Some(text.clone());
                }
            }
            (path, action)
        })
        .collect();

    let mut progress = record.progress.clone().unwrap_or_default();
    if progress.steps.is_empty() {
        progress.steps = initial_steps(&plan);
    }
    progress.error = None;
    progress.started_at = Some(now.to_rfc3339());
    let mut report = ApplyReport {
        plan_id: plan.plan_id.clone(),
        kind: Some(plan.kind),
        kit: plan.kit.clone(),
        from: plan.from.clone(),
        to: plan.to.clone(),
        lock_path: env.lock_path.display().to_string(),
        ..Default::default()
    };

    // Step 3: workers.
    let to_add: Vec<Value> = plan
        .workers
        .iter()
        .filter(|w| {
            matches!(
                w.action,
                WorkerAction::Add | WorkerAction::Update | WorkerAction::Redeclare
            )
        })
        .filter_map(|w| {
            w.range
                .as_ref()
                .map(|range| json!({ "worker": w.name, "version": range }))
        })
        .collect();
    let to_remove: Vec<String> = plan
        .workers
        .iter()
        .filter(|w| remove_workers.contains(&w.name))
        .map(|w| w.container.clone().unwrap_or_else(|| w.name.clone()))
        .collect();
    if to_add.is_empty() && to_remove.is_empty() {
        set_step(
            &mut progress,
            "workers",
            StepState::Skipped,
            Some("no worker changes".into()),
        );
    } else {
        set_step(&mut progress, "workers", StepState::Running, None);
        save_progress(env, &mut record, &progress, on_progress);
        let result = run_workers(
            compose,
            &to_add,
            &to_remove,
            &plan,
            timing,
            &mut progress,
            &mut record,
            env,
            on_progress,
        )
        .await;
        if let Err(e) = result {
            set_step(&mut progress, "workers", StepState::Failed, Some(e.clone()));
            progress.error = Some(e.clone());
            save_progress(env, &mut record, &progress, on_progress);
            return Err(ApplyError::Failed(e));
        }
        for w in &plan.workers {
            match w.action {
                WorkerAction::Add => report.workers_added.push(w.name.clone()),
                WorkerAction::Update | WorkerAction::Redeclare => {
                    report.workers_updated.push(w.name.clone())
                }
                _ => {}
            }
        }
        report.workers_removed = remove_workers.to_vec();
        set_step(&mut progress, "workers", StepState::Done, None);
    }
    save_progress(env, &mut record, &progress, on_progress);

    // Step 4: files.
    set_step(&mut progress, "files", StepState::Running, None);
    save_progress(env, &mut record, &progress, on_progress);
    let total = plan.files.len();
    for (i, file) in plan.files.iter().enumerate() {
        let action = &actions[&file.path];
        let abs = env
            .abs(&file.path)
            .ok_or_else(|| ApplyError::Failed(format!("invalid install path {}", file.path)))?;
        if let Some(text) = &action.write {
            if let Err(e) = crate::sources::write_file_atomic(&abs, text.as_bytes()) {
                set_step(&mut progress, "files", StepState::Failed, Some(e.clone()));
                progress.error = Some(e.clone());
                save_progress(env, &mut record, &progress, on_progress);
                return Err(ApplyError::Failed(e));
            }
            progress
                .written
                .insert(file.path.clone(), sha256_hex(text.as_bytes()));
            if action.decision == Decision::Merged {
                report.files_merged.push(file.path.clone());
            } else {
                report.files_written.push(file.path.clone());
            }
        } else if action.delete {
            if abs.exists() {
                crate::sources::mark_self_write(&abs);
                if let Err(e) = std::fs::remove_file(&abs) {
                    let e = format!("delete {}: {e}", abs.display());
                    set_step(&mut progress, "files", StepState::Failed, Some(e.clone()));
                    progress.error = Some(e.clone());
                    save_progress(env, &mut record, &progress, on_progress);
                    return Err(ApplyError::Failed(e));
                }
                prune_empty_dirs(&abs, &env.skills_folder);
            }
            progress.written.insert(file.path.clone(), String::new());
            report.files_removed.push(file.path.clone());
        } else {
            report.files_kept.push(file.path.clone());
        }
        if action.write.is_some() || action.delete {
            match file.kind {
                FileKind::Agent => report.agents_changed = true,
                FileKind::Skill => report.skills_changed = true,
            }
        }
        set_step(
            &mut progress,
            "files",
            StepState::Running,
            Some(format!("{}/{total}", i + 1)),
        );
        if (i + 1) % 10 == 0 || i + 1 == total {
            save_progress(env, &mut record, &progress, on_progress);
        }
    }
    let changed =
        report.files_written.len() + report.files_merged.len() + report.files_removed.len();
    set_step(
        &mut progress,
        "files",
        StepState::Done,
        Some(format!(
            "{changed} changed, {} kept",
            report.files_kept.len()
        )),
    );
    save_progress(env, &mut record, &progress, on_progress);

    // Step 5: kits.lock, last.
    set_step(&mut progress, "lock", StepState::Running, None);
    let mut lock = env.read_lock()?;
    let previous = lock.kits.get(&plan.kit).cloned();
    match plan.kind {
        PlanKind::Remove => {
            lock.kits.remove(&plan.kit);
        }
        PlanKind::Install | PlanKind::Update => {
            let entry = next_lock_entry(&plan, previous.as_ref(), &actions, &downloaded, now);
            report.agents = entry
                .files
                .iter()
                .filter(|(_, f)| !f.skipped)
                .filter_map(|(p, _)| super::paths::agent_id_for_path(p))
                .collect();
            lock.kits.insert(plan.kit.clone(), entry);
            // A path this kit took over from another kit is skipped there.
            for file in &plan.files {
                let Some(c) = &file.collision else { continue };
                if c.owner != SourceKind::Kit || actions[&file.path].decision != Decision::Overwrite
                {
                    continue;
                }
                if let Some(other) = c.kit.as_ref().and_then(|k| lock.kits.get_mut(k)) {
                    if let Some(f) = other.files.get_mut(&file.path) {
                        f.skipped = true;
                    }
                }
            }
        }
    }
    if let Err(e) = lock.write(&env.lock_path) {
        set_step(&mut progress, "lock", StepState::Failed, Some(e.clone()));
        progress.error = Some(e.clone());
        save_progress(env, &mut record, &progress, on_progress);
        return Err(ApplyError::Failed(e));
    }
    set_step(&mut progress, "lock", StepState::Done, None);
    on_progress(&progress);
    report.steps = progress.steps.clone();
    let _ = store::delete(&env.plans_dir, &plan.plan_id);
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
async fn run_workers(
    compose: &dyn ComposeControl,
    to_add: &[Value],
    to_remove: &[String],
    plan: &Plan,
    timing: ApplyTiming,
    progress: &mut ApplyProgress,
    record: &mut PlanRecord,
    env: &KitsEnv,
    on_progress: &(dyn Fn(&ApplyProgress) + Send + Sync),
) -> Result<(), String> {
    compose.available().await.map_err(|e| {
        format!(
            "D514 compose_not_running: the Compose daemon did not answer ({e}). Start the project \
             with `iii compose --up` and retry the apply."
        )
    })?;
    let suffix = plan.plan_id.trim_start_matches("kp_");
    if !to_add.is_empty() {
        let names: Vec<String> = to_add
            .iter()
            .map(|w| {
                format!(
                    "{}@{}",
                    w["worker"].as_str().unwrap_or("?"),
                    w["version"].as_str().unwrap_or("?")
                )
            })
            .collect();
        set_step(
            progress,
            "workers",
            StepState::Running,
            Some(format!("compose::add {}", names.join(", "))),
        );
        save_progress(env, record, progress, on_progress);
        let op = format!(
            "kits-add-{suffix}-{}",
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .get(..8)
                .unwrap_or("0")
        );
        let op = compose
            .add(to_add.to_vec(), &op)
            .await
            .map_err(|e| format!("D515 compose_error: compose::add failed: {e}"))?;
        let header = format!("compose::add {}", names.join(", "));
        let mut last = String::new();
        let outcome = wait_operation(compose, &op, timing.poll, timing.deadline, |detail| {
            last = detail.to_string();
        })
        .await?;
        if !last.is_empty() {
            set_step(
                progress,
                "workers",
                StepState::Running,
                Some(format!("{header} — {last}")),
            );
            save_progress(env, record, progress, on_progress);
        }
        if let OperationOutcome::Failed(detail) = outcome {
            return Err(format!(
                "D515 compose_error: compose::add did not finish: {detail}. Nothing was written; \
                 fix the worker and retry the apply."
            ));
        }
    }
    if !to_remove.is_empty() {
        set_step(
            progress,
            "workers",
            StepState::Running,
            Some(format!("compose::remove {}", to_remove.join(", "))),
        );
        save_progress(env, record, progress, on_progress);
        let op = format!("kits-remove-{suffix}");
        let op = compose
            .remove(to_remove.to_vec(), &op)
            .await
            .map_err(|e| format!("D515 compose_error: compose::remove failed: {e}"))?;
        let outcome = wait_operation(compose, &op, timing.poll, timing.deadline, |_| {}).await?;
        if let OperationOutcome::Failed(detail) = outcome {
            return Err(format!(
                "D515 compose_error: compose::remove did not finish: {detail}"
            ));
        }
    }
    Ok(())
}

fn save_progress(
    env: &KitsEnv,
    record: &mut PlanRecord,
    progress: &ApplyProgress,
    on_progress: &(dyn Fn(&ApplyProgress) + Send + Sync),
) {
    record.progress = Some(progress.clone());
    if let Err(e) = store::save(&env.plans_dir, record) {
        tracing::warn!(error = %e, "could not journal kit apply progress");
    }
    on_progress(progress);
}

/// The kit's lock entry after this apply.
fn next_lock_entry(
    plan: &Plan,
    previous: Option<&LockedKit>,
    actions: &BTreeMap<String, FileAction>,
    downloaded: &BTreeMap<String, String>,
    now: chrono::DateTime<chrono::Utc>,
) -> LockedKit {
    let mut files = BTreeMap::new();
    for (path, sha) in downloaded {
        let entry = match plan.files.iter().find(|f| &f.path == path) {
            Some(file) => {
                if file.change == Change::Removed {
                    continue;
                }
                let skipped = matches!(actions[path].decision, Decision::Keep);
                LockedFile {
                    sha256: sha.clone(),
                    skipped,
                }
            }
            // Unchanged by this update: carry the entry (and any skip) over.
            None => previous
                .and_then(|p| p.files.get(path).cloned())
                .unwrap_or(LockedFile {
                    sha256: sha.clone(),
                    skipped: false,
                }),
        };
        files.insert(path.clone(), entry);
    }
    let workers = plan
        .workers
        .iter()
        .filter_map(|w| w.range.as_ref().map(|r| (w.name.clone(), r.clone())))
        .collect();
    LockedKit {
        requested: plan.requested.clone(),
        version: plan.to.clone().unwrap_or_default(),
        installed_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        workers,
        files,
        ignored_versions: previous
            .map(|p| p.ignored_versions.clone())
            .unwrap_or_default(),
    }
}

/// After deleting a kit skill, drop directories it leaves empty, never
/// climbing above `skills_folder`.
fn prune_empty_dirs(file: &std::path::Path, skills_folder: &std::path::Path) {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if !d.starts_with(skills_folder) || d == skills_folder {
            break;
        }
        if std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

/// Lock helper for `directory::kits::ignore`.
pub fn set_ignored(
    lock: &mut KitsLock,
    kit: &str,
    version: &str,
    ignore: bool,
) -> Result<(), String> {
    let entry = lock
        .kits
        .get_mut(kit)
        .ok_or_else(|| format!("D510 not_found: kit {kit:?} is not installed in this project."))?;
    entry.ignored_versions.retain(|v| v != version);
    if ignore {
        entry.ignored_versions.push(version.to_string());
        entry.ignored_versions.sort();
    }
    Ok(())
}
