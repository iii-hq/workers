//! `context-clm::assemble` — the CLM strategy behind the harness's
//! `context::assemble` contract (spec §5.2), and the per-session plumbing it
//! shares with the `session::deleted` cleanup.

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::config::ClmConfig;
use crate::configuration::ConfigCell;
use crate::controller::{
    cap_newest, evaluate, exec_body, gate_rejects, note_commands, pop_step, raw_count, real_edit,
    rollback_target, Cfg, Ctl, Decision, Nudge,
};
use crate::mirror::{mirror_rel, read_mirror, remove_mirror, write_mirror};
use crate::projection::{
    mirror_change, msg_hash, parse_mirror, render_mirror, strip_thinking, text_msg, MirrorChange,
};
use crate::prompt::{
    clm_section, compaction_hint, final_text, ledger_text, persistent_text, readout_text,
    receipt_applied, receipt_grew, receipt_no_change_exit, receipt_no_change_matched,
    receipt_rejected, receipt_still_over, tier_text, user_side_assistant, LedgerArgs,
    STRATEGY_STOP_REASON,
};
use crate::store::{CachedOut, SessionState, Store};
use crate::types::{AssembleRequest, AssembleResponse, Block, ModelRef, Msg};

pub const ASSEMBLE_ID: &str = "context-clm::assemble";
pub const ON_SESSION_DELETED_ID: &str = "context-clm::on-session-deleted";
/// Every step is delegated here for the model limits and the final fit.
pub const CONTEXT_ASSEMBLE_ID: &str = "context::assemble";
/// The spike's ceiling for the delegated hop (a long window is serialized twice).
const DELEGATE_TIMEOUT_MS: u64 = 300_000;

/// The `context::assemble` hop, behind a trait so the flow runs without an engine.
pub trait Delegate: Send + Sync {
    fn assemble(&self, payload: Value) -> impl Future<Output = Result<Value, String>> + Send;
}

/// Production delegate: `context::assemble` over the bus.
pub struct IiiDelegate(pub Arc<IIIClient>);

impl Delegate for IiiDelegate {
    async fn assemble(&self, payload: Value) -> Result<Value, String> {
        self.0
            .trigger(TriggerRequest {
                function_id: CONTEXT_ASSEMBLE_ID.into(),
                payload,
                action: None,
                timeout_ms: Some(DELEGATE_TIMEOUT_MS),
            })
            .await
            .map_err(|e| e.to_string())
    }
}

/// One async mutex per session id: steps of one session never interleave.
// NOTE: entries live until session::deleted; add idle eviction if the map ever shows up.
#[derive(Default)]
pub struct Locks(Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl Locks {
    pub fn get(&self, sid: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(sid.to_owned())
            .or_default()
            .clone()
    }

    /// Drop a session's entry. Call while holding that session's lock, once
    /// its files are gone: a step that arrives later gets a fresh mutex and
    /// finds nothing, and none can run between the cleanup and the forget.
    // NOTE: a step that fetched the old mutex earlier and is parked on it
    // still runs once the guard drops, possibly beside a step on a fresh
    // mutex. harness::delete-session-tree stops a session's turns before it
    // deletes the session, so none should be parked.
    pub fn forget(&self, sid: &str) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).remove(sid);
    }
}

pub struct Deps<D> {
    pub delegate: D,
    pub store: Store,
    pub config: ConfigCell<ClmConfig>,
    pub locks: Locks,
}

/// One harness step. Without an absolute filesystem root (or a session id to
/// key state by) the call is context-manager's, unchanged.
pub async fn assemble<D: Delegate>(
    deps: &Deps<D>,
    mut req: AssembleRequest,
) -> Result<AssembleResponse, Error> {
    let sid = req
        .options
        .session_id
        .clone()
        .or_else(|| req.options.lease_key.clone());
    let (Some(root), Some(sid)) = (req.options.filesystem_root.clone(), sid) else {
        return forward(&deps.delegate, &req).await;
    };
    if req.messages.is_empty() {
        return forward(&deps.delegate, &req).await;
    }
    if !Path::new(&root).is_absolute() {
        // A relative or `~` root would resolve against this worker's cwd, where
        // the model's execs never see the mirror.
        tracing::warn!(session_id = %sid, %root, "context-clm: filesystem root is not absolute; passing the step through");
        return forward(&deps.delegate, &req).await;
    }
    // Every store call below (load, quarantine, save, append_log) runs under
    // this guard: the store's temp file name is per session.
    let lock = deps.locks.get(&sid);
    let _serialized = lock.lock().await;
    let config = deps.config.read().await.clone();
    let mut st = match deps.store.load(&sid) {
        Ok(st) => st.unwrap_or_default(),
        // A torn write or a state from an incompatible worker version must not
        // wedge the session: set it aside and rebuild from the window (the
        // rebuilt path strips thinking). Other I/O errors stay fatal.
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            tracing::warn!(session_id = %sid, error = %e, "context-clm: unreadable session state set aside as .json.corrupt; rebuilding from the window");
            if let Err(e) = deps.store.quarantine(&sid) {
                tracing::warn!(session_id = %sid, error = %e, "context-clm: could not set the unreadable state aside; this step's save replaces it");
            }
            SessionState::default()
        }
        Err(e) => return Err(step_state_error(&sid, &e)),
    };

    // 1. Idempotency: a re-assembly of the step already served, over the
    // window it was served from, resends it unchanged. The harness's one-shot
    // overflow recovery re-assembles the same step with a larger request
    // overhead: that one is recomputed against the smaller room, unless the
    // step stopped the run. So is a redelivery whose window grew (the reply
    // and the results of calls that already ran, notices persisted between
    // attempts): resending the cached request would hide them.
    let key = req.options.turn_id.clone().zip(req.options.step);
    let overhead = req.options.request_overhead_tokens.unwrap_or(0);
    let same_step = key.is_some() && st.last_step == key;
    let same_window =
        || req.messages.len() == st.covered && window_hash(&req.messages) == st.prefix_hash;
    let replay = same_step && (st.stopped || (st.last_overhead == overhead && same_window()));
    let prev_root = st.root.clone();
    let line = match st.last_out.as_ref().filter(|_| replay) {
        Some(cached) => {
            req.messages = cached.messages.clone();
            req.system_prompt = Some(cached.system_prompt.clone());
            None
        }
        None => Some(run_step(&config, &sid, &root, &mut st, &mut req, same_step)),
    };
    let model = req.model.clone();
    let resp = finish(&deps.delegate, &mut req, st.stopped).await;
    let learned = match &resp {
        // A stop's request is never sent (preview_only, its count replaced).
        _ if st.stopped => false,
        Ok(r) => {
            learn(&mut st, &model, overhead, r) | note_rewrite(&mut st, &r.messages, line.is_some())
        }
        // An overflow's count is what is left after context-manager's own
        // reductions; a preview of the same request counts what CLM sent,
        // so the next step plans against the model's room.
        Err(e) if e.to_string().contains("context/overflow:") => {
            req.options
                .rest
                .insert("preview_only".into(), Value::Bool(true));
            match forward(&deps.delegate, &req).await {
                Ok(r) => learn(&mut st, &model, overhead, &r),
                Err(e) => {
                    tracing::warn!(session_id = %sid, error = %e, "context-clm: could not count the overflowing request; the model's room stays unknown");
                    false
                }
            }
        }
        Err(_) => false,
    };
    match line {
        Some(line) => commit(
            &deps.store,
            &sid,
            &root,
            prev_root.as_deref(),
            &mut st,
            config.step_log.then_some(line),
        )?,
        // A replay writes the state only when the delegated call taught something.
        None if learned => deps
            .store
            .save(&sid, &st)
            .map_err(|e| step_state_error(&sid, &e))?,
        None => {}
    }
    resp
}

/// Learn from a delegated call of the request CLM sent: the model's usable
/// room, and context-manager's count of that request (`initial_token_count`,
/// before its own reductions, less `overhead`, the request's) beside CLM's
/// raw count of it, and the tool schemas' share of that count
/// (`breakdown.tools_tokens`, 0 when absent). True when the state changed.
fn learn(st: &mut SessionState, model: &ModelRef, overhead: u64, resp: &AssembleResponse) -> bool {
    let tokens = resp
        .applied
        .get("initial_token_count")
        .and_then(Value::as_u64)
        .unwrap_or(resp.token_count)
        .saturating_sub(overhead);
    let tools = resp
        .rest
        .get("breakdown")
        .and_then(|b| b.get("tools_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let key = model_key(model);
    let learned = (resp.usable, tokens, st.raw_sent, tools);
    let changed = (
        st.last_usable,
        st.last_cm_tokens,
        st.last_cm_raw,
        st.last_cm_tools,
    ) != learned
        || st.last_usable_model != key;
    (
        st.last_usable,
        st.last_cm_tokens,
        st.last_cm_raw,
        st.last_cm_tools,
    ) = learned;
    st.last_usable_model = key;
    changed
}

/// context-manager returned what CLM sent with messages rewritten (an
/// emergency reduction, a result cap, an image marker). Its copies replace
/// CLM's own (the task, the ledger or the projection entry at that index), so
/// the next request carries what the provider saw and a rewrite repeated on
/// every request changes the prefix only once. The first changed message is
/// marked, so the next step strips thinking from there on. `mirror`: the
/// mirror text is written after this (`commit`), so render it again. True
/// when the state changed.
fn note_rewrite(st: &mut SessionState, returned: &[Msg], mirror: bool) -> bool {
    let p = prefix_len(st);
    let Some(sent) = &st.last_out else {
        return false;
    };
    let (mut first, mut adopted) = (None, false);
    for (i, (a, b)) in sent.messages.iter().zip(returned).enumerate() {
        if a.content == b.content {
            continue;
        }
        first.get_or_insert(i);
        let slot = match i {
            0 => st.raw0.as_mut(),
            1 if p == 2 => st.ledger.as_mut(),
            _ => i.checked_sub(p).and_then(|j| st.proj.get_mut(j)),
        };
        // Not the user-side copy of a trailing assistant turn: CLM holds the turn.
        if let Some(slot) = slot.filter(|m| *m == a) {
            slot.content.clone_from(&b.content);
            adopted = true;
        }
    }
    if adopted && mirror {
        if let Some(written) = &mut st.written {
            *written = render_mirror(&st.proj);
        }
    }
    let k = first.unwrap_or(returned.len());
    if !st.sent_hashes.get(k).is_some_and(|h| !h.is_empty()) {
        return adopted;
    }
    // An empty hash never matches.
    st.sent_hashes.truncate(k);
    st.sent_hashes.push(String::new());
    true
}

/// The model's room for this request in CLM's raw ruler: context-manager's
/// usable room, less this request's overhead and the tool schemas, converted
/// by the ratio of the two counts of the last request CLM sent to this model
/// (context-manager's without its tools). That factor covers what
/// context-manager counts and the ruler does not (JSON framing, ids,
/// signatures); the tools are a fixed term that does not grow with the
/// window, so they come off both sides. `None` until a delegated call to this
/// model taught it.
fn room_raw(st: &SessionState, model: &ModelRef, overhead: u64) -> Option<u64> {
    let counted = st.last_cm_tokens.saturating_sub(st.last_cm_tools);
    if st.last_usable == 0
        || counted == 0
        || st.last_cm_raw == 0
        || st.last_usable_model != model_key(model)
    {
        return None;
    }
    let usable = st
        .last_usable
        .saturating_sub(overhead)
        .saturating_sub(st.last_cm_tools);
    let room = u128::from(usable) * u128::from(st.last_cm_raw) / u128::from(counted);
    Some(u64::try_from(room).unwrap_or(u64::MAX))
}

fn model_key(m: &ModelRef) -> String {
    format!("{}/{}", m.provider.as_deref().unwrap_or_default(), m.id)
}

/// Ruler tokens of the longest nudge. A nudge is pushed after `tc` is
/// measured, so a limit clamped to the model's room keeps this headroom.
fn nudge_allowance(hint: &str) -> u64 {
    let longest = tier_text(1.0, u64::MAX, u64::MAX, hint)
        .len()
        .max(persistent_text(u64::MAX, u64::MAX, hint).len());
    longest as u64 / 4 + 8
}

/// Persist the step, then write its mirror (moved when the root changed) and
/// its step-log line. The state goes first: a failed save leaves the mirror
/// the saved state wrote, so the next step cannot read its own render as a
/// model edit. A failed mirror write never fails the step: the state is saved
/// again without it, and edits are read again once a write succeeds.
fn commit(
    store: &Store,
    sid: &str,
    root: &str,
    prev_root: Option<&str>,
    st: &mut SessionState,
    line: Option<Value>,
) -> Result<(), Error> {
    let save = |st: &SessionState| store.save(sid, st).map_err(|e| step_state_error(sid, &e));
    save(st)?;
    if let Some(text) = &st.written {
        if let Err(e) = write_mirror(Path::new(root), sid, text) {
            tracing::warn!(session_id = sid, error = %e, "context-clm: mirror write failed; edits are read again once a write succeeds");
            st.written = None;
            save(st)?;
        }
    }
    if let Some(prev) = prev_root.filter(|p| *p != root) {
        // A previous root that is gone is fine (remove_mirror skips it).
        if let Err(e) = remove_mirror(Path::new(prev), sid) {
            tracing::warn!(session_id = sid, root = %prev, error = %e, "context-clm: could not remove the mirror under the previous root");
        }
    }
    if let Some(line) = line {
        if let Err(e) = store.append_log(sid, &line) {
            tracing::warn!(session_id = sid, error = %e, "context-clm: step log append failed");
        }
    }
    Ok(())
}

/// Spec §5.2 steps 2–8: rewrite `req` into this step's outgoing projection
/// and the state `commit` persists; returns the step-log line. `recompute` =
/// the step already served, assembled again (it is not counted as a new step).
fn run_step(
    config: &ClmConfig,
    sid: &str,
    root: &str,
    st: &mut SessionState,
    req: &mut AssembleRequest,
    recompute: bool,
) -> Value {
    let raw = std::mem::take(&mut req.messages);
    let model = req.model.clone();
    let overhead = req.options.request_overhead_tokens.unwrap_or(0);

    // 2. A rewritten raw prefix (fork, external compaction, edited history)
    // rebuilds the projection from the window.
    if raw.len() < st.covered
        || (!st.prefix_hash.is_empty() && window_hash(&raw[..st.covered]) != st.prefix_hash)
    {
        let old = std::mem::take(st);
        *st = SessionState {
            ratio: old.ratio,
            raw_sent: old.raw_sent,
            sent_hashes: old.sent_hashes,
            root: old.root,
            stats: old.stats,
            budget: old.budget,
            last_usable: old.last_usable,
            last_usable_model: old.last_usable_model,
            last_cm_tokens: old.last_cm_tokens,
            last_cm_raw: old.last_cm_raw,
            last_cm_tools: old.last_cm_tools,
            section_limit: old.section_limit,
            ..SessionState::default()
        };
    }
    // A stop ended the previous turn (a replay of it was served above): this
    // step starts a new run with a fresh controller.
    if st.stopped {
        st.ctl = Ctl::default();
    }
    let is_new = st.raw0.is_none();
    if is_new {
        st.raw0 = Some(raw[0].clone());
        st.covered = 1;
    }
    // A projection built from a window the provider saw in another shape (a
    // reset, or a session that opted in mid-way): strip everything after the task.
    let rebuilt = is_new && raw.len() > 1;
    let mut fresh = raw[st.covered..].to_vec();
    if !recompute {
        st.stats.steps += 1;
    }

    // Model calls since the last request CLM sent: the first assistant turn
    // past `covered` answers it (none after a stop, whose request was
    // preview-only). Any later one answers a request CLM did not serve (a
    // turn on context-manager, a step passed through), so its thinking is
    // bound to a prefix CLM never sends: strip from there on. A rebuilt
    // projection is stripped whole below.
    // NOTE: a CLM request that got no reply (the turn failed) makes the next
    // foreign reply look like CLM's; only per-entry provenance would tell.
    let replies: Vec<usize> = (0..fresh.len())
        .filter(|&i| fresh[i].role == "assistant")
        .collect();
    let answered = usize::from(!st.stopped);
    let mut stripped = 0u64;
    if let Some(&i) = replies.get(answered) {
        if config.strip_thinking && !is_new {
            stripped += strip_all(&mut fresh[i..]);
        }
    }

    // Calibrate the ruler against the provider's count of the request CLM
    // sent (§5.6): only its direct reply, with no other model call after it.
    let usage = (answered == 1 && replies.len() == 1 && !is_new)
        .then(|| fresh[replies[0]].rest.get("usage"))
        .flatten();
    if let (Some(usage), true) = (usage, st.raw_sent > 0) {
        let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
        let r = (n("input") + n("cache_read") + n("cache_write")) as f64 / st.raw_sent as f64;
        // The range also keeps NaN/inf out of the persisted state.
        if (0.5..=3.0).contains(&r) {
            st.ratio = r;
        }
    }
    let ratio = st.ratio;

    let rel = mirror_rel(sid);
    let hint = compaction_hint(&rel);
    let mut cfg = Cfg::from_config(config, req.options.budget_tokens);
    let configured = cfg.limit();
    if let Some(room) = room_raw(st, &model, overhead) {
        // Never plan past the model: its room, less the nudge pushed after
        // `tc` is measured, in the controller's (calibrated) unit. The limit
        // becomes min(budget - reserve, that room); never 0, which would
        // switch every budget action off.
        let room = (room.saturating_sub(nudge_allowance(&hint)) as f64 * ratio) as u64;
        cfg.budget = cfg.budget.min(room.max(1).saturating_add(cfg.reserve));
    }
    let clamped = cfg.limit() < configured;
    st.budget = cfg.budget;
    let limit = cfg.limit();
    // The system prompt is part of the preserved-thinking prefix, so the
    // section states the session's first limit for good; a later clamp shows
    // in the readout, receipts and ledger only.
    if st.section_limit == 0 {
        st.section_limit = limit;
    }
    let sys = format!(
        "{}{}",
        req.system_prompt.as_deref().unwrap_or_default(),
        clm_section(&rel, st.section_limit)
    );
    let cal = |msgs: &[Msg]| (raw_count(&sys, msgs) as f64 * ratio) as u64;

    // The newest call and its result: this step's receipt and readout land there.
    let last_call = fresh.iter().rev().find(|m| m.is_assistant_call());
    let call_ids = last_call.map(Msg::call_ids).unwrap_or_default();
    let last_res = fresh.iter().rposition(|m| {
        m.role == "function_result"
            && m.function_call_id()
                .is_some_and(|id| call_ids.iter().any(|c| c == id))
    });
    let exec = last_res.map(|i| exec_body(&fresh[i].result_text(), cfg.observation_max_chars));
    let cmd_text = last_call
        .and_then(|m| m.content.iter().find(|b| b.kind == "function_call"))
        .and_then(|b| b.rest.get("arguments"))
        .map(Value::to_string)
        .unwrap_or_default();

    // 3. Read back the model's edit of the mirror (from the previous root when it moved).
    let read_root = st.root.clone().unwrap_or_else(|| root.to_owned());
    let mut notes: Vec<String> = Vec::new();
    let mut edit = json!({ "kind": "none" });
    if let Some(written) = st.written.clone() {
        // An unreadable mirror (a planted `.clm` link, a hard-linked file)
        // reads as missing: the projection is kept, the step goes on.
        let now = read_mirror(Path::new(&read_root), sid).unwrap_or_else(|e| {
            tracing::warn!(session_id = sid, error = %e, "context-clm: mirror read failed; keeping the projection");
            None
        });
        // Receipts and the gate use the readout's unit: calibrated tokens.
        let before = cal(&window(st, &st.proj, &[]));
        match mirror_change(now.as_deref(), &written) {
            change @ (MirrorChange::Edited | MirrorChange::Emptied) => {
                let kind = if matches!(change, MirrorChange::Emptied) {
                    "emptied"
                } else {
                    "edited"
                };
                let cand = parse_mirror(now.as_deref().unwrap_or_default(), &st.proj, &model);
                let after = cal(&window(st, &cand, &[]));
                if gate_rejects(before, after, cfg.fit, limit) {
                    notes.push(receipt_rejected(before, after, before, cfg.fit, limit));
                    st.stats.edits_rejected += 1;
                    edit = json!({ "kind": kind, "gate": "rejected", "before": before, "after": after });
                } else {
                    let prefix_tokens = cal(&window(st, &[], &[]));
                    let turns = 1 + prefix_len(st) + cand.len();
                    notes.push(if after > before {
                        receipt_grew(before, after)
                    } else if limit > 0 && after > limit {
                        receipt_still_over(before, after, limit)
                    } else {
                        receipt_applied(before, after, turns)
                    });
                    edit = json!({
                        "kind": kind,
                        "gate": "applied",
                        "before": before,
                        "after": after,
                        "real": real_edit(before, after, prefix_tokens),
                        "free": exec.as_ref().is_some_and(|e| e.silent && e.rc == Some(0)),
                    });
                    st.edit_mark = cand.len();
                    st.proj = cand;
                    st.ctl.consec = 0;
                    st.stats.edits_applied += 1;
                }
            }
            MirrorChange::Same if cmd_text.contains(".clm/") => {
                notes.push(
                    match exec.as_ref().and_then(|e| e.rc).filter(|rc| *rc != 0) {
                        Some(rc) => receipt_no_change_exit(rc, before),
                        None => receipt_no_change_matched(before),
                    },
                );
                edit = json!({ "kind": "no-change" });
            }
            MirrorChange::Missing => edit = json!({ "kind": "missing" }),
            MirrorChange::Same => {}
        }
    }

    // 4. Ingest: results become `<output>\n\n(exit_code=N)`, capped to the room left.
    // The room is in the controller's (calibrated) units and keeps space for
    // the readout and receipts the newest result carries; +2 absorbs the
    // ruler's per-piece rounding. Otherwise a capped output tips the step
    // over the limit and the rollback drops the output it was cut to keep.
    let tail =
        (readout_text(limit, limit, None).len() + notes.iter().map(String::len).sum::<usize>()) / 4
            + 2;
    let raw_limit = (limit as f64 / ratio) as i64 - tail as i64;
    let mut bodies: Vec<Option<String>> = fresh
        .iter()
        .map(|m| {
            (m.role == "function_result")
                .then(|| exec_body(&m.result_text(), cfg.observation_max_chars).body)
        })
        .collect();
    let mut readout_at = None;
    // Cap rooms of the result batch being ingested, its last result first.
    let mut rooms: Vec<i64> = Vec::new();
    for (i, m) in fresh.iter().enumerate() {
        if bodies[i].is_none() {
            st.proj.push(m.clone());
            continue;
        }
        if limit > 0 && rooms.is_empty() {
            // The results of one parallel call share the room left.
            let sizes: Vec<i64> = bodies[i..]
                .iter()
                .map_while(Option::as_ref)
                .map(|b| (b.len() as i64 + 1) / 4)
                .collect();
            let room = raw_limit - raw_count(&sys, &window(st, &st.proj, &[])) as i64;
            rooms = split_room(&sizes, room);
            rooms.reverse();
        }
        let body = bodies[i].take().unwrap_or_default();
        let body = if limit > 0 {
            cap_newest(&body, rooms.pop().unwrap_or(0), limit)
        } else {
            body
        };
        st.proj.push(with_text(m, &body));
        if Some(i) == last_res {
            readout_at = Some(st.proj.len() - 1);
        }
    }
    st.covered = raw.len();
    st.prefix_hash = window_hash(&raw);

    // 5. Readout and receipts on the newest result.
    if let Some(i) = readout_at.filter(|_| limit > 0) {
        let tokens = cal(&window(st, &st.proj, &[]));
        let text = format!(
            "{}{}{}",
            st.proj[i].result_text(),
            readout_text(tokens, limit, Some(rel.as_str())),
            notes.concat()
        );
        st.proj[i] = with_text(&st.proj[i], &text);
    }

    // 6. Budget controller: nudge, rollback (+ ledger), final turn, stop.
    let tc = cal(&window(st, &st.proj, &[]));
    let obs_max = st.proj[st.edit_mark.min(st.proj.len())..]
        .iter()
        .filter(|m| m.role == "function_result")
        .rev()
        .take(3)
        .map(|m| (raw_count("", std::slice::from_ref(m)) as f64 * ratio) as u64)
        .max()
        .unwrap_or(0);
    let mut dec = evaluate(&cfg, &mut st.ctl, tc, obs_max, st.proj.len(), &hint);
    let mut rolled = Value::Null;
    if matches!(dec, Decision::Rollback) {
        let target = rollback_target(&cfg, &st.ctl);
        // Never drop an operation the model has not seen yet: the trailing
        // user messages (nothing answered them, in this step or in a
        // recomputed replay) are held aside while rolling back.
        let mut keep = Vec::new();
        while st.proj.last().is_some_and(|m| m.role == "user") {
            keep.extend(st.proj.pop());
        }
        keep.reverse();
        let kept = keep.len();
        // The ledger that goes out with the request once `dropped` is gone.
        let ledger = |ctl: &Ctl, dropped: &[Msg], tokens_after: u64| {
            let mut ctl = ctl.clone();
            note_commands(&mut ctl, dropped);
            let text = ledger_text(&LedgerArgs {
                retry: ctl.n_retry,
                max_retry: cfg.max_retry,
                n_dropped: dropped.len(),
                n_rolled: ctl.n_rolled + dropped.len(),
                tokens_after,
                limit,
                hint: hint.clone(),
                cmds: ctl.cmds,
            });
            text_msg("user", &text, &model)
        };
        // Pop until the request fits the target with its ledger in place,
        // the ledger's figure at its widest, so the ledger sent never pushes
        // it back over.
        let mut dropped = Vec::new();
        loop {
            st.ledger = Some(ledger(&st.ctl, &dropped, u64::MAX));
            if st.proj.is_empty() || cal(&window(st, &st.proj, &keep)) <= target {
                break;
            }
            dropped.extend(pop_step(&mut st.proj));
        }
        st.proj.append(&mut keep);
        st.edit_mark = st.edit_mark.min(st.proj.len());
        // The figure the ledger states and the degrade check below count the
        // ledger: measured again after the figure's digits changed its length.
        let mut tokens_after = cal(&window(st, &st.proj, &[]));
        for _ in 0..2 {
            st.ledger = Some(ledger(&st.ctl, &dropped, tokens_after));
            tokens_after = cal(&window(st, &st.proj, &[]));
        }
        note_commands(&mut st.ctl, &dropped);
        st.ctl.n_rolled += dropped.len();
        st.stats.rollbacks += 1;
        rolled = json!({
            "dropped": dropped.len(),
            "target": target,
            "tokens_after": tokens_after,
            "kept_ops": kept,
        });
        if tokens_after > limit {
            // Only the unseen operation is left: degrade to the final-turn path.
            dec = if st.ctl.finalized {
                Decision::Stop
            } else {
                Decision::Final(final_text(tokens_after, limit))
            };
            st.ctl.finalized = true;
        }
    }
    // A clamped limit is the model's room: a final turn over it cannot be sent
    // (context-manager would throw `context/overflow`, which the harness
    // finalizes at once as an overflow), so the run stops here instead.
    if clamped && matches!(dec, Decision::Final(_)) {
        dec = Decision::Stop;
    }
    match &dec {
        // Over the configured limit only: the model's room still holds it.
        Decision::Final(text) => st.proj.push(text_msg("user", text, &model)),
        // A recompute that ingested nothing new pushed its nudge on the first run.
        Decision::Ok(Some(Nudge::Tier { text, .. } | Nudge::Persistent { text })) => {
            if !recompute || !fresh.is_empty() {
                st.proj.push(text_msg("user", text, &model));
            }
        }
        Decision::Stop => st.stats.stops += 1,
        Decision::Ok(None) | Decision::Rollback => {}
    }
    st.stopped = matches!(dec, Decision::Stop);
    st.stats.peak_tc = st.stats.peak_tc.max(tc);

    // 7. Preserved thinking: every block after the first changed message is
    // bound to a prefix the provider will not see again.
    let mut out = window(st, &st.proj, &[]);
    let first_changed = (0..st.sent_hashes.len())
        .find(|&i| out.get(i).map(msg_hash).as_ref() != Some(&st.sent_hashes[i]))
        .unwrap_or(st.sent_hashes.len());
    let changed = rebuilt || first_changed < st.sent_hashes.len();
    let k = if rebuilt { 1 } else { first_changed };
    if config.strip_thinking && changed {
        stripped += strip_all(out.get_mut(k..).unwrap_or_default());
        // Persist the stripped copies so a later step cannot resurrect them.
        if st.ledger.is_some() {
            st.ledger = Some(out[1].clone());
        }
        st.proj = out[prefix_len(st)..].to_vec();
    }
    st.stats.stripped += stripped;
    st.raw_sent = raw_count(&sys, &out);
    // A trailing assistant turn cannot be prefilled on Claude: deliver its
    // text user-side, on this request only.
    if let Some(last) = out.last_mut().filter(|m| m.role == "assistant") {
        let text: String = last
            .content
            .iter()
            .filter(|b| b.kind == "text")
            .filter_map(Block::text)
            .collect();
        *last = text_msg("user", &user_side_assistant(&text), &model);
    }
    st.sent_hashes = out.iter().map(msg_hash).collect();

    // 8. The mirror text for the next step; `commit` writes it after the
    // state (under this root; the previous root's copy is removed).
    st.written = Path::new(root).is_dir().then(|| render_mirror(&st.proj));
    st.root = Some(root.to_owned());
    st.last_step = req.options.turn_id.clone().zip(req.options.step);
    st.last_overhead = overhead;
    st.last_out = Some(CachedOut {
        messages: out.clone(),
        system_prompt: sys.clone(),
    });
    let action = match &dec {
        Decision::Ok(_) => "ok",
        Decision::Rollback => "rollback",
        Decision::Final(_) => "final",
        Decision::Stop => "stop",
    };
    let nudge = match &dec {
        Decision::Ok(Some(Nudge::Tier { tier, .. })) => json!(tier),
        Decision::Ok(Some(Nudge::Persistent { .. })) => json!("persistent"),
        _ => Value::Null,
    };
    let line = json!({
        "t": now_ms(),
        "turn_id": req.options.turn_id,
        "step": req.options.step,
        "raw": raw.len(),
        "proj": st.proj.len(),
        "tc": tc,
        "limit": limit,
        "ratio": st.ratio,
        "edit": edit,
        "action": action,
        "nudge": nudge,
        "rolled": rolled,
        "k": k,
        "stripped": stripped,
        "rebuilt": rebuilt,
    });
    req.messages = out;
    req.system_prompt = Some(sys);
    line
}

/// Delegate the outgoing request with compaction and pruning off (CLM owns
/// the context). `applied` always comes back an object; when the controller
/// stopped, the response is the deliberate stop the harness reports as
/// `harness.context_budget_stop` (spec §4.4). A stop is over the limit by
/// definition, so it goes as a `preview_only` assemble: otherwise
/// context-manager reduces it ungated and throws `context/overflow`, which
/// the harness would report as an overflow instead of the stop.
async fn finish<D: Delegate>(
    delegate: &D,
    req: &mut AssembleRequest,
    stopped: bool,
) -> Result<AssembleResponse, Error> {
    req.options.allow_prune = Some(false);
    req.options
        .rest
        .insert("allow_compaction".into(), Value::Bool(false));
    if stopped {
        req.options
            .rest
            .insert("preview_only".into(), Value::Bool(true));
    }
    let mut resp = forward(delegate, req).await?;
    if !resp.applied.is_object() {
        resp.applied = json!({});
    }
    if stopped {
        resp.token_count = resp.usable.saturating_add(1);
        resp.applied["strategy_stop"] = json!(STRATEGY_STOP_REASON);
    }
    Ok(resp)
}

async fn forward<D: Delegate>(
    delegate: &D,
    req: &AssembleRequest,
) -> Result<AssembleResponse, Error> {
    let payload = serde_json::to_value(req).map_err(|e| Error::Serde(e.to_string()))?;
    let value = delegate
        .assemble(payload)
        .await
        .map_err(|e| Error::Handler(format!("{CONTEXT_ASSEMBLE_ID}: {e}")))?;
    serde_json::from_value(value)
        .map_err(|e| Error::Serde(format!("{CONTEXT_ASSEMBLE_ID} response: {e}")))
}

/// Strip thinking from every message; returns how many changed.
fn strip_all(msgs: &mut [Msg]) -> u64 {
    let mut n = 0;
    for m in msgs {
        let bare = strip_thinking(m);
        if bare != *m {
            *m = bare;
            n += 1;
        }
    }
    n
}

/// Split `room` (ruler tokens) among a batch of results sized `sizes`: a
/// result that fits an even share is kept whole and leaves the rest of its
/// share to the larger ones, which split what is left evenly.
fn split_room(sizes: &[i64], room: i64) -> Vec<i64> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| sizes[i]);
    let (mut room, mut left) = (room, sizes.len() as i64);
    let mut rooms = vec![0; sizes.len()];
    for i in order {
        rooms[i] = room / left;
        room -= sizes[i].min(rooms[i]).max(0);
        left -= 1;
    }
    rooms
}

fn prefix_len(st: &SessionState) -> usize {
    usize::from(st.raw0.is_some()) + usize::from(st.ledger.is_some())
}

/// The fixed prefix (task, ledger) followed by `body` and `tail`.
// NOTE: clones per count, O(n²) in a deep rollback; count incrementally if a profile shows it.
pub(crate) fn window(st: &SessionState, body: &[Msg], tail: &[Msg]) -> Vec<Msg> {
    st.raw0
        .iter()
        .chain(&st.ledger)
        .chain(body)
        .chain(tail)
        .cloned()
        .collect()
}

fn with_text(m: &Msg, text: &str) -> Msg {
    Msg {
        content: vec![Block::new_text(text)],
        ..m.clone()
    }
}

/// Identity of a raw window prefix.
// NOTE: rehashes the whole raw window twice a step; keep a running digest if it shows.
fn window_hash(msgs: &[Msg]) -> String {
    let mut h = Sha256::new();
    for m in msgs {
        h.update(msg_hash(m));
    }
    format!("{:x}", h.finalize())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A session state that could not be read or written (`status`, cleanup).
pub(crate) fn state_error(sid: &str, e: &std::io::Error) -> Error {
    Error::Handler(state_text(sid, e))
}

/// The same failure on an assemble step, which fails the turn: name the way
/// back to the default strategy.
fn step_state_error(sid: &str, e: &std::io::Error) -> Error {
    Error::Handler(format!(
        "{} Remove `context_strategy` from the session metadata to use the default.",
        state_text(sid, e)
    ))
}

fn state_text(sid: &str, e: &std::io::Error) -> String {
    format!("context-clm: session state for {sid} could not be read or written: {e}.")
}

/// `session::deleted` payload (only the field we read).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SessionDeletedEvent {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionDeletedAck {
    pub ok: bool,
    /// Whether the session had CLM state to remove (an unreadable state counts).
    pub removed: bool,
}

/// Remove a deleted session's mirror, state and step log, under the session
/// lock so a step in flight cannot write them back. Cleanup is best effort
/// past the lock: a mirror that cannot be removed (a planted `.clm` link, an
/// unreadable state that names no root) is logged and the state and log are
/// still deleted, so the ack stays `ok`.
pub async fn on_session_deleted<D>(
    deps: &Deps<D>,
    event: SessionDeletedEvent,
) -> Result<SessionDeletedAck, Error> {
    let sid = event.session_id;
    let lock = deps.locks.get(&sid);
    let serialized = lock.lock().await;
    let loaded = deps.store.load(&sid);
    if let Err(e) = &loaded {
        tracing::warn!(session_id = %sid, error = %e, "context-clm: unreadable state; its mirror cannot be located");
    }
    let removed = !matches!(loaded, Ok(None));
    if let Some(root) = loaded.ok().flatten().and_then(|s| s.root) {
        if let Err(e) = remove_mirror(Path::new(&root), &sid) {
            tracing::warn!(session_id = %sid, %root, error = %e, "context-clm: mirror removal failed; state and step log are removed anyway");
        }
    }
    deps.store.remove(&sid).map_err(|e| state_error(&sid, &e))?;
    // Forget under the guard: a step arriving between a release and a later
    // forget would take the old mutex and could then overlap one on a fresh
    // mutex.
    deps.locks.forget(&sid);
    drop(serialized);
    Ok(SessionDeletedAck { ok: true, removed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PersistentNudge;
    use crate::mirror::safe_id;
    use crate::prompt::cap_note;
    use std::path::PathBuf;

    const SID: &str = "sess/1";

    struct Fake {
        calls: Mutex<Vec<Value>>,
        /// The `usable` room context-manager reports for the model.
        usable: u64,
        /// context-manager's count per token of CLM's raw ruler, plus the
        /// request overhead (0 = a fixed count of 10).
        scale: u64,
        /// Fail with context-manager's overflow past `usable` unless
        /// `preview_only` is set, as context-manager does.
        strict: bool,
        /// Return this message rewritten (as an emergency reduction would).
        rewrite: Option<usize>,
        /// Tokens of the tool schemas, a fixed term of context-manager's
        /// count (`breakdown.tools_tokens`) the ruler never sees.
        tools: u64,
    }

    impl Default for Fake {
        fn default() -> Self {
            Fake {
                calls: Mutex::default(),
                usable: 1_000_000,
                scale: 0,
                strict: false,
                rewrite: None,
                tools: 0,
            }
        }
    }

    impl Fake {
        fn last(&self) -> Value {
            self.calls
                .lock()
                .unwrap()
                .last()
                .cloned()
                .expect("delegate called")
        }
        fn count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl Delegate for Fake {
        async fn assemble(&self, payload: Value) -> Result<Value, String> {
            self.calls.lock().unwrap().push(payload.clone());
            let system = payload["system_prompt"].as_str().unwrap_or_default();
            let msgs: Vec<Msg> = serde_json::from_value(payload["messages"].clone()).unwrap();
            let token_count = match self.scale {
                0 => 10,
                k => {
                    let overhead = payload["options"]["request_overhead_tokens"]
                        .as_u64()
                        .unwrap_or(0);
                    k * raw_count(system, &msgs) + self.tools + overhead
                }
            };
            let preview = payload["options"]["preview_only"] == true;
            if self.strict && token_count > self.usable && !preview {
                // Reported after its emergency reduction: what is left, not
                // what was sent.
                return Err(format!(
                    "context/overflow: assembled context requires {} tokens but usable budget is {}",
                    self.usable + 1,
                    self.usable
                ));
            }
            let mut messages = payload["messages"].clone();
            if let Some(i) = self.rewrite {
                messages[i]["content"] = json!([{ "type": "text", "text": "(output pruned)" }]);
            }
            Ok(json!({
                "system_prompt": system,
                "messages": messages,
                "token_count": token_count,
                "usable": self.usable,
                "effective_max_output_tokens": 4096,
                "applied": { "compacted": false, "initial_token_count": token_count },
                "breakdown": { "tools_tokens": self.tools },
            }))
        }
    }

    fn deps(dir: &Path, cfg: ClmConfig) -> Deps<Fake> {
        Deps {
            delegate: Fake::default(),
            store: Store::new(dir.join("state")),
            config: crate::configuration::new_cell(cfg),
            locks: Locks::default(),
        }
    }

    fn setup(cfg: ClmConfig) -> (tempfile::TempDir, PathBuf, Deps<Fake>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let d = deps(dir.path(), cfg);
        (dir, root, d)
    }

    fn request(raw: &[Value], root: Option<&Path>, step: u64) -> AssembleRequest {
        serde_json::from_value(json!({
            "messages": raw,
            "model": { "id": "claude-opus-5-5", "provider": "anthropic" },
            "system_prompt": "BASE",
            "tools": [],
            "options": {
                "lease_key": SID,
                "session_id": SID,
                "turn_id": "turn-1",
                "step": step,
                "filesystem_root": root.map(|p| p.to_string_lossy().into_owned()),
                "request_overhead_tokens": 0,
            },
        }))
        .unwrap()
    }

    fn user(text: &str) -> Value {
        json!({ "role": "user", "content": [{ "type": "text", "text": text }], "timestamp": 1 })
    }

    fn call(id: &str, cmd: &str) -> Value {
        json!({
            "role": "assistant",
            "content": [
                { "type": "thinking", "text": "plan the next command", "signature": "sig" },
                { "type": "function_call", "id": id, "function_id": "shell::exec",
                  "arguments": { "command": "bash", "args": ["-c", cmd] } },
            ],
            "stop_reason": "function_call",
            "model": "claude-opus-5-5",
            "provider": "anthropic",
            "timestamp": 2,
        })
    }

    fn result(id: &str, stdout: &str) -> Value {
        let text = json!({ "exit_code": 0, "stdout": stdout, "stderr": "" }).to_string();
        json!({
            "role": "function_result",
            "function_call_id": id,
            "function_id": "shell::exec",
            "content": [{ "type": "text", "text": text }],
            "details": {},
            "is_error": false,
            "timestamp": 3,
        })
    }

    fn reply(text: &str) -> Value {
        json!({
            "role": "assistant",
            "content": [{ "type": "text", "text": text }],
            "stop_reason": "end",
            "model": "claude-opus-5-5",
            "provider": "anthropic",
            "timestamp": 4,
        })
    }

    fn messages(payload: &Value) -> Vec<Value> {
        payload["messages"].as_array().cloned().expect("messages")
    }

    fn has_thinking(m: &Value) -> bool {
        m["content"]
            .as_array()
            .is_some_and(|c| c.iter().any(|b| b["type"] == "thinking"))
    }

    fn has_call(m: &Value, id: &str) -> bool {
        m["content"].as_array().is_some_and(|c| {
            c.iter()
                .any(|b| b["type"] == "function_call" && b["id"] == id)
        })
    }

    /// The system section a session keeps: the limit of its first CLM step.
    fn section(limit: u64) -> String {
        format!("BASE{}", clm_section(&mirror_rel(SID), limit))
    }

    #[tokio::test]
    async fn passthrough_without_filesystem_root() {
        let dir = tempfile::tempdir().unwrap();
        let d = deps(dir.path(), ClmConfig::default());
        // Compared with the payload as the harness sent it, not with a
        // re-serialized request: what the types do not model must survive.
        let payload = json!({
            "messages": [{
                "role": "user",
                "content": [{ "type": "text", "text": "task" }],
                "parts": [{ "kind": "attachment", "id": "a1" }],
                "timestamp": 1,
            }],
            "model": { "id": "claude-opus-5-5", "provider": "anthropic" },
            "system_prompt": "BASE",
            "tools": [],
            "options": { "session_id": SID, "step": 1, "future_option": { "on": true } },
        });
        let req = serde_json::from_value(payload.clone()).unwrap();
        let resp = assemble(&d, req).await.unwrap();
        assert_eq!(d.delegate.last(), payload);
        assert_eq!(resp.token_count, 10);
        assert_eq!(
            resp.applied,
            json!({ "compacted": false, "initial_token_count": 10 })
        );
        assert!(d.store.load(SID).unwrap().is_none());
    }

    /// A delegate that always fails with one text.
    struct Failing(&'static str);

    impl Delegate for Failing {
        async fn assemble(&self, _: Value) -> Result<Value, String> {
            Err(self.0.to_owned())
        }
    }

    #[tokio::test]
    async fn delegate_errors_keep_the_text_the_harness_classifies() {
        // The harness's assemble_error: an overflow anywhere in the text is
        // `context_overflow`; only a text that *starts* with the strategy's
        // own not-found is `context_strategy_unavailable`, so a nested one
        // (context-manager down behind CLM) must carry a prefix.
        let overflow =
            "context/overflow: assembled context requires 9 tokens but usable budget is 8";
        let missing = "remote error (function_not_found): Function context::assemble not found";
        for root in [false, true] {
            for text in [overflow, missing] {
                let dir = tempfile::tempdir().unwrap();
                let d = Deps {
                    delegate: Failing(text),
                    store: Store::new(dir.path().join("state")),
                    config: crate::configuration::new_cell(ClmConfig::default()),
                    locks: Locks::default(),
                };
                let req = request(&[user("task")], root.then_some(dir.path()), 1);
                let err = assemble(&d, req).await.unwrap_err();
                let Error::Handler(got) = &err else {
                    panic!("not a handler error: {err:?}");
                };
                assert_eq!(got, &format!("{CONTEXT_ASSEMBLE_ID}: {text}"));
            }
        }
    }

    #[test]
    fn locks_hand_out_one_mutex_per_session_until_forgotten() {
        let locks = Locks::default();
        let a = locks.get("a");
        assert!(Arc::ptr_eq(&a, &locks.get("a")));
        assert!(!Arc::ptr_eq(&a, &locks.get("b")));
        locks.forget("a");
        assert!(!Arc::ptr_eq(&a, &locks.get("a")));
        locks.forget("never-seen");
    }

    #[tokio::test]
    async fn a_state_that_cannot_be_read_fails_the_step_with_the_way_out() {
        let (dir, root, d) = setup(ClmConfig::default());
        let state = dir.path().join(format!("state/{}.json", safe_id(SID)));
        std::fs::create_dir_all(state).unwrap();
        let req = request(&[user("task")], Some(&root), 1);
        let err = assemble(&d, req).await.unwrap_err().to_string();
        assert!(err.contains(&format!("session state for {SID}")), "{err}");
        assert!(err.contains("Remove `context_strategy`"), "{err}");
    }

    #[tokio::test]
    async fn relative_root_passes_through() {
        let dir = tempfile::tempdir().unwrap();
        let d = deps(dir.path(), ClmConfig::default());
        for root in ["work/project", "~/project"] {
            let req = request(&[user("task")], Some(Path::new(root)), 1);
            let expected = serde_json::to_value(&req).unwrap();
            assemble(&d, req).await.unwrap();
            assert_eq!(d.delegate.last(), expected, "{root}");
        }
        assert!(d.store.load(SID).unwrap().is_none());
    }

    #[tokio::test]
    async fn first_step_sends_the_task_with_the_clm_section_and_writes_the_mirror() {
        let (_dir, root, d) = setup(ClmConfig::default());
        let raw = vec![user("task")];
        let resp = assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        let sent = d.delegate.last();
        assert_eq!(sent["messages"], json!(raw));
        let limit = Cfg::from_config(&ClmConfig::default(), None).limit();
        assert_eq!(sent["system_prompt"], section(limit));
        assert_eq!(sent["options"]["allow_compaction"], false);
        assert_eq!(sent["options"]["allow_prune"], false);
        assert_eq!(resp.token_count, 10);
        assert!(read_mirror(&root, SID).unwrap().is_some());
        assert!(root.join(".clm/.gitignore").is_file());
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!((st.covered, st.stats.steps), (1, 1));
        assert_eq!((st.budget, st.last_usable), (65_536, 1_000_000));
        assert_eq!(st.section_limit, limit);
    }

    #[tokio::test]
    async fn append_only_steps_keep_every_thinking_block() {
        let (_dir, root, d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        raw.extend([call("c2", "cat a.txt"), result("c2", "hello")]);
        assemble(&d, request(&raw, Some(&root), 3)).await.unwrap();
        let sent = messages(&d.delegate.last());
        assert_eq!(sent.len(), 5, "{sent:#?}");
        assert!(has_thinking(&sent[1]) && has_thinking(&sent[3]));
        let readout = sent[2]["content"][0]["text"].as_str().unwrap();
        assert!(readout.contains("[context: ~"), "{readout}");
        assert_eq!(d.store.load(SID).unwrap().unwrap().stats.stripped, 0);
    }

    #[tokio::test]
    async fn an_applied_edit_strips_thinking_from_the_first_changed_message_on() {
        let (_dir, root, d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        raw.extend([
            call("c2", "cat big.log"),
            result("c2", &"log line\n".repeat(400)),
        ]);
        assemble(&d, request(&raw, Some(&root), 3)).await.unwrap();

        // The model collapses turn 4 (the big result) into a one-line note.
        let mirror = root.join(mirror_rel(SID));
        let text = std::fs::read_to_string(&mirror).unwrap();
        let at = text.find("[[CTX_TURN 4 ").expect("turn 4 header");
        let header_end = at + text[at..].find('\n').unwrap();
        let edited = format!("{}\nbig.log: 400 identical lines", &text[..header_end]);
        std::fs::write(&mirror, edited).unwrap();

        raw.extend([call("c3", "echo done"), result("c3", "done")]);
        assemble(&d, request(&raw, Some(&root), 4)).await.unwrap();
        let sent = messages(&d.delegate.last());
        assert!(
            has_thinking(&sent[1]),
            "the turn before the edit keeps its thinking"
        );
        assert!(sent[3..].iter().all(|m| !has_thinking(m)), "{sent:#?}");
        let c3 = sent
            .iter()
            .find(|m| has_call(m, "c3"))
            .expect("c3 call kept");
        assert_eq!(c3["role"], "assistant");
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!(st.stats.edits_applied, 1);
        assert!(st.stats.stripped >= 1);
    }

    #[tokio::test]
    async fn a_replayed_step_resends_the_cached_request_without_the_controller() {
        let (_dir, root, d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        let first = d.delegate.last();
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        let again = d.delegate.last();
        assert_eq!(again["messages"], first["messages"]);
        assert_eq!(again["system_prompt"], first["system_prompt"]);
        assert_eq!(d.delegate.count(), 3);
        assert_eq!(d.store.load(SID).unwrap().unwrap().stats.steps, 2);
    }

    #[tokio::test]
    async fn a_replayed_step_whose_window_grew_is_recomputed_with_the_new_messages() {
        let (_dir, root, d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        // The attempt died after the reply's call ran: the redelivered step
        // carries the reply and its result.
        raw.extend([call("c2", "touch b.txt"), result("c2", "")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        let sent = messages(&d.delegate.last());
        assert!(sent.iter().any(|m| has_call(m, "c2")), "{sent:#?}");
        assert!(sent.iter().any(|m| m["function_call_id"] == "c2"));
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!(
            (st.covered, st.stats.steps),
            (5, 2),
            "a recompute is not a new step"
        );
    }

    #[tokio::test]
    async fn calibration_ignores_usage_from_requests_clm_did_not_send() {
        let (_dir, root, d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        let raw_sent = d.store.load(SID).unwrap().unwrap().raw_sent;
        // CLM's turn ends; a turn on context-manager follows, whose request
        // was three times the one CLM last sent.
        let mut c2 = call("c2", "cat a.txt");
        c2["usage"] = json!({ "input": 3 * raw_sent });
        raw.extend([
            reply("done"),
            user("plain turn"),
            c2,
            result("c2", "hello"),
            reply("ok"),
            user("with clm again"),
        ]);
        let mut next = request(&raw, Some(&root), 1);
        next.options.turn_id = Some("turn-2".into());
        assemble(&d, next).await.unwrap();
        assert_eq!(d.store.load(SID).unwrap().unwrap().ratio, 1.0);
    }

    #[tokio::test]
    async fn turns_generated_while_on_context_manager_lose_their_thinking() {
        let (_dir, root, d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        // CLM's reply ends turn 1; turn 2 runs on context-manager, so its
        // thinking is signed over a prefix CLM never sends.
        let mut done = reply("done");
        done["content"] = json!([
            { "type": "thinking", "text": "wrap up", "signature": "sig" },
            { "type": "text", "text": "done" },
        ]);
        let mut six = done.clone();
        six["content"][1]["text"] = json!("six bytes");
        raw.extend([
            done,
            user("plain turn"),
            call("c2", "wc a.txt"),
            result("c2", "1 1 6"),
            six,
            user("with clm again"),
        ]);
        let mut next = request(&raw, Some(&root), 1);
        next.options.turn_id = Some("turn-2".into());
        assemble(&d, next).await.unwrap();
        let sent = messages(&d.delegate.last());
        assert_eq!(sent.len(), 9, "{sent:#?}");
        assert!(
            has_thinking(&sent[1]) && has_thinking(&sent[3]),
            "{sent:#?}"
        );
        assert!(sent[4..].iter().all(|m| !has_thinking(m)), "{sent:#?}");
        assert_eq!(d.store.load(SID).unwrap().unwrap().stats.stripped, 2);
    }

    #[tokio::test]
    async fn a_corrupt_state_file_rebuilds_instead_of_failing() {
        let (dir, root, d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        // A torn write (or a state from an incompatible worker version).
        let state = dir.path().join(format!("state/{}.json", safe_id(SID)));
        std::fs::write(&state, r#"{"covered": 3, "proj": [{"role": "us"#).unwrap();

        raw.extend([call("c2", "cat a.txt"), result("c2", "hello")]);
        let resp = assemble(&d, request(&raw, Some(&root), 3)).await.unwrap();
        assert_eq!(resp.token_count, 10);
        let sent = messages(&d.delegate.last());
        assert_eq!(sent.len(), 5, "rebuilt from the whole window: {sent:#?}");
        assert!(
            sent[1..].iter().all(|m| !has_thinking(m)),
            "the rebuilt path strips thinking after the task"
        );
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!((st.covered, st.stats.steps), (5, 1));
        let aside = dir
            .path()
            .join(format!("state/{}.json.corrupt", safe_id(SID)));
        assert!(aside.is_file());
        d.store.remove(SID).unwrap();
        assert!(!aside.exists(), "the copy set aside goes with the session");
    }

    #[tokio::test]
    async fn the_limit_never_exceeds_the_usable_room_the_delegate_reported() {
        let cfg = ClmConfig {
            observation_max_chars: 100_000,
            ..ClmConfig::default()
        };
        let (_dir, root, mut d) = setup(cfg);
        (d.delegate.usable, d.delegate.scale) = (5_000, 1);
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        let first = d.delegate.last();
        assert_eq!(first["system_prompt"], section(63_488));
        assert_eq!(d.store.load(SID).unwrap().unwrap().last_usable, 5_000);

        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        let sent = d.delegate.last();
        // The system section is part of the preserved-thinking prefix: it
        // keeps the first step's limit; the clamp shows in the readout only.
        assert_eq!(sent["system_prompt"], first["system_prompt"]);
        let readout = messages(&sent)[2]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned();
        // The model's room, less the headroom for a nudge pushed after measuring.
        let limit = 5_000 - nudge_allowance(&compaction_hint(&mirror_rel(SID)));
        assert!(readout.contains(&format!("/{limit} tokens")), "{readout}");
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!((st.budget, st.section_limit), (limit + 2_048, 63_488));

        // ~6000 tokens of output fit the configured 63488-token limit, not the model.
        raw.extend([call("c2", "cat big.log"), result("c2", &"y".repeat(24_000))]);
        assemble(&d, request(&raw, Some(&root), 3)).await.unwrap();
        let sent = messages(&d.delegate.last());
        assert!(
            sent.iter().all(|m| m.to_string().len() < 24_000),
            "{sent:#?}"
        );
        // The newest result survives, cut to the clamped room under its cap note.
        let newest = sent
            .iter()
            .find(|m| m["function_call_id"] == "c2")
            .expect("the newest result is kept");
        let text = newest["content"][0]["text"].as_str().unwrap();
        let note = cap_note(0, limit);
        let note = &note[note.find(" characters").unwrap()..];
        assert!(text.contains(note), "{text}");
    }

    #[tokio::test]
    async fn the_clamp_converts_context_managers_room_into_the_rulers_units() {
        let mut limits = Vec::new();
        for scale in [1, 2] {
            let (_dir, root, mut d) = setup(ClmConfig::default());
            (d.delegate.usable, d.delegate.scale) = (20_000, scale);
            d.delegate.tools = 5_000;
            let mut raw = vec![user("task")];
            assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
            raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
            assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
            limits.push(d.store.load(SID).unwrap().unwrap().budget - 2_048);
        }
        // The tool schemas take a fixed 5000 off the room; context-manager
        // counts twice the ruler for the rest: the room in ruler tokens halves.
        let a = nudge_allowance(&compaction_hint(&mirror_rel(SID)));
        assert_eq!(limits, [15_000 - a, 7_500 - a]);
    }

    #[tokio::test]
    async fn a_model_switch_plans_without_the_previous_models_room() {
        let (_dir, root, mut d) = setup(ClmConfig::default());
        (d.delegate.usable, d.delegate.scale) = (5_000, 1);
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        assert!(d.store.load(SID).unwrap().unwrap().budget < 65_536);

        // Another model: its room is unknown until context-manager reports it.
        d.delegate.usable = 150_000;
        raw.extend([call("c2", "pwd"), result("c2", "/work")]);
        let mut next = request(&raw, Some(&root), 3);
        next.model.id = "claude-sonnet-5-5".into();
        assemble(&d, next).await.unwrap();
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!(st.budget, 65_536, "the configured budget, unclamped");
        assert_eq!(st.last_usable, 150_000);
        assert_eq!(st.last_usable_model, "anthropic/claude-sonnet-5-5");
    }

    #[tokio::test]
    async fn an_overflow_teaches_the_room_so_the_next_turn_fits() {
        let cfg = ClmConfig {
            observation_max_chars: 100_000,
            ..ClmConfig::default()
        };
        let (_dir, root, mut d) = setup(cfg);
        (d.delegate.usable, d.delegate.scale, d.delegate.strict) = (20_000, 1, true);
        // The tool schemas: a fixed term the overflowing count carries.
        d.delegate.tools = 5_000;
        // ~40k tokens from before the session opted in: under the configured
        // limit, over the model, whose room CLM has not learned yet. Small
        // steps, so the rollback lands close to its target.
        let mut raw = vec![user("task")];
        for i in 1..=40 {
            let id = format!("c{i}");
            raw.extend([call(&id, "cat part"), result(&id, &"y".repeat(4_000))]);
        }
        let err = assemble(&d, request(&raw, Some(&root), 1))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("context/overflow"), "{err}");
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!(st.last_usable, 20_000);

        raw.push(user("next"));
        let mut next = request(&raw, Some(&root), 1);
        next.options.turn_id = Some("turn-2".into());
        assemble(&d, next).await.unwrap();
        assert_eq!(d.store.load(SID).unwrap().unwrap().stats.rollbacks, 1);
    }

    #[tokio::test]
    async fn the_nudge_pushed_after_measuring_still_fits_a_clamped_model() {
        let cfg = ClmConfig {
            observation_max_chars: 100_000,
            ..ClmConfig::default()
        };
        let (_dir, root, mut d) = setup(cfg);
        (d.delegate.usable, d.delegate.scale, d.delegate.strict) = (7_000, 1, true);
        d.delegate.tools = 1_000;
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        // The result is capped to fill the room, and the adaptive nudge fires on top.
        raw.extend([call("c1", "cat big.log"), result("c1", &"y".repeat(40_000))]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        let sent = messages(&d.delegate.last());
        let nudge = sent.last().unwrap()["content"][0]["text"].as_str().unwrap();
        assert!(nudge.starts_with("[context warning: act now]"), "{nudge}");
    }

    #[tokio::test]
    async fn a_result_context_manager_rewrote_strips_thinking_from_there_on() {
        let (_dir, root, mut d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        // context-manager returned the c1 result rewritten.
        d.delegate.rewrite = Some(2);
        raw.extend([call("c2", "cat a.txt"), result("c2", "hello")]);
        assemble(&d, request(&raw, Some(&root), 3)).await.unwrap();
        d.delegate.rewrite = None;
        raw.extend([call("c3", "wc a.txt"), result("c3", "1 1 6")]);
        assemble(&d, request(&raw, Some(&root), 4)).await.unwrap();
        let sent = messages(&d.delegate.last());
        assert!(has_thinking(&sent[1]), "before the rewrite: {sent:#?}");
        assert!(sent[2..].iter().all(|m| !has_thinking(m)), "{sent:#?}");
    }

    #[tokio::test]
    async fn a_rewrite_context_manager_repeats_is_a_change_only_once() {
        let (_dir, root, mut d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        // From here on context-manager rewrites the c1 result on every
        // request (as `normalize_media` replaces an image with a marker).
        d.delegate.rewrite = Some(2);
        for (step, id) in [(3, "c2"), (4, "c3"), (5, "c4")] {
            raw.extend([call(id, "cat a.txt"), result(id, "hello")]);
            assemble(&d, request(&raw, Some(&root), step))
                .await
                .unwrap();
            // The mirror shows the copy CLM now holds, so it reads as unedited.
            let mirror = read_mirror(&root, SID).unwrap().unwrap();
            assert!(mirror.contains("(output pruned)"), "step {step}: {mirror}");
        }
        let sent = messages(&d.delegate.last());
        // CLM sends the copy the provider saw...
        assert_eq!(sent[2]["content"][0]["text"], "(output pruned)");
        // ...so only the first rewrite (step 3) stripped, once, at step 4:
        // c4, the reply to the second rewritten request, keeps its thinking.
        assert!(has_thinking(&sent[1]), "{sent:#?}");
        assert!(sent[3..7].iter().all(|m| !has_thinking(m)), "{sent:#?}");
        assert!(
            has_call(&sent[7], "c4") && has_thinking(&sent[7]),
            "{sent:#?}"
        );
    }

    #[tokio::test]
    async fn a_capped_result_fits_the_calibrated_limit_and_is_not_rolled_back() {
        let cfg = ClmConfig {
            budget_tokens: 8_192,
            observation_max_chars: 100_000,
            ..ClmConfig::default()
        };
        let (_dir, root, d) = setup(cfg);
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        // The provider counted twice the ruler for the request step 2 sent.
        let raw_sent = d.store.load(SID).unwrap().unwrap().raw_sent;
        let mut c2 = call("c2", "cat big.log");
        c2["usage"] = json!({ "input": 2 * raw_sent });
        raw.extend([c2, result("c2", &"y".repeat(40_000))]);
        assemble(&d, request(&raw, Some(&root), 3)).await.unwrap();

        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!((st.ratio, st.stats.rollbacks), (2.0, 0));
        let sent = messages(&d.delegate.last());
        let newest = sent
            .iter()
            .find(|m| m["function_call_id"] == "c2")
            .expect("the capped result is kept");
        let text = newest["content"][0]["text"].as_str().unwrap();
        let note = cap_note(0, 6_144);
        assert!(
            text.contains(&note[note.find(" characters").unwrap()..]),
            "{text}"
        );
    }

    #[tokio::test]
    async fn a_replay_with_a_larger_request_overhead_is_recomputed() {
        let cfg = ClmConfig {
            nudge_ratios: vec![],
            persistent_nudge: PersistentNudge::Off,
            observation_max_chars: 100_000,
            ..ClmConfig::default()
        };
        let (_dir, root, mut d) = setup(cfg);
        (d.delegate.usable, d.delegate.scale) = (20_000, 1);
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        for i in 1..=3u64 {
            let id = format!("c{i}");
            raw.extend([call(&id, "cat part"), result(&id, &"y".repeat(16_000))]);
            assemble(&d, request(&raw, Some(&root), 1 + i))
                .await
                .unwrap();
        }
        raw.extend([reply("checkpoint"), user("unseen")]);
        assemble(&d, request(&raw, Some(&root), 5)).await.unwrap();
        let first = d.delegate.last();
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!((st.stats.rollbacks, st.stats.steps), (0, 5));

        // The harness's one-shot overflow recovery: same (turn_id, step), more overhead.
        let mut retry = request(&raw, Some(&root), 5);
        retry.options.request_overhead_tokens = Some(10_000);
        assemble(&d, retry.clone()).await.unwrap();
        let again = d.delegate.last();
        assert_ne!(again["messages"], first["messages"]);
        // The 10000-token room drives the rollback; the section keeps the first limit.
        assert_eq!(again["system_prompt"], first["system_prompt"]);
        assert_eq!(again["system_prompt"], section(63_488));
        let sent = messages(&again);
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!((st.stats.rollbacks, st.last_overhead), (1, 10_000));
        assert_eq!(st.stats.steps, 5, "a recompute is not a new step");
        assert_eq!(
            sent[1],
            serde_json::to_value(st.ledger.as_ref().unwrap()).unwrap()
        );
        assert_eq!(sent.last().unwrap()["content"][0]["text"], "unseen");

        // Replayed again with that overhead: the recomputed step is the cache.
        let calls = d.delegate.count();
        assemble(&d, retry).await.unwrap();
        assert_eq!(d.delegate.count(), calls + 1);
        assert_eq!(d.delegate.last()["messages"], again["messages"]);
    }

    #[tokio::test]
    async fn a_rollback_keeps_the_user_message_the_model_has_not_seen() {
        let cfg = ClmConfig {
            budget_tokens: 20_000,
            reserve_tokens: 0,
            rollback_margin_tokens: 2_000,
            observation_max_chars: 100_000,
            ..ClmConfig::default()
        };
        let (_dir, root, d) = setup(cfg);
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        for i in 1..=4u64 {
            let id = format!("c{i}");
            raw.extend([call(&id, "cat part"), result(&id, &"y".repeat(12_000))]);
            assemble(&d, request(&raw, Some(&root), 1 + i))
                .await
                .unwrap();
        }
        assert_eq!(d.store.load(SID).unwrap().unwrap().stats.rollbacks, 0);
        let unseen = "z".repeat(32_000);
        raw.extend([reply("checkpoint"), user(&unseen)]);
        assemble(&d, request(&raw, Some(&root), 6)).await.unwrap();

        let sent = messages(&d.delegate.last());
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!(st.stats.rollbacks, 1);
        assert_eq!(sent.last().unwrap()["content"][0]["text"], unseen.as_str());
        assert_eq!(
            sent[1],
            serde_json::to_value(st.ledger.as_ref().unwrap()).unwrap()
        );
        assert!(
            !sent.iter().any(|m| has_call(m, "c4")),
            "the newest call was rolled back"
        );
    }

    #[tokio::test]
    async fn the_rollback_figure_counts_the_ledger_it_sends() {
        let cfg = ClmConfig {
            budget_tokens: 20_000,
            reserve_tokens: 0,
            rollback_margin_tokens: 2_000,
            observation_max_chars: 100_000,
            strip_thinking: false,
            ..ClmConfig::default()
        };
        let (dir, root, d) = setup(cfg);
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        for i in 1..=4u64 {
            let id = format!("c{i}");
            raw.extend([call(&id, "cat part"), result(&id, &"y".repeat(12_000))]);
            assemble(&d, request(&raw, Some(&root), 1 + i))
                .await
                .unwrap();
        }
        raw.extend([reply("checkpoint"), user(&"z".repeat(32_000))]);
        assemble(&d, request(&raw, Some(&root), 6)).await.unwrap();

        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!(st.stats.rollbacks, 1);
        let log =
            std::fs::read_to_string(dir.path().join(format!("state/log/{}.jsonl", safe_id(SID))))
                .unwrap();
        let line: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        let sent: Vec<Msg> = st
            .raw0
            .iter()
            .chain(&st.ledger)
            .chain(&st.proj)
            .cloned()
            .collect();
        assert_eq!(
            line["rolled"]["tokens_after"],
            raw_count(&section(st.section_limit), &sent)
        );
    }

    #[tokio::test]
    async fn a_rollback_pops_room_for_its_ledger_instead_of_ending_the_run() {
        let cfg = ClmConfig {
            reserve_tokens: 0,
            rollback_margin_tokens: 0,
            nudge_ratios: vec![],
            persistent_nudge: PersistentNudge::Off,
            observation_max_chars: 100_000,
            ..ClmConfig::default()
        };
        let (dir, root, d) = setup(cfg);
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        for i in 1..=4u64 {
            let id = format!("c{i}");
            raw.extend([call(&id, "cat part"), result(&id, &"y".repeat(12_000))]);
            assemble(&d, request(&raw, Some(&root), 1 + i))
                .await
                .unwrap();
        }
        raw.extend([reply("checkpoint"), user("next")]);
        // A limit 20 tokens over the task, the c1 and c2 steps and the
        // unseen message: popping to there leaves no room for the ledger.
        let st = d.store.load(SID).unwrap().unwrap();
        let msgs: Vec<Msg> = st
            .raw0
            .iter()
            .chain(&st.proj[..4])
            .cloned()
            .chain([serde_json::from_value(user("next")).unwrap()])
            .collect();
        let limit = raw_count(&section(st.section_limit), &msgs) + 20;
        let mut req = request(&raw, Some(&root), 6);
        req.options.budget_tokens = Some(limit);
        assemble(&d, req).await.unwrap();

        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!(st.stats.rollbacks, 1);
        assert!(!st.ctl.finalized, "the run goes on");
        let sent = messages(&d.delegate.last());
        assert_eq!(sent.last().unwrap()["content"][0]["text"], "next");
        assert!(has_call(&sent[2], "c1") && !sent.iter().any(|m| has_call(m, "c2")));
        let log =
            std::fs::read_to_string(dir.path().join(format!("state/log/{}.jsonl", safe_id(SID))))
                .unwrap();
        let line: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert!(line["rolled"]["tokens_after"].as_u64().unwrap() <= limit);
    }

    #[tokio::test]
    async fn the_results_of_a_parallel_call_share_the_cap_room() {
        let cfg = ClmConfig {
            budget_tokens: 8_192,
            observation_max_chars: 100_000,
            ..ClmConfig::default()
        };
        let (_dir, root, d) = setup(cfg);
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        let mut batch = call("p1", "cat part");
        for id in ["p2", "p3"] {
            let block = call(id, "cat part")["content"][1].clone();
            batch["content"].as_array_mut().unwrap().push(block);
        }
        let big = "y".repeat(16_000);
        raw.extend([
            batch,
            result("p1", &big),
            result("p2", &big),
            result("p3", &big),
        ]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();

        assert_eq!(d.store.load(SID).unwrap().unwrap().stats.rollbacks, 0);
        let sent = messages(&d.delegate.last());
        let note = cap_note(0, 6_144);
        let note = &note[note.find(" characters").unwrap()..];
        let texts: Vec<usize> = ["p1", "p2", "p3"]
            .iter()
            .map(|id| {
                let r = sent.iter().find(|m| m["function_call_id"] == *id).unwrap();
                let text = r["content"][0]["text"].as_str().unwrap();
                assert!(text.contains(note), "{id}: {text}");
                text.len()
            })
            .collect();
        // Even shares: none of them took the room from the others.
        assert!(
            texts[0] < 2 * texts[1] && texts[1] < 2 * texts[0],
            "{texts:?}"
        );
    }

    #[test]
    fn a_batch_room_keeps_small_results_whole_and_splits_the_rest() {
        assert_eq!(split_room(&[10, 1_000, 1_000], 610), [203, 300, 300]);
        assert_eq!(split_room(&[1_000, 10], 610), [600, 305]);
        assert!(split_room(&[100, 100], -10).iter().all(|&r| r <= 0));
    }

    #[tokio::test]
    async fn opting_in_on_a_long_session_rolls_back_with_a_ledger_and_never_stops() {
        let cfg = ClmConfig {
            budget_tokens: 16_384,
            observation_max_chars: 100_000,
            ..ClmConfig::default()
        };
        let (_dir, root, d) = setup(cfg);
        // ~40k tokens of history from before the session opted in.
        let mut raw = vec![user("task")];
        for i in 1..=10 {
            let id = format!("c{i}");
            raw.extend([call(&id, "cat part"), result(&id, &"y".repeat(16_000))]);
        }
        raw.extend([reply("checkpoint"), user("next: summarize the parts")]);
        let history: Vec<Msg> = serde_json::from_value(json!(raw)).unwrap();
        assert!(raw_count("", &history) > 40_000);

        let resp = assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        assert_eq!(resp.token_count, 10, "served, not stopped");
        assert!(resp.applied.get("strategy_stop").is_none());
        let st = d.store.load(SID).unwrap().unwrap();
        assert!(!st.stopped);
        assert_eq!((st.stats.rollbacks, st.stats.stops), (1, 0));
        let sent = messages(&d.delegate.last());
        assert_eq!(
            sent[1],
            serde_json::to_value(st.ledger.as_ref().unwrap()).unwrap()
        );
        assert_eq!(
            sent.last().unwrap()["content"][0]["text"],
            "next: summarize the parts"
        );
        assert!(sent[1..].iter().all(|m| !has_thinking(m)), "{sent:#?}");
    }

    #[tokio::test]
    async fn over_budget_after_the_final_turn_returns_the_stop_response() {
        let cfg = ClmConfig {
            budget_tokens: 3_000,
            reserve_tokens: 0,
            rollback_margin_tokens: 4_000,
            ..ClmConfig::default()
        };
        let (_dir, root, d) = setup(cfg);
        let mut raw = vec![user(&"t".repeat(16_000))];
        let first = assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        assert_eq!(first.token_count, 10, "the final turn is still served");
        raw.extend([reply("working on it"), user("continue")]);
        let resp = assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        assert_eq!(resp.token_count, resp.usable + 1);
        assert_eq!(resp.applied["strategy_stop"], STRATEGY_STOP_REASON);
        assert_eq!(resp.applied["compacted"], false);
        let st = d.store.load(SID).unwrap().unwrap();
        assert!(st.stopped);
        assert_eq!(st.stats.stops, 1);
    }

    #[tokio::test]
    async fn a_turn_after_a_stop_gets_a_final_turn_before_stopping_again() {
        let cfg = ClmConfig {
            budget_tokens: 3_000,
            reserve_tokens: 0,
            rollback_margin_tokens: 4_000,
            ..ClmConfig::default()
        };
        let (_dir, root, d) = setup(cfg);
        let mut raw = vec![user(&"t".repeat(16_000))];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([reply("working on it"), user("continue")]);
        let stop = assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        assert_eq!(stop.applied["strategy_stop"], STRATEGY_STOP_REASON);

        // Opted out for a turn on context-manager, then back in: a new turn.
        raw.extend([
            user("plain turn"),
            reply("done without clm"),
            user("with clm again"),
        ]);
        let mut next = request(&raw, Some(&root), 1);
        next.options.turn_id = Some("turn-2".into());
        let resp = assemble(&d, next).await.unwrap();
        assert!(resp.applied.get("strategy_stop").is_none(), "{resp:?}");
        assert_eq!(resp.token_count, 10);
        let st = d.store.load(SID).unwrap().unwrap();
        assert!(
            !st.stopped && st.ctl.finalized,
            "a fresh run took its final turn"
        );
        assert_eq!(st.stats.stops, 1);
    }

    #[tokio::test]
    async fn an_opt_in_after_a_rewritten_history_rebuilds_and_strips_from_the_task_on() {
        let (_dir, root, d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        // The run had stopped before the user opted out.
        let mut old = d.store.load(SID).unwrap().unwrap();
        old.stopped = true;
        old.ctl.finalized = true;
        old.ctl.n_retry = 50;
        d.store.save(SID, &old).unwrap();

        // While opted out, context-manager rewrote a covered message; turns went on.
        let rewritten = vec![
            user("task"),
            call("c1", "ls"),
            result("c1", "a.txt (output pruned)"),
            call("c2", "cat a.txt"),
            result("c2", "hello"),
            reply("done"),
            user("again, with clm"),
        ];
        let mut req = request(&rewritten, Some(&root), 1);
        req.options.turn_id = Some("turn-2".into());
        let resp = assemble(&d, req).await.unwrap();
        assert!(resp.applied.get("strategy_stop").is_none());
        let st = d.store.load(SID).unwrap().unwrap();
        assert!(!st.stopped);
        assert_eq!((st.ctl.n_retry, st.ctl.finalized), (0, false));
        assert_eq!(st.covered, rewritten.len());
        let sent = messages(&d.delegate.last());
        assert_eq!(sent.len(), rewritten.len(), "{sent:#?}");
        assert!(sent[1..].iter().all(|m| !has_thinking(m)), "{sent:#?}");
        assert_eq!(
            sent.last().unwrap()["content"][0]["text"],
            "again, with clm"
        );
    }

    #[tokio::test]
    async fn a_final_turn_over_a_clamped_limit_stops_instead_of_overflowing() {
        let cfg = ClmConfig {
            max_rollbacks: 0,
            ..ClmConfig::default()
        };
        let (_dir, root, mut d) = setup(cfg);
        (d.delegate.usable, d.delegate.scale, d.delegate.strict) = (3_000, 1, true);
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();

        // Over the clamped limit with no rollback left: the limit is the
        // model's room, so a final turn could not be sent. The harness would
        // finalize that overflow at once; the step is the stop instead.
        raw.extend([reply("working on it"), user(&"x".repeat(16_000))]);
        let resp = assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        assert_eq!(resp.token_count, resp.usable + 1);
        assert_eq!(resp.applied["strategy_stop"], STRATEGY_STOP_REASON);
        assert_eq!(d.delegate.last()["options"]["preview_only"], true);
        let st = d.store.load(SID).unwrap().unwrap();
        assert!(st.stopped && st.ctl.finalized);
        assert_eq!(st.stats.stops, 1);
    }

    #[tokio::test]
    async fn a_replay_after_a_stop_serves_the_stop_and_changes_nothing() {
        let cfg = ClmConfig {
            budget_tokens: 3_000,
            reserve_tokens: 0,
            rollback_margin_tokens: 4_000,
            ..ClmConfig::default()
        };
        let (dir, root, d) = setup(cfg);
        let mut raw = vec![user(&"t".repeat(16_000))];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([reply("working on it"), user("continue")]);
        let stop = assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        assert_eq!(stop.applied["strategy_stop"], STRATEGY_STOP_REASON);
        let state = dir.path().join(format!("state/{}.json", safe_id(SID)));
        let mirror = root.join(mirror_rel(SID));
        let files = || {
            (
                std::fs::read(&state).unwrap(),
                std::fs::read(&mirror).unwrap(),
            )
        };
        let before = files();

        for overhead in [0, 10_000] {
            let mut again = request(&raw, Some(&root), 2);
            again.options.request_overhead_tokens = Some(overhead);
            let resp = assemble(&d, again).await.unwrap();
            assert_eq!(resp.token_count, resp.usable + 1, "overhead {overhead}");
            assert_eq!(resp.applied["strategy_stop"], STRATEGY_STOP_REASON);
            assert_eq!(d.delegate.last()["options"]["preview_only"], true);
            assert_eq!(
                d.delegate.last()["messages"],
                d.delegate.calls.lock().unwrap()[1]["messages"]
            );
        }
        assert!(
            files() == before,
            "a replay writes neither state nor mirror"
        );
        assert_eq!(d.store.load(SID).unwrap().unwrap().stats.stops, 1);
    }

    #[tokio::test]
    async fn a_recompute_does_not_push_its_nudge_twice() {
        let cfg = ClmConfig {
            nudge_ratios: vec![],
            persistent_nudge: PersistentNudge::Ratio(0.5),
            observation_max_chars: 100_000,
            ..ClmConfig::default()
        };
        let (_dir, root, mut d) = setup(cfg);
        (d.delegate.usable, d.delegate.scale) = (20_000, 1);
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "cat part"), result("c1", &"y".repeat(40_000))]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        let first = d.delegate.last();
        let nudge = messages(&first).last().unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned();
        let hint = compaction_hint(&mirror_rel(SID));
        let lead = persistent_text(0, 0, &hint);
        let lead = &lead[..=lead.find(']').unwrap()];
        assert!(nudge.starts_with(lead), "{nudge}");

        // Still over half of the smaller limit, but under it: no rollback.
        let mut retry = request(&raw, Some(&root), 2);
        retry.options.request_overhead_tokens = Some(2_000);
        assemble(&d, retry).await.unwrap();
        let again = messages(&d.delegate.last());
        assert_eq!(d.store.load(SID).unwrap().unwrap().last_overhead, 2_000);
        let nudges = again
            .iter()
            .filter(|m| {
                m["content"][0]["text"]
                    .as_str()
                    .is_some_and(|t| t.starts_with(lead))
            })
            .count();
        assert_eq!(nudges, 1, "{again:#?}");
        assert_eq!(again, messages(&first));
    }

    #[tokio::test]
    async fn a_recompute_over_a_grown_window_pushes_the_tier_it_crosses() {
        let cfg = ClmConfig {
            nudge_ratios: vec![0.25],
            persistent_nudge: PersistentNudge::Off,
            observation_max_chars: 100_000,
            ..ClmConfig::default()
        };
        let (_dir, root, d) = setup(cfg);
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        // Step 2 redelivered with the reply and a ~20k-token result: over a
        // quarter of the 65536-token budget for the first time.
        raw.extend([call("c2", "cat big.log"), result("c2", &"y".repeat(80_000))]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        let sent = messages(&d.delegate.last());
        let nudge = sent.last().unwrap()["content"][0]["text"].as_str().unwrap();
        assert!(nudge.starts_with("[context note]"), "{nudge}");
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!((st.ctl.nudged.clone(), st.stats.steps), (vec![0.25], 2));
    }

    #[tokio::test]
    async fn receipts_and_the_gate_count_in_calibrated_tokens() {
        let (dir, root, d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        raw.extend([call("c1", "ls"), result("c1", &"a.txt\n".repeat(200))]);
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        let st = d.store.load(SID).unwrap().unwrap();
        let window: Vec<Msg> = st.raw0.iter().chain(&st.proj).cloned().collect();
        let raw_before = raw_count(&section(63_488), &window);

        // The model condenses turn 2 (the listing) into a note.
        let mirror = root.join(mirror_rel(SID));
        let text = std::fs::read_to_string(&mirror).unwrap();
        let at = text.find("[[CTX_TURN 2 ").expect("turn 2 header");
        let header_end = at + text[at..].find('\n').unwrap();
        std::fs::write(&mirror, format!("{}\nnote: 200 files", &text[..header_end])).unwrap();
        // The provider counted twice the ruler for the request step 2 sent.
        let mut c2 = call("c2", "echo done");
        c2["usage"] = json!({ "input": 2 * st.raw_sent });
        raw.extend([c2, result("c2", "done")]);
        assemble(&d, request(&raw, Some(&root), 3)).await.unwrap();

        let log =
            std::fs::read_to_string(dir.path().join(format!("state/log/{}.jsonl", safe_id(SID))))
                .unwrap();
        let line: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        let edit = &line["edit"];
        assert_eq!(edit["gate"], "applied");
        let (before, after) = (
            edit["before"].as_u64().unwrap(),
            edit["after"].as_u64().unwrap(),
        );
        assert_eq!(before, 2 * raw_before, "the readout's unit");
        let sent = messages(&d.delegate.last());
        let newest = sent.last().unwrap()["content"][0]["text"].as_str().unwrap();
        // task + the two condensed turns + the c2 call and result
        assert!(
            newest.ends_with(&receipt_applied(before, after, 4)),
            "{newest}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_planted_clm_link_stops_mirror_io_without_failing_the_step() {
        let (dir, root, d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        // The agent swaps `.clm` for a link to a mirror it planted elsewhere.
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let name = Path::new(&mirror_rel(SID)).file_name().unwrap().to_owned();
        std::fs::write(elsewhere.join(name), "[[CTX_TURN 1 role=user]]\nPLANTED").unwrap();
        std::fs::remove_dir_all(root.join(".clm")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join(".clm")).unwrap();

        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        let resp = assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        assert_eq!(resp.token_count, 10, "the step is served");
        let sent = messages(&d.delegate.last());
        assert_eq!(sent.len(), 3, "{sent:#?}");
        assert!(!sent.iter().any(|m| m.to_string().contains("PLANTED")));
        let st = d.store.load(SID).unwrap().unwrap();
        assert!(
            st.written.is_none(),
            "mirror I/O stops; the projection is kept"
        );
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_save_leaves_the_mirror_the_saved_state_wrote() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, root, d) = setup(ClmConfig::default());
        let mut raw = vec![user("task")];
        assemble(&d, request(&raw, Some(&root), 1)).await.unwrap();
        let mirror = root.join(mirror_rel(SID));
        let before = std::fs::read_to_string(&mirror).unwrap();

        let state_dir = dir.path().join("state");
        let mode = |m| std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(m));
        mode(0o500).unwrap();
        raw.extend([call("c1", "ls"), result("c1", "a.txt")]);
        let failed = assemble(&d, request(&raw, Some(&root), 2)).await;
        mode(0o700).unwrap();
        assert!(failed.is_err(), "{failed:?}");
        assert_eq!(std::fs::read_to_string(&mirror).unwrap(), before);

        // The retry reads no edit: the mirror still matches the saved state.
        assemble(&d, request(&raw, Some(&root), 2)).await.unwrap();
        let st = d.store.load(SID).unwrap().unwrap();
        assert_eq!((st.stats.edits_applied, st.covered), (0, 3));
    }

    #[tokio::test]
    async fn session_deleted_removes_mirror_state_and_step_log() {
        let (dir, root, d) = setup(ClmConfig::default());
        assemble(&d, request(&[user("task")], Some(&root), 1))
            .await
            .unwrap();
        let log = dir.path().join(format!("state/log/{}.jsonl", safe_id(SID)));
        assert!(log.is_file());
        assert!(read_mirror(&root, SID).unwrap().is_some());

        let event = || SessionDeletedEvent {
            session_id: SID.into(),
        };
        let ack = on_session_deleted(&d, event()).await.unwrap();
        assert_eq!(
            ack,
            SessionDeletedAck {
                ok: true,
                removed: true
            }
        );
        assert!(read_mirror(&root, SID).unwrap().is_none());
        assert!(d.store.load(SID).unwrap().is_none());
        assert!(!log.exists());
        // A redelivery finds nothing and still acknowledges.
        let again = on_session_deleted(&d, event()).await.unwrap();
        assert_eq!(
            again,
            SessionDeletedAck {
                ok: true,
                removed: false
            }
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn session_deleted_survives_a_planted_clm_link() {
        let (dir, root, d) = setup(ClmConfig::default());
        assemble(&d, request(&[user("task")], Some(&root), 1))
            .await
            .unwrap();
        let log = dir.path().join(format!("state/log/{}.jsonl", safe_id(SID)));
        // The agent swaps `.clm` for a link to a file it planted elsewhere.
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let name = Path::new(&mirror_rel(SID)).file_name().unwrap().to_owned();
        let planted = elsewhere.join(name);
        std::fs::write(&planted, "PLANTED").unwrap();
        std::fs::remove_dir_all(root.join(".clm")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join(".clm")).unwrap();
        assert!(
            crate::mirror::remove_mirror(&root, SID).is_err(),
            "the link makes mirror removal fail"
        );

        let ack = on_session_deleted(
            &d,
            SessionDeletedEvent {
                session_id: SID.into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ack,
            SessionDeletedAck {
                ok: true,
                removed: true
            }
        );
        assert!(d.store.load(SID).unwrap().is_none());
        assert!(!log.exists());
        assert_eq!(std::fs::read_to_string(&planted).unwrap(), "PLANTED");
    }

    #[tokio::test]
    async fn session_deleted_removes_an_unreadable_state() {
        let (dir, _root, d) = setup(ClmConfig::default());
        let state = dir.path().join(format!("state/{}.json", safe_id(SID)));
        std::fs::create_dir_all(state.parent().unwrap()).unwrap();
        std::fs::write(&state, "{ torn").unwrap();
        let ack = on_session_deleted(
            &d,
            SessionDeletedEvent {
                session_id: SID.into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            ack,
            SessionDeletedAck {
                ok: true,
                removed: true
            }
        );
        assert!(!state.exists());
    }
}
