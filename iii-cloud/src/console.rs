//! The page's fixes on iii cloud's console side: `iii-cloud::config::import`
//! copies the compose file's local config folder into the environment's
//! console config, and `iii-cloud::override::choose` keeps the person's pick
//! per console override.
//!
//! iii cloud keeps console config by manifest container key
//! (`ConfigView.workers[].name`, the plan's `container_configs`), and the
//! planner's `LOCAL_CONFIG_NOT_SET` goes once any container key has an entry.
//! The local `fs` adapter keys an entry by its `id` (`<id>.yaml`), which
//! compose names after the container: its `config_name`, else
//! `<namespace>-<key>`; older packages use the bare key. A file maps to a
//! container through those, else it is `not_a_worker`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::check::selected;
use crate::cloud::Req;
use crate::ctx::Ctx;
use crate::error::{CloudError, Res};
use crate::local::Manifest;
use crate::store::{State, StoredChoice};
use crate::types::*;

/// A config file larger than this is not read.
const MAX_FILE: u64 = 1 << 20;
/// A key whose last segment holds one of these is a secret's.
const SECRET_WORDS: [&str; 6] = [
    "SECRET",
    "TOKEN",
    "KEY",
    "PASSWORD",
    "PRIVATE",
    "CREDENTIAL",
];

fn env_changed() -> CloudError {
    CloudError::new(
        "ENVIRONMENT_CHANGED",
        "Another environment is selected now: read it again.",
    )
}

/// The `fs` adapter's `directory` of the manifest's `configuration` engine
/// worker, the way the planner finds it (`crates/plan/src/lib.rs:331-347`).
pub fn config_directory(m: &Manifest) -> Option<String> {
    let workers = m.yaml.pointer("/engine/workers")?.as_object()?;
    workers.iter().find_map(|(key, worker)| {
        let adapter = worker.get("adapter")?;
        let is_fs = adapter.get("name")?.as_str() == Some("fs");
        let directory = adapter.pointer("/config/directory")?.as_str()?;
        (key.split('#').next() == Some("configuration") && is_fs).then(|| directory.to_string())
    })
}

/// The container whose config entry is `id`.
pub fn container_of(m: &Manifest, id: &str) -> Option<String> {
    let namespace = m
        .yaml
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or("default");
    m.containers()
        .into_iter()
        .find(
            |(key, c)| match c.get("config_name").and_then(Value::as_str) {
                Some(name) => name == id,
                None => key == id || format!("{namespace}-{key}") == id,
            },
        )
        .map(|(key, _)| key)
}

/// A configuration entry as the engine's `fs` adapter reads one.
#[derive(Deserialize)]
struct Entry {
    id: String,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    description: String,
    #[serde(default)]
    value: Value,
}

/// `value`'s leaves as dotted keys; lists stay whole, nulls (unset) are left out.
fn flatten(prefix: &str, value: &Value, out: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(map) => {
            for (k, child) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, child, out);
            }
        }
        Value::Null => {}
        leaf => out.push((prefix.to_string(), leaf.clone())),
    }
}

fn secret_key(key: &str) -> bool {
    let last = key.rsplit('.').next().unwrap_or(key).to_uppercase();
    SECRET_WORDS.iter().any(|w| last.contains(w))
}

/// `${NAME}`: a reference to the vault, the form iii cloud wants for a secret.
fn reference(value: &Value) -> bool {
    value
        .as_str()
        .and_then(|s| s.strip_prefix("${")?.strip_suffix('}'))
        .is_some_and(|name| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// A value that reads like a credential: a URL with a password, a known token
/// prefix, a private key, or a long token-looking string.
pub fn credential_like(value: &Value) -> bool {
    match value {
        Value::String(s) => credential_text(s.trim()),
        Value::Array(items) => items.iter().any(credential_like),
        Value::Object(map) => map.values().any(credential_like),
        _ => false,
    }
}

fn credential_text(s: &str) -> bool {
    let password_url = s
        .split_once("://")
        .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
        .and_then(|authority| authority.rsplit_once('@'))
        .is_some_and(|(info, _)| info.contains(':'));
    let prefixed = [
        "sk-",
        "sk_live_",
        "rk_live_",
        "ghp_",
        "gho_",
        "ghs_",
        "github_pat_",
        "glpat-",
        "xoxb-",
        "xoxp-",
        "AKIA",
        "eyJ",
        "-----BEGIN",
    ]
    .iter()
    .any(|p| s.starts_with(p));
    let token = s.len() >= 32
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_+=".contains(c))
        && s.chars().any(|c| c.is_ascii_digit())
        && s.chars().any(|c| c.is_ascii_alphabetic())
        && uuid::Uuid::parse_str(s).is_err();
    password_url || prefixed || token
}

/// `inner` sits under `outer` (`cors` / `cors.allowed_origins`).
fn nested(outer: &str, inner: &str) -> bool {
    inner
        .strip_prefix(outer)
        .is_some_and(|rest| rest.starts_with('.'))
}

/// What `view` (`GET …/config`) and the local folder give, before any write:
/// one block per file that maps to a worker iii cloud runs.
pub fn preview(root: &Path, m: &Manifest, view: &Value) -> Res<ConfigImported> {
    let directory = config_directory(m).ok_or_else(|| {
        CloudError::new(
            "NO_LOCAL_CONFIG",
            "The compose file keeps no config folder (engine.workers.configuration with the fs adapter).",
        )
    })?;
    let root = root
        .canonicalize()
        .map_err(|_| CloudError::new("CONFIG_FOLDER_MISSING", "The project folder is gone."))?;
    let dir = m.dir.join(&directory);
    let Ok(meta) = std::fs::symlink_metadata(&dir) else {
        return Err(CloudError::new(
            "CONFIG_FOLDER_MISSING",
            format!("{directory} isn't there next to the compose file: nothing to copy."),
        ));
    };
    let canonical = dir.canonicalize().ok().filter(|c| c.starts_with(&root));
    let (true, Some(canonical)) = (meta.is_dir(), canonical) else {
        return Err(CloudError::new(
            "CONFIG_FOLDER_REFUSED",
            format!(
                "{directory} is a link or outside the repository: only a folder inside it is copied."
            ),
        ));
    };
    let rel_dir = canonical
        .strip_prefix(&root)
        .unwrap_or(Path::new(""))
        .to_path_buf();
    let mut names: Vec<String> = std::fs::read_dir(&canonical)
        .map_err(|_| {
            CloudError::new(
                "CONFIG_FOLDER_MISSING",
                format!("{directory} can't be read."),
            )
        })?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        // What the engine's `fs` adapter loads: `*.yaml`, never a backup.
        .filter(|n| n.ends_with(".yaml") && !n.ends_with(".bak.yaml") && !n.ends_with(".bkup.yaml"))
        .collect();
    names.sort();

    let listed = view
        .get("workers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut workers = vec![];
    let mut skipped = vec![];
    for name in names {
        let rel: PathBuf = rel_dir.join(&name);
        let file = rel.to_string_lossy().into_owned();
        let mut skip = |reason| {
            skipped.push(ConfigSkippedFile {
                file: file.clone(),
                reason,
            })
        };
        let Some(path) = crate::values::inside(&root, &rel) else {
            skip(ConfigSkipReason::NotAPlainFile);
            continue;
        };
        if std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_FILE) {
            skip(ConfigSkipReason::TooLarge);
            continue;
        }
        let entry = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_yaml_ng::from_str::<Entry>(&text).ok());
        let Some(entry) = entry else {
            skip(ConfigSkipReason::Unreadable);
            continue;
        };
        let Some((worker, shown)) = container_of(m, &entry.id).and_then(|key| {
            let shown = listed.iter().find(|w| w["name"].as_str() == Some(&key))?;
            Some((key, shown))
        }) else {
            skip(ConfigSkipReason::NotAWorker);
            continue;
        };
        // The fields iii cloud or the compose file set: the console can't change them.
        let fixed: Vec<&str> = shown
            .get("fields")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|f| f.get("editable") == Some(&Value::Bool(false)))
            .filter_map(|f| f.get("key").and_then(Value::as_str))
            .collect();
        let mut leaves = vec![];
        flatten("", &entry.value, &mut leaves);
        let mut block = ConfigImportWorker {
            worker,
            file,
            fields: vec![],
            left_out: vec![],
            set_elsewhere: vec![],
        };
        for (key, value) in leaves {
            if fixed
                .iter()
                .any(|f| *f == key || nested(f, &key) || nested(&key, f))
            {
                block.set_elsewhere.push(key);
            } else if (secret_key(&key) && !reference(&value)) || credential_like(&value) {
                block.left_out.push(key);
            } else {
                block.fields.push(ConfigImportField { key, value });
            }
        }
        workers.push(block);
    }
    Ok(ConfigImported {
        environment_id: String::new(),
        directory: rel_dir.to_string_lossy().into_owned(),
        version: view.get("version").and_then(Value::as_i64).unwrap_or(0),
        applied: false,
        draft_changed: view.get("draft_changed") == Some(&Value::Bool(true)),
        workers,
        skipped,
    })
}

fn changed(env: &str) -> CloudError {
    CloudError::new(
        "CONFIG_CHANGED",
        format!("Someone changed {env}'s config; review again."),
    )
    .with_status(409)
}

/// A write iii cloud refused, in the page's words.
fn refusal(env: &str, e: CloudError) -> CloudError {
    match e.status {
        Some(409) => changed(env),
        Some(403) => CloudError::new(
            "ROLE_REQUIRED",
            format!("Changing {env}'s config takes a role that edits config in iii cloud (Developer, or Admin in production)."),
        )
        .with_status(403),
        _ => e,
    }
}

/// `iii-cloud::config::import`: the page's own. A dry run reads only; an
/// apply writes the draft with the imported fields alone, then applies it at
/// `expected_version`.
pub async fn import(ctx: &Arc<Ctx>, request: ConfigImport) -> Res<ConfigImported> {
    let sel = selected(ctx)?;
    if request.environment_id != sel.env.id {
        return Err(env_changed());
    }
    if !request.dry_run && request.expected_version.is_none() {
        return Err(CloudError::invalid(
            "expected_version: the version the dry run returned",
        ));
    }
    let project = ctx
        .store
        .read(|s| s.project.clone())
        .ok_or_else(CloudError::no_project)?;
    // The person's folder: the config folder is not in the repository, so no worktree has it.
    let root = PathBuf::from(&project.path);
    let manifest = crate::values::manifest_of(ctx, &sel, None)?;
    let m = Manifest::load(&root, Some(&manifest)).ok_or_else(|| {
        CloudError::new(
            "MANIFEST_UNREADABLE",
            format!("{manifest} can't be read in your folder."),
        )
    })?;
    let base = format!("/teams/{}/environments/{}/config", sel.team_id, sel.env.id);
    let view = ctx.call(&Req::get(base.clone())).await?;
    let mut out = preview(&root, &m, &view)?;
    out.environment_id = sel.env.id.clone();
    if request.dry_run {
        return Ok(out);
    }
    let env = sel.env.name.clone();
    if request.expected_version != Some(out.version) {
        return Err(changed(&env));
    }
    if out.draft_changed {
        return Err(CloudError::new(
            "CONFIG_DRAFT_PENDING",
            format!(
                "{env} has config edits in iii cloud that aren't applied yet: apply or drop them there, then copy again."
            ),
        ));
    }
    let mut workers = Map::new();
    for w in &out.workers {
        let fields = workers.entry(w.worker.clone()).or_insert_with(|| json!({}));
        for f in &w.fields {
            fields[&f.key] = f.value.clone();
        }
    }
    workers.retain(|_, fields| fields.as_object().is_some_and(|f| !f.is_empty()));
    if workers.is_empty() {
        return Err(CloudError::new(
            "NOTHING_TO_IMPORT",
            "No field of the config folder can be copied: see what was left out.",
        ));
    }
    let drafted = ctx
        .call(&Req::patch(
            format!("{base}/draft"),
            json!({ "workers": workers }),
        ))
        .await
        .map_err(|e| refusal(&env, e))?;
    // Already iii cloud's values: nothing new to apply.
    if drafted.get("draft_changed") == Some(&Value::Bool(true)) {
        let applied = ctx
            .call(&Req::post(
                format!("{base}/apply"),
                json!({ "expected_version": out.version }),
            ))
            .await
            .map_err(|e| refusal(&env, e))?;
        out.version = applied
            .get("number")
            .and_then(Value::as_i64)
            .unwrap_or(out.version + 1);
    }
    out.applied = true;
    for f in out.workers.iter_mut().flat_map(|w| w.fields.iter_mut()) {
        f.value = Value::Null;
    }
    let _ = crate::status::status(
        ctx,
        StatusRequest {
            refresh: Some(true),
        },
    )
    .await;
    Ok(out)
}

fn hash(value: &Value) -> String {
    hex::encode(Sha256::digest(crate::check::canonical(value).as_bytes()))
}

/// The newest check's console overrides of `env`: setting → (console, file) hashes.
fn overrides_now(s: &State, env: &str) -> BTreeMap<String, (String, String)> {
    s.check
        .iter()
        .filter(|c| c.environment_id == env)
        .flat_map(|c| c.plan.confirmations.overrides.iter())
        .map(|o| (o.setting.clone(), (hash(&o.console), hash(&o.file))))
        .collect()
}

/// `env`'s choices made for the newest check's value pairs.
pub fn choices_of(s: &State, env: &str) -> BTreeMap<String, OverrideChoice> {
    let now = overrides_now(s, env);
    s.override_choices
        .get(env)
        .into_iter()
        .flatten()
        .filter(|(setting, c)| {
            now.get(*setting)
                .is_some_and(|(console, file)| *console == c.console_hash && *file == c.file_hash)
        })
        .map(|(setting, c)| (setting.clone(), c.choice))
        .collect()
}

/// `iii-cloud::override::choose`: the page's own. Kept for the newest check's
/// console and file values of the setting; a changed pair drops it.
pub async fn choose(ctx: &Arc<Ctx>, request: OverrideChoose) -> Res<OverrideChosen> {
    let sel = selected(ctx)?;
    if request.environment_id != sel.env.id {
        return Err(env_changed());
    }
    let env = sel.env.id.clone();
    let chosen = ctx.store.update(|s| {
        let (console_hash, file_hash) = overrides_now(s, &env).remove(&request.setting)?;
        s.override_choices.entry(env.clone()).or_default().insert(
            request.setting.clone(),
            StoredChoice {
                console_hash,
                file_hash,
                choice: request.choice,
            },
        );
        Some(choices_of(s, &env))
    });
    let override_choices = chosen.ok_or_else(|| {
        CloudError::new(
            "NO_SUCH_OVERRIDE",
            format!(
                "The newest check has no console override of {}: check again.",
                request.setting
            ),
        )
    })?;
    Ok(OverrideChosen {
        environment_id: env,
        override_choices,
    })
}
