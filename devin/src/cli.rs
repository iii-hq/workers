//! The `devin::run` turn: spawn the local Devin CLI (the SWE-1.6 coding agent)
//! in non-interactive mode (`devin [extra args] --print -- "<prompt>"`), stream
//! its stdout verbatim onto `devin::events`, mirror a terminal AgentEvent frame
//! onto `agent::events` so the console renders the turn like any other agent
//! worker, and record the Devin session id it prints.
//!
//! `--print` output is plain text (not a structured event protocol), so we
//! treat each line as text. When the CLI opens a session it prints
//! `{"id":"devin-...","url":"..."}`, which we parse out to link the record.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use iii_sdk::IIIClient;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::events::emit;
use crate::functions::types::{extract_prompt, RunRequest};
use crate::iii_prompt::III_CONTEXT_PROMPT;
use crate::state::{load_session, save_session};
use crate::wire::{assistant_message, now_ms, ContentBlock, SessionRecord, Status};

/// A minimal cancellation token (avoids pulling tokio-util just for this).
mod cancel {
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
use cancel::CancellationToken;

pub struct LiveRun {
    cancel: CancellationToken,
}

/// session_id -> live run. The single handle devin::stop targets.
static LIVE: Lazy<Mutex<HashMap<String, LiveRun>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub async fn is_live(session_id: &str) -> bool {
    LIVE.lock().await.contains_key(session_id)
}

/// Atomically reserve the single live slot for a session: false if one is
/// already active. Check + insert happen under one lock so two concurrent runs
/// for the same session can't both pass.
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

/// Build the CLI argv for a headless run: configured extra args, then
/// `--print` (the CLI's non-interactive "print response and exit" mode), then
/// `--`, then the prompt. The bare `-- <prompt>` form starts an interactive
/// session and needs a TTY; `--print` is the scriptable mode a worker uses.
fn build_args(prompt: &str, cfg: &Config) -> Vec<String> {
    let mut a: Vec<String> = cfg.cli_extra_args.clone();
    a.push("--print".into());
    a.push("--".into());
    a.push(prompt.to_string());
    a
}

/// Compose the prompt for a turn: prepend the iii runtime context on the first
/// turn of a session when the context is enabled, so the agent discovers engine
/// functions through the iii CLI. Resumed turns already carry it in history.
pub fn compose_prompt(prompt: &str, want_ctx: bool, is_first_turn: bool) -> String {
    if want_ctx && is_first_turn {
        format!("{III_CONTEXT_PROMPT}\n\n{prompt}")
    } else {
        prompt.to_string()
    }
}

/// Pull the Devin session id and url out of the CLI's stdout. When `devin` opens
/// a session it prints `{"id":"devin-...","url":"..."}`; we scan from the last
/// line back for that object so the record links to the real Devin session and
/// the caller can address it via `devin::session::*`.
fn parse_session_ref(text: &str) -> (Option<String>, Option<String>) {
    for line in text.lines().rev() {
        let t = line.trim();
        if t.starts_with('{') {
            if let Ok(v) = serde_json::from_str::<Value>(t) {
                let id = v.get("id").and_then(Value::as_str).map(String::from);
                let url = v.get("url").and_then(Value::as_str).map(String::from);
                if id.is_some() || url.is_some() {
                    return (id, url);
                }
            }
        }
    }
    (None, None)
}

/// Run one `devin::run` turn and return the result map.
pub async fn run(iii: IIIClient, cfg: Arc<Config>, req: RunRequest) -> Value {
    let session_id = req
        .session_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

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

    let prior = match load_session(&iii, &session_id).await {
        Ok(p) => p,
        Err(e) => {
            release(&session_id).await;
            return json!({
                "session_id": session_id,
                "is_error": true,
                "stop_reason": "error",
                "result": format!("could not load prior session state: {e}")
            });
        }
    };

    let want_ctx = req.iii_context.unwrap_or(cfg.iii_context);
    let full_prompt = compose_prompt(&prompt, want_ctx, prior.is_none());
    let cwd = req.cwd.clone().unwrap_or_default();
    let argv = build_args(&full_prompt, &cfg);

    let mut record = prior.unwrap_or(SessionRecord {
        session_id: session_id.clone(),
        devin_session_id: None,
        cwd: cwd.clone(),
        model: String::new(),
        status: Status::Working,
        turns: 0,
        updated_at_ms: now_ms(),
    });
    record.cwd = cwd.clone();

    let mut command = Command::new(cfg.devin_bin());
    command
        .args(&argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !cwd.is_empty() {
        command.current_dir(&cwd);
    }

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            release(&session_id).await;
            return json!({
                "session_id": session_id,
                "is_error": true,
                "stop_reason": "error",
                "result": format!("failed to spawn devin CLI: {e}")
            });
        }
    };

    // One-shot: close stdin so a headless run never blocks waiting on it.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.shutdown().await;
    }

    let stderr_tail = drain_stderr(child.stderr.take());

    record.status = Status::Working;
    record.updated_at_ms = now_ms();
    let _ = save_session(&iii, &record).await;

    let mut outcome = stream_turn(&iii, &cfg, &session_id, &mut child, &cancel).await;

    release(&session_id).await;

    if outcome.is_error && outcome.result_text.is_empty() {
        if let Ok(tail) = stderr_tail.await {
            let tail = tail.trim();
            if !tail.is_empty() {
                outcome.result_text = tail.chars().take(2000).collect();
            }
        }
    }

    // Link the record to the Devin session the CLI opened (it prints its id on
    // stdout), so devin::status / devin::sessions::list point at the real
    // session and the caller can fetch its cloud detail via devin::session::get.
    let (parsed_id, session_url) = parse_session_ref(&outcome.result_text);
    if record.devin_session_id.is_none() {
        record.devin_session_id = parsed_id;
    }

    record.status = if outcome.is_error {
        Status::Error
    } else {
        Status::Done
    };
    record.turns += 1;
    record.updated_at_ms = now_ms();
    let _ = save_session(&iii, &record).await;

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
        "devin_session_id": record.devin_session_id,
        "url": session_url,
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
            outcome.result_text = "devin CLI produced no stdout".to_string();
            return outcome;
        }
    };
    let mut lines = BufReader::new(stdout).lines();

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
            Ok(None) => break,
            Err(e) => {
                outcome.is_error = true;
                outcome.stop_reason = "error".to_string();
                outcome.result_text = format!("stdout read error: {e}");
                break;
            }
        };
        // Mirror the raw line verbatim onto the raw stream.
        emit(
            iii,
            &cfg.raw_events_stream,
            session_id,
            json!({ "type": "stdout", "line": line }),
        )
        .await;
        // Accumulate into the turn result.
        if !outcome.result_text.is_empty() {
            outcome.result_text.push('\n');
        }
        outcome.result_text.push_str(&line);
    }

    let exit = child.wait().await;
    if outcome.stop_reason != "aborted" && !outcome.is_error {
        let bad = matches!(&exit, Ok(s) if !s.success()) || exit.is_err();
        if bad {
            outcome.is_error = true;
            outcome.stop_reason = "error".to_string();
            if outcome.result_text.is_empty() {
                outcome.result_text = match &exit {
                    Ok(s) => format!("devin CLI exited with {s}"),
                    Err(e) => format!("devin CLI wait failed: {e}"),
                };
            }
        }
    }
    outcome
}

const STDERR_TAIL_CAP: usize = 8192;

fn drain_stderr(stderr: Option<tokio::process::ChildStderr>) -> tokio::task::JoinHandle<String> {
    tokio::spawn(async move {
        let mut tail: Vec<u8> = Vec::new();
        if let Some(mut e) = stderr {
            use tokio::io::AsyncReadExt;
            let mut chunk = [0u8; 4096];
            while let Ok(n) = e.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&chunk[..n]);
                if tail.len() > STDERR_TAIL_CAP {
                    let drop = tail.len() - STDERR_TAIL_CAP;
                    tail.drain(..drop);
                }
            }
        }
        String::from_utf8_lossy(&tail).into_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iii_context_prepended_on_first_turn() {
        let out = compose_prompt("do X", true, true);
        assert!(out.starts_with(III_CONTEXT_PROMPT));
        assert!(out.trim_end().ends_with("do X"));
        assert!(out.contains("iii runtime"));
    }

    #[test]
    fn iii_context_absent_when_disabled() {
        assert_eq!(compose_prompt("do X", false, true), "do X");
    }

    #[test]
    fn iii_context_not_repeated_on_resume() {
        assert_eq!(compose_prompt("do X", true, false), "do X");
    }

    #[test]
    fn cli_argv_uses_print_mode() {
        let cfg = Config::default();
        let argv = build_args("build a thing", &cfg);
        // default permission mode, then --print (non-interactive), then -- prompt
        assert_eq!(
            argv,
            vec![
                "--permission-mode".to_string(),
                "dangerous".to_string(),
                "--print".to_string(),
                "--".to_string(),
                "build a thing".to_string(),
            ]
        );
    }

    #[test]
    fn parses_devin_session_ref_from_stdout() {
        let text = "starting...\n{\"id\":\"devin-abc123\",\"url\":\"https://app.devin.ai/sessions/abc123\"}";
        let (id, url) = parse_session_ref(text);
        assert_eq!(id, Some("devin-abc123".to_string()));
        assert_eq!(
            url,
            Some("https://app.devin.ai/sessions/abc123".to_string())
        );
        let (n1, n2) = parse_session_ref("no json here");
        assert!(n1.is_none() && n2.is_none());
    }
}
