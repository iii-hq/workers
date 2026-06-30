//! The Grok turn: spawn `grok --single <prompt> --output-format streaming-json`,
//! parse the streaming-json event stream, mirror it verbatim onto
//! `grok::events`, translate it onto `agent::events`, and persist the session
//! record.

pub mod args;
pub mod events_types;
pub mod translate;

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use iii_sdk::IIIClient;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio_util_compat::CancellationToken;

use crate::config::Config;
use crate::events::emit;
use crate::functions::types::{extract_prompt, RunRequest};
use crate::iii_prompt::III_CONTEXT_PROMPT;
use crate::state::{load_session, save_session};
use crate::wire::{assistant_message, now_ms, ContentBlock, SessionRecord, Status};
use events_types::GrokEvent;

/// A minimal cancellation token (avoids pulling tokio-util just for this).
mod tokio_util_compat {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[derive(Clone, Default)]
    pub struct CancellationToken(Arc<AtomicBool>);
    impl CancellationToken {
        pub fn cancel(&self) {
            self.0.store(true, Ordering::SeqCst);
        }
        pub fn is_cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }
}

pub struct LiveRun {
    cancel: CancellationToken,
}

/// session_id -> live run. The single handle grok::stop targets.
static LIVE: Lazy<Mutex<HashMap<String, LiveRun>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub async fn is_live(session_id: &str) -> bool {
    LIVE.lock().await.contains_key(session_id)
}

/// Atomically reserve the single live slot for a session: returns false if one
/// is already active. Check + insert happen under one lock so two concurrent
/// runs for the same session can't both pass.
async fn try_reserve(session_id: &str, cancel: CancellationToken) -> bool {
    let mut live = LIVE.lock().await;
    if live.contains_key(session_id) {
        return false;
    }
    live.insert(session_id.to_string(), LiveRun { cancel });
    true
}

async fn release(session_id: &str) {
    LIVE.lock().await.remove(session_id);
}

pub async fn stop(session_id: &str) -> bool {
    if let Some(run) = LIVE.lock().await.get(session_id) {
        run.cancel.cancel();
        true
    } else {
        false
    }
}

fn grok_bin(cfg: &Config) -> String {
    if cfg.grok_executable.is_empty() {
        "grok".to_string()
    } else {
        cfg.grok_executable.clone()
    }
}

/// Run one Grok turn and return the result map. `iii_context_default` is the
/// config-level toggle; the payload field overrides it.
pub async fn run(iii: IIIClient, cfg: Arc<Config>, req: RunRequest) -> Value {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // One live run per session: reserve the slot atomically up front. The
    // in-process handle is what grok::stop targets, so a second concurrent run
    // would clobber it and race the record. Every early return past this point
    // must `release` the slot.
    let cancel = CancellationToken::default();
    if !try_reserve(&session_id, cancel.clone()).await {
        return json!({
            "session_id": session_id,
            "busy": true,
            "reason": "a run is already active for this session"
        });
    }

    let prompt = match extract_prompt(&req) {
        Ok(p) => p,
        Err(e) => {
            release(&session_id).await;
            return json!({ "session_id": session_id, "is_error": true, "result": e.to_string() });
        }
    };

    // A load failure is corruption/transient, not "no prior session": log it
    // and proceed fresh rather than silently masking it.
    let prior = match load_session(&iii, &session_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(session_id, error = %e, "load_session failed; proceeding without resume");
            None
        }
    };
    let prior_thread = prior.as_ref().and_then(|r| r.grok_thread_id.clone());
    let want_ctx = req.iii_context.unwrap_or(cfg.iii_context);

    let opts = args::resolve(
        &req,
        &cfg,
        prior.as_ref().map(|r| r.model.as_str()),
        prior.as_ref().map(|r| r.cwd.as_str()),
        if want_ctx {
            Some(III_CONTEXT_PROMPT)
        } else {
            None
        },
    );

    let argv = args::build_args(&prompt, &opts, prior_thread.as_deref());

    let mut record = prior.unwrap_or(SessionRecord {
        session_id: session_id.clone(),
        grok_thread_id: None,
        cwd: opts.cwd.clone(),
        model: opts.model.clone(),
        status: Status::Working,
        turns: 0,
        updated_at_ms: now_ms(),
    });
    record.cwd = opts.cwd.clone();
    record.model = opts.model.clone();

    // Spawn first; only persist `working` once the child + live handle exist,
    // so a spawn failure never leaves a stuck `working` record.
    let mut child = match Command::new(grok_bin(&cfg))
        .args(&argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            release(&session_id).await;
            return json!({ "session_id": session_id, "is_error": true, "stop_reason": "error", "result": format!("failed to spawn grok: {e}") });
        }
    };

    // Prompt is passed as an argv flag (`--single <prompt>`); close stdin so the
    // headless run never blocks waiting on it.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.shutdown().await;
    }

    // Drain stderr in the background so a chatty child can't fill the pipe
    // buffer and block. Tail is captured for the error path.
    let stderr_tail = drain_stderr(child.stderr.take());

    record.status = Status::Working;
    record.updated_at_ms = now_ms();
    let _ = save_session(&iii, &record).await;

    let mut outcome = stream_turn(&iii, &cfg, &session_id, &mut child, &cancel, &mut record).await;

    release(&session_id).await;

    // On error with no result text, surface the captured stderr tail.
    if outcome.is_error && outcome.result_text.is_empty() {
        if let Ok(tail) = stderr_tail.await {
            let tail = tail.trim();
            if !tail.is_empty() {
                outcome.result_text = tail.chars().take(2000).collect();
            }
        }
    }

    record.status = if outcome.is_error {
        Status::Error
    } else {
        Status::Done
    };
    record.turns += 1;
    record.updated_at_ms = now_ms();
    let _ = save_session(&iii, &record).await;

    // turn_end + agent_end on the translated stream.
    let final_msg = assistant_message(
        vec![ContentBlock::Text {
            text: outcome.result_text.clone(),
        }],
        &record.model,
        &outcome.stop_reason,
    );
    emit(
        &iii,
        &cfg.events_stream,
        &session_id,
        json!({ "type": "turn_end", "message": final_msg, "function_results": [] }),
    )
    .await;
    emit(
        &iii,
        &cfg.events_stream,
        &session_id,
        json!({ "type": "agent_end", "messages": [] }),
    )
    .await;

    json!({
        "session_id": session_id,
        "grok_thread_id": record.grok_thread_id,
        "result": outcome.result_text,
        "stop_reason": outcome.stop_reason,
        "is_error": outcome.is_error,
        "num_turns": record.turns,
    })
}

struct Outcome {
    result_text: String,
    stop_reason: String,
    is_error: bool,
}

async fn stream_turn(
    iii: &IIIClient,
    cfg: &Config,
    session_id: &str,
    child: &mut tokio::process::Child,
    cancel: &CancellationToken,
    record: &mut SessionRecord,
) -> Outcome {
    let mut outcome = Outcome {
        result_text: String::new(),
        stop_reason: "end".to_string(),
        is_error: false,
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            outcome.is_error = true;
            outcome.stop_reason = "error".to_string();
            outcome.result_text = "grok produced no stdout".to_string();
            return outcome;
        }
    };
    let mut lines = BufReader::new(stdout).lines();
    let mut state = translate::TurnState::new(record.model.clone());

    loop {
        if cancel.is_cancelled() {
            let _ = child.start_kill();
            outcome.stop_reason = "aborted".to_string();
            break;
        }
        let line = tokio::select! {
            l = lines.next_line() => l,
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => continue,
        };
        let line = match line {
            Ok(Some(l)) => l,
            Ok(None) => break, // stdout closed
            Err(e) => {
                outcome.is_error = true;
                outcome.stop_reason = "error".to_string();
                outcome.result_text = format!("stdout read error: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let raw: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(session_id, error = %e, "skipping non-JSON line from grok exec");
                continue;
            }
        };
        // verbatim onto the raw stream
        emit(iii, &cfg.raw_events_stream, session_id, raw.clone()).await;

        let event: GrokEvent = match serde_json::from_value(raw) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let had_thread = state.thread_id.is_some();
        let frames = translate::step(&mut state, event);
        // persist the thread id the first time it appears (enables resume)
        if !had_thread {
            if let Some(tid) = &state.thread_id {
                record.grok_thread_id = Some(tid.clone());
                let _ = save_session(iii, record).await;
            }
        }
        for frame in frames {
            emit(iii, &cfg.events_stream, session_id, frame).await;
        }
    }

    let exit = child.wait().await;
    // fold the accumulated turn state into the outcome (cancel sets aborted above)
    if !outcome.is_error && outcome.stop_reason != "aborted" {
        outcome.is_error = state.is_error;
        outcome.stop_reason = state.stop_reason;
        outcome.result_text = state.result_text;
    }
    // Backstop: a non-zero exit with no error event observed (e.g. the CLI
    // crashed) must not be reported as success. The aborted path is expected
    // to exit non-zero and is left as-is.
    if !outcome.is_error && outcome.stop_reason != "aborted" {
        let bad = matches!(&exit, Ok(s) if !s.success()) || exit.is_err();
        if bad {
            outcome.is_error = true;
            outcome.stop_reason = "error".to_string();
            if outcome.result_text.is_empty() {
                outcome.result_text = match &exit {
                    Ok(s) => format!("grok exited with {s}"),
                    Err(e) => format!("grok wait failed: {e}"),
                };
            }
        }
    }
    outcome
}

/// Drain a child's stderr in the background into a captured tail, so a chatty
/// process can't block on a full pipe. Returns a handle yielding the text.
fn drain_stderr(stderr: Option<tokio::process::ChildStderr>) -> tokio::task::JoinHandle<String> {
    tokio::spawn(async move {
        let mut buf = String::new();
        if let Some(mut e) = stderr {
            use tokio::io::AsyncReadExt;
            let _ = e.read_to_string(&mut buf).await;
        }
        buf
    })
}
