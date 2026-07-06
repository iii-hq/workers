//! Sub-agent spawning (harness.md § Sub-agents). A child is an ordinary
//! harness run in a child session, seeded through the same CAS as
//! `harness::send`. From a turn it is a pending dispatch: the parent parks and
//! the child's completion resolves the parent call.

use serde_json::{json, Value};

use crate::config::WorkerConfig;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::functions::send::normalize_message;
use crate::functions::spawn::SpawnRequest;
use crate::ids;
use crate::policy;
use crate::prompt;
use crate::trigger::{PendingInfo, ResultData};
use crate::types::content::ContentBlock;
use crate::types::message::AgentMessage;
use crate::types::turn::{
    ParentLink, TurnOptions, TurnRecord, TurnStatus, FS_SCOPE_KEY, FS_SCOPE_ROOT_KEY,
};

/// The ids of a freshly-seeded child turn.
pub struct ChildIds {
    pub session_id: String,
    pub turn_id: String,
}

/// Dispatch-path spawn: enforce guards, subset the policy, seed the child, and
/// report the call pending. Guard violations return an `is_error` result (never
/// a throw) so the model can adapt.
pub async fn spawn_pending(
    deps: &Deps,
    parent: &TurnRecord,
    call_id: &str,
    arguments: &Value,
) -> Result<PendingInfo, ResultData> {
    let cfg = deps.cfg().await;
    let mut req: SpawnRequest = serde_json::from_value(arguments.clone()).map_err(|e| {
        is_error(
            "harness/invalid_request",
            format!("invalid spawn arguments: {e}"),
        )
    })?;
    // Reactive bookkeeping is stamped ONLY by harness::react (which spawns
    // through the direct function entry, never this dispatch path). A model
    // could otherwise spoof these to defeat the self-edge breaker or the
    // reactive depth cap.
    req.spawned_by_subscription_id = None;
    req.reactive_depth = None;

    // Depth budget.
    if parent.depth + 1 > cfg.max_depth {
        return Err(is_error(
            "harness/spawn_depth_exceeded",
            format!(
                "spawn depth {} exceeds max_depth {}",
                parent.depth + 1,
                cfg.max_depth
            ),
        ));
    }
    // Fan-out budget: non-terminal children of this turn.
    let live = parent.live_children().len() as u32;
    if live >= cfg.max_children {
        return Err(is_error(
            "harness/spawn_fanout_exceeded",
            format!(
                "{live} live children at or above max_children {}",
                cfg.max_children
            ),
        ));
    }

    // Worktree isolation: create the child's workspace BEFORE seeding it,
    // so the child's very first step already runs jailed to it.
    let worktree = match req.options.as_ref().and_then(|o| o.isolation) {
        Some(crate::functions::spawn::Isolation::Worktree) => {
            Some(create_child_worktree(deps, &cfg, parent, req.session_id.as_deref()).await?)
        }
        None => None,
    };

    let parent_link = ParentLink {
        session_id: parent.session_id.clone(),
        turn_id: parent.turn_id.clone(),
        function_call_id: call_id.to_string(),
    };
    let seeded = seed_child_with_root(
        deps,
        &cfg,
        &req,
        Some(&parent_link),
        Some(parent),
        worktree.as_ref().map(|w| w.path.as_str()),
    )
    .await;
    let child = match seeded {
        Ok(child) => child,
        Err(e) => {
            // Best-effort orphan cleanup: the worktree was created for a
            // child that never came to exist.
            if let Some(wt) = &worktree {
                remove_child_worktree(deps, &cfg, parent, wt).await;
            }
            return Err(is_error(e.code(), e.to_string()));
        }
    };

    let pending_timeout_ms = req
        .options
        .as_ref()
        .and_then(|o| o.pending_timeout_ms)
        .unwrap_or(cfg.default_pending_timeout_ms);

    Ok(PendingInfo {
        pending_timeout_ms: Some(pending_timeout_ms),
        held_by: None,
        child_session_id: Some(child.session_id),
        child_turn_id: Some(child.turn_id),
        worktree,
    })
}

/// Create the isolated worktree for a spawn with `isolation: "worktree"`.
/// The name derives from the caller-picked child session id (sanitized)
/// or a fresh slug; the parent's filesystem root is the git repository.
async fn create_child_worktree(
    deps: &Deps,
    cfg: &WorkerConfig,
    parent: &TurnRecord,
    child_session_id: Option<&str>,
) -> Result<crate::types::turn::WorktreeRef, ResultData> {
    let Some(root) = parent.options.filesystem_root().map(str::to_string) else {
        return Err(is_error(
            "harness/worktree_requires_fs_root",
            "worktree isolation requires this turn to have a filesystem root \
             (metadata.fs_scope.root) inside a git repository — spawn without \
             isolation, or run in a session with a working directory"
                .to_string(),
        ));
    };
    let name = match child_session_id {
        Some(id) => crate::ids::sanitize(id),
        None => format!("child-{}", &crate::ids::new_session_id()[2..10]),
    };
    let grants =
        crate::filesystem_grants::roots(&deps.iii, &parent.session_id, cfg.session_timeout_ms)
            .await
            .unwrap_or_default();
    let args = crate::filesystem_scope::inject(
        "coder::worktree-add",
        json!({ "name": name }),
        Some(&root),
        &grants,
        Some((&parent.session_id, &parent.turn_id)),
    );
    let engine = deps.engine().await;
    match engine.dispatch("coder::worktree-add", args).await {
        Ok(value) => {
            let path = value.get("path").and_then(Value::as_str);
            let branch = value.get("branch").and_then(Value::as_str);
            match (path, branch) {
                (Some(path), Some(branch)) => Ok(crate::types::turn::WorktreeRef {
                    name,
                    path: path.to_string(),
                    branch: branch.to_string(),
                }),
                _ => Err(is_error(
                    "harness/worktree_add_failed",
                    format!("coder::worktree-add returned an unexpected shape: {value}"),
                )),
            }
        }
        Err(e) => Err(is_error(
            "harness/worktree_add_failed",
            format!(
                "could not create the child's worktree: {} — spawn without \
                 isolation, or fix the repository state",
                e.message
            ),
        )),
    }
}

/// Best-effort worktree removal (orphan cleanup / child completion).
/// Failures are logged, never surfaced — the worktree is inert on disk.
pub(crate) async fn remove_child_worktree(
    deps: &Deps,
    cfg: &WorkerConfig,
    parent: &TurnRecord,
    worktree: &crate::types::turn::WorktreeRef,
) -> Option<Value> {
    let root = parent.options.filesystem_root()?.to_string();
    let grants =
        crate::filesystem_grants::roots(&deps.iii, &parent.session_id, cfg.session_timeout_ms)
            .await
            .unwrap_or_default();
    let args = crate::filesystem_scope::inject(
        "coder::worktree-remove",
        json!({ "name": worktree.name }),
        Some(&root),
        &grants,
        Some((&parent.session_id, &parent.turn_id)),
    );
    let engine = deps.engine().await;
    match engine.dispatch("coder::worktree-remove", args).await {
        Ok(value) => Some(value),
        Err(e) => {
            tracing::warn!(
                session_id = %parent.session_id,
                worktree = %worktree.path,
                error = %e.message,
                "worktree cleanup failed; directory may remain"
            );
            None
        }
    }
}

/// Direct-call entry (a consumer starting a linked child). No parent linkage or
/// subsetting — the request's policy applies as-is.
pub async fn spawn_child(
    deps: &Deps,
    req: &SpawnRequest,
    parent: Option<&ParentLink>,
) -> Result<ChildIds, HarnessError> {
    let cfg = deps.cfg().await;
    seed_child(deps, &cfg, req, parent, None).await
}

/// Seed a child session + turn and enqueue its first step. When
/// `parent_record` is set the policy is subset against it, `max_turns` is
/// capped at the parent's remaining budget, and linkage metadata is recorded.
async fn seed_child(
    deps: &Deps,
    cfg: &WorkerConfig,
    req: &SpawnRequest,
    parent: Option<&ParentLink>,
    parent_record: Option<&TurnRecord>,
) -> Result<ChildIds, HarnessError> {
    seed_child_with_root(deps, cfg, req, parent, parent_record, None).await
}

/// [`seed_child`] with an explicit filesystem root for the child (worktree
/// isolation): the override replaces the inherited parent scope.
async fn seed_child_with_root(
    deps: &Deps,
    cfg: &WorkerConfig,
    req: &SpawnRequest,
    parent: Option<&ParentLink>,
    parent_record: Option<&TurnRecord>,
    fs_root_override: Option<&str>,
) -> Result<ChildIds, HarnessError> {
    let session = deps.session().await;

    let model = req
        .model
        .clone()
        .or_else(|| parent_record.map(|p| p.options.model.clone()))
        .ok_or_else(|| HarnessError::InvalidRequest("spawn requires a model".into()))?;
    let provider = match req
        .provider
        .clone()
        .or_else(|| parent_record.and_then(|p| p.options.provider.clone()))
    {
        Some(p) => Some(p),
        // No caller/parent provider: resolve from the router catalog so the
        // child's prompt family matches where the model actually routes.
        None => crate::functions::send::resolve_provider(deps, &model).await,
    };

    // Code mode without an explicit policy requests the curated coding
    // surface; with a parent it still narrows through subset_policy below,
    // so a code-mode child never escalates past its parent.
    let code_default = (req.options.as_ref().and_then(|o| o.mode) == Some(prompt::Mode::Code))
        .then(crate::policy::code_mode_policy);
    let requested_policy = req
        .options
        .as_ref()
        .and_then(|o| o.functions.as_ref())
        .or(code_default.as_ref());
    let functions = match parent_record {
        Some(p) => policy::subset_policy(p.options.functions.as_ref(), requested_policy),
        // Parentless (direct/CLI/trigger-fired) spawn: explicit options win;
        // otherwise the configured read-only baseline instead of deny-all.
        None => requested_policy
            .cloned()
            .or_else(|| cfg.default_functions.clone()),
    };

    let depth = parent_record.map(|p| p.depth + 1).unwrap_or(0);
    let requested_turns = req
        .options
        .as_ref()
        .and_then(|o| o.max_turns)
        .unwrap_or(cfg.default_max_turns);
    let max_turns = match parent_record {
        Some(p) => {
            let remaining = p.options.max_turns.saturating_sub(p.turn_count).max(1);
            requested_turns.min(remaining).max(1)
        }
        None => requested_turns,
    };

    // Child session, with sub-agent linkage merged into SessionMeta.metadata.
    // A live parent turn gives the full linkage (resolve + display). A direct /
    // trigger-fired spawn has no parent turn, but a caller-supplied
    // `parent_session_id` still writes a display-only link so the console nests
    // the child (no policy inheritance, no parent-call resolution).
    let linkage = match parent {
        Some(p) => Some(json!({
            "parent_session_id": p.session_id,
            "parent_turn_id": p.turn_id,
            "function_call_id": p.function_call_id,
            "depth": depth,
        })),
        None => req.parent_session_id.as_ref().map(|psid| {
            json!({
                "parent_session_id": psid,
                "depth": depth,
            })
        }),
    };
    let child_session_id = match &req.session_id {
        Some(id) => {
            let created = session.ensure(id, None, linkage.as_ref()).await?;
            if !created {
                // Reuse is legitimate (a fork, or delivering a reaction into an
                // existing chat) but silent reuse of a stale id is a classic
                // pipeline bug: the old transcript carries over and the console
                // keeps the session nested under whoever created it first.
                tracing::info!(
                    child_session_id = %id,
                    "harness::spawn reused an existing session — prior transcript and parent linkage retained"
                );
            }
            id.clone()
        }
        None => session.create(None, linkage.as_ref()).await?,
    };

    // The task is the child's opening user message. A worktree-isolated
    // child gets an explicit commit instruction: its code prompt forbids
    // commits, but committing to the worktree branch is exactly how its
    // work reaches the parent (clean-only removal + `git merge wt/<name>`).
    let mut task = normalize_message(req.task.clone())?;
    if fs_root_override.is_some() {
        if let AgentMessage::User(user) = &mut task {
            user.content.push(ContentBlock::text(
                "[isolation] You are working in a git worktree dedicated to \
                 this task, on its own branch. When your work is complete and \
                 verified, COMMIT it (git add -A && git commit -m \"<what you \
                 did>\" via shell::exec) — this is the sanctioned exception to \
                 the no-commit rule: your branch is how the work reaches the \
                 agent that spawned you, and an uncommitted worktree cannot be \
                 merged. Do not push.",
            ));
        }
    }
    session
        .append(&child_session_id, &task, None, None, None)
        .await?;

    let turn_id = ids::new_turn_id();
    let now = AgentMessage::now_ms();
    let record = TurnRecord {
        turn_id: turn_id.clone(),
        session_id: child_session_id.clone(),
        status: TurnStatus::Running,
        step: 0,
        turn_count: 0,
        depth,
        abort: false,
        watermark_entry_id: None,
        stream_request_id: None,
        options: TurnOptions {
            model,
            provider: provider.clone(),
            system_prompt: prompt::resolve_system_prompt(
                req.options.as_ref().and_then(|o| o.system_prompt.clone()),
                req.options
                    .as_ref()
                    .map(|o| o.system_prompt_strategy)
                    .unwrap_or_default(),
                req.options.as_ref().and_then(|o| o.mode),
                provider.as_deref(),
            ),
            mode: req.options.as_ref().and_then(|o| o.mode),
            max_turns,
            thinking_level: req.options.as_ref().and_then(|o| o.thinking_level),
            output: req
                .options
                .as_ref()
                .and_then(|o| o.output.clone())
                .unwrap_or_default(),
            functions,
            metadata: match fs_root_override {
                Some(root) => Some(json!({ FS_SCOPE_KEY: { FS_SCOPE_ROOT_KEY: root } })),
                None => inherit_filesystem_scope(parent_record),
            },
            max_validation_retries: cfg.max_validation_retries,
        },
        calls: Default::default(),
        parent: parent.cloned(),
        // Self-parent is dropped: a reaction delivered INTO session X (e.g. a
        // reporter posting into the chat) must not carry X as its display
        // parent, or its own turn-completed would match a
        // `parent_session_id: X` subscription and re-fire the reaction — an
        // infinite loop.
        display_parent_session_id: match parent {
            Some(_) => None,
            None => req
                .parent_session_id
                .clone()
                .filter(|p| p != &child_session_id),
        },
        spawned_by_subscription_id: req.spawned_by_subscription_id.clone(),
        reactive_depth: req.reactive_depth,
        env_context: None,
        result: None,
        result_error: None,
        validation_retries: 0,
        created_at: now,
        updated_at: now,
    };
    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
    crate::turn_loop::enqueue_step(&deps.iii, &child_session_id, &turn_id, 0).await?;

    Ok(ChildIds {
        session_id: child_session_id,
        turn_id,
    })
}

/// A child inherits only the parent's filesystem scope, so the session's
/// picked directory stays reachable from sub-agent shell/coder calls. The rest
/// of the parent metadata belongs to the parent's turn and must not leak.
fn inherit_filesystem_scope(parent: Option<&TurnRecord>) -> Option<Value> {
    let root = parent.and_then(|p| p.options.filesystem_root())?;
    Some(json!({ FS_SCOPE_KEY: { FS_SCOPE_ROOT_KEY: root } }))
}

fn is_error(code: &str, message: String) -> ResultData {
    ResultData {
        content: vec![ContentBlock::text(message.clone())],
        is_error: true,
        details: json!({ "error": code, "message": message }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::output::OutputContract;

    fn parent_record(metadata: Option<Value>) -> TurnRecord {
        TurnRecord {
            turn_id: "t_parent".into(),
            session_id: "s_parent".into(),
            status: TurnStatus::AwaitingFunctions,
            step: 1,
            turn_count: 1,
            depth: 0,
            abort: false,
            watermark_entry_id: None,
            stream_request_id: None,
            options: TurnOptions {
                model: "m".into(),
                provider: None,
                system_prompt: None,
                mode: None,
                max_turns: 16,
                thinking_level: None,
                output: OutputContract::Text,
                functions: None,
                metadata,
                max_validation_retries: 2,
            },
            calls: Default::default(),
            parent: None,
            display_parent_session_id: None,
            spawned_by_subscription_id: None,
            reactive_depth: None,
            env_context: None,
            result: None,
            result_error: None,
            validation_retries: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn child_inherits_the_parent_filesystem_root() {
        let parent = parent_record(Some(json!({
            "fs_scope": { "root": "/work/project" },
            "message_id": "m_1",
            "session_id": "s_console",
        })));
        assert_eq!(
            inherit_filesystem_scope(Some(&parent)),
            Some(json!({ "fs_scope": { "root": "/work/project" } }))
        );
    }

    #[test]
    fn child_metadata_stays_none_without_a_parent_filesystem_root() {
        // Direct spawns have no parent record; parents without a picked
        // directory must not fabricate one. Other metadata keys are per-turn
        // tracing and never leak onto the child.
        assert_eq!(inherit_filesystem_scope(None), None);
        let unscoped = parent_record(Some(json!({ "message_id": "m_1" })));
        assert_eq!(inherit_filesystem_scope(Some(&unscoped)), None);
        let bare = parent_record(None);
        assert_eq!(inherit_filesystem_scope(Some(&bare)), None);
    }
}
