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
use crate::types::turn::{ParentLink, TurnOptions, TurnRecord, TurnStatus};

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
    let req: SpawnRequest = serde_json::from_value(arguments.clone()).map_err(|e| {
        is_error(
            "harness/invalid_request",
            format!("invalid spawn arguments: {e}"),
        )
    })?;

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

    let parent_link = ParentLink {
        session_id: parent.session_id.clone(),
        turn_id: parent.turn_id.clone(),
        function_call_id: call_id.to_string(),
    };
    let child = seed_child(deps, &cfg, &req, Some(&parent_link), Some(parent))
        .await
        .map_err(|e| is_error(e.code(), e.to_string()))?;

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
    })
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
    let session = deps.session().await;

    let model = req
        .model
        .clone()
        .or_else(|| parent_record.map(|p| p.options.model.clone()))
        .ok_or_else(|| HarnessError::InvalidRequest("spawn requires a model".into()))?;
    let provider = req
        .provider
        .clone()
        .or_else(|| parent_record.and_then(|p| p.options.provider.clone()));

    let requested_policy = req.options.as_ref().and_then(|o| o.functions.as_ref());
    let functions = match parent_record {
        Some(p) => policy::subset_policy(p.options.functions.as_ref(), requested_policy),
        None => requested_policy.cloned(),
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
    let linkage = parent.map(|p| {
        json!({
            "parent_session_id": p.session_id,
            "parent_turn_id": p.turn_id,
            "function_call_id": p.function_call_id,
            "depth": depth,
        })
    });
    let child_session_id = match &req.session_id {
        Some(id) => {
            session.ensure(id, None, linkage.as_ref()).await?;
            id.clone()
        }
        None => session.create(None, linkage.as_ref()).await?,
    };

    // The task is the child's opening user message.
    let task = normalize_message(req.task.clone())?;
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
            metadata: None,
            max_validation_retries: cfg.max_validation_retries,
        },
        calls: Default::default(),
        parent: parent.cloned(),
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

fn is_error(code: &str, message: String) -> ResultData {
    ResultData {
        content: vec![ContentBlock::text(message.clone())],
        is_error: true,
        details: json!({ "error": code, "message": message }),
    }
}
