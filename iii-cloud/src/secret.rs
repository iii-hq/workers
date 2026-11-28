//! `iii-cloud::secret` and `iii-cloud::secret::provide` (spec §5). The value
//! reaches iii cloud and the worktree's `.env` from the page only. It is never
//! logged, recorded, emitted or returned: this module has no `Debug` on it.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use serde_json::json;

use crate::check::selected;
use crate::cloud::Req;
use crate::ctx::{Ctx, new_id, now};
use crate::error::{CloudError, Res, WorkerError};
use crate::tool;
use crate::types::*;

pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    !name.starts_with("III_")
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Ask the person for a value (or annotate the row). Asking the same name
/// twice returns the open request.
pub async fn ask(ctx: &Arc<Ctx>, request: SecretAsk) -> Res<SecretRequest> {
    if !valid_name(&request.name) {
        return Err(CloudError::new(
            "NOT_A_SECRET",
            format!(
                "`iii-cloud::secret` asks the person for the value of one secret the compose file reads. `{}` is not a variable name. A decision goes to the person with `harness::ask`.",
                request.name
            ),
        ));
    }
    // A database URL is never the person's to give: iii cloud makes the database.
    let external = request.external.unwrap_or(false);
    if crate::local::database_shaped(&request.name) && !external {
        return Err(CloudError::new(
            "DATABASE_FROM_CLOUD",
            format!(
                "Do not ask for {}: iii cloud creates the database. Add the `database` worker (package://database, `version: latest`) to the compose file and set `{}: ${{DATABASE_PRIMARY_URL}}` on the container that reads it. iii cloud creates a managed Postgres (pgvector included) when the deploy is confirmed; the run here starts a local one. Pass `external: true` only when the person chose a database outside iii cloud.",
                request.name, request.name
            ),
        ));
    }
    let sel = crate::check::selected_for(ctx, request.handoff_id.as_deref()).await?;
    let mode = request.mode.unwrap_or_default();
    // Asking for a value is for a name the compose file reads. A note (not needed, decide) may name
    // one the file no longer reads, so only an ask is checked. No readable file: nothing to check.
    if mode == SecretMode::Ask
        && let Ok(manifest) = crate::values::manifest_of(ctx, &sel, None)
        && let Some(m) = crate::local::Manifest::load(&sel.workspace, Some(&manifest))
        && !m.variable_uses().0.iter().any(|u| u.name == request.name)
    {
        return Err(CloudError::new(
            "NOT_READ_BY_FILE",
            format!(
                "`{}` is not read by {manifest}. Add `${{{}}}` where it is used first, or ask the person with `harness::ask`.",
                request.name, request.name
            ),
        ));
    }
    let existing = ctx.store.read(|s| {
        s.secrets
            .iter()
            .find(|r| {
                r.environment_id == sel.env.id
                    && r.name == request.name
                    && r.state == SecretState::Waiting
            })
            .cloned()
    });
    if let (Some(open), SecretMode::Ask) = (existing, mode) {
        return Ok(open);
    }
    let record = SecretRequest {
        id: new_id(),
        environment_id: sel.env.id.clone(),
        name: request.name.clone(),
        description: request.description,
        mode,
        state: if mode == SecretMode::Ask {
            SecretState::Waiting
        } else {
            SecretState::Noted
        },
        cloud: None,
        local: None,
        asked_at: now(),
        resolved_at: None,
        error: None,
        external,
        target: Some(sel),
    };
    ctx.store.update(|s| {
        // A new note for a name replaces the older rows of that name.
        s.secrets.retain(|r| {
            !(r.environment_id == record.environment_id
                && r.name == record.name
                && r.state != SecretState::Set)
        });
        s.secrets.insert(0, record.clone());
    });
    emit(ctx, &record);
    Ok(record)
}

pub fn emit(ctx: &Ctx, r: &SecretRequest) {
    let summary = match r.state {
        SecretState::Waiting => format!("{} is waiting for a value", r.name),
        SecretState::Set => format!("{} is set", r.name),
        SecretState::LeftEmpty => format!("{} was left empty", r.name),
        SecretState::Noted => format!("{} noted as {:?}", r.name, r.mode),
        SecretState::Failed => format!("{} could not be saved", r.name),
    };
    ctx.emit(
        EventKind::Secret,
        &r.id,
        r.state != SecretState::Waiting,
        summary,
        r,
    );
}

/// The page's call: the value goes to the cloud and to `.env`.
pub async fn provide(ctx: &Arc<Ctx>, request: ProvideRequest) -> Res<SecretRequest> {
    let leave_empty = request.leave_empty.unwrap_or(false);
    if leave_empty == request.value.is_some() {
        return Err(CloudError::invalid("send either value or leave_empty"));
    }
    let current = ctx
        .store
        .read(|s| {
            s.secrets
                .iter()
                .find(|r| r.id == request.request_id)
                .cloned()
        })
        .ok_or_else(|| CloudError::invalid("no such request"))?;
    if !matches!(current.state, SecretState::Waiting | SecretState::Failed) {
        return Err(CloudError::invalid("the request is already resolved"));
    }
    let mut next = current.clone();
    next.error = None;
    if let Some(value) = request.value.as_deref() {
        if value.contains(['\n', '\r', '\0']) {
            return Err(CloudError::invalid("the value must fit on one line"));
        }
        if value.len() >= 3 {
            ctx.known_secrets.lock().unwrap().push(value.to_string());
        }
        // The request's own environment, not the one selected now. A request
        // stored before it kept its target: only while that one is still selected.
        let mut sel = match current.target.clone() {
            Some(target) => target,
            None => selected(ctx)?,
        };
        if sel.env.id != current.environment_id {
            return Err(CloudError::invalid(
                "this request is of another environment: ask for the value again",
            ));
        }
        // The manifest dir may have changed in the cloud since the environment was
        // selected: write the `.env` where it is now.
        if let Some(fresh) = crate::status::refresh_environment(ctx, &sel).await {
            sel.env = fresh;
        }
        let written = write_value(
            ctx,
            &sel,
            &current.name,
            "secret",
            value,
            sel.env.manifest.as_deref(),
            true,
        )
        .await;
        // A key whose permission does not cover variables: the value is not in the cloud.
        let mut key_refused = false;
        next.error = match &written.cloud_error {
            Some(e) if e.code == "NEEDS_BROWSER_SIGN_IN" || e.status == Some(403) => {
                key_refused = true;
                Some(WorkerError {
                    code: "NEEDS_BROWSER_SIGN_IN".into(),
                    message: "This API key cannot write variables. Sign in to iii cloud in the browser, or set the value in the console.".into(),
                    status: e.status,
                    fix: None,
                })
            }
            // The cloud's own message is not copied: a fixed text and the code.
            Some(e) => Some(WorkerError {
                code: e.code.clone(),
                message: "the cloud refused the value".into(),
                status: e.status,
                fix: None,
            }),
            None => None,
        };
        next.cloud = Some(match (&written.cloud_error, key_refused) {
            (None, _) => CloudOutcome::Set,
            (Some(_), true) => CloudOutcome::Skipped,
            (Some(_), false) => CloudOutcome::Failed,
        });
        next.local = Some(written.local);
        if let Some(e) = written.local_error {
            next.error.get_or_insert(e);
        }
        // Saved nowhere is a failure, not "set".
        let took = next.cloud == Some(CloudOutcome::Set) || next.local == Some(LocalOutcome::Set);
        if !took && next.error.is_none() && !key_refused {
            next.error = Some(WorkerError {
                code: "NOT_SAVED".into(),
                message:
                    "Neither iii cloud nor a local .env took the value. Set it in the console."
                        .into(),
                status: None,
                fix: None,
            });
        }
        let failed = next.cloud == Some(CloudOutcome::Failed) || next.error.is_some();
        next.state = if key_refused {
            // Never "set" when the cloud did not take it: the request stays open.
            SecretState::Waiting
        } else if failed {
            SecretState::Failed
        } else {
            SecretState::Set
        };
    } else {
        next.state = SecretState::LeftEmpty;
        next.cloud = Some(CloudOutcome::Skipped);
        next.local = Some(LocalOutcome::Skipped);
    }
    next.resolved_at =
        (!matches!(next.state, SecretState::Failed | SecretState::Waiting)).then(now);
    ctx.store.update(|s| {
        if let Some(slot) = s.secrets.iter_mut().find(|r| r.id == next.id) {
            *slot = next.clone();
        }
    });
    emit(ctx, &next);
    Ok(next)
}

/// What [`write_value`] did. Holds no value.
pub struct Written {
    /// The cloud's refusal: its code and status (its text is never shown).
    pub cloud_error: Option<CloudError>,
    pub local: LocalOutcome,
    /// The run's `.env` could not be written.
    pub local_error: Option<WorkerError>,
}

/// Write `value` for `name` (`secret` or `variable`) in `sel`'s environment:
/// one PUT to iii cloud, then the run's `.env` in `manifest`'s folder of the
/// workspace when the compose file reads `${NAME}` (with `local_fallback`,
/// also when the cloud did not take it). Never into a tracked `.env`; without
/// `local_fallback`, never a value of several lines. The value is not logged,
/// kept or returned.
pub async fn write_value(
    ctx: &Ctx,
    sel: &crate::check::Selected,
    name: &str,
    kind: &str,
    value: &str,
    manifest: Option<&str>,
    local_fallback: bool,
) -> Written {
    let mut put = Req::post(
        format!(
            "/teams/{}/environments/{}/variables/{name}",
            sel.team_id, sel.env.id
        ),
        json!({ "type": kind, "value": value }),
    );
    put.verb = Some(reqwest::Method::PUT);
    let cloud_error = ctx.call(&put).await.err();
    let manifest_dir = manifest
        .and_then(crate::local::confined)
        .and_then(|m| Path::new(m).parent())
        .unwrap_or(Path::new(""));
    let env_file = sel.workspace.join(manifest_dir).join(".env");
    // A `${NAME}` the run needs, and a `${NAME:-}` too once iii cloud has a value for it
    // (the run takes that one from the folder then): the value typed on the page reaches
    // the run's `.env` as well. (`wanted` below counts a cloud write that succeeded.)
    let run_needs = crate::local::Manifest::load(&sel.workspace, manifest).is_some_and(|m| {
        crate::local::required_variables(&m)
            .iter()
            .chain(
                // An empty value is what the run takes anyway.
                crate::local::empty_default_variables(&m)
                    .iter()
                    .filter(|_| !value.is_empty()),
            )
            .any(|n| n == name)
    });
    let wanted = match cloud_error {
        None => run_needs,
        Some(_) => local_fallback,
    };
    // `provide` keeps reporting a value `.env` can't hold; the page's save skips it.
    let skip_multiline = !local_fallback && value.contains(['\n', '\r', '\0']);
    // Exit 1 is "not tracked"; anything else (tracked, no repository, no git) keeps it out.
    let untracked = || async {
        tool::run(
            "git",
            // Literal: the folder's name may hold `[` or `*`.
            &[
                "--literal-pathspecs",
                "ls-files",
                "--error-unmatch",
                &env_file.to_string_lossy(),
            ],
            Some(&sel.workspace),
            None,
            "TOOL_MISSING",
        )
        .await
        .is_ok_and(|o| o.code == Some(1))
    };
    let (local, local_error) = if !wanted || skip_multiline || !untracked().await {
        (LocalOutcome::Skipped, None)
    } else {
        match set_env_line(&env_file, name, value) {
            Ok(()) => (LocalOutcome::Set, None),
            Err(e) => (LocalOutcome::Skipped, Some(e.to_worker_error())),
        }
    };
    Written {
        cloud_error,
        local,
        local_error,
    }
}

/// One writer per `.env` at a time: two secrets provided together both land.
fn env_lock(path: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static LOCKS: OnceLock<Mutex<HashMap<std::path::PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(path.to_path_buf())
        .or_default()
        .clone()
}

/// Replace `NAME=...` in place or append it. Atomic, mode 0600, serialized per file.
pub fn set_env_line(path: &Path, name: &str, value: &str) -> Res<()> {
    let lock = env_lock(path);
    let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
    if value.contains(['\n', '\r', '\0']) {
        return Err(CloudError::invalid("the value must fit on one line"));
    }
    let io = |e: std::io::Error| {
        CloudError::new(
            "INVALID_REQUEST",
            format!("cannot write {}: {}", path.display(), e.kind()),
        )
    };
    // Only a missing file starts empty: an unreadable or non-UTF-8 one is left alone.
    let current = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io(e)),
    };
    // As written, else in single quotes, else in double quotes with `"` escaped:
    // the first that `parse_env` reads back as the value.
    let reads_back = |text: &str| {
        crate::local::parse_env(text)
            .into_iter()
            .find(|(n, _)| n == name)
            .is_some_and(|(_, v)| v == value)
    };
    let not_saved = || {
        CloudError::new(
            "LOCAL_NOT_SAVED",
            format!(
                "{name} can't be written to {} so that it reads back the same.",
                path.display()
            ),
        )
    };
    let line = [
        value.to_string(),
        format!("'{value}'"),
        format!("\"{}\"", value.replace('"', "\\\"")),
    ]
    .into_iter()
    .map(|v| format!("{name}={v}"))
    .find(|l| reads_back(l))
    .ok_or_else(not_saved)?;
    // One line for the name: the first is replaced, later ones (which would win) go.
    let mut replaced = false;
    let mut lines: Vec<String> = vec![];
    for l in current.lines() {
        let key = l
            .trim_start()
            .strip_prefix("export ")
            .unwrap_or(l.trim_start());
        if !key.split_once('=').is_some_and(|(k, _)| k.trim() == name) {
            lines.push(l.to_string());
        } else if !replaced {
            replaced = true;
            lines.push(line.clone());
        }
    }
    if !replaced {
        lines.push(line);
    }
    let text = lines.join("\n") + "\n";
    if !reads_back(&text) {
        return Err(not_saved());
    }
    let tmp = path.with_file_name(".env.tmp");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&tmp).map_err(io)?;
    file.write_all(text.as_bytes()).map_err(io)?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(io)
}
