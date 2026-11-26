//! Hand-offs to the agent (spec: "the harness takes forever and I don't know
//! what's happening"). The page hands work to the agent with a marker at the top
//! of the prompt; the worker finds that session, follows it, and publishes what
//! the agent is doing as `status.agent`. Only descriptions are ever stored or
//! emitted: no message body, tool input or result.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::check::selected;
use crate::ctx::{Ctx, new_id};
use crate::error::{CloudError, Res, cut};
use crate::types::*;

/// No new message for this long while the session works: stalled.
const STALL_SECONDS: u64 = 180;
/// Stop looking for a session that never shows up.
const GIVE_UP_SECONDS: u64 = 1800;
const CURRENT_MAX: usize = 120;
const QUESTION_MAX: usize = 200;

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// What one poll saw of the session.
#[derive(Debug, Default, Clone)]
pub struct Observed {
    /// `working`, `idle`, `done` or `error`.
    pub status: String,
    pub has_assistant: bool,
    pub last_activity_ms: Option<i64>,
    pub question: Option<String>,
}

/// The phase of the agent's work. `seen_working` remembers that the session
/// began a turn, so an idle session that has not started yet is not "done".
pub fn derive_phase(obs: &Observed, now_ms: i64, stall_ms: i64, seen_working: bool) -> AgentPhase {
    if obs.question.is_some() {
        return AgentPhase::WaitingForYou;
    }
    if obs.status == "working" {
        let quiet = obs
            .last_activity_ms
            .is_some_and(|at| now_ms - at > stall_ms);
        return if quiet {
            AgentPhase::Stalled
        } else {
            AgentPhase::Working
        };
    }
    if seen_working || obs.has_assistant {
        AgentPhase::Done
    } else {
        AgentPhase::Starting
    }
}

/// A call's label: its `arguments.description`, else the call's own
/// `description`, else its function id. Plain text, cut.
pub fn call_label(block: &Value) -> Option<String> {
    let text = block
        .pointer("/arguments/description")
        .and_then(Value::as_str)
        .or_else(|| block.get("description").and_then(Value::as_str))
        .or_else(|| block.get("function_id").and_then(Value::as_str))?;
    let plain: String = text.chars().filter(|c| !c.is_control()).collect();
    let plain = plain.trim();
    (!plain.is_empty()).then(|| cut(plain, CURRENT_MAX))
}

fn blocks(message: &Value) -> &[Value] {
    message
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn is_call(block: &Value) -> bool {
    block.get("type").and_then(Value::as_str) == Some("function_call")
}

fn first_text(message: &Value) -> String {
    blocks(message)
        .iter()
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn env_values(ctx: &Ctx) -> Vec<String> {
    let mut out: Vec<String> = ctx.known_secrets.lock().unwrap().clone();
    let Ok(sel) = selected(ctx) else {
        return out;
    };
    let checked = ctx
        .store
        .read(|s| s.check.as_ref().and_then(|c| c.manifest.clone()));
    let dir_of = |m: Option<&str>| {
        sel.workspace.join(
            m.and_then(|m| std::path::Path::new(m).parent())
                .unwrap_or(std::path::Path::new("")),
        )
    };
    let mut dirs = vec![sel.workspace.clone(), dir_of(sel.env.manifest.as_deref())];
    // The latest check's file may sit in another folder, with its own `.env`.
    let other = dir_of(checked.as_deref());
    if !dirs.contains(&other) {
        dirs.push(other);
    }
    for dir in dirs {
        if let Ok(text) = std::fs::read_to_string(dir.join(".env")) {
            out.extend(crate::local::parse_env(&text).into_iter().map(|(_, v)| v));
        }
    }
    out
}

/// The worker's fix branches: a hand-off works on one of them, never another.
const FIX_PREFIX: &str = "iii-cloud/ready-";

/// A worker worktree on a fix branch, not the person's checkout.
fn own_worktree(checkout: &str, path: &str, branch: &str) -> bool {
    branch.starts_with(FIX_PREFIX)
        && !crate::project::same_path(Path::new(checkout), Path::new(path))
}

/// `iii-cloud::handoff::worktree`: the hand-off's own worktree. One that has it
/// answers it; one that points at the person's folder (made before hand-offs
/// had their own) gets one, made like `use {from: checkout, bring_changes:
/// true}`, and works there from now on. The selection never changes.
pub async fn worktree(ctx: &Arc<Ctx>, request: HandoffWorktreeRequest) -> Res<HandoffWorktree> {
    let selected_env = ctx.store.read(|s| s.environment_id().map(str::to_owned));
    let h = ctx
        .store
        .read(|s| {
            s.handoffs
                .iter()
                .find(|h| match &request.handoff_id {
                    Some(id) => h.id == *id,
                    None => {
                        Some(&h.environment_id) == selected_env.as_ref()
                            && !matches!(h.agent.state, AgentPhase::Done | AgentPhase::Stopped)
                    }
                })
                .cloned()
        })
        .ok_or_else(|| {
            CloudError::invalid("no such hand-off (or none open for the selected environment)")
        })?;
    let (project_path, selected_source) = ctx.store.read(|s| {
        let p = s.project.as_ref();
        (
            p.map(|p| p.path.clone()),
            p.and_then(|p| p.workspace.as_ref())
                .filter(|_| s.environment_id() == Some(h.environment_id.as_str()))
                .and_then(|w| w.source.clone()),
        )
    });
    // The person's checkout: the repository's main worktree, else the selection's folder.
    let checkout = match h.workspace_path.is_empty() {
        false => crate::project::main_checkout(Path::new(&h.workspace_path)).await?,
        true => PathBuf::from(project_path.ok_or_else(CloudError::no_project)?),
    };
    let checkout_s = checkout.to_string_lossy().into_owned();
    if !h.workspace_path.is_empty()
        && Path::new(&h.workspace_path).exists()
        && own_worktree(&checkout_s, &h.workspace_path, &h.branch)
    {
        let base = ctx
            .store
            .read(|s| s.worktree_base.get(&h.workspace_path).cloned());
        return Ok(HandoffWorktree {
            path: h.workspace_path.clone(),
            branch: h.branch.clone(),
            base: base.map(|(branch, sha)| BriefBase { branch, sha }),
            created: false,
            merged_from: None,
            conflict: None,
        });
    }
    let env = match h.env.clone() {
        Some(env) => env,
        None => selected(ctx)
            .ok()
            .map(|sel| sel.env)
            .filter(|e| e.id == h.environment_id)
            .ok_or_else(|| {
                CloudError::invalid("this hand-off's environment is not known; select it first")
            })?,
    };
    let wt = crate::project::handoff_worktree(ctx, &checkout, &env, selected_source).await?;
    let agent = ctx.store.update(|s| {
        let x = s.handoffs.iter_mut().find(|x| x.id == h.id)?;
        x.workspace_path = wt.path.clone();
        x.branch = wt.branch.clone();
        x.worktree_note = wt.conflict.clone();
        Some(x.agent.clone())
    });
    if let Some(agent) = agent {
        emit(ctx, &agent);
    }
    Ok(wt)
}

/// `iii-cloud::handoff`: the page hands work to the agent.
pub async fn create(ctx: &Arc<Ctx>, request: HandoffRequest) -> Res<HandoffResponse> {
    let sel = selected(ctx)?;
    // `import` is for a project iii cloud does not have yet: no check to point at.
    let (check_id, manifest) = match (&request.check_id, request.kind) {
        (None, HandoffKind::Import) => (None, None),
        (Some(id), _) => {
            let found = ctx
                .store
                .read(|s| s.find_check(id).cloned())
                .filter(|c| c.environment_id == sel.env.id)
                .ok_or_else(|| CloudError::invalid("no such check for the selected environment"))?;
            // A check of another compose file: the agent's checks keep to it.
            let other = found
                .manifest
                .filter(|m| sel.env.manifest.as_deref() != Some(m.as_str()));
            (Some(found.id), other)
        }
        (None, _) => return Err(CloudError::invalid("check_id is required for this kind")),
    };
    if request.kind == HandoffKind::DeployFix && request.deployment_id.is_none() {
        return Err(CloudError::invalid("deploy_fix needs the deployment_id"));
    }
    // A double click: an equal hand-off for this environment is still starting or
    // working (or was made under 90 s ago with no chat yet). That one stands.
    let same = ctx.store.read(|s| {
        s.handoffs
            .iter()
            .find(|h| {
                h.environment_id == sel.env.id
                    && h.kind == request.kind
                    && h.deployment_id == request.deployment_id
                    && h.check_id == check_id
                    && match h.agent.state {
                        AgentPhase::Starting | AgentPhase::Working | AgentPhase::Waiting => true,
                        AgentPhase::WaitingForYou | AgentPhase::Stalled => {
                            h.agent.session_id.is_none()
                                && now_ms() - h.asked_at_ms
                                    < ctx.secs(NO_SESSION_SECONDS).as_millis() as i64
                        }
                        AgentPhase::Done | AgentPhase::Stopped => false,
                    }
            })
            .cloned()
    });
    // Another kind while one is starting, working or waiting on an event for this
    // environment: the request goes into that agent as a follow-up, never into a
    // second agent.
    let same = same.or_else(|| {
        ctx.store.read(|s| {
            s.handoffs
                .iter()
                .find(|h| {
                    h.environment_id == sel.env.id
                        && matches!(
                            h.agent.state,
                            AgentPhase::Starting | AgentPhase::Working | AgentPhase::Waiting
                        )
                })
                .cloned()
        })
    });
    if let Some(h) = same {
        let delivered = match (&h.agent.session_id, request.text.as_deref()) {
            (Some(session), Some(text)) => {
                deliver(ctx, session, &format!("{}\n{text}", h.marker), &new_id()).await
            }
            (Some(_), None) => true,
            (None, Some(text)) => {
                // No chat yet: the text waits and goes in once the chat is found.
                let text = text.to_string();
                ctx.store.update(|s| {
                    if let Some(h2) = s.handoffs.iter_mut().find(|x| x.id == h.id) {
                        h2.pending_text = Some(match h2.pending_text.take() {
                            Some(old) => format!("{old}\n\n{text}"),
                            None => text,
                        });
                    }
                });
                false
            }
            (None, None) => false,
        };
        return Ok(HandoffResponse {
            id: h.id,
            marker: h.marker,
            delivered,
            session_id: h.agent.session_id,
            reused: true,
        });
    }
    let (mut workspace_path, mut branch, checkout, source) = ctx.store.read(|s| {
        let p = s.project.as_ref();
        let w = p.and_then(|p| p.workspace.as_ref());
        (
            w.map(|w| w.path.clone()).unwrap_or_default(),
            w.map(|w| w.branch.clone()).unwrap_or_default(),
            p.map(|p| p.path.clone()).unwrap_or_default(),
            w.and_then(|w| w.source.clone()),
        )
    });
    // A hand-off never works in the person's checkout ("edits this folder"),
    // nor on a branch that is not a fix branch: it gets the worker's worktree.
    let mut worktree_note = None;
    if request.kind != HandoffKind::Import
        && !workspace_path.is_empty()
        && !own_worktree(&checkout, &workspace_path, &branch)
    {
        let wt =
            crate::project::handoff_worktree(ctx, Path::new(&checkout), &sel.env, source).await?;
        (workspace_path, branch, worktree_note) = (wt.path, wt.branch, wt.conflict);
    }
    let id = new_id();
    let marker = format!("iii-cloud hand-off {}", &id[..8]);
    let asked = now_ms();
    let handoff = Handoff {
        id: id.clone(),
        marker: marker.clone(),
        environment_id: sel.env.id.clone(),
        check_id,
        kind: request.kind,
        deployment_id: request.deployment_id,
        asked_at_ms: asked,
        agent: AgentState {
            handoff_id: id.clone(),
            kind: Some(request.kind),
            session_id: None,
            state: AgentPhase::Starting,
            started_at: rfc3339(asked),
            last_activity_at: None,
            steps: 0,
            current: None,
            question: None,
            working_on: vec![],
            note: None,
            waiting_for: None,
        },
        checked: false,
        pending_text: None,
        workspace_path,
        branch,
        team_id: sel.team_id.clone(),
        env: Some(sel.env.clone()),
        manifest,
        candidate: request.candidate == Some(true),
        worktree_note,
    };
    // A follow-up goes into the chat the previous hand-off used, when it is still
    // there: the console's composer would only keep the text as a draft while
    // that chat streams.
    let mut handoff = handoff;
    let mut delivered_to = None;
    let previous = ctx.store.read(|s| {
        s.handoffs
            .iter()
            .find(|h| h.environment_id == handoff.environment_id)
            .and_then(|h| h.agent.session_id.clone())
    });
    if let (Some(session), Some(text)) = (previous, request.text.as_deref())
        && deliver(ctx, &session, &format!("{marker}\n{text}"), &id).await
    {
        handoff.agent.session_id = Some(session.clone());
        delivered_to = Some(session);
    }
    // One open hand-off per environment: a new one replaces it.
    ctx.store.update(|s| {
        s.handoffs
            .retain(|h| h.environment_id != handoff.environment_id);
        s.handoffs.push(handoff.clone());
    });
    emit(ctx, &handoff.agent);
    follow(ctx, id.clone());
    Ok(HandoffResponse {
        id,
        marker,
        delivered: delivered_to.is_some(),
        session_id: delivered_to,
        reused: false,
    })
}

/// The console's own function policy for a send (ade `FALLBACK_FUNCTION_POLICY`).
fn function_policy() -> Value {
    json!({
        "allow": ["*"],
        "deny": ["approval::*", "configuration::register", "configuration::ensure", "shell::workspace::*"],
        "expose": "agent_call",
    })
}

/// Send `message` into an existing session with `harness::send`, built like the
/// console's `buildSendRequest`: the session's own model and folder, never an
/// agent (that would change an existing session's profile). The harness queues
/// it when the session is busy. False when the session is gone or the send fails.
async fn deliver(ctx: &Ctx, session: &str, message: &str, handoff_id: &str) -> bool {
    let Ok(meta) = ctx
        .plat
        .call("session::get", json!({ "session_id": session }))
        .await
    else {
        return false;
    };
    let Some(meta) = meta.get("meta").filter(|m| !m.is_null()) else {
        return false;
    };
    let metadata = meta.get("metadata");
    let mut request = json!({
        "session_id": session,
        "message": message,
        "idempotency_key": format!("iii-cloud-handoff-{handoff_id}"),
        "options": { "functions": function_policy() },
    });
    // The session's model, `provider::model`; absent, the harness uses its last turn's.
    if let Some((provider, model)) = metadata
        .and_then(|m| m.get("model"))
        .and_then(Value::as_str)
        .and_then(|m| m.split_once("::"))
    {
        request["model"] = json!(model);
        request["provider"] = json!(provider);
    }
    // The session's own root, so the turn works where the chat does.
    let root = metadata.and_then(|m| {
        m.pointer("/fs_scope/root")
            .or_else(|| m.get("working_dir"))
            .or_else(|| m.get("root"))
            .and_then(Value::as_str)
    });
    if let Some(root) = root {
        request["options"]["metadata"] = json!({ "fs_scope": { "root": root } });
    }
    ctx.plat.call("harness::send", request).await.is_ok()
}

fn emit(ctx: &Ctx, agent: &AgentState) {
    let terminal = matches!(agent.state, AgentPhase::Done | AgentPhase::Stopped);
    let summary = match (&agent.state, &agent.current) {
        (AgentPhase::Working, Some(c)) => format!("the agent is working: {c}"),
        (AgentPhase::WaitingForYou, _) => "the agent is waiting for you".to_string(),
        (AgentPhase::Waiting, _) => format!(
            "the agent waits for {}",
            agent.waiting_for.as_deref().unwrap_or("an event")
        ),
        (state, _) => format!("the agent is {state:?}").to_lowercase(),
    };
    ctx.emit(
        EventKind::Agent,
        &agent.handoff_id,
        terminal,
        summary,
        agent,
    );
}

/// `iii-cloud::handoff::stop`: stop the session's running turn with the
/// harness's own `harness::stop`.
pub async fn stop(ctx: &Arc<Ctx>, request: HandoffStopRequest) -> Res<HandoffStopResponse> {
    let handoff = ctx
        .store
        .read(|s| s.handoffs.iter().find(|h| h.id == request.id).cloned())
        .ok_or_else(|| CloudError::invalid("no such hand-off"))?;
    let session = handoff
        .agent
        .session_id
        .clone()
        .ok_or_else(|| CloudError::invalid("the agent has no session yet; nothing to stop"))?;
    let out = ctx
        .plat
        .call("harness::stop", json!({ "session_id": session }))
        .await?;
    let stopping = out
        .get("stopping")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let agent = ctx.store.update(|s| {
        let h = s.handoffs.iter_mut().find(|h| h.id == request.id)?;
        h.agent.state = AgentPhase::Stopped;
        h.agent.working_on.clear();
        h.agent.note = None;
        Some(h.agent.clone())
    });
    if let Some(agent) = agent {
        emit(ctx, &agent);
    }
    Ok(HandoffStopResponse {
        id: request.id,
        stopping,
    })
}

/// The problem ids of the newest check of the environment that `names` (ids or
/// codes) stand for.
fn resolve_working_on(s: &crate::store::State, env: &str, names: &[String]) -> Vec<String> {
    let Some(check) = s.check.as_ref().filter(|c| c.environment_id == env) else {
        return vec![];
    };
    let mut out: Vec<String> = vec![];
    for name in names {
        for p in &check.problems {
            let hit =
                p.id == *name || p.code == *name || p.cloud_code.as_deref() == Some(name.as_str());
            if hit && !out.contains(&p.id) {
                out.push(p.id.clone());
            }
        }
    }
    out
}

/// Keep only the ids the newest check still has.
pub fn still_open(s: &crate::store::State, env: &str, ids: &[String]) -> Vec<String> {
    let Some(check) = s.check.as_ref().filter(|c| c.environment_id == env) else {
        return vec![];
    };
    ids.iter()
        .filter(|id| check.problems.iter().any(|p| p.id == **id))
        .cloned()
        .collect()
}

/// `iii-cloud::handoff::progress`: the agent says which problems it starts on.
pub async fn progress(
    ctx: &Arc<Ctx>,
    request: HandoffProgressRequest,
) -> Res<HandoffProgressResponse> {
    let sel = crate::check::selected_for(ctx, request.handoff_id.as_deref()).await?;
    let secrets = env_values(ctx);
    let note = request
        .note
        .as_deref()
        .map(|n| n.chars().filter(|c| !c.is_control()).collect::<String>())
        .map(|n| cut(crate::run::redact(n.trim(), &secrets).as_str(), CURRENT_MAX))
        .filter(|n| !n.is_empty());
    let (resolved, agent) = ctx
        .store
        .update(|s| {
            let resolved = resolve_working_on(s, &sel.env.id, &request.working_on);
            let h = s.handoffs.iter_mut().find(|h| match &request.handoff_id {
                // The caller's own hand-off, not whichever is first open.
                Some(id) => h.id == *id,
                None => {
                    h.environment_id == sel.env.id
                        && !matches!(h.agent.state, AgentPhase::Done | AgentPhase::Stopped)
                }
            })?;
            h.agent.working_on = resolved.clone();
            h.agent.note = note.clone();
            Some((resolved, h.agent.clone()))
        })
        .ok_or_else(|| CloudError::invalid("no open hand-off for the selected environment"))?;
    emit(ctx, &agent);
    Ok(HandoffProgressResponse {
        working_on: resolved,
        note,
    })
}

/// Follow every open hand-off again after a restart.
pub fn resume(ctx: &Arc<Ctx>) {
    let open: Vec<String> = ctx.store.read(|s| {
        s.handoffs
            .iter()
            // Done is provisional: a hand-off is followed while its chat exists.
            .filter(|h| h.agent.state != AgentPhase::Stopped && !retired(ctx, h))
            .map(|h| h.id.clone())
            .collect()
    });
    for id in open {
        follow(ctx, id);
    }
}

pub fn follow(ctx: &Arc<Ctx>, id: String) {
    let key = format!("handoff:{id}");
    if !ctx.busy.lock().unwrap().insert(key.clone()) {
        return;
    }
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let mut local = Local::default();
        loop {
            match poll(&ctx, &id, &mut local).await {
                Poll::Continue => {}
                Poll::Finished => break,
            }
            // A Done hand-off idle for 30 min is retired: no more polling, the record stays.
            if ctx
                .store
                .read(|s| s.handoffs.iter().find(|h| h.id == id).cloned())
                .is_some_and(|h| retired(&ctx, &h))
            {
                break;
            }
            // Stop looking for a chat that never shows up; a known chat is followed on.
            let has_session = ctx.store.read(|s| {
                s.handoffs
                    .iter()
                    .find(|h| h.id == id)
                    .is_some_and(|h| h.agent.session_id.is_some())
            });
            if !has_session && started.elapsed() > ctx.secs(GIVE_UP_SECONDS) {
                // Not a silent stop: say the hand-off made no progress.
                let agent = ctx.store.update(|s| {
                    let h = s.handoffs.iter_mut().find(|h| h.id == id)?;
                    h.agent.state = AgentPhase::Stalled;
                    h.agent.question = Some(NO_PROGRESS_TEXT.to_string());
                    Some(h.agent.clone())
                });
                if let Some(agent) = agent {
                    emit(&ctx, &agent);
                }
                break;
            }
            tokio::time::sleep(ctx.secs(5)).await;
        }
        ctx.busy.lock().unwrap().remove(&key);
    });
}

/// A Done hand-off with no activity for 30 minutes.
pub fn retired(ctx: &Ctx, h: &Handoff) -> bool {
    if h.agent.state != AgentPhase::Done {
        return false;
    }
    let last = h
        .agent
        .last_activity_at
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp_millis())
        .unwrap_or(h.asked_at_ms);
    now_ms() - last > ctx.secs(GIVE_UP_SECONDS).as_millis() as i64
}

#[derive(Default)]
struct Local {
    /// Sessions already read and found not to carry the marker.
    other: HashSet<String>,
    watermark: Option<String>,
    steps: u32,
    seen_working: bool,
    /// The fast check of this idle stretch has run (reset when the session works again).
    idle_checked: bool,
}

enum Poll {
    Continue,
    Finished,
}

async fn find_session(ctx: &Ctx, handoff: &Handoff, local: &mut Local) -> Option<String> {
    let list = ctx
        .plat
        .call(
            "session::list",
            json!({ "limit": 20, "order": "created_desc" }),
        )
        .await
        .ok()?;
    for meta in list.get("sessions").and_then(Value::as_array)? {
        let id = meta.get("session_id").and_then(Value::as_str)?;
        // Newest first: older than the hand-off, and so is everything after it.
        if meta.get("created_at").and_then(Value::as_i64).unwrap_or(0) < handoff.asked_at_ms - 5_000
        {
            break;
        }
        if local.other.contains(id) {
            continue;
        }
        let first = ctx
            .plat
            .call(
                "session::messages",
                json!({ "session_id": id, "limit": 1, "roles": ["user"] }),
            )
            .await
            .ok();
        let text = first
            .as_ref()
            .and_then(|v| v.pointer("/messages/0/message"))
            .map(first_text)
            .unwrap_or_default();
        if text.contains(&handoff.marker) {
            return Some(id.to_string());
        }
        local.other.insert(id.to_string());
    }
    None
}

/// Count the function calls added since the watermark.
async fn count_steps(ctx: &Ctx, session: &str, local: &mut Local) {
    for _ in 0..2 {
        let mut payload = json!({ "session_id": session, "roles": ["assistant"], "content": "usage", "limit": 200 });
        if let Some(wm) = &local.watermark {
            payload["after_entry_id"] = json!(wm);
        }
        match ctx.plat.call("session::messages", payload).await {
            Ok(page) => {
                for item in page
                    .get("messages")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(message) = item.get("message") {
                        local.steps += blocks(message).iter().filter(|b| is_call(b)).count() as u32;
                    }
                    if let Some(id) = item.get("entry_id").and_then(Value::as_str) {
                        local.watermark = Some(id.to_string());
                    }
                }
                return;
            }
            // The watermark left the path: count again from the top.
            Err(_) if local.watermark.is_some() => {
                local.watermark = None;
                local.steps = 0;
            }
            Err(_) => return,
        }
    }
}

async fn poll(ctx: &Arc<Ctx>, id: &str, local: &mut Local) -> Poll {
    let Some(handoff) = ctx
        .store
        .read(|s| s.handoffs.iter().find(|h| h.id == id).cloned())
    else {
        return Poll::Finished; // replaced by a newer hand-off
    };
    // Only a stop ends it. Done is provisional: new activity or a pending question moves it back.
    if handoff.agent.state == AgentPhase::Stopped {
        return Poll::Finished;
    }
    let mut agent = handoff.agent.clone();
    if agent.session_id.is_none() {
        match find_session(ctx, &handoff, local).await {
            Some(session) => agent.session_id = Some(session),
            None => {
                // Never `starting` forever: no chat after 90 s means the message did not land.
                let waited = now_ms() - handoff.asked_at_ms;
                if waited > ctx.secs(NO_SESSION_SECONDS).as_millis() as i64
                    && handoff.agent.state != AgentPhase::WaitingForYou
                {
                    agent.state = AgentPhase::WaitingForYou;
                    agent.question = Some(NO_CHAT_TEXT.to_string());
                    ctx.store.update(|s| {
                        if let Some(h) = s.handoffs.iter_mut().find(|h| h.id == id) {
                            h.agent.state = agent.state;
                            h.agent.question = agent.question.clone();
                        }
                    });
                    emit(ctx, &agent);
                }
                return Poll::Continue;
            }
        }
    }
    let session = agent.session_id.clone().unwrap_or_default();
    if let Some(text) = handoff.pending_text.as_deref() {
        // Text that arrived before the chat did.
        if deliver(
            ctx,
            &session,
            &format!("{}\n{text}", handoff.marker),
            &new_id(),
        )
        .await
        {
            ctx.store.update(|s| {
                if let Some(h) = s.handoffs.iter_mut().find(|h| h.id == id) {
                    h.pending_text = None;
                }
            });
        }
    }
    let mut obs = Observed::default();
    // The session's own status.
    if let Ok(meta) = ctx
        .plat
        .call("session::get", json!({ "session_id": session }))
        .await
    {
        // The chat is gone: nothing left to follow.
        if meta.get("meta").is_none_or(Value::is_null) {
            return Poll::Finished;
        }
        obs.status = meta
            .pointer("/meta/status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        obs.last_activity_ms = meta.pointer("/meta/updated_at").and_then(Value::as_i64);
    }
    count_steps(ctx, &session, local).await;
    // The newest messages: when, and what the last call says it does.
    let secrets = env_values(ctx);
    let mut current = agent.current.clone();
    let mut confirm_pending = false;
    if let Ok(tail) = ctx
        .plat
        .call(
            "session::messages-tail",
            json!({ "session_id": session, "limit": 4 }),
        )
        .await
    {
        let items = tail
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(ts) = items
            .iter()
            .rev()
            .find_map(|i| i.pointer("/message/timestamp").and_then(Value::as_i64))
        {
            obs.last_activity_ms = Some(ts);
        }
        confirm_pending = awaits_confirmation(&items);
        for item in items.iter().rev() {
            let Some(message) = item.get("message") else {
                continue;
            };
            if message.get("role").and_then(Value::as_str) == Some("assistant") {
                obs.has_assistant = true;
                // A call from before this hand-off (an old chat) is not what the agent does now.
                let at = message
                    .get("timestamp")
                    .and_then(Value::as_i64)
                    .unwrap_or(i64::MAX);
                if at < handoff.asked_at_ms - 2_000 {
                    continue;
                }
                if let Some(label) = blocks(message)
                    .iter()
                    .rev()
                    .find(|b| is_call(b))
                    .and_then(call_label)
                {
                    current = Some(label);
                    break;
                }
            }
        }
    }
    // An open question for the person.
    if let Ok(asks) = ctx
        .plat
        .call(
            "harness::ask::list",
            json!({ "session_id": session, "state": "open", "limit": 3 }),
        )
        .await
    {
        obs.question = asks
            .pointer("/entries/0/questions/0/question")
            .and_then(Value::as_str)
            .map(|q| cut(q.trim(), QUESTION_MAX));
    }
    local.seen_working |= obs.status == "working";
    obs.has_assistant |= local.steps > 0;
    let phase = derive_phase(
        &obs,
        now_ms(),
        ctx.secs(STALL_SECONDS).as_millis() as i64,
        local.seen_working,
    );
    if obs.status == "working" {
        local.idle_checked = false;
    }
    // An old chat the hand-off was sent into: idle and quiet since before the
    // hand-off means the turn has not begun, not that it is over.
    let fresh = obs
        .last_activity_ms
        .is_some_and(|t| t >= handoff.asked_at_ms - 2_000);
    let phase = if phase == AgentPhase::Done && !fresh && !local.seen_working {
        AgentPhase::Starting
    } else {
        phase
    };
    let mut question = obs.question.clone();
    let mut phase = phase;
    // A turn that ended (or went quiet) on a wake the agent armed waits on an
    // event, not on the person; once the wake fires or expires the rules below apply.
    let mut waiting_for = None;
    if matches!(phase, AgentPhase::Done | AgentPhase::Stalled) && !confirm_pending {
        waiting_for = awaited_wake(ctx, &session).await;
    }
    if waiting_for.is_some() {
        phase = AgentPhase::Waiting;
    } else if phase == AgentPhase::Done {
        // The turn ended, but not because the work is finished.
        if confirm_pending {
            // The agent proposed a worktree folder and waits for the person's confirmation.
            phase = AgentPhase::WaitingForYou;
            question = Some(CONFIRM_TEXT.to_string());
        } else if matches!(handoff.kind, HandoffKind::Ready | HandoffKind::DeployFix) {
            // One fast check per idle stretch: does the agent still have problems to fix?
            if !local.idle_checked {
                local.idle_checked = true;
                ctx.store.update(|s| {
                    if let Some(h) = s.handoffs.iter_mut().find(|h| h.id == id) {
                        h.checked = true;
                    }
                });
                let _ = crate::check::check(
                    ctx,
                    CheckRequest {
                        handoff_id: Some(id.to_string()),
                        ..Default::default()
                    },
                )
                .await;
            }
            if agent_problems_remain(ctx, &handoff) {
                phase = AgentPhase::WaitingForYou;
                question = Some(STOPPED_TEXT.to_string());
            } else if !passed_run_here(ctx, &handoff) {
                // Done is a ready check and a run here that passed for its commit.
                phase = AgentPhase::WaitingForYou;
                question = Some(NO_RUN_TEXT.to_string());
            }
        }
    }
    agent.state = phase;
    agent.steps = local.steps;
    // Nothing is "being done" once it is done.
    let current = if phase == AgentPhase::Done {
        None
    } else {
        current
    };
    agent.current = current.map(|c| crate::run::redact(&c, &secrets));
    agent.question = question.map(|q| crate::run::redact(&q, &secrets));
    agent.last_activity_at = obs.last_activity_ms.map(rfc3339);
    agent.waiting_for = waiting_for;
    // Store, and tell whoever listens when something that matters changed.
    let before = handoff.agent.clone();
    ctx.store.update(|s| {
        let open = s.check.clone();
        if let Some(h) = s.handoffs.iter_mut().find(|h| h.id == id) {
            // A stop that landed meanwhile stays. The agent's own progress is
            // kept as the agent last said it (a call may have landed during
            // this poll), minus the problems that left the newest check.
            if h.agent.state != AgentPhase::Stopped {
                let (working_on, note) = (h.agent.working_on.clone(), h.agent.note.clone());
                let env = h.environment_id.clone();
                h.agent = agent.clone();
                let ended = matches!(agent.state, AgentPhase::Done | AgentPhase::Stopped);
                h.agent.working_on = match (&open, ended) {
                    (Some(check), false) if check.environment_id == env => working_on
                        .into_iter()
                        .filter(|id| check.problems.iter().any(|p| p.id == *id))
                        .collect(),
                    _ => vec![],
                };
                h.agent.note = if ended { None } else { note };
            }
        }
    });
    let changed = (
        before.state,
        &before.session_id,
        before.steps,
        &before.current,
        &before.question,
        &before.waiting_for,
    ) != (
        agent.state,
        &agent.session_id,
        agent.steps,
        &agent.current,
        &agent.question,
        &agent.waiting_for,
    );
    if changed {
        emit(ctx, &agent);
    }
    Poll::Continue
}

const NO_SESSION_SECONDS: u64 = 90;
const NO_PROGRESS_TEXT: &str = "No progress for 30 min: the chat never started. Open the chat and send the message, or try again.";
const NO_CHAT_TEXT: &str =
    "The chat didn't start. Open the chat and send the message, or try again.";
const CONFIRM_TEXT: &str =
    "Confirm the worktree folder in the chat (Use for chat), then the agent continues.";
const STOPPED_TEXT: &str = "Your agent stopped; open the chat to see what it needs.";
const NO_RUN_TEXT: &str = "Your agent stopped before running it here like the cloud.";
const PROPOSE_FN: &str = "ade::working-directory::propose";

/// What the session's armed wake waits for, from `harness::observers::list`.
/// `None` when it has none or the list fails: the caller keeps its mapping.
async fn awaited_wake(ctx: &Ctx, session: &str) -> Option<String> {
    let list = ctx
        .plat
        .call("harness::observers::list", json!({ "session_id": session }))
        .await
        .ok()?;
    waiting_for(&list, now_ms())
}

/// The newest armed wake in an `harness::observers::list` answer, as plain
/// text: an observer with no `target` (it notifies the owner session), with
/// fires left (`once`: none yet; else under `max_fires`, default 1) and not
/// past `expires_at`.
pub fn waiting_for(list: &Value, now_ms: i64) -> Option<String> {
    let armed = |o: &&Value| {
        let fires = o.get("fires").and_then(Value::as_u64).unwrap_or(0);
        let once = o.get("once").and_then(Value::as_bool).unwrap_or(false);
        let max = o.get("max_fires").and_then(Value::as_u64).unwrap_or(1);
        o.get("target").is_none_or(Value::is_null)
            && fires < if once { 1 } else { max }
            && o.get("expires_at")
                .and_then(Value::as_i64)
                .is_none_or(|at| at > now_ms)
    };
    let o = list
        .get("observers")?
        .as_array()?
        .iter()
        .rev()
        .find(armed)?;
    let text = |k: &str| {
        o.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| cut(t, QUESTION_MAX))
    };
    let function = o.get("function_id").and_then(Value::as_str);
    if function == Some("iii-cloud::on-change") {
        let kind = o
            .pointer("/filters/input.kind")
            .map(|f| f.get("$eq").unwrap_or(f));
        match kind.and_then(Value::as_str) {
            Some("run") => return Some("the run here to end".into()),
            Some("check") => return Some("the check to end".into()),
            Some("deploy") => return Some("the deploy to end".into()),
            _ => {}
        }
    }
    text("label")
        .or_else(|| text("action"))
        .or_else(|| Some(format!("{} to fire", function.unwrap_or("an event"))))
}

/// The newest function result is `ade::working-directory::propose` asking for
/// confirmation, and the person has not written since.
pub fn awaits_confirmation(items: &[Value]) -> bool {
    let mut call_fn: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut pending = false;
    for item in items {
        let Some(message) = item.get("message") else {
            continue;
        };
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                for b in blocks(message).iter().filter(|b| is_call(b)) {
                    if let (Some(id), Some(f)) = (
                        b.get("id").and_then(Value::as_str),
                        b.get("function_id").and_then(Value::as_str),
                    ) {
                        call_fn.insert(id.to_string(), f.to_string());
                    }
                }
            }
            Some("function_result") => {
                let f = message
                    .get("function_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| {
                        message
                            .get("function_call_id")
                            .and_then(Value::as_str)
                            .and_then(|id| call_fn.get(id).cloned())
                    });
                if f.as_deref() == Some(PROPOSE_FN) {
                    let mut text = message
                        .get("details")
                        .map(Value::to_string)
                        .unwrap_or_default();
                    for b in blocks(message) {
                        text.push_str(b.get("text").and_then(Value::as_str).unwrap_or_default());
                    }
                    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                    pending = text.contains("\"requires_confirmation\":true");
                }
            }
            Some("user") => pending = false,
            _ => {}
        }
    }
    pending
}

/// The newest check still has problems that are the agent's to fix.
fn agent_problems_remain(ctx: &Ctx, handoff: &Handoff) -> bool {
    ctx.store.read(|s| {
        s.check
            .as_ref()
            .filter(|c| c.environment_id == handoff.environment_id)
            .is_some_and(|c| {
                !c.ready
                    && c.problems
                        .iter()
                        .any(|p| p.owner == Owner::Agent && p.severity != Severity::Info)
            })
    })
}

/// The newest check of the hand-off's environment has a passed run for its own commit.
fn passed_run_here(ctx: &Ctx, handoff: &Handoff) -> bool {
    ctx.store.read(|s| {
        let mut mine = s
            .check
            .iter()
            .chain(s.checks.iter())
            .filter(|c| c.environment_id == handoff.environment_id);
        let Some(latest) = mine.next() else {
            return false;
        };
        // The fast check after the turn is a new check of the same commit: the
        // run the agent made belongs to an earlier check of that commit.
        s.check
            .iter()
            .chain(s.checks.iter())
            .filter(|c| c.environment_id == handoff.environment_id && c.commit == latest.commit)
            .any(|c| {
                c.run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Passed)
            })
    })
}

/// Used by tests: the phase now, from what was observed.
pub fn phase_of(obs: &Observed, seen_working: bool, stall: Duration) -> AgentPhase {
    derive_phase(obs, now_ms(), stall.as_millis() as i64, seen_working)
}

/// One look at a hand-off's session, from a fresh follower: what `status.agent`
/// would show now. For probes against a real engine.
pub async fn poll_now(ctx: &Arc<Ctx>, id: &str) -> Option<AgentState> {
    let mut local = Local::default();
    poll(ctx, id, &mut local).await;
    ctx.store.read(|s| {
        s.handoffs
            .iter()
            .find(|h| h.id == id)
            .map(|h| h.agent.clone())
    })
}

/// Tracked `*compose*.y*ml` files of the worktree that declare `containers:`.
pub fn compose_files(worktree: &std::path::Path) -> Vec<String> {
    // `-z`: names as they are, not quoted (a non-ASCII name would be).
    let Ok(out) = std::process::Command::new("git")
        .args(["ls-files", "-z", "--", "*compose*.yaml", "*compose*.yml"])
        .current_dir(worktree)
        .output()
    else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|f| !f.is_empty())
        .filter(|f| {
            std::fs::read_to_string(worktree.join(f)).is_ok_and(|t| {
                t.lines()
                    .any(|l| l.trim_end() == "containers:" || l.starts_with("containers:"))
            })
        })
        .map(str::to_owned)
        .collect()
}

/// The compose files other than the one checked (at most 10).
fn other_compose_files(worktree: &std::path::Path, manifest: &str) -> Vec<String> {
    compose_files(worktree)
        .into_iter()
        .filter(|f| f != manifest)
        .take(10)
        .collect()
}

/// The fix branch is on origin but is not part of this worktree's history: an
/// earlier try left it. (Local tracking ref; `use` fetched it with `--prune`.)
/// Only ever about a fix branch: never another branch, never a force for one.
fn earlier_try_note(worktree: &std::path::Path, branch: &str) -> Option<String> {
    if !branch.starts_with(FIX_PREFIX) {
        return None;
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(worktree)
            .output()
            .ok()
    };
    let remote = format!("origin/{branch}");
    let exists = git(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/remotes/{remote}"),
    ])?
    .status
    .success();
    if !exists {
        return None;
    }
    let ancestor = git(&["merge-base", "--is-ancestor", &remote, "HEAD"])?
        .status
        .success();
    (!ancestor).then(|| {
        format!("an earlier try left {remote}; your first push replaces it with --force-with-lease")
    })
}

fn brief_problem(p: &Problem, secrets: &[String]) -> BriefProblem {
    let clean = |t: &str, max: usize| cut(&crate::run::redact(t, secrets), max);
    BriefProblem {
        title: clean(&p.title, 160),
        file: p.file.clone(),
        line: p.line,
        detail: p.detail.as_deref().map(|d| clean(d, 400)),
        fix: p.fix.as_deref().map(|f| clean(f, 500)),
    }
}

/// What the agent needs for a hand-off, from the stored state. Redacted.
pub fn brief(s: &crate::store::State, h: &Handoff, secrets: &[String]) -> HandoffBrief {
    let project = s.project.as_ref();
    // An import from the add-project steps is of the folder being added; a
    // candidate left from an add the person cancelled is not.
    let source = match (h.kind, h.candidate, s.candidate.as_ref()) {
        (HandoffKind::Import, true, Some(c)) => Some(c),
        _ => project,
    };
    let check = h.check_id.as_deref().and_then(|id| s.find_check(id));
    let clean = |t: &str, max: usize| cut(&crate::run::redact(t, secrets), max);
    let problems = |owner: Owner| -> Vec<BriefProblem> {
        check
            .map(|c| {
                c.problems
                    .iter()
                    .filter(|p| p.owner == owner)
                    .map(|p| brief_problem(p, secrets))
                    .collect()
            })
            .unwrap_or_default()
    };
    let deploy = h
        .deployment_id
        .as_deref()
        .and_then(|id| s.deploys.iter().find(|d| d.id == id))
        .map(|d| BriefDeploy {
            title: d.failure.as_ref().map(|f| f.title.clone()),
            release_command: d.failure.as_ref().and_then(|f| f.release_command.clone()),
            silent: d.failure.as_ref().is_some_and(|f| f.silent),
            id: d.id.clone(),
            stage: d.error.as_ref().map(|e| e.stage.clone()),
            worker: d.error.as_ref().and_then(|e| e.worker.clone()),
            code: d.error.as_ref().map(|e| e.code.clone()),
            hint: d
                .failure
                .as_ref()
                .map(|f| clean(&f.hint, 400))
                .or_else(|| d.error.as_ref().map(|e| clean(&e.hint, 400))),
            // The failing stage's own log when we have it, else the deploy's last lines.
            log_tail: match &d.failure {
                Some(f) if !f.log_tail.is_empty() => {
                    f.log_tail.iter().map(|l| clean(l, 400)).collect()
                }
                _ => d
                    .log_tail
                    .iter()
                    .rev()
                    .take(20)
                    .rev()
                    .map(|l| clean(l, 400))
                    .collect(),
            },
        });
    let next_step = match h.kind {
        HandoffKind::Ready => "Fix the problems that are yours in the worktree, commit, run `iii-cloud::check {}`, then `iii-cloud::check {run: true}` until it is ready, then open the pull request.",
        HandoffKind::DeployFix => "Read the failure, fix it in the worktree, commit, run `iii-cloud::check {}`, then `iii-cloud::check {run: true}` until it is ready, then open a new pull request.",
        HandoffKind::Import => "Import the project: `iii-cloud::project::resolve`, follow its import steps (the GitHub install is the person's one click), then `iii-cloud::project::use`.",
        HandoffKind::PullRequest => "Push the worktree's branch and open the pull request with `gh pr create`, using the ready check's report as the body.",
    }
    .to_string();
    let env_manifest = project
        .and_then(|p| p.environment.as_ref())
        .and_then(|e| e.manifest.as_deref())
        .and_then(crate::local::confined)
        .unwrap_or("worker-compose.yaml");
    // The hand-off's own file when its check was of another one.
    let manifest = h
        .manifest
        .as_deref()
        .and_then(crate::local::confined)
        .unwrap_or(env_manifest)
        .to_string();
    // The hand-off's own worktree (an old hand-off may lack one: the selection's).
    let worktree = match (h.workspace_path.is_empty(), h.branch.is_empty()) {
        (false, false) => Some(BriefWorkspace {
            path: h.workspace_path.clone(),
            branch: h.branch.clone(),
        }),
        _ => project
            .and_then(|p| p.workspace.as_ref())
            .map(|w| BriefWorkspace {
                path: w.path.clone(),
                branch: w.branch.clone(),
            }),
    };
    let other_compose_files = worktree
        .as_ref()
        .map(|w| other_compose_files(Path::new(&w.path), &manifest))
        .unwrap_or_default();
    let pushes_directly = project.is_some_and(|p| {
        p.environment.as_ref().is_some_and(|e| {
            crate::project::tracks_other_branch(p.default_branch.as_deref(), &e.branch)
        })
    });
    // Every kind but the import works in the worktree: on a direct environment, none opens a pull request.
    let next_step = if pushes_directly && h.kind != HandoffKind::Import {
        format!(
            "Fix what is yours in the worktree, commit, push only your fix branch, run `iii-cloud::check {{}}`, then `iii-cloud::check {{run: true}}` until it is ready. No pull request: {} deploys from its own branch, and the person's deploy on the page pushes the checked commit to it. Never push {} yourself. Your job ends there: tell the person to review and deploy on the page; you never deploy.",
            project
                .and_then(|p| p.environment.as_ref())
                .map(|e| e.name.as_str())
                .unwrap_or("the environment"),
            project
                .and_then(|p| p.environment.as_ref())
                .map(|e| e.branch.as_str())
                .unwrap_or("its branch"),
        )
    } else {
        next_step
    };
    // Where it works, and the fix branch by its real name (`ready-<env>-2` when
    // the first name was taken). Only a fix branch is ever pushed with a lease.
    let next_step = match &worktree {
        Some(w) if h.kind != HandoffKind::Import => {
            let push = match w.branch.starts_with(FIX_PREFIX) {
                true => format!(
                    " Your fix branch is {}: push it with `git push --force-with-lease -u origin HEAD` from {}.",
                    w.branch, w.path
                ),
                false => String::new(),
            };
            format!(
                "{next_step} Work only in {} on {}. If this hand-off points at the person's folder, call `iii-cloud::handoff::worktree` first.{push}",
                w.path, w.branch
            )
        }
        _ => next_step,
    };
    let next_step = match &h.worktree_note {
        Some(note) if h.kind != HandoffKind::Import => format!("{next_step} First: {note}"),
        _ => next_step,
    };
    let next_step = if manifest == env_manifest {
        format!("{next_step} Only {manifest} is checked and deployed.")
    } else {
        let env = project
            .and_then(|p| p.environment.as_ref())
            .map(|e| e.name.as_str())
            .unwrap_or("the environment");
        format!(
            "{next_step} Only {manifest} is checked and deployed: your checks on this hand-off use it, and {env} switches from {env_manifest} to it when the person confirms the deploy. Never change the environment's compose file yourself."
        )
    };
    let remote_branch_note = worktree
        .as_ref()
        .and_then(|w| earlier_try_note(Path::new(&w.path), &w.branch));
    let base = worktree
        .as_ref()
        .and_then(|w| s.worktree_base.get(&w.path))
        .map(|(branch, sha)| BriefBase {
            branch: branch.clone(),
            sha: sha.clone(),
        });
    HandoffBrief {
        base,
        manifest,
        pushes_directly,
        other_compose_files,
        remote_branch_note,
        handoff_id: h.id.clone(),
        kind: h.kind,
        repository: source.map(|p| p.repository.clone()),
        project: source.map(|p| {
            let name = p
                .cloud_project
                .as_ref()
                .map(|c| c.name.clone())
                .unwrap_or_else(|| p.repository.clone());
            match &p.environment {
                Some(e) => format!("{name} · {}", e.name),
                None => name,
            }
        }),
        worktree,
        checkout_path: source.map(|p| p.path.clone()),
        check_id: check.map(|c| c.id.clone()),
        commit: check.map(|c| c.commit.clone()),
        agent_problems: problems(Owner::Agent),
        person_problems: problems(Owner::Person),
        deploy,
        next_step,
    }
}
