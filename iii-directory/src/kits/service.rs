//! The kit operations behind the `directory::kits::*` functions, written
//! against the [`KitRegistry`] and [`ComposeControl`] seams so every flow
//! runs in tests without an engine or a registry.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::apply::{apply_plan, ApplyError, ApplyReport, ApplyTiming, DecisionInput};
use super::compose::ComposeControl;
use super::kitref::{KitRef, VersionRef};
use super::lock::{file_state, local_sha, FileState, KitsLock, LockedKit};
use super::plan::{
    auto_applicable, build_install_plan, build_remove_plan, build_update_plan, summary, KitsEnv,
    LocalView, Plan, PlanContents, PlanKind,
};
use super::registry::{Deprecation, KitDetail, KitRegistry, UpdateQuery};
use super::store::{self, ApplyProgress, PlanRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// A plan is waiting for review (or for `directory::kits::apply`).
    Planned,
    /// `apply=true` and the plan needed no review: it was applied.
    Applied,
    /// The installed version is already the one asked for.
    UpToDate,
    /// The project changed since the plan was made; this is a fresh plan.
    Replanned,
}

/// Response of `directory::download-kit`, `plan-update`, `remove` and `apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KitPlanResponse {
    pub status: PlanStatus,
    pub kit: String,
    /// One sentence for a terminal user.
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ApplyReport>,
}

/// Resolve a kit ref to the detail of one version.
pub async fn resolve_version(
    registry: &dyn KitRegistry,
    kit: &str,
    version: &VersionRef,
) -> Result<KitDetail, String> {
    let wanted = match version {
        VersionRef::Exact(v) | VersionRef::Tag(v) => v.clone(),
        VersionRef::Range(range) => {
            let versions = registry
                .versions(kit)
                .await?
                .ok_or_else(|| kit_not_found(kit))?;
            let req = semver::VersionReq::parse(range).map_err(|e| e.to_string())?;
            versions
                .versions
                .iter()
                .filter_map(|v| semver::Version::parse(&v.version).ok())
                .filter(|v| req.matches(v))
                .max()
                .map(|v| v.to_string())
                .ok_or_else(|| {
                    format!(
                        "D510 not_found: no version of {kit} satisfies {range:?}. \
                         Next: open the kit's versions in the registry."
                    )
                })?
        }
    };
    match registry.detail(kit, &wanted).await? {
        Some(mut detail) => {
            if detail.id.is_empty() {
                detail.id = kit.to_string();
            }
            Ok(detail)
        }
        None => match registry.detail(kit, "latest").await? {
            Some(_) => Err(format!(
                "D510 not_found: {kit} has no version or tag {wanted:?}. \
                 Next: call directory::download-kit kit={kit} for the latest release."
            )),
            None => Err(kit_not_found(kit)),
        },
    }
}

/// What the lock records as `requested`. An exact version installs exactly
/// that version but is recorded as its caret range (`1.3.0` → `^1.3.0`),
/// the way npm saves an exact install, so later compatible releases are
/// offered as updates. Tags and ranges are recorded as given.
pub fn requested_for(version: &VersionRef) -> String {
    match version {
        VersionRef::Exact(v) => format!("^{v}"),
        other => other.as_str().to_string(),
    }
}

fn kit_not_found(kit: &str) -> String {
    format!(
        "D510 not_found: kit {kit:?} does not exist in the registry. \
         Next: check the <author>/<kit> spelling on the registry's kits page."
    )
}

async fn compose_ok(compose: &dyn ComposeControl) -> bool {
    compose.available().await.is_ok()
}

fn save_plan(env: &KitsEnv, plan: &Plan, contents: PlanContents) -> Result<(), String> {
    store::save(
        &env.plans_dir,
        &PlanRecord {
            plan: plan.clone(),
            contents,
            progress: None,
        },
    )
}

/// Drop older pending plans of the same kit and kind: one review at a time.
fn supersede(env: &KitsEnv, kit: &str, kind: PlanKind, keep: &str) {
    for record in store::for_kit(&env.plans_dir, kit, chrono::Utc::now()) {
        if record.plan.kind == kind && record.plan.plan_id != keep {
            let _ = store::delete(&env.plans_dir, &record.plan.plan_id);
        }
    }
}

fn planned_message(plan: &Plan) -> String {
    let verb = match plan.kind {
        PlanKind::Install => format!("Install {} {}", plan.kit, plan.to.as_deref().unwrap_or("")),
        PlanKind::Update => format!(
            "Update {} {} → {}",
            plan.kit,
            plan.from.as_deref().unwrap_or("?"),
            plan.to.as_deref().unwrap_or("?")
        ),
        PlanKind::Remove => format!("Remove {} {}", plan.kit, plan.from.as_deref().unwrap_or("")),
    };
    let mut extra = Vec::new();
    if !plan.blocking.is_empty() {
        extra.push(format!("blocked: {}", plan.blocking[0].message));
    }
    if !plan.warnings.is_empty() {
        extra.push(format!(
            "{} warning{}",
            plan.warnings.len(),
            if plan.warnings.len() == 1 { "" } else { "s" }
        ));
    }
    if plan.counts.decisions_required > 0 {
        extra.push(format!(
            "{} decision(s) required",
            plan.counts.decisions_required
        ));
    }
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!(" ({})", extra.join("; "))
    };
    format!(
        "{verb} is planned{extra}. Review it in the ADE: Directory → Kits, or apply it with \
         `iii trigger directory::kits::apply plan_id={}`.",
        plan.plan_id
    )
}

/// `directory::download-kit`.
#[allow(clippy::too_many_arguments)]
pub async fn download_kit(
    env: &KitsEnv,
    registry: &dyn KitRegistry,
    compose: &dyn ComposeControl,
    kit_ref: &str,
    apply: bool,
    timing: ApplyTiming,
    on_progress: &(dyn Fn(&ApplyProgress) + Send + Sync),
) -> Result<KitPlanResponse, String> {
    let parsed = KitRef::parse(kit_ref).map_err(|e| {
        format!("D511 invalid_input: {e}. Use kit=<author>/<kit>[@<version|tag|range>].")
    })?;
    let kit = parsed.id();
    let lock = env.read_lock()?;
    let installed = lock.kits.get(&kit).cloned();
    let (version, requested) = match (&parsed.version, &installed) {
        (Some(v), _) => (v.clone(), requested_for(v)),
        (None, Some(locked)) => (
            VersionRef::parse(&locked.requested).map_err(|e| format!("D511 invalid_input: {e}"))?,
            locked.requested.clone(),
        ),
        (None, None) => (VersionRef::Tag("latest".into()), "latest".to_string()),
    };
    let detail = resolve_version(registry, &kit, &version).await?;
    let available = compose_ok(compose).await;
    let view = LocalView::load(env, &lock, available);
    let now = chrono::Utc::now();
    let (plan, contents) = match &installed {
        Some(locked) if locked.version == detail.version => {
            return Ok(KitPlanResponse {
                status: PlanStatus::UpToDate,
                kit: kit.clone(),
                message: format!("{kit} {} is already installed.", locked.version),
                plan: None,
                report: None,
            })
        }
        Some(locked) => {
            build_update_plan(&view, registry, locked, &detail, &requested, now).await?
        }
        None => build_install_plan(&view, registry, &detail, &requested, now).await?,
    };
    save_plan(env, &plan, contents)?;
    supersede(env, &kit, plan.kind, &plan.plan_id);
    if apply && auto_applicable(&plan) {
        return run_apply(
            env,
            registry,
            compose,
            &plan.plan_id,
            &BTreeMap::new(),
            &[],
            timing,
            on_progress,
        )
        .await;
    }
    let mut message = planned_message(&plan);
    if apply {
        message = format!("Not applied automatically: the plan needs review. {message}");
    }
    Ok(KitPlanResponse {
        status: PlanStatus::Planned,
        kit,
        message,
        plan: Some(plan),
        report: None,
    })
}

/// `directory::kits::plan-update`.
pub async fn plan_update(
    env: &KitsEnv,
    registry: &dyn KitRegistry,
    compose: &dyn ComposeControl,
    kit: &str,
    version: Option<&str>,
) -> Result<KitPlanResponse, String> {
    super::kitref::validate_kit_id(kit).map_err(|e| format!("D511 invalid_input: {e}"))?;
    let lock = env.read_lock()?;
    let locked = lock
        .kits
        .get(kit)
        .cloned()
        .ok_or_else(|| not_installed(kit))?;
    let wanted = VersionRef::parse(version.unwrap_or(&locked.requested))
        .map_err(|e| format!("D511 invalid_input: {e}"))?;
    let requested = match version {
        Some(_) => requested_for(&wanted),
        None => locked.requested.clone(),
    };
    let detail = resolve_version(registry, kit, &wanted).await?;
    if detail.version == locked.version {
        return Ok(KitPlanResponse {
            status: PlanStatus::UpToDate,
            kit: kit.to_string(),
            message: format!("{kit} {} is up to date for {requested:?}.", locked.version),
            plan: None,
            report: None,
        });
    }
    let available = compose_ok(compose).await;
    let view = LocalView::load(env, &lock, available);
    let (plan, contents) = build_update_plan(
        &view,
        registry,
        &locked,
        &detail,
        &requested,
        chrono::Utc::now(),
    )
    .await?;
    save_plan(env, &plan, contents)?;
    supersede(env, kit, plan.kind, &plan.plan_id);
    Ok(KitPlanResponse {
        status: PlanStatus::Planned,
        kit: kit.to_string(),
        message: planned_message(&plan),
        plan: Some(plan),
        report: None,
    })
}

fn not_installed(kit: &str) -> String {
    format!(
        "D510 not_found: kit {kit:?} is not installed in this project. \
         Next: call directory::kits::list to see installed kits."
    )
}

/// `directory::kits::remove`.
pub async fn plan_remove(
    env: &KitsEnv,
    compose: &dyn ComposeControl,
    kit: &str,
) -> Result<KitPlanResponse, String> {
    super::kitref::validate_kit_id(kit).map_err(|e| format!("D511 invalid_input: {e}"))?;
    let lock = env.read_lock()?;
    let locked = lock
        .kits
        .get(kit)
        .cloned()
        .ok_or_else(|| not_installed(kit))?;
    let available = compose_ok(compose).await;
    let view = LocalView::load(env, &lock, available);
    let (plan, contents) = build_remove_plan(&view, kit, &locked, chrono::Utc::now());
    save_plan(env, &plan, contents)?;
    supersede(env, kit, plan.kind, &plan.plan_id);
    Ok(KitPlanResponse {
        status: PlanStatus::Planned,
        kit: kit.to_string(),
        message: planned_message(&plan),
        plan: Some(plan),
        report: None,
    })
}

/// Rebuild a stale plan of the same kind (and target).
async fn replan(
    env: &KitsEnv,
    registry: &dyn KitRegistry,
    compose: &dyn ComposeControl,
    old: &Plan,
) -> Result<KitPlanResponse, String> {
    let _ = store::delete(&env.plans_dir, &old.plan_id);
    match old.kind {
        PlanKind::Remove => plan_remove(env, compose, &old.kit).await,
        PlanKind::Install | PlanKind::Update => {
            let target = old.to.clone().unwrap_or_else(|| old.requested.clone());
            let lock = env.read_lock()?;
            let detail = resolve_version(registry, &old.kit, &VersionRef::parse(&target)?).await?;
            let view = LocalView::load(env, &lock, compose_ok(compose).await);
            let now = chrono::Utc::now();
            let (plan, contents) = match lock.kits.get(&old.kit) {
                Some(locked) if locked.version == detail.version => {
                    return Ok(KitPlanResponse {
                        status: PlanStatus::UpToDate,
                        kit: old.kit.clone(),
                        message: format!("{} {} is already installed.", old.kit, locked.version),
                        plan: None,
                        report: None,
                    })
                }
                Some(locked) => {
                    build_update_plan(&view, registry, locked, &detail, &old.requested, now).await?
                }
                None => build_install_plan(&view, registry, &detail, &old.requested, now).await?,
            };
            save_plan(env, &plan, contents)?;
            Ok(KitPlanResponse {
                status: PlanStatus::Planned,
                kit: old.kit.clone(),
                message: planned_message(&plan),
                plan: Some(plan),
                report: None,
            })
        }
    }
}

/// `directory::kits::apply`.
#[allow(clippy::too_many_arguments)]
pub async fn run_apply(
    env: &KitsEnv,
    registry: &dyn KitRegistry,
    compose: &dyn ComposeControl,
    plan_id: &str,
    decisions: &BTreeMap<String, DecisionInput>,
    remove_workers: &[String],
    timing: ApplyTiming,
    on_progress: &(dyn Fn(&ApplyProgress) + Send + Sync),
) -> Result<KitPlanResponse, String> {
    let record = store::load(&env.plans_dir, plan_id)?.ok_or_else(|| {
        format!(
            "D512 not_found: plan {plan_id:?} does not exist (applied, discarded or expired). \
             Next: call directory::kits::list to see pending plans."
        )
    })?;
    let old = record.plan.clone();
    match apply_plan(
        env,
        registry,
        compose,
        record,
        decisions,
        remove_workers,
        timing,
        on_progress,
    )
    .await
    {
        Ok(report) => {
            let message = applied_message(&old, &report);
            Ok(KitPlanResponse {
                status: PlanStatus::Applied,
                kit: old.kit.clone(),
                message,
                plan: None,
                report: Some(report),
            })
        }
        Err(ApplyError::Stale(reason)) => {
            let mut fresh = replan(env, registry, compose, &old).await?;
            if fresh.status == PlanStatus::Planned {
                fresh.status = PlanStatus::Replanned;
                fresh.message = format!("Not applied: {reason}. {}", fresh.message);
            }
            Ok(fresh)
        }
        Err(ApplyError::Failed(e)) => Err(e),
    }
}

fn applied_message(plan: &Plan, report: &ApplyReport) -> String {
    let what = match plan.kind {
        PlanKind::Install => format!(
            "Installed {} {}",
            plan.kit,
            plan.to.as_deref().unwrap_or("")
        ),
        PlanKind::Update => format!(
            "Updated {} {} → {}",
            plan.kit,
            plan.from.as_deref().unwrap_or("?"),
            plan.to.as_deref().unwrap_or("?")
        ),
        PlanKind::Remove => format!("Removed {}", plan.kit),
    };
    let mut parts = vec![format!(
        "{} file(s) written",
        report.files_written.len() + report.files_merged.len()
    )];
    if !report.files_removed.is_empty() {
        parts.push(format!("{} removed", report.files_removed.len()));
    }
    if !report.files_kept.is_empty() {
        parts.push(format!("{} kept", report.files_kept.len()));
    }
    let workers = report.workers_added.len() + report.workers_updated.len();
    if workers > 0 {
        parts.push(format!("{workers} worker(s) added or updated"));
    }
    format!("{what}: {}.", parts.join(", "))
}

/// `directory::kits::discard`.
pub fn discard(env: &KitsEnv, plan_id: &str) -> Result<bool, String> {
    store::delete(&env.plans_dir, plan_id)
}

// ───────────────────────── update checks ─────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitUpdate {
    pub current: String,
    /// Newest version satisfying `requested`, unless the user ignored it.
    pub available: Option<String>,
    pub latest: Option<String>,
    pub major: bool,
    /// `available` would be this version, but the user ignored it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecation: Option<Deprecation>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub not_found: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UpdatesCache {
    pub checked_at: Option<String>,
    #[serde(default)]
    pub kits: BTreeMap<String, KitUpdate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn read_updates(env: &KitsEnv) -> UpdatesCache {
    std::fs::read(&env.updates_cache)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_updates(env: &KitsEnv, cache: &UpdatesCache) {
    if let Ok(bytes) = serde_json::to_vec_pretty(cache) {
        if let Err(e) = crate::sources::write_file_atomic(&env.updates_cache, &bytes) {
            tracing::warn!(error = %e, "could not cache kit update check");
        }
    }
}

/// `directory::kits::check-updates`: ask the registry about every installed
/// kit (or one), honour `ignored_versions`, cache the answer. Returns the
/// cache and whether the set of available updates changed.
pub async fn check_updates(
    env: &KitsEnv,
    registry: &dyn KitRegistry,
    only: Option<&str>,
) -> Result<(UpdatesCache, bool), String> {
    let lock = env.read_lock()?;
    let queries: Vec<UpdateQuery> = lock
        .kits
        .iter()
        .filter(|(k, _)| only.is_none_or(|o| o == k.as_str()))
        .map(|(k, l)| UpdateQuery {
            kit: k.clone(),
            version: l.version.clone(),
            requested: l.requested.clone(),
        })
        .collect();
    if let Some(o) = only {
        if queries.is_empty() {
            return Err(not_installed(o));
        }
    }
    let previous = read_updates(env);
    let mut cache = UpdatesCache {
        checked_at: Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        kits: if only.is_some() {
            previous.kits.clone()
        } else {
            BTreeMap::new()
        },
        error: None,
    };
    let answers = match registry.updates(&queries).await {
        Ok(a) => a,
        Err(e) => {
            cache.kits = previous.kits.clone();
            cache.error = Some(e.clone());
            write_updates(env, &cache);
            return Err(e);
        }
    };
    for answer in answers {
        let Some(locked) = lock.kits.get(&answer.kit) else {
            continue;
        };
        let ignored = answer
            .available
            .as_ref()
            .filter(|v| locked.ignored_versions.contains(v))
            .cloned();
        cache.kits.insert(
            answer.kit.clone(),
            KitUpdate {
                current: locked.version.clone(),
                available: if ignored.is_some() {
                    None
                } else {
                    answer.available.clone()
                },
                latest: answer.latest.clone(),
                major: answer.major && ignored.is_none(),
                ignored,
                deprecation: answer.deprecation.clone(),
                not_found: answer.not_found,
            },
        );
    }
    // Forget kits that are no longer installed.
    cache.kits.retain(|k, _| lock.kits.contains_key(k));
    let changed = available_set(&cache) != available_set(&previous);
    write_updates(env, &cache);
    Ok((cache, changed))
}

fn available_set(cache: &UpdatesCache) -> BTreeMap<String, String> {
    cache
        .kits
        .iter()
        .filter_map(|(k, u)| u.available.clone().map(|v| (k.clone(), v)))
        .collect()
}

// ───────────────────────── list / get ────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileCounts {
    pub agents: usize,
    pub skills: usize,
    pub intact: usize,
    pub edited: usize,
    pub missing: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InstalledKit {
    pub kit: String,
    pub version: String,
    pub requested: String,
    pub installed_at: String,
    pub workers: BTreeMap<String, String>,
    pub files: FileCounts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<KitUpdate>,
    pub registry_url: String,
    /// Plans of this kit waiting for review.
    pub pending: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitsListing {
    pub kits: Vec<InstalledKit>,
    /// Pending plans (summaries).
    pub pending: Vec<serde_json::Value>,
    pub updates_checked_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates_error: Option<String>,
    /// Pending plans plus available updates: what the Kits badge counts.
    pub attention: usize,
    pub lock_path: String,
}

fn counts_for(env: &KitsEnv, locked: &LockedKit) -> (FileCounts, Vec<InstalledFile>) {
    let mut counts = FileCounts::default();
    let mut files = Vec::new();
    for (path, entry) in &locked.files {
        let local = env.abs(path).as_deref().and_then(local_sha);
        let state = file_state(entry, local.as_deref());
        if path.starts_with("agents/") {
            counts.agents += 1;
        } else {
            counts.skills += 1;
        }
        match state {
            FileState::Intact => counts.intact += 1,
            FileState::Edited => counts.edited += 1,
            FileState::Missing => counts.missing += 1,
            FileState::Skipped => counts.skipped += 1,
        }
        files.push(InstalledFile {
            path: path.clone(),
            kind: if path.starts_with("agents/") {
                "agent".into()
            } else {
                "skill".into()
            },
            id: super::paths::agent_id_for_path(path).unwrap_or_else(|| {
                path.trim_start_matches("skills/")
                    .trim_end_matches(".md")
                    .to_string()
            }),
            state,
            sha256: entry.sha256.clone(),
            local_sha256: local,
        });
    }
    (counts, files)
}

/// `directory::kits::list`.
pub fn list(env: &KitsEnv) -> Result<KitsListing, String> {
    let lock = env.read_lock()?;
    let now = chrono::Utc::now();
    let pending = store::list(&env.plans_dir, now);
    let updates = read_updates(env);
    let kits: Vec<InstalledKit> = lock
        .kits
        .iter()
        .map(|(kit, locked)| {
            let (files, _) = counts_for(env, locked);
            InstalledKit {
                kit: kit.clone(),
                version: locked.version.clone(),
                requested: locked.requested.clone(),
                installed_at: locked.installed_at.clone(),
                workers: locked.workers.clone(),
                files,
                update: updates
                    .kits
                    .get(kit)
                    .filter(|u| u.current == locked.version)
                    .cloned(),
                registry_url: env.kit_web_url(kit),
                pending: pending
                    .iter()
                    .filter(|r| &r.plan.kit == kit)
                    .map(|r| r.plan.plan_id.clone())
                    .collect(),
            }
        })
        .collect();
    let attention = pending.len()
        + kits
            .iter()
            .filter(|k| {
                k.update.as_ref().is_some_and(|u| u.available.is_some())
                    && !pending
                        .iter()
                        .any(|r| r.plan.kit == k.kit && r.plan.kind == PlanKind::Update)
            })
            .count();
    Ok(KitsListing {
        kits,
        pending: pending.iter().map(|r| summary(&r.plan)).collect(),
        updates_checked_at: updates.checked_at,
        updates_error: updates.error,
        attention,
        lock_path: env.lock_path.display().to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InstalledFile {
    pub path: String,
    pub kind: String,
    /// Agent id, or the skill id.
    pub id: String,
    pub state: FileState,
    /// Base sha256 from the lock.
    pub sha256: String,
    /// sha256 on disk, `null` when missing.
    pub local_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KitInfo {
    #[serde(flatten)]
    pub installed: InstalledKit,
    pub file_list: Vec<InstalledFile>,
    /// Registry detail of the installed version, when reachable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<KitDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readme: Option<String>,
    /// Workers as Compose has them now.
    pub worker_status: Vec<serde_json::Value>,
}

/// `directory::kits::get`.
pub async fn get(
    env: &KitsEnv,
    registry: &dyn KitRegistry,
    kit: &str,
    include_registry: bool,
) -> Result<KitInfo, String> {
    let listing = list(env)?;
    let installed = listing
        .kits
        .into_iter()
        .find(|k| k.kit == kit)
        .ok_or_else(|| not_installed(kit))?;
    let lock = env.read_lock()?;
    let locked = &lock.kits[kit];
    let (_, mut file_list) = counts_for(env, locked);
    for f in &mut file_list {
        if f.kind == "skill" {
            if let Some(source) = super::paths::source_for_install_path(kit, &f.path) {
                if let Some(id) = super::paths::skill_id_for_source(kit, &source) {
                    f.id = id;
                }
            }
        }
    }
    let (detail, readme) = if include_registry {
        let detail = registry.detail(kit, &locked.version).await.ok().flatten();
        let readme = registry.readme(kit, &locked.version).await.ok().flatten();
        (detail, readme)
    } else {
        (None, None)
    };
    let declared = super::compose::declared_workers(&env.compose_file);
    let worker_status = locked
        .workers
        .iter()
        .map(|(name, range)| {
            let d = declared.get(name);
            let installed = d.and_then(|d| d.resolved.clone());
            serde_json::json!({
                "name": name,
                "range": range,
                "declared": d.and_then(|d| d.selector.clone()),
                "installed": installed,
                "satisfied": installed.as_deref().and_then(|v| super::kitref::satisfies(range, v)),
                "path": d.is_some_and(|d| d.path),
                "registry_url": env.worker_web_url(name),
            })
        })
        .collect();
    Ok(KitInfo {
        installed,
        file_list,
        detail,
        readme,
        worker_status,
    })
}

/// `directory::kits::diff`: the base the kit installed and the file on disk.
pub async fn file_sides(
    env: &KitsEnv,
    registry: &dyn KitRegistry,
    kit: &str,
    path: &str,
) -> Result<serde_json::Value, String> {
    let lock = env.read_lock()?;
    let locked = lock.kits.get(kit).ok_or_else(|| not_installed(kit))?;
    let entry = locked
        .files
        .get(path)
        .ok_or_else(|| format!("D510 not_found: {path} is not a file of {kit}."))?;
    let base = registry.blob(&entry.sha256).await?;
    let local = env.abs(path).and_then(|p| std::fs::read_to_string(p).ok());
    Ok(serde_json::json!({
        "kit": kit,
        "path": path,
        "base": base,
        "local": local,
        "state": file_state(entry, local.as_deref().map(|t| super::lock::sha256_hex(t.as_bytes())).as_deref()),
    }))
}

/// Kit namespaces (`<handle>/<kit>`) from the project's lock, for the skill
/// reader. Never fails.
pub fn kit_namespaces() -> Vec<String> {
    KitsLock::read_lenient(&super::paths::kits_lock()).namespaces()
}
