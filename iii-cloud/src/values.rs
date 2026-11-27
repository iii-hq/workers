//! `iii-cloud::values` and `iii-cloud::values::save`: the names the compose
//! file reads, where each value can come from, and the page's save. A value
//! from a file or typed by the person is never logged, stored, emitted or
//! returned: `values` answers file names and flags only, and the save's input
//! types have no `Debug`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::check::{Selected, selected};
use crate::cloud::Req;
use crate::ctx::{Ctx, now};
use crate::error::{CloudError, Res, WorkerError};
use crate::local::{Manifest, confined, manifest_path, parse_env};
use crate::types::*;

/// An env file larger than this is not read.
const MAX_ENV_FILE: u64 = 1 << 20;
/// A value larger than this is refused.
const MAX_VALUE: usize = 64 << 10;
/// Why a name `secret::valid_name` refuses can't be saved.
const INVALID_NAME: &str = "iii cloud takes names of A-Z, 0-9 and _ that don't start with a digit: rename it in the compose file.";

/// What iii cloud stores for a name (`GET .../variables`).
struct Stored {
    kind: String,
    set: bool,
    value: Option<String>,
    updated_at: Option<String>,
}

async fn stored(ctx: &Ctx, team_id: &str, env_id: &str) -> Option<HashMap<String, Stored>> {
    let body = ctx
        .call(&Req::get(format!(
            "/teams/{team_id}/environments/{env_id}/variables"
        )))
        .await
        .ok()?;
    let mut out = HashMap::new();
    for v in body.get("variables").and_then(Value::as_array)? {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        let Some(name) = s("name") else { continue };
        let kind = s("type").unwrap_or_else(|| "variable".into());
        out.insert(
            name,
            Stored {
                set: s("status").as_deref() == Some("set"),
                // A secret's value never leaves iii cloud; a variable's may be shown.
                value: (kind == "variable").then(|| s("value")).flatten(),
                updated_at: s("updated_at"),
                kind,
            },
        );
    }
    Some(out)
}

/// The compose file a call means: the request's, else the latest check's
/// when it is of this environment, else the environment's.
pub(crate) fn manifest_of(ctx: &Ctx, sel: &Selected, asked: Option<&str>) -> Res<String> {
    let manifest = asked
        .map(str::to_owned)
        .or_else(|| ctx.store.read(|s| s.work_file(&sel.env.id)))
        .or_else(|| sel.env.manifest.clone())
        .unwrap_or_else(|| "worker-compose.yaml".into());
    if !manifest_path(&manifest) || confined(&manifest).is_none() {
        return Err(CloudError::invalid(
            "manifest must be a worker-compose*.yaml of the repository, from its root",
        ));
    }
    Ok(manifest)
}

/// The env files a compose file's values may come from, in suggestion order,
/// each with whether it is preselected: `.env.<env>`, `.env.production`
/// (preselected only for production), `.env.local`, `.env`. Each from the
/// manifest's folder, else the repository root: the first file of a name wins.
/// `.env.example` is never read.
fn env_files(root: &Path, manifest: &str, env: &EnvironmentInfo) -> Vec<(String, bool)> {
    let mut names: Vec<(String, bool)> = vec![];
    if let Some(own) = own_file(env) {
        names.push((own, true));
    }
    if env.name != "production" {
        names.push((".env.production".into(), env.production));
    }
    names.push((".env.local".into(), true));
    names.push((".env".into(), true));
    names
        .into_iter()
        .filter_map(|(name, preselect)| Some((located(root, manifest, &name)?, preselect)))
        .collect()
}

/// `name` in the manifest's folder, else at the repository root: the first
/// that is a plain file inside `root`, from the root.
fn located(root: &Path, manifest: &str, name: &str) -> Option<String> {
    let dir = Path::new(manifest).parent().unwrap_or(Path::new(""));
    [dir.join(name), PathBuf::from(name)]
        .into_iter()
        .find(|rel| inside(root, rel).is_some())
        .map(|rel| rel.to_string_lossy().into_owned())
}

/// The values the run here takes from the person's folder (`checkout`) for
/// the `wanted` names: from `.env.<env>`, `.env.local`, `.env`, then
/// `.env.production`, the first file that has the name. Each as (name,
/// value, file). The same file rules as `values`. The values are for the
/// run's environment only: never stored, logged or returned.
pub fn fill_here(
    checkout: &Path,
    manifest: &str,
    env: &EnvironmentInfo,
    wanted: &[String],
) -> Vec<(String, String, String)> {
    let mut order: Vec<String> = own_file(env).into_iter().collect();
    for n in [".env.local", ".env", ".env.production"] {
        if !order.iter().any(|o| o == n) {
            order.push(n.into());
        }
    }
    let mut out: Vec<(String, String, String)> = vec![];
    for file in order.iter().filter_map(|n| located(checkout, manifest, n)) {
        for (name, value) in read_env(checkout, &file).unwrap_or_default() {
            if wanted.contains(&name) && !out.iter().any(|(n, _, _)| *n == name) {
                out.push((name, value, file.clone()));
            }
        }
    }
    out
}

/// The names the run here needs that its own `.env` lacks: `${NAME}` with no
/// default, never iii's own, a binding or a database URL (the run makes its
/// databases, and a production one must never be used here).
fn run_needs(m: &Manifest, have: &[String]) -> Vec<String> {
    let bindings = m.database_bindings();
    crate::local::required_variables(m)
        .into_iter()
        .filter(|n| {
            !n.starts_with("IIICLOUD_")
                && !n.starts_with("III_")
                && !bindings.contains(n)
                && !crate::local::database_shaped(n)
                && !have.contains(n)
        })
        .collect()
}

/// What the run here takes from the person's folder for the compose file
/// `manifest` (default `worker-compose.yaml`) of `workspace`. See [`fill_here`].
pub fn here_values(
    checkout: &Path,
    workspace: &Path,
    manifest: Option<&str>,
    env: &EnvironmentInfo,
) -> Vec<(String, String, String)> {
    let manifest = manifest.unwrap_or("worker-compose.yaml");
    let Some(m) = Manifest::load(workspace, Some(manifest)) else {
        return vec![];
    };
    let have: Vec<String> = crate::local::worktree_env(workspace, &m)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    fill_here(checkout, manifest, env, &run_needs(&m, &have))
}

/// `.env.<environment>`, when the name is `^[a-z0-9-]+$` (never `.env.example`).
fn own_file(env: &EnvironmentInfo) -> Option<String> {
    let plain = !env.name.is_empty()
        && env.name != "example"
        && env
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    plain.then(|| format!(".env.{}", env.name))
}

/// Every allowed env file name for `manifest`, whether it exists or not.
fn allowed_files(manifest: &str, env: &EnvironmentInfo) -> Vec<String> {
    let dir = Path::new(manifest).parent().unwrap_or(Path::new(""));
    let mut names: Vec<String> = own_file(env).into_iter().collect();
    for n in [".env.production", ".env.local", ".env"] {
        if !names.iter().any(|x| x == n) {
            names.push(n.into());
        }
    }
    names
        .iter()
        .flat_map(|n| [dir.join(n), PathBuf::from(n)])
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// `rel` in `root`, when it is a plain file (not a link) that stays inside `root`.
pub(crate) fn inside(root: &Path, rel: &Path) -> Option<PathBuf> {
    let path = root.join(rel);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    let canonical = path.canonicalize().ok()?;
    (meta.file_type().is_file() && canonical.starts_with(root.canonicalize().ok()?))
        .then_some(canonical)
}

/// An env file's lines, when it is in `root`, small and UTF-8; else nothing.
fn read_env(root: &Path, rel: &str) -> Option<Vec<(String, String)>> {
    let path = inside(root, Path::new(rel))?;
    if std::fs::metadata(&path).ok()?.len() > MAX_ENV_FILE {
        return None;
    }
    let text = String::from_utf8(std::fs::read(path).ok()?).ok()?;
    Some(parse_env(&text))
}

/// A value that only means something on this machine.
fn local_looking(value: &str) -> bool {
    ["localhost", "127.0.0.1", "0.0.0.0", "host.docker.internal"]
        .iter()
        .any(|h| value.contains(h))
}

/// A name the environment's address fills.
fn address_name(name: &str) -> bool {
    let rest = ["NEXT_PUBLIC_", "VITE_", "PUBLIC_"]
        .iter()
        .find_map(|p| name.strip_prefix(p))
        .unwrap_or(name);
    ["APP", "SITE", "BASE", "PUBLIC"].iter().any(|head| {
        rest.strip_prefix(head)
            .map(|r| r.strip_prefix("_BASE").unwrap_or(r))
            .is_some_and(|r| r == "_URL")
    })
}

fn project_of(ctx: &Ctx) -> Res<Project> {
    ctx.store
        .read(|s| s.project.clone())
        .ok_or_else(CloudError::no_project)
}

/// `iii-cloud::values`.
pub async fn values(ctx: &Arc<Ctx>, request: ValuesRequest) -> Res<Values> {
    let sel = selected(ctx)?;
    let project = project_of(ctx)?;
    let manifest = manifest_of(ctx, &sel, request.manifest.as_deref())?;
    let from = request.from.unwrap_or_default();
    let checkout = PathBuf::from(&project.path);
    let root = match from {
        ValuesFrom::Workspace => sel.workspace.clone(),
        ValuesFrom::Checkout => checkout.clone(),
    };
    let m = Manifest::load(&root, Some(&manifest)).ok_or_else(|| {
        CloudError::new(
            "MANIFEST_NOT_FOUND",
            format!("{manifest} isn't in {}, or isn't YAML.", root.display()),
        )
    })?;
    let bindings = m.database_bindings();
    let skip = |name: &str| {
        name.starts_with("IIICLOUD_")
            || name.starts_with("III_")
            || bindings.iter().any(|b| b == name)
    };
    // The names, in the file's order; a name read with no default anywhere is required.
    let mut items: Vec<ValueItem> = vec![];
    for u in m.variable_uses().0 {
        if skip(&u.name) {
            continue;
        }
        let need = match u.default.as_deref() {
            None => Need::Required,
            Some("") => Need::Optional,
            Some(_) => Need::HasDefault,
        };
        if let Some(item) = items.iter_mut().find(|i| i.name == u.name) {
            if !item.containers.contains(&u.container) {
                item.containers.push(u.container);
            }
            let rank = |n: Need| match n {
                Need::Required => 0,
                Need::Optional => 1,
                Need::HasDefault => 2,
            };
            if rank(need) < rank(item.need) {
                item.need = need;
                item.default = u.default;
            }
            continue;
        }
        items.push(ValueItem {
            database: crate::local::database_shaped(&u.name),
            name: u.name,
            need,
            default: u.default,
            kind: ValueKind::Secret,
            containers: vec![u.container],
            line: u.line,
            in_cloud: InCloud::Unknown,
            value: None,
            updated_at: None,
            asked: None,
            suggestions: vec![],
            invalid_name: None,
            here: Here::NotNeeded,
            here_file: None,
            skipped: false,
        });
    }
    // An agent's open ask for a name the file does not read is a row too.
    let asks: Vec<SecretRequest> = ctx.store.read(|s| {
        s.secrets
            .iter()
            .filter(|r| r.environment_id == sel.env.id && r.state == SecretState::Waiting)
            .cloned()
            .collect()
    });
    for ask in &asks {
        if !items.iter().any(|i| i.name == ask.name) && !skip(&ask.name) {
            items.push(ValueItem {
                database: crate::local::database_shaped(&ask.name),
                name: ask.name.clone(),
                need: Need::Required,
                default: None,
                kind: ValueKind::Secret,
                containers: vec![],
                line: None,
                in_cloud: InCloud::Unknown,
                value: None,
                updated_at: None,
                asked: None,
                suggestions: vec![],
                invalid_name: None,
                here: Here::NotNeeded,
                here_file: None,
                skipped: false,
            });
        }
    }
    let here = stored(ctx, &sel.team_id, &sel.env.id).await;
    // Other environments of the project, for copies: one read each, unreadable ones left out.
    let mut others: Vec<(EnvironmentBrief, HashMap<String, Stored>)> = vec![];
    for e in project.environments.iter().filter(|e| e.id != sel.env.id) {
        if let Some(list) = stored(ctx, &sel.team_id, &e.id).await {
            others.push((e.clone(), list));
        }
    }
    // Env files are the person's folder's: a worktree never carries untracked `.env*`.
    let files: Vec<(String, bool, HashMap<String, bool>)> =
        env_files(&checkout, &manifest, &sel.env)
            .into_iter()
            .filter_map(|(file, preselect)| {
                let lines = read_env(&checkout, &file)?;
                // Only whether each name points at this machine: the values go no further.
                let local = lines
                    .into_iter()
                    .map(|(name, value)| (name, local_looking(&value)))
                    .collect();
                Some((file, preselect, local))
            })
            .collect();
    let url = sel.env.url.clone();
    for item in &mut items {
        item.asked = asks
            .iter()
            .find(|a| a.name == item.name)
            .map(|a| ValueAsked {
                request_id: a.id.clone(),
                description: a.description.clone(),
            });
        let stored = here.as_ref().and_then(|h| h.get(&item.name));
        item.in_cloud = match (&here, stored) {
            (None, _) => InCloud::Unknown,
            (Some(_), Some(s)) if s.set => InCloud::Set,
            (Some(_), _) => InCloud::Unset,
        };
        let address = url.is_some() && address_name(&item.name);
        if stored.is_some_and(|s| s.kind == "variable") || address {
            item.kind = ValueKind::Variable;
        }
        if let Some(s) = stored.filter(|s| s.set) {
            item.value = s.value.clone();
            item.updated_at = s.updated_at.clone();
        }
        // A row already set is kept as it is, iii cloud makes a database, and a
        // name it refuses can't be saved: nothing is preselected for any of them.
        if !crate::secret::valid_name(&item.name) {
            item.invalid_name = Some(INVALID_NAME.into());
        }
        let open = item.in_cloud != InCloud::Set;
        let pick = open && !item.database && item.invalid_name.is_none();
        item.suggestions = file_suggestions(&files, &item.name, pick, false);
        if let (true, Some(url)) = (address, url.as_ref()) {
            item.suggestions.push(Suggestion::EnvUrl {
                url: url.clone(),
                preselect: pick,
            });
        }
        item.suggestions
            .extend(file_suggestions(&files, &item.name, pick, true));
        if open {
            for (env, list) in &others {
                if list.get(&item.name).is_some_and(|s| s.set) {
                    item.suggestions.push(Suggestion::Copy {
                        environment_id: env.id.clone(),
                        environment: env.name.clone(),
                        preselect: false,
                    });
                }
            }
        }
    }
    // Whether the run here has each name: its own `.env`, else the folder's
    // env files (only the file name is kept).
    let have: Vec<String> = Manifest::load(&sel.workspace, Some(&manifest))
        .map(|w| {
            crate::local::worktree_env(&sel.workspace, &w)
                .into_iter()
                .map(|(k, _)| k)
                .collect()
        })
        .unwrap_or_default();
    let wanted: Vec<String> = items
        .iter()
        .filter(|i| i.need == Need::Required && !i.containers.is_empty())
        .map(|i| i.name.clone())
        .collect();
    let wanted = run_needs(&m, &have)
        .into_iter()
        .filter(|n| wanted.contains(n))
        .collect::<Vec<_>>();
    let from_files: Vec<(String, String)> = fill_here(&checkout, &manifest, &sel.env, &wanted)
        .into_iter()
        .map(|(name, _, file)| (name, file))
        .collect();
    for item in &mut items {
        (item.here, item.here_file) = if item.need != Need::Required || item.containers.is_empty() {
            (Here::NotNeeded, None)
        } else if have.contains(&item.name) {
            (Here::Set, None)
        } else if let Some((_, file)) = from_files.iter().find(|(n, _)| *n == item.name) {
            (Here::FromFile, Some(file.clone()))
        } else {
            (Here::Unset, None)
        };
    }
    let skipped = skipped_of(ctx, &sel.env.id);
    for item in &mut items {
        item.skipped = skipped.contains(&item.name);
    }
    let save = may_save(&ctx.conn().await, &sel);
    Ok(Values {
        environment_id: sel.env.id.clone(),
        environment: sel.env.name.clone(),
        manifest,
        read_from: from,
        cloud: match here {
            Some(_) => CloudRead::Read,
            None => CloudRead::Unreadable,
        },
        save,
        items,
    })
}

/// The env files that have `name`: the named ones first (`late` false), or
/// `.env.local` and `.env` (`late`), which are preselected unless the value
/// points at this machine. Only with `pick` is anything preselected.
fn file_suggestions(
    files: &[(String, bool, HashMap<String, bool>)],
    name: &str,
    pick: bool,
    late: bool,
) -> Vec<Suggestion> {
    files
        .iter()
        .filter_map(|(file, preselect, names)| {
            let base = Path::new(file).file_name()?.to_string_lossy().into_owned();
            let is_late = base == ".env" || base == ".env.local";
            if is_late != late {
                return None;
            }
            let local = *names.get(name)?;
            Some(Suggestion::EnvFile {
                file: file.clone(),
                preselect: pick && *preselect && !(is_late && local),
                local,
            })
        })
        .collect()
}

/// The names the person skipped for now in `env_id`.
pub fn skipped_of(ctx: &Ctx, env_id: &str) -> std::collections::BTreeSet<String> {
    ctx.store
        .read(|s| s.skipped_values.get(env_id).cloned())
        .unwrap_or_default()
}

/// `iii-cloud::values::skip`: the page's own. Records (or with `skip: false`
/// undoes) "skip for now" for names of the selected environment. Names only.
pub async fn skip(ctx: &Arc<Ctx>, request: ValuesSkip) -> Res<ValuesSkipped> {
    let sel = selected(ctx)?;
    if request.environment_id != sel.env.id {
        return Err(CloudError::new(
            "ENVIRONMENT_CHANGED",
            "Another environment is selected now: read the values again.",
        ));
    }
    if let Some(i) = request
        .names
        .iter()
        .position(|n| !crate::secret::valid_name(n))
    {
        return Err(CloudError::invalid(format!(
            "name {}: {INVALID_NAME}",
            i + 1
        )));
    }
    let names = ctx.store.update(|s| {
        let set = s.skipped_values.entry(sel.env.id.clone()).or_default();
        for n in &request.names {
            if request.skip {
                set.insert(n.clone());
            } else {
                set.remove(n);
            }
        }
        let names: Vec<String> = set.iter().cloned().collect();
        if set.is_empty() {
            s.skipped_values.remove(&sel.env.id);
        }
        names
    });
    Ok(ValuesSkipped {
        environment_id: sel.env.id.clone(),
        names,
    })
}

/// The cloud warnings "Skip for now" may take: a skipped one turns to info.
pub const SKIPPABLE_PROBLEMS: [&str; 1] = ["LOCAL_CONFIG_NOT_SET"];

/// The warning codes the person skipped for now in `env_id`.
pub fn skipped_problems_of(ctx: &Ctx, env_id: &str) -> std::collections::BTreeSet<String> {
    ctx.store
        .read(|s| s.skipped_problems.get(env_id).cloned())
        .unwrap_or_default()
}

/// `iii-cloud::problem::skip`: the page's own. Records (or with `skip: false`
/// undoes) "skip for now" for a cloud warning of the selected environment;
/// the next check shows it skipped, as info.
pub async fn skip_problem(ctx: &Arc<Ctx>, request: ProblemSkip) -> Res<ProblemsSkipped> {
    let sel = selected(ctx)?;
    if request.environment_id != sel.env.id {
        return Err(CloudError::new(
            "ENVIRONMENT_CHANGED",
            "Another environment is selected now: check again.",
        ));
    }
    if !SKIPPABLE_PROBLEMS.contains(&request.code.as_str()) {
        return Err(CloudError::invalid(format!(
            "code: only these warnings can be skipped: {}",
            SKIPPABLE_PROBLEMS.join(", ")
        )));
    }
    let codes = ctx.store.update(|s| {
        let set = s.skipped_problems.entry(sel.env.id.clone()).or_default();
        if request.skip {
            set.insert(request.code.clone());
        } else {
            set.remove(&request.code);
        }
        let codes: Vec<String> = set.iter().cloned().collect();
        if set.is_empty() {
            s.skipped_problems.remove(&sel.env.id);
        }
        codes
    });
    Ok(ProblemsSkipped {
        environment_id: sel.env.id.clone(),
        codes,
    })
}

fn failed(code: &str, message: impl Into<String>, status: Option<u16>) -> WorkerError {
    WorkerError {
        code: code.into(),
        message: message.into(),
        status,
        fix: None,
    }
}

/// A refusal that stops the rest: the session can't write here.
fn stops(e: &CloudError) -> bool {
    matches!(e.status, Some(401 | 403))
        || matches!(
            e.code.as_str(),
            "NEEDS_BROWSER_SIGN_IN" | "SIGN_IN_REQUIRED" | "KEY_REJECTED" | "NOT_CONNECTED"
        )
}

/// The cloud's refusal of one item, in fixed words: its code, never its text.
fn refused(e: &CloudError) -> WorkerError {
    failed(
        "NOT_SAVED",
        format!("iii cloud refused it ({}).", e.code),
        e.status,
    )
}

/// Whether this connection may write `sel`'s values: not with an API key, and
/// production's only as the team's Owner or Admin.
fn may_save(conn: &Connection, sel: &Selected) -> SaveAllowed {
    if conn.method == Some(Method::ApiKey) {
        SaveAllowed::ApiKey
    } else if sel.env.production
        && !conn.teams.iter().any(|t| {
            t.id == sel.team_id && matches!(t.role.to_lowercase().as_str(), "owner" | "admin")
        })
    {
        SaveAllowed::Role
    } else {
        SaveAllowed::Ok
    }
}

/// How many names `save` writes to iii cloud at once (it has no batch write).
const WRITE_AT_A_TIME: usize = 4;

/// `iii-cloud::values::save`: the page's own.
pub async fn save(ctx: &Arc<Ctx>, request: ValuesSave) -> Res<ValuesSaved> {
    let conn = ctx.conn().await;
    if conn.method == Some(Method::ApiKey) {
        return Err(CloudError::new(
            "NEEDS_BROWSER_SIGN_IN",
            "An API key can't write values. Set them in iii cloud's console, or sign in with the browser.",
        ));
    }
    let sel = selected(ctx)?;
    if request.environment_id != sel.env.id {
        return Err(CloudError::new(
            "ENVIRONMENT_CHANGED",
            "Another environment is selected now: read the values again.",
        ));
    }
    if may_save(&conn, &sel) == SaveAllowed::Role {
        return Err(CloudError::new(
            "ROLE_REQUIRED",
            "Changing production's values takes Owner or Admin.",
        ));
    }
    let project = project_of(ctx)?;
    let manifest = manifest_of(ctx, &sel, request.manifest.as_deref())?;
    let allowed = allowed_files(&manifest, &sel.env);
    // The whole call is refused before anything is written. Never the value.
    let mut results: Vec<Option<SaveResult>> = request.items.iter().map(|_| None).collect();
    let mut names: Vec<&str> = vec![];
    for (i, item) in request.items.iter().enumerate() {
        let name = item.name();
        // A name iii cloud refuses fails alone; the rest still run.
        if !crate::secret::valid_name(name) {
            results[i] = Some(result(
                item,
                Err(failed("INVALID_REQUEST", INVALID_NAME, None)),
            ));
        }
        if names.contains(&name) {
            return Err(CloudError::invalid(format!(
                "item {} ({name}): the name appears twice",
                i + 1
            )));
        }
        names.push(name);
        match item {
            SaveItem::Typed { value, .. } if value.len() > MAX_VALUE || value.contains('\0') => {
                return Err(CloudError::invalid(format!(
                    "item {} ({name}): the value is over 64 KiB or holds a NUL",
                    i + 1
                )));
            }
            SaveItem::EnvFile { file, .. } if !allowed.contains(file) => {
                return Err(CloudError::invalid(format!(
                    "item {} ({name}): that file is not an env file this compose file's values come from",
                    i + 1
                )));
            }
            SaveItem::Copy { environment_id, .. }
                if *environment_id == sel.env.id
                    || !project.environments.iter().any(|e| e.id == *environment_id) =>
            {
                return Err(CloudError::invalid(format!(
                    "item {} ({name}): copy only from another environment of this project",
                    i + 1
                )));
            }
            _ => {}
        }
    }
    let mut stopped: Option<String> = None;
    let not_tried = |code: &str| {
        failed(
            "NOT_TRIED",
            format!("Not tried: iii cloud refused an earlier value ({code})."),
            None,
        )
    };
    // Copies: one call per source environment, never overwriting.
    let mut sources: Vec<&str> = vec![];
    for item in &request.items {
        if let SaveItem::Copy { environment_id, .. } = item
            && !sources.contains(&environment_id.as_str())
        {
            sources.push(environment_id);
        }
    }
    for source in sources {
        let picked: Vec<usize> = request
            .items
            .iter()
            .enumerate()
            .filter(|(i, it)| results[*i].is_none() && matches!(it, SaveItem::Copy { environment_id, .. } if environment_id == source))
            .map(|(i, _)| i)
            .collect();
        if picked.is_empty() {
            continue;
        }
        if let Some(code) = &stopped {
            for &i in &picked {
                results[i] = Some(result(&request.items[i], Err(not_tried(code))));
            }
            continue;
        }
        let wanted: Vec<&str> = picked.iter().map(|&i| request.items[i].name()).collect();
        let answer = ctx
            .call(&Req::post(
                format!(
                    "/teams/{}/environments/{}/variables/copy",
                    sel.team_id, sel.env.id
                ),
                json!({ "from_environment_id": source, "names": wanted }),
            ))
            .await;
        let listed = |body: &Value, k: &str, n: &str| {
            body.get(k)
                .and_then(Value::as_array)
                .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(n)))
        };
        for &i in &picked {
            let name = request.items[i].name();
            let outcome = match &answer {
                Ok(body) if listed(body, "copied", name) => Ok(()),
                Ok(body) if listed(body, "skipped", name) => Err(failed(
                    "ALREADY_SET",
                    format!(
                        "{name} is already set in {}: a copy never overwrites.",
                        sel.env.name
                    ),
                    None,
                )),
                Ok(_) => Err(failed(
                    "NOT_SAVED",
                    format!("iii cloud did not copy {name}."),
                    None,
                )),
                Err(e) => Err(refused(e)),
            };
            results[i] = Some(result(&request.items[i], outcome));
        }
        if let Err(e) = &answer
            && stops(e)
        {
            stopped = Some(e.code.clone());
        }
    }
    // Every other item: one PUT each, WRITE_AT_A_TIME at once. A refusal of the
    // session stops the chunks after the one it came in.
    let mut saved_values: Vec<String> = vec![];
    let mut set_names: Vec<(String, LocalOutcome)> = vec![];
    for (i, item) in request.items.iter().enumerate() {
        if matches!(item, SaveItem::Copy { .. }) && results[i].as_ref().is_some_and(|r| r.ok) {
            set_names.push((item.name().to_string(), LocalOutcome::Skipped));
        }
    }
    let pending: Vec<usize> = (0..request.items.len())
        .filter(|&i| results[i].is_none() && !matches!(request.items[i], SaveItem::Copy { .. }))
        .collect();
    for chunk in pending.chunks(WRITE_AT_A_TIME) {
        if let Some(code) = &stopped {
            for &i in chunk {
                results[i] = Some(result(&request.items[i], Err(not_tried(code))));
            }
            continue;
        }
        let done = futures_util::future::join_all(chunk.iter().map(|&i| {
            let (ctx, sel, project, manifest) = (ctx, &sel, &project, &manifest);
            let item = &request.items[i];
            async move {
                let name = item.name();
                let chosen: Result<(ValueKind, String), WorkerError> = match item {
                    SaveItem::Typed { kind, value, .. } => Ok((*kind, value.clone())),
                    SaveItem::EnvFile { kind, file, .. } => {
                        // Read again now: the file may have changed since the page listed it.
                        match read_env(Path::new(&project.path), file)
                            .and_then(|lines| lines.into_iter().find(|(n, _)| n == name))
                        {
                            None => Err(failed(
                                "FILE_CHANGED",
                                format!("{name} is no longer in {file}."),
                                None,
                            )),
                            Some((_, value)) if value.len() > MAX_VALUE || value.contains('\0') => {
                                Err(failed(
                                    "INVALID_REQUEST",
                                    format!("{name} in {file} is over 64 KiB or holds a NUL."),
                                    None,
                                ))
                            }
                            Some((_, value)) => Ok((*kind, value)),
                        }
                    }
                    SaveItem::EnvUrl { .. } => sel
                        .env
                        .url
                        .clone()
                        .map(|url| (ValueKind::Variable, url))
                        .ok_or_else(|| {
                            failed(
                                "NO_URL",
                                format!("{} has no address yet.", sel.env.name),
                                None,
                            )
                        }),
                    SaveItem::LeaveEmpty { .. } => Ok((ValueKind::Variable, String::new())),
                    SaveItem::Copy { .. } => unreachable!("copies are above"),
                };
                match chosen {
                    Err(e) => (i, Err(e)),
                    Ok((kind, value)) => {
                        let written = crate::secret::write_value(
                            ctx,
                            sel,
                            name,
                            kind.as_str(),
                            &value,
                            Some(manifest.as_str()),
                            false,
                        )
                        .await;
                        (i, Ok((value, written)))
                    }
                }
            }
        }))
        .await;
        for (i, outcome) in done {
            let item = &request.items[i];
            let (value, written) = match outcome {
                Ok(v) => v,
                Err(e) => {
                    results[i] = Some(result(item, Err(e)));
                    continue;
                }
            };
            let outcome = match &written.cloud_error {
                None => {
                    if value.len() >= 3 {
                        saved_values.push(value);
                    }
                    set_names.push((item.name().to_string(), written.local));
                    Ok(())
                }
                Some(e) => {
                    if stops(e) && stopped.is_none() {
                        stopped = Some(e.code.clone());
                    }
                    Err(refused(e))
                }
            };
            let mut r = result(item, outcome);
            if r.ok
                && let Some(local) = written.local_error
            {
                // Saved in iii cloud; only the run here's `.env` could not be written.
                r.error = Some(failed("LOCAL_NOT_SAVED", local.message, None));
            }
            results[i] = Some(r);
        }
    }
    ctx.known_secrets.lock().unwrap().extend(saved_values);
    // The agent's asks for these names are answered.
    let answered: Vec<SecretRequest> = ctx.store.update(|s| {
        let mut out = vec![];
        for r in s
            .secrets
            .iter_mut()
            .filter(|r| r.environment_id == sel.env.id && r.state == SecretState::Waiting)
        {
            if let Some((_, local)) = set_names.iter().find(|(n, _)| *n == r.name) {
                r.state = SecretState::Set;
                r.cloud = Some(CloudOutcome::Set);
                r.local = Some(*local);
                r.error = None;
                r.resolved_at = Some(now());
                out.push(r.clone());
            }
        }
        out
    });
    for r in &answered {
        crate::secret::emit(ctx, r);
    }
    crate::status::refresh_variables(ctx, &sel).await;
    Ok(ValuesSaved {
        environment_id: sel.env.id.clone(),
        results: results.into_iter().flatten().collect(),
    })
}

fn result(item: &SaveItem, outcome: Result<(), WorkerError>) -> SaveResult {
    SaveResult {
        name: item.name().to_string(),
        ok: outcome.is_ok(),
        error: outcome.err(),
    }
}
