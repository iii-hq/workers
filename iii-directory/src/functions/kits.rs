//! `directory::download-kit` and `directory::kits::*` — install, update and
//! remove kits through reviewed plans (see [`crate::kits`]).
//!
//! Every write goes plan → review → apply: `download-kit`, `plan-update`
//! and `remove` only build a plan (stored for 24h under
//! `.iii/directory/kit-plans/`), and `apply` executes one. The plan JSON is
//! the contract the Kits page and the chat cards render.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{SharedConfig, SkillsConfig};
use crate::functions::skills::RegisteredWorkersCache;
use crate::kits::apply::{ApplyTiming, DecisionInput};
use crate::kits::compose::EngineCompose;
use crate::kits::plan::{KitsEnv, PlanKind};
use crate::kits::registry::HttpKitRegistry;
use crate::kits::service::{self, KitPlanResponse, PlanStatus};
use crate::kits::store::{self, ApplyProgress};
use crate::trigger_types;

/// How often installed kits are checked for updates (also at boot).
const UPDATE_CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DownloadKitInput {
    /// `<author>/<kit>`, optionally `@<version|tag|range>` (default: the
    /// installed kit's `requested`, else `latest`).
    pub kit: String,
    /// Apply right away when the plan has no warnings, no blocking issue and
    /// no file that needs a decision (for scripts). Otherwise the plan waits
    /// for review.
    #[serde(default)]
    pub apply: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApplyInput {
    pub plan_id: String,
    /// Per-file choices by install path: `"overwrite" | "keep" | "kit" | "mine"
    /// | "merged" | "remove"`, or `{ "choice": "merged", "content": "…" }`.
    /// Files left out take the plan's `default`.
    #[serde(default)]
    pub decisions: Option<BTreeMap<String, DecisionInput>>,
    /// Workers the kit no longer declares (update) or declares (removal) to
    /// remove from the compose file. Never removed otherwise.
    #[serde(default)]
    pub remove_workers: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlanUpdateInput {
    /// Installed kit, `<author>/<kit>`.
    pub kit: String,
    /// Target version, tag or range (default: what the kit was installed with).
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KitInput {
    /// Installed kit, `<author>/<kit>`.
    pub kit: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckUpdatesInput {
    /// Check only this installed kit (default: every installed kit).
    #[serde(default)]
    pub kit: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetInput {
    /// Installed kit, `<author>/<kit>`.
    pub kit: String,
    /// Also fetch the installed version's registry detail and README.
    #[serde(default)]
    pub registry: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlanIdInput {
    pub plan_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlanGetInput {
    pub plan_id: String,
    /// Include file bodies (`blobs` by sha256, `ours` and `merged` by path).
    #[serde(default)]
    pub contents: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IgnoreInput {
    /// Installed kit, `<author>/<kit>`.
    pub kit: String,
    /// Version to skip in update checks.
    pub version: String,
    /// `false` takes the version back off the list (default `true`).
    #[serde(default)]
    pub ignore: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffInput {
    /// Installed kit, `<author>/<kit>`.
    pub kit: String,
    /// Install path from `directory::kits::get` (`agents/<id>.md`, `skills/…`).
    pub path: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiscardOutput {
    pub plan_id: String,
    pub discarded: bool,
}

/// The registry client, rebuilt only when `registry_url` changes so its blob
/// cache survives across calls.
#[derive(Default)]
struct RegistryCell(Mutex<Option<(String, Arc<HttpKitRegistry>)>>);

impl RegistryCell {
    fn get(&self, cfg: &SkillsConfig) -> Arc<HttpKitRegistry> {
        let mut slot = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match &*slot {
            Some((base, reg)) if base == cfg.registry_base() => reg.clone(),
            _ => {
                let reg = Arc::new(HttpKitRegistry::new(
                    cfg.registry_base(),
                    cfg.download_timeout_ms,
                ));
                *slot = Some((cfg.registry_base().to_string(), reg.clone()));
                reg
            }
        }
    }
}

#[derive(Clone)]
struct Ctx {
    iii: Arc<IIIClient>,
    cfg: SharedConfig,
    subs: super::Subscribers,
    cache: Arc<RegisteredWorkersCache>,
    registry: Arc<RegistryCell>,
    /// One apply at a time.
    apply_lock: Arc<tokio::sync::Mutex<()>>,
}

impl Ctx {
    fn env(&self) -> (Arc<SkillsConfig>, KitsEnv, Arc<HttpKitRegistry>) {
        let cfg = self.cfg.load_full();
        let env = KitsEnv::from_config(&cfg);
        let reg = self.registry.get(&cfg);
        (cfg, env, reg)
    }

    async fn emit(&self, payload: Value) {
        trigger_types::dispatch(&self.iii, &self.subs.kits, payload).await;
    }

    fn progress_emitter(
        &self,
        plan_id: String,
        kit: String,
    ) -> impl Fn(&ApplyProgress) + Send + Sync {
        let iii = self.iii.clone();
        let kits = self.subs.kits.clone();
        let rt = tokio::runtime::Handle::current();
        move |progress: &ApplyProgress| {
            let payload = json!({
                "op": "progress",
                "plan_id": plan_id,
                "kit": kit,
                "steps": progress.steps,
                "error": progress.error,
            });
            let iii = iii.clone();
            let kits = kits.clone();
            rt.spawn(async move {
                trigger_types::dispatch(&iii, &kits, payload).await;
            });
        }
    }

    /// Fan out after a plan was created or applied.
    async fn after(&self, response: &KitPlanResponse, cfg: &SkillsConfig) {
        match response.status {
            PlanStatus::Planned | PlanStatus::Replanned => {
                if let Some(plan) = &response.plan {
                    self.emit(json!({
                        "op": "plan",
                        "kit": plan.kit,
                        "plan_id": plan.plan_id,
                        "kind": plan.kind,
                    }))
                    .await;
                }
            }
            PlanStatus::Applied => {
                let Some(report) = &response.report else {
                    return;
                };
                let payload = json!({ "op": "kit", "kit": report.kit, "plan_id": report.plan_id });
                if report.agents_changed {
                    trigger_types::dispatch(&self.iii, &self.subs.agents, payload.clone()).await;
                }
                if report.skills_changed {
                    trigger_types::dispatch(&self.iii, &self.subs.skills, payload.clone()).await;
                }
                self.emit(json!({
                    "op": "apply",
                    "kit": report.kit,
                    "plan_id": report.plan_id,
                    "kind": report.kind,
                }))
                .await;
                let moved: Vec<String> = report
                    .workers_added
                    .iter()
                    .chain(&report.workers_updated)
                    .cloned()
                    .collect();
                if !moved.is_empty() || !report.workers_removed.is_empty() {
                    self.cache.invalidate().await;
                }
                if !moved.is_empty() {
                    self.download_worker_bundles(cfg.clone(), moved);
                }
                if report.kind != Some(PlanKind::Remove) {
                    self.check_kit_updates(report.kit.clone());
                }
            }
            PlanStatus::UpToDate => {}
        }
    }

    /// After an install or update, ask the registry right away whether a
    /// newer release already exists, so the list shows it without waiting
    /// for the 6-hour check.
    fn check_kit_updates(&self, kit: String) {
        let ctx = self.clone();
        tokio::spawn(async move {
            let (_cfg, env, reg) = ctx.env();
            match service::check_updates(&env, reg.as_ref(), Some(&kit)).await {
                Ok((_, true)) => ctx.emit(json!({ "op": "updates", "kit": kit })).await,
                Ok(_) => {}
                Err(e) => tracing::debug!(kit, error = %e, "kit update check after apply failed"),
            }
        });
    }

    /// Workers a kit just added or moved get their own skills and profiles
    /// now (kit-owned profiles are skipped), rather than at the next boot
    /// reconcile.
    fn download_worker_bundles(&self, cfg: SkillsConfig, workers: Vec<String>) {
        let ctx = self.clone();
        tokio::spawn(async move {
            let spec = crate::sources::registry::VersionSpec::Tag("latest".into());
            let mut any = false;
            for worker in workers {
                match super::download::download_worker_skills(&cfg, &worker, &spec).await {
                    Ok(true) => any = true,
                    Ok(false) => {}
                    Err(e) => {
                        tracing::warn!(worker, error = %e, "worker bundle download after kit apply failed")
                    }
                }
            }
            if any {
                ctx.cache.invalidate().await;
                let payload = json!({ "op": "download", "source": "registry" });
                trigger_types::dispatch(&ctx.iii, &ctx.subs.skills, payload.clone()).await;
                trigger_types::dispatch(&ctx.iii, &ctx.subs.agents, payload).await;
            }
        });
    }
}

pub fn register(
    iii: &Arc<IIIClient>,
    cfg: &SharedConfig,
    subs: &super::Subscribers,
    cache: &Arc<RegisteredWorkersCache>,
) {
    let ctx = Ctx {
        iii: iii.clone(),
        cfg: cfg.clone(),
        subs: subs.clone(),
        cache: cache.clone(),
        registry: Arc::new(RegistryCell::default()),
        apply_lock: Arc::new(tokio::sync::Mutex::new(())),
    };

    let c = ctx.clone();
    iii.register_function(
        "directory::download-kit",
        RegisterFunction::new_async(move |req: DownloadKitInput| {
            let ctx = c.clone();
            async move {
                let (cfg, env, reg) = ctx.env();
                let compose = EngineCompose::new(ctx.iii.clone());
                let _guard = if req.apply.unwrap_or(false) {
                    Some(ctx.apply_lock.clone().lock_owned().await)
                } else {
                    None
                };
                let emitter = ctx.progress_emitter(String::new(), req.kit.clone());
                let out = service::download_kit(
                    &env,
                    reg.as_ref(),
                    &compose,
                    &req.kit,
                    req.apply.unwrap_or(false),
                    ApplyTiming::default(),
                    &emitter,
                )
                .await
                .map_err(Error::Handler)?;
                ctx.after(&out, &cfg).await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Plan the install of a kit — agent profiles, skills and registry workers published \
             as <author>/<kit>[@version|tag|range] — without writing anything; an installed \
             kit gets an update plan. The plan lists every file, collisions with existing \
             profiles (kit profiles win, with a warning), worker changes and the functions the \
             profiles preload. Review it in the ADE (Directory → Kits) or apply it with \
             directory::kits::apply plan_id=…. apply=true applies at once only when the plan has \
             no warnings, blocks or pending decisions.",
        )
        .metadata(json!({"tool": {"label": "Download kit"}})),
    );

    let c = ctx.clone();
    iii.register_function(
        "directory::kits::apply",
        RegisterFunction::new_async(move |req: ApplyInput| {
            let ctx = c.clone();
            async move {
                let (cfg, env, reg) = ctx.env();
                let compose = EngineCompose::new(ctx.iii.clone());
                let _guard = ctx.apply_lock.clone().lock_owned().await;
                let kit = store::load(&env.plans_dir, &req.plan_id)
                    .ok()
                    .flatten()
                    .map(|r| r.plan.kit)
                    .unwrap_or_default();
                let emitter = ctx.progress_emitter(req.plan_id.clone(), kit);
                let out = service::run_apply(
                    &env,
                    reg.as_ref(),
                    &compose,
                    &req.plan_id,
                    &req.decisions.unwrap_or_default(),
                    &req.remove_workers.unwrap_or_default(),
                    ApplyTiming::default(),
                    &emitter,
                )
                .await
                .map_err(Error::Handler)?;
                ctx.after(&out, &cfg).await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Apply a kit plan from directory::download-kit / plan-update / remove: re-checks the \
             project, downloads the kit (counted), adds or moves its workers through Compose, \
             writes the files atomically and records kits.lock last. decisions maps install \
             paths to overwrite|keep|kit|mine|merged|remove (or {choice: merged, content}); \
             files left out take the plan's default. If the project changed since the plan, \
             nothing is applied and a fresh plan comes back (status replanned).",
        )
        .metadata(json!({"tool": {"label": "Apply kit plan"}})),
    );

    let c = ctx.clone();
    iii.register_function(
        "directory::kits::plan-update",
        RegisterFunction::new_async(move |req: PlanUpdateInput| {
            let ctx = c.clone();
            async move {
                let (cfg, env, reg) = ctx.env();
                let compose = EngineCompose::new(ctx.iii.clone());
                let out = service::plan_update(
                    &env,
                    reg.as_ref(),
                    &compose,
                    &req.kit,
                    req.version.as_deref(),
                )
                .await
                .map_err(Error::Handler)?;
                ctx.after(&out, &cfg).await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Plan the update of an installed kit to `version` (default: the newest release \
             matching what it was installed with). Files you edited are merged three-way; \
             conflicts need a decision before directory::kits::apply.",
        )
        .metadata(json!({"tool": {"label": "Plan kit update"}})),
    );

    let c = ctx.clone();
    iii.register_function(
        "directory::kits::check-updates",
        RegisterFunction::new_async(move |req: CheckUpdatesInput| {
            let ctx = c.clone();
            async move {
                let (_cfg, env, reg) = ctx.env();
                let (cache, changed) =
                    service::check_updates(&env, reg.as_ref(), req.kit.as_deref())
                        .await
                        .map_err(Error::Handler)?;
                if changed {
                    ctx.emit(json!({ "op": "updates" })).await;
                }
                Ok::<_, Error>(updates_output(&cache))
            }
        })
        .description(
            "Ask the registry whether installed kits (or one) have a newer release matching \
             what they were installed with. Ignored versions are skipped. Also runs at boot and \
             every 6 hours.",
        )
        .metadata(json!({"tool": {"label": "Check kit updates"}})),
    );

    let c = ctx.clone();
    iii.register_function(
        "directory::kits::list",
        RegisterFunction::new_async(move |_req: ListInput| {
            let ctx = c.clone();
            async move {
                let (_cfg, env, _reg) = ctx.env();
                service::list(&env).map_err(Error::Handler)
            }
        })
        .description(
            "Installed kits (version, file states, worker ranges, available update) and the \
             plans waiting for review. `attention` counts both.",
        ),
    );

    let c = ctx.clone();
    iii.register_function(
        "directory::kits::get",
        RegisterFunction::new_async(move |req: GetInput| {
            let ctx = c.clone();
            async move {
                let (_cfg, env, reg) = ctx.env();
                service::get(&env, reg.as_ref(), &req.kit, req.registry.unwrap_or(false))
                    .await
                    .map_err(Error::Handler)
            }
        })
        .description(
            "One installed kit: every file with its state (intact, edited, missing, skipped), \
             its workers as Compose has them, and with registry=true the installed version's \
             detail and README.",
        ),
    );

    let c = ctx.clone();
    iii.register_function(
        "directory::kits::remove",
        RegisterFunction::new_async(move |req: KitInput| {
            let ctx = c.clone();
            async move {
                let (cfg, env, _reg) = ctx.env();
                let compose = EngineCompose::new(ctx.iii.clone());
                let out = service::plan_remove(&env, &compose, &req.kit)
                    .await
                    .map_err(Error::Handler)?;
                ctx.after(&out, &cfg).await;
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Plan the removal of an installed kit: its files (edited ones are kept unless \
             chosen) and, only if ticked in apply's remove_workers, its workers. Apply with \
             directory::kits::apply.",
        )
        .metadata(json!({"tool": {"label": "Plan kit removal"}})),
    );

    let c = ctx.clone();
    iii.register_function(
        "directory::kits::discard",
        RegisterFunction::new_async(move |req: PlanIdInput| {
            let ctx = c.clone();
            async move {
                let (_cfg, env, _reg) = ctx.env();
                let kit = store::load(&env.plans_dir, &req.plan_id)
                    .ok()
                    .flatten()
                    .map(|r| r.plan.kit);
                let discarded = service::discard(&env, &req.plan_id).map_err(Error::Handler)?;
                if discarded {
                    ctx.emit(json!({ "op": "discard", "plan_id": req.plan_id, "kit": kit }))
                        .await;
                }
                Ok::<_, Error>(DiscardOutput {
                    plan_id: req.plan_id,
                    discarded,
                })
            }
        })
        .description("Discard a pending kit plan."),
    );

    let c = ctx.clone();
    iii.register_function(
        "directory::kits::plan",
        RegisterFunction::new_async(move |req: PlanGetInput| {
            let ctx = c.clone();
            async move {
                let (_cfg, env, _reg) = ctx.env();
                let record = store::load(&env.plans_dir, &req.plan_id)
                    .map_err(Error::Handler)?
                    .ok_or_else(|| {
                        Error::Handler(format!(
                            "D512 not_found: plan {:?} does not exist (applied, discarded or expired). \
                             Next: call directory::kits::list to see pending plans.",
                            req.plan_id
                        ))
                    })?;
                let mut out = json!({ "plan": record.plan, "progress": record.progress });
                if req.contents.unwrap_or(false) {
                    out["contents"] = json!(record.contents);
                }
                Ok::<_, Error>(out)
            }
        })
        .description(
            "Read a pending kit plan, with contents=true the file bodies it refers to (kit \
             blobs by sha256, local files and merge results by install path).",
        ),
    );

    let c = ctx.clone();
    iii.register_function(
        "directory::kits::ignore",
        RegisterFunction::new_async(move |req: IgnoreInput| {
            let ctx = c.clone();
            async move {
                let (_cfg, env, _reg) = ctx.env();
                let mut lock = env.read_lock().map_err(Error::Handler)?;
                crate::kits::apply::set_ignored(
                    &mut lock,
                    &req.kit,
                    &req.version,
                    req.ignore.unwrap_or(true),
                )
                .map_err(Error::Handler)?;
                lock.write(&env.lock_path).map_err(Error::Handler)?;
                let ignored = lock.kits[&req.kit].ignored_versions.clone();
                // Ignoring the version an update plan targets retires the plan.
                if req.ignore.unwrap_or(true) {
                    for record in store::for_kit(&env.plans_dir, &req.kit, chrono::Utc::now()) {
                        if record.plan.kind == PlanKind::Update
                            && record.plan.to.as_deref() == Some(req.version.as_str())
                        {
                            let _ = store::delete(&env.plans_dir, &record.plan.plan_id);
                        }
                    }
                }
                // Re-filter the cached check so the badge follows at once.
                let mut cache = service::read_updates(&env);
                if let Some(update) = cache.kits.get_mut(&req.kit) {
                    if req.ignore.unwrap_or(true)
                        && update.available.as_deref() == Some(req.version.as_str())
                    {
                        update.ignored = update.available.take();
                        update.major = false;
                    } else if !req.ignore.unwrap_or(true)
                        && update.ignored.as_deref() == Some(req.version.as_str())
                    {
                        update.available = update.ignored.take();
                    }
                    if let Ok(bytes) = serde_json::to_vec_pretty(&cache) {
                        let _ = crate::sources::write_file_atomic(&env.updates_cache, &bytes);
                    }
                }
                ctx.emit(json!({ "op": "updates", "kit": req.kit })).await;
                Ok::<_, Error>(json!({ "kit": req.kit, "ignored_versions": ignored }))
            }
        })
        .description(
            "Skip (or, with ignore=false, stop skipping) one version of an installed kit in \
             update checks; recorded in kits.lock ignored_versions.",
        ),
    );

    let c = ctx.clone();
    iii.register_function(
        "directory::kits::diff",
        RegisterFunction::new_async(move |req: DiffInput| {
            let ctx = c.clone();
            async move {
                let (_cfg, env, reg) = ctx.env();
                service::file_sides(&env, reg.as_ref(), &req.kit, &req.path)
                    .await
                    .map_err(Error::Handler)
            }
        })
        .description(
            "The two sides of an installed kit file: `base` (what the kit installed, from the \
             registry) and `local` (the file on disk), for a \"view my changes\" diff.",
        ),
    );

    // Update checks: shortly after boot, then every 6 hours.
    let c = ctx;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            let (_cfg, env, reg) = c.env();
            if env.read_lock().map(|l| !l.kits.is_empty()).unwrap_or(false) {
                match service::check_updates(&env, reg.as_ref(), None).await {
                    Ok((_, true)) => c.emit(json!({ "op": "updates" })).await,
                    Ok(_) => {}
                    Err(e) => tracing::debug!(error = %e, "kit update check failed"),
                }
            }
            tokio::time::sleep(UPDATE_CHECK_EVERY).await;
        }
    });
}

fn updates_output(cache: &service::UpdatesCache) -> Value {
    let kits: Vec<Value> = cache
        .kits
        .iter()
        .map(|(kit, u)| {
            let mut v = json!(u);
            v["kit"] = json!(kit);
            v
        })
        .collect();
    let available = cache
        .kits
        .values()
        .filter(|u| u.available.is_some())
        .count();
    json!({
        "checked_at": cache.checked_at,
        "available": available,
        "kits": kits,
        "error": cache.error,
    })
}
