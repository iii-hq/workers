//! Budget controller: the token ruler, tier/persistent nudges, rollback of
//! the newest steps, the final turn and the stop, plus the caps applied to
//! tool output. Behaviour follows the CLM spike (`clm/src/controller.ts`);
//! every model-facing string comes from `crate::prompt`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{ClmConfig, EditGate, PersistentNudge};
use crate::prompt;
use crate::types::{Block, Msg};

/// Per-session controller state, persisted with the session.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ctl {
    /// Tier fractions already nudged.
    pub nudged: Vec<f64>,
    /// The final turn was granted.
    pub finalized: bool,
    /// Rollbacks so far.
    pub n_retry: u32,
    /// Consecutive rollbacks without an applied edit.
    pub consec: u32,
    /// Messages rolled back so far.
    pub n_rolled: usize,
    /// Commands whose steps were rolled back (for the ledger).
    pub cmds: Vec<String>,
}

/// Controller settings for one session, resolved from the worker config and
/// the session's optional budget override.
#[derive(Clone, Debug, PartialEq)]
pub struct Cfg {
    pub budget: u64,
    pub reserve: u64,
    pub nudge_ratios: Vec<f64>,
    pub persistent: PersistentNudge,
    /// The adaptive band keeps room for `obs_mult` x the largest recent result.
    pub obs_mult: f64,
    pub max_retry: u32,
    pub retry_margin: u64,
    /// Edit gate: `true` = an edit may grow while it still fits the limit.
    pub fit: bool,
    pub observation_max_chars: usize,
}

impl Cfg {
    /// A session override at or below the reserve leaves no room (limit 0 would
    /// silently disable every budget action), so it is ignored and logged.
    pub fn from_config(c: &ClmConfig, budget_override: Option<u64>) -> Cfg {
        let budget = match budget_override {
            Some(x) if x <= c.reserve_tokens => {
                tracing::warn!(
                    budget_override = x,
                    reserve_tokens = c.reserve_tokens,
                    "ignoring the session budget override: it leaves no room above the reserve"
                );
                c.budget_tokens
            }
            Some(x) => x,
            None => c.budget_tokens,
        };
        Cfg {
            budget,
            reserve: c.reserve_tokens,
            nudge_ratios: c.nudge_ratios.clone(),
            persistent: c.persistent_nudge,
            obs_mult: 2.0,
            max_retry: c.max_rollbacks,
            retry_margin: c.rollback_margin_tokens,
            fit: matches!(c.edit_gate, EditGate::Fit),
            observation_max_chars: c.observation_max_chars,
        }
    }

    /// Hard limit = budget - reserve (0 disables every budget action).
    pub fn limit(&self) -> u64 {
        self.budget.saturating_sub(self.reserve)
    }
}

/// Python's `round()` (ties to even); only used for displayed percentages.
pub fn py_round(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// Token count a rollback pops down to. The depth escalates only after three
/// consecutive rollbacks with no applied edit.
pub fn rollback_target(cfg: &Cfg, ctl: &Ctl) -> u64 {
    let limit = cfg.limit();
    let depth = u64::from(ctl.consec).saturating_sub(3).max(1);
    let margin = cfg
        .retry_margin
        .saturating_mul(depth)
        .min((3 * limit / 4).max(cfg.retry_margin));
    limit.saturating_sub(margin)
}

/// Pop the newest step without orphaning a call: a result takes its sibling
/// results and the assistant call that produced them. Returned oldest first.
pub fn pop_step(proj: &mut Vec<Msg>) -> Vec<Msg> {
    let Some(last) = proj.pop() else {
        return Vec::new();
    };
    let mut dropped = vec![last];
    if dropped[0].role == "function_result" {
        while let Some(r) = proj.pop_if(|m| m.role == "function_result") {
            dropped.push(r);
        }
        if let Some(call) = proj.pop_if(|m| m.is_assistant_call()) {
            dropped.push(call);
        }
    }
    dropped.reverse();
    dropped
}

/// Remember the commands of rolled-back calls for the ledger: whitespace
/// collapsed, cut to 70 chars, deduplicated across the run, last 5 kept.
pub fn note_commands(ctl: &mut Ctl, dropped: &[Msg]) {
    for m in dropped {
        for b in m.content.iter().filter(|b| b.kind == "function_call") {
            let cmd = command_line(b.rest.get("arguments").unwrap_or(&Value::Null));
            let cmd: String = cmd
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(70)
                .collect();
            if !cmd.is_empty() && !ctl.cmds.contains(&cmd) {
                ctl.cmds.push(cmd);
            }
        }
    }
    let n = ctl.cmds.len();
    if n > 5 {
        ctl.cmds.drain(..n - 5);
    }
}

/// `shell::exec` arguments as a command line (`bash -c X` shows just X).
fn command_line(args: &Value) -> String {
    let list: &[Value] = args
        .get("args")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    let command = args.get("command").unwrap_or(&Value::Null);
    if command.as_str() == Some("bash") && list.first().and_then(Value::as_str) == Some("-c") {
        return list.get(1).map(plain).unwrap_or_default();
    }
    std::iter::once(command)
        .chain(list)
        .filter(|v| !v.is_null())
        .map(plain)
        .collect::<Vec<_>>()
        .join(" ")
}

/// JS `String(v ?? '')`: strings verbatim, null empty, anything else as JSON.
fn plain(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn piece_len(blocks: &[Block]) -> usize {
    let mut n = 0;
    for b in blocks {
        let field = |k: &str| b.rest.get(k).and_then(Value::as_str).map_or(0, str::len);
        match b.kind.as_str() {
            "text" | "thinking" => n += field("text"),
            "function_call" => {
                n += field("function_id");
                n += b
                    .rest
                    .get("arguments")
                    .filter(|v| !v.is_null())
                    .map_or(2, |v| v.to_string().len());
            }
            "function_result" => {
                if let Some(Ok(inner)) = b.rest.get("content").map(Vec::<Block>::deserialize) {
                    n += piece_len(&inner);
                }
            }
            _ => {}
        }
    }
    n
}

/// The raw ruler: (bytes of system prompt + text pieces + 1) / 4. Pieces are
/// text and thinking text, call function ids and JSON arguments, and nested
/// result text. Bytes (not chars) match context-manager's estimator; the
/// per-session calibration ratio absorbs the difference to provider tokens.
pub fn raw_count(system: &str, msgs: &[Msg]) -> u64 {
    let n = system.len() + msgs.iter().map(|m| piece_len(&m.content)).sum::<usize>();
    ((n + 1) / 4) as u64
}

/// Edit gate: reject iff the edit grew context and either the gate is
/// `shrink` or the grown context no longer fits the limit.
pub fn gate_rejects(baseline: u64, after: u64, fit: bool, limit: u64) -> bool {
    after > baseline && !(fit && limit > 0 && after <= limit)
}

/// An edit is real (not mirror noise) iff it moves at least
/// max(32, 1% of the editable region) tokens.
pub fn real_edit(baseline: u64, after: u64, prefix_tokens: u64) -> bool {
    baseline.abs_diff(after) >= (baseline.saturating_sub(prefix_tokens).max(1) / 100).max(32)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Nudge {
    Tier { tier: f64, text: String },
    Persistent { text: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Ok(Option<Nudge>),
    Rollback,
    Final(String),
    Stop,
}

/// Token level of a tier: a share of the budget.
fn level(budget: u64, ratio: f64) -> u64 {
    (budget as f64 * ratio).floor() as u64
}

/// One controller step (`tc` and `obs_max` already calibrated). `obs_max` is
/// the largest of the last three tool results since the last applied edit.
/// Tiers are shares of the budget; every other figure the model sees is the
/// enforced limit (budget minus reserve).
pub fn evaluate(
    cfg: &Cfg,
    ctl: &mut Ctl,
    tc: u64,
    obs_max: u64,
    proj_len: usize,
    hint: &str,
) -> Decision {
    let limit = cfg.limit();
    ctl.nudged.retain(|&f| tc >= level(cfg.budget, f));
    if limit > 0 && tc > limit {
        if ctl.n_retry < cfg.max_retry && limit > cfg.retry_margin && proj_len > 0 {
            ctl.n_retry += 1;
            ctl.consec += 1;
            return Decision::Rollback;
        }
        if ctl.finalized {
            return Decision::Stop;
        }
        ctl.finalized = true;
        return Decision::Final(prompt::final_text(tc, limit));
    }
    ctl.finalized = false;
    if limit == 0 {
        return Decision::Ok(None);
    }
    match cfg.persistent {
        PersistentNudge::Ratio(r) if tc >= (limit as f64 * r).floor() as u64 => {
            let text = prompt::persistent_text(tc, limit, hint);
            return Decision::Ok(Some(Nudge::Persistent { text }));
        }
        PersistentNudge::Adaptive => {
            let by_ratio = limit / 10;
            let by_obs = (cfg.obs_mult * obs_max as f64).floor() as u64;
            let need = if by_obs <= by_ratio {
                by_ratio
            } else {
                by_obs.min(limit / 2)
            };
            if limit - tc < need {
                let text = prompt::persistent_text(tc, limit, hint);
                return Decision::Ok(Some(Nudge::Persistent { text }));
            }
        }
        _ => {}
    }
    let mut crossed: Vec<f64> = cfg
        .nudge_ratios
        .iter()
        .copied()
        .filter(|&f| f > 0.0 && f < 1.0 && tc >= level(cfg.budget, f))
        .collect();
    crossed.sort_by(f64::total_cmp);
    let Some(tier) = crossed
        .iter()
        .rev()
        .copied()
        .find(|f| !ctl.nudged.contains(f))
    else {
        return Decision::Ok(None);
    };
    for f in crossed {
        if !ctl.nudged.contains(&f) {
            ctl.nudged.push(f);
        }
    }
    let text = prompt::tier_text(tier, cfg.budget, tc, hint);
    Decision::Ok(Some(Nudge::Tier { tier, text }))
}

/// A `shell::exec` result rendered as `<output>\n\n(exit_code=N)`.
#[derive(Clone, Debug, PartialEq)]
pub struct ExecBody {
    pub body: String,
    /// `None` when the result was not a `shell::exec` JSON payload.
    pub rc: Option<i64>,
    /// Neither stdout nor stderr printed anything.
    pub silent: bool,
}

pub fn exec_body(text: &str, max_chars: usize) -> ExecBody {
    if let Ok(Value::Object(j)) = serde_json::from_str::<Value>(text) {
        if j.contains_key("exit_code") {
            let stdout = j.get("stdout").map(plain).unwrap_or_default();
            let stderr = j.get("stderr").map(plain).unwrap_or_default();
            let joined = match (stdout.is_empty(), stderr.is_empty()) {
                (_, true) => stdout.clone(),
                (true, false) => stderr.clone(),
                (false, false) => format!("{stdout}\n{stderr}"),
            };
            let out = joined.trim_end();
            let rc = match j.get("exit_code").and_then(Value::as_i64) {
                Some(rc) => rc,
                None if j.get("timed_out").and_then(Value::as_bool) == Some(true) => 124,
                None => -1,
            };
            let shown = if out.is_empty() {
                prompt::NO_OUTPUT.to_string()
            } else {
                truncate_observation(out, max_chars)
            };
            return ExecBody {
                body: format!("{shown}\n\n(exit_code={rc})"),
                rc: Some(rc),
                silent: stdout.trim().is_empty() && stderr.trim().is_empty(),
            };
        }
    }
    ExecBody {
        body: truncate_observation(text, max_chars),
        rc: None,
        silent: false,
    }
}

/// Head (`head` chars), tail (`tail` chars) and the number of chars between
/// them; `None` when `s` is not longer than `head + tail`.
fn head_tail(s: &str, head: usize, tail: usize) -> Option<(&str, &str, usize)> {
    let n = s.chars().count();
    if n <= head + tail {
        return None;
    }
    let at = |k: usize| s.char_indices().nth(k).map_or(s.len(), |(i, _)| i);
    Some((&s[..at(head)], &s[at(n - tail)..], n - head - tail))
}

/// Like [`head_tail`] but `head` and `tail` are byte budgets: the head ends on
/// a char boundary at or below `head`, the tail starts on one at or above
/// `s.len() - tail`. The elided count is still in chars (the cap note says
/// "characters").
fn head_tail_bytes(s: &str, head: usize, tail: usize) -> Option<(&str, &str, usize)> {
    if s.len() <= head + tail {
        return None;
    }
    let mut h = head;
    while !s.is_char_boundary(h) {
        h -= 1;
    }
    let mut t = s.len() - tail;
    while !s.is_char_boundary(t) {
        t += 1;
    }
    Some((&s[..h], &s[t..], s[h..t].chars().count()))
}

/// Output longer than `max_chars` keeps 5000 chars of head and tail (half of
/// `max_chars` each when that is smaller) behind the too-long hint.
pub fn truncate_observation(out: &str, max_chars: usize) -> String {
    let half = (max_chars / 2).min(5000);
    match head_tail(out, half, half) {
        Some((head, tail, elided)) if out.chars().count() > max_chars => {
            prompt::observation_cut(head, elided, tail)
        }
        _ => out.to_string(),
    }
}

/// Cap the newest tool output to the room left under the limit (`room` is in
/// ruler tokens and may be negative). The fit check and the cut both count
/// UTF-8 bytes (the unit of `raw_count`), so multi-byte text honours the room
/// too. Text that does not fit keeps a head and a tail sized to the room,
/// never less than 256 bytes each. `limit` is the enforced limit
/// (`Cfg::limit()`), the figure the cut note shows the model.
pub fn cap_newest(text: &str, room: i64, limit: u64) -> String {
    if (text.len() as i64 + 1) / 4 <= room {
        return text.to_string();
    }
    let keep = (room.max(128) - 48).max(128) as usize;
    let half = (keep * 4).max(256) / 2;
    match head_tail_bytes(text, half, half) {
        Some((head, tail, elided)) => {
            format!("{head}\n\n{}\n\n{tail}", prompt::cap_note(elided, limit))
        }
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(v: Value) -> Msg {
        serde_json::from_value(v).unwrap()
    }

    fn clm() -> Cfg {
        Cfg::from_config(&ClmConfig::default(), Some(32768))
    }

    const H: &str = "HINT";

    fn base() -> Cfg {
        Cfg {
            nudge_ratios: vec![],
            persistent: PersistentNudge::Off,
            max_retry: 0,
            ..clm()
        }
    }

    fn call(ids: &[&str]) -> Msg {
        let content: Vec<Value> = ids
            .iter()
            .map(|id| json!({"type": "function_call", "id": id, "function_id": "shell::exec", "arguments": {"command": "ls"}}))
            .collect();
        msg(json!({"role": "assistant", "content": content, "timestamp": 0}))
    }

    fn result(id: &str) -> Msg {
        msg(
            json!({"role": "function_result", "function_call_id": id, "content": [], "timestamp": 0}),
        )
    }

    fn user(text: &str) -> Msg {
        msg(json!({"role": "user", "content": [{"type": "text", "text": text}], "timestamp": 0}))
    }

    #[test]
    fn from_config_uses_defaults_and_override() {
        let d = Cfg::from_config(&ClmConfig::default(), None);
        assert_eq!((d.budget, d.reserve, d.limit()), (65536, 2048, 63488));
        assert_eq!(d.nudge_ratios, vec![0.25, 0.5, 0.75]);
        assert_eq!(d.persistent, PersistentNudge::Adaptive);
        assert_eq!((d.max_retry, d.retry_margin, d.fit), (50, 2048, true));
        assert_eq!((d.obs_mult, d.observation_max_chars), (2.0, 10000));
        assert_eq!(clm().limit(), 30720);
        let tiny = Cfg {
            budget: 1000,
            ..clm()
        };
        assert_eq!(tiny.limit(), 0);
    }

    #[test]
    fn an_override_at_or_below_the_reserve_is_ignored() {
        let c = ClmConfig::default();
        assert_eq!(Cfg::from_config(&c, Some(2048)).budget, 65536);
        assert_eq!(Cfg::from_config(&c, Some(0)).budget, 65536);
        let kept = Cfg::from_config(&c, Some(2049));
        assert_eq!((kept.budget, kept.limit()), (2049, 1));
    }

    #[test]
    fn py_round_is_half_to_even() {
        let got: Vec<i64> = [72.5, 73.5, 25.0, 74.6].into_iter().map(py_round).collect();
        assert_eq!(got, vec![72, 74, 25, 75]);
    }

    #[test]
    fn raw_count_uses_the_fallback_ruler() {
        assert_eq!(raw_count("abcd", &[user("efgh")]), 2);
        let c = call(&["c1"]);
        // "shell::exec" (11) + {"command":"ls"} (16) = 27, +1, /4 = 7
        assert_eq!(raw_count("", &[c]), 7);
        let thinking = msg(json!({"role": "assistant", "content": [
            {"type": "thinking", "text": "abc", "signature": "s"},
            {"type": "redacted_thinking", "data": "zzzzzzzz"}
        ], "timestamp": 0}));
        assert_eq!(raw_count("", &[thinking]), 1);
    }

    #[test]
    fn gate_and_real_edit() {
        assert!(!gate_rejects(100, 90, true, 200));
        assert!(!gate_rejects(100, 150, true, 200));
        assert!(gate_rejects(100, 250, true, 200));
        assert!(gate_rejects(100, 150, false, 200));
        assert!(gate_rejects(100, 150, true, 0));
        assert!(real_edit(1000, 960, 0));
        assert!(!real_edit(1000, 990, 0));
        assert!(real_edit(10_000, 9_900, 0));
        assert!(!real_edit(10_000, 9_901, 0));
        assert!(!real_edit(10_000, 9_969, 9_000));
    }

    #[test]
    fn rollback_depth_escalates_only_after_three_consecutive() {
        let mut ctl = Ctl {
            consec: 3,
            ..Ctl::default()
        };
        assert_eq!(rollback_target(&clm(), &ctl), 30720 - 2048);
        ctl.consec = 6;
        assert_eq!(rollback_target(&clm(), &ctl), 30720 - 3 * 2048);
        ctl.consec = 40;
        assert_eq!(rollback_target(&clm(), &ctl), 30720 - 30720 * 3 / 4);
    }

    #[test]
    fn pop_step_never_orphans_a_call() {
        let (u, a, r1, r2) = (user("q"), call(&["c1", "c2"]), result("c1"), result("c2"));
        let mut proj = vec![u.clone(), a.clone(), r1.clone(), r2.clone()];
        assert_eq!(pop_step(&mut proj), vec![a.clone(), r1, r2]);
        assert_eq!(proj, vec![u.clone()]);
        assert_eq!(pop_step(&mut proj), vec![u.clone()]);
        assert!(pop_step(&mut proj).is_empty());
        let mut pending = vec![u.clone(), a.clone()];
        assert_eq!(pop_step(&mut pending), vec![a]);
        assert_eq!(pending, vec![u]);
    }

    #[test]
    fn note_commands_collapses_dedupes_and_keeps_five() {
        let shell = |cmd: Value| {
            msg(json!({"role": "assistant", "content": [
                {"type": "function_call", "id": "c", "function_id": "shell::exec", "arguments": cmd}
            ], "timestamp": 0}))
        };
        let mut ctl = Ctl::default();
        note_commands(
            &mut ctl,
            &[
                shell(json!({"command": "bash", "args": ["-c", "cat   big.log |\n grep x"]})),
                shell(json!({"command": "ls", "args": ["-la", "src"]})),
                shell(json!({"command": "ls", "args": ["-la", "src"]})),
                shell(json!({"pattern": "foo"})),
                user("not a call"),
            ],
        );
        assert_eq!(ctl.cmds, vec!["cat big.log | grep x", "ls -la src"]);
        let long = "x".repeat(100);
        note_commands(&mut ctl, &[shell(json!({"command": long}))]);
        assert_eq!(ctl.cmds[2].len(), 70);
        for i in 0..4 {
            note_commands(&mut ctl, &[shell(json!({"command": format!("c{i}")}))]);
        }
        assert_eq!(
            ctl.cmds,
            vec![
                "x".repeat(70),
                "c0".into(),
                "c1".into(),
                "c2".into(),
                "c3".into()
            ]
        );
    }

    #[test]
    fn tiers_fire_once_and_rearm_below_their_level() {
        let fixed = Cfg {
            persistent: PersistentNudge::Off,
            ..clm()
        };
        let mut ctl = Ctl::default();
        assert_eq!(
            evaluate(&fixed, &mut ctl, 5000, 0, 3, H),
            Decision::Ok(None)
        );
        // 17000 crosses 0.25 (8192) and 0.5 (16384): only the highest fires.
        let d1 = evaluate(&fixed, &mut ctl, 17000, 0, 3, H);
        let want = prompt::tier_text(0.5, 32768, 17000, H);
        assert_eq!(
            d1,
            Decision::Ok(Some(Nudge::Tier {
                tier: 0.5,
                text: want
            }))
        );
        assert_eq!(
            evaluate(&fixed, &mut ctl, 17500, 0, 3, H),
            Decision::Ok(None)
        );
        evaluate(&fixed, &mut ctl, 1000, 0, 3, H); // condensed: both re-arm
        assert!(ctl.nudged.is_empty());
        let d3 = evaluate(&fixed, &mut ctl, 9000, 0, 3, H);
        let want = prompt::tier_text(0.25, 32768, 9000, H);
        assert!(!want.contains(H));
        assert_eq!(
            d3,
            Decision::Ok(Some(Nudge::Tier {
                tier: 0.25,
                text: want
            }))
        );
        let d4 = evaluate(&fixed, &mut ctl, 25000, 0, 3, H);
        assert!(matches!(d4, Decision::Ok(Some(Nudge::Tier { tier, .. })) if tier == 0.75));
    }

    #[test]
    fn adaptive_persistent_fires_when_room_is_below_twice_the_largest_result() {
        let mut ctl = Ctl::default();
        // limit 30720, room 5720 < need 2 x 4000
        let d = evaluate(&clm(), &mut ctl, 25000, 4000, 3, H);
        let want = prompt::persistent_text(25000, 30720, H);
        assert!(want.contains("25000/30720 tokens"));
        assert_eq!(d, Decision::Ok(Some(Nudge::Persistent { text: want })));
        // small results: the band is 10% of the limit (3072); room 5720 is enough
        let d = evaluate(&clm(), &mut Ctl::default(), 25000, 100, 3, H);
        assert!(matches!(d, Decision::Ok(Some(Nudge::Tier { .. }))));
        // a huge result is capped at half the limit (15360)
        let d = evaluate(&clm(), &mut Ctl::default(), 16000, 50_000, 3, H);
        assert!(matches!(d, Decision::Ok(Some(Nudge::Persistent { .. }))));
        let d = evaluate(&clm(), &mut Ctl::default(), 15000, 50_000, 3, H);
        assert!(matches!(d, Decision::Ok(Some(Nudge::Tier { .. }))));
    }

    #[test]
    fn fixed_ratio_persistent_fires_at_its_threshold() {
        let ratio = Cfg {
            persistent: PersistentNudge::Ratio(0.9),
            ..clm()
        };
        let d = evaluate(&ratio, &mut Ctl::default(), 27648, 0, 3, H);
        let want = prompt::persistent_text(27648, 30720, H);
        assert_eq!(d, Decision::Ok(Some(Nudge::Persistent { text: want })));
        let d = evaluate(&ratio, &mut Ctl::default(), 27647, 0, 3, H);
        assert!(matches!(d, Decision::Ok(Some(Nudge::Tier { .. }))));
    }

    #[test]
    fn over_the_limit_rolls_back_then_final_then_stop() {
        let one = Cfg {
            max_retry: 1,
            ..clm()
        };
        let mut ctl = Ctl::default();
        assert_eq!(evaluate(&one, &mut ctl, 31000, 0, 4, H), Decision::Rollback);
        assert_eq!((ctl.n_retry, ctl.consec), (1, 1));
        // one denominator: the final turn names the enforced limit, not the budget
        let want = prompt::final_text(31000, 30720);
        assert_eq!(
            evaluate(&one, &mut ctl, 31000, 0, 4, H),
            Decision::Final(want)
        );
        assert_eq!(evaluate(&one, &mut ctl, 31000, 0, 4, H), Decision::Stop);
        let mut b = Ctl::default();
        assert!(matches!(
            evaluate(&base(), &mut b, 31000, 0, 4, H),
            Decision::Final(_)
        ));
        assert_eq!(
            evaluate(&base(), &mut b, 30000, 0, 4, H),
            Decision::Ok(None)
        );
        assert!(matches!(
            evaluate(&base(), &mut b, 31000, 0, 4, H),
            Decision::Final(_)
        ));
        // nothing left to roll back: straight to the final turn
        let mut e = Ctl::default();
        assert!(matches!(
            evaluate(&clm(), &mut e, 31000, 0, 0, H),
            Decision::Final(_)
        ));
        // a limit below the rollback margin never rolls back
        let small = Cfg {
            budget: 4000,
            ..clm()
        };
        assert!(matches!(
            evaluate(&small, &mut Ctl::default(), 3000, 0, 4, H),
            Decision::Final(_)
        ));
    }

    #[test]
    fn zero_limit_disables_the_controller() {
        let off = Cfg {
            budget: 1000,
            ..clm()
        };
        assert_eq!(
            evaluate(&off, &mut Ctl::default(), 999_999, 0, 3, H),
            Decision::Ok(None)
        );
    }

    #[test]
    fn exec_results_render_as_output_and_exit_code() {
        let ok = exec_body(
            &json!({"stdout": "hi\n", "stderr": "", "exit_code": 0}).to_string(),
            10000,
        );
        assert_eq!(
            ok,
            ExecBody {
                body: "hi\n\n(exit_code=0)".into(),
                rc: Some(0),
                silent: false
            }
        );
        let both = exec_body(
            &json!({"stdout": "out", "stderr": "err\n", "exit_code": 2}).to_string(),
            10000,
        );
        assert_eq!(both.body, "out\nerr\n\n(exit_code=2)");
        let quiet = exec_body(
            &json!({"stdout": "", "stderr": "", "exit_code": 0}).to_string(),
            10000,
        );
        assert!(quiet.silent);
        assert_eq!(
            quiet.body,
            format!("{}\n\n(exit_code=0)", prompt::NO_OUTPUT)
        );
        let timed = exec_body(
            &json!({"stdout": "", "stderr": "", "exit_code": null, "timed_out": true}).to_string(),
            10000,
        );
        assert_eq!(timed.rc, Some(124));
        let killed = exec_body(
            &json!({"stdout": "", "stderr": "", "exit_code": null}).to_string(),
            10000,
        );
        assert_eq!(killed.rc, Some(-1));
        let long = exec_body(
            &json!({"stdout": "x".repeat(20000), "stderr": "", "exit_code": 1}).to_string(),
            10000,
        );
        assert!(long.body.starts_with(prompt::too_long_hint()));
        assert!(long.body.ends_with("(exit_code=1)"));
        let other = exec_body("plain tool text", 10000);
        assert_eq!(
            other,
            ExecBody {
                body: "plain tool text".into(),
                rc: None,
                silent: false
            }
        );
        let no_exit = exec_body(&json!({"stdout": "x"}).to_string(), 10000);
        assert_eq!(no_exit.rc, None);
    }

    #[test]
    fn truncate_observation_keeps_head_and_tail() {
        assert_eq!(truncate_observation("short", 10000), "short");
        let fits = "y".repeat(15000);
        assert_eq!(truncate_observation(&fits, 20000), fits);
        let text = format!("{}{}", "a".repeat(8000), "b".repeat(8000));
        let cut = truncate_observation(&text, 10000);
        assert_eq!(
            cut,
            prompt::observation_cut(&"a".repeat(5000), 6000, &"b".repeat(5000))
        );
        let small = truncate_observation(&text, 1000);
        assert_eq!(
            small,
            prompt::observation_cut(&"a".repeat(500), 15000, &"b".repeat(500))
        );
        let wide = "é".repeat(30);
        assert_eq!(
            truncate_observation(&wide, 10),
            prompt::observation_cut("ééééé", 20, "ééééé")
        );
    }

    #[test]
    fn cap_newest_keeps_fitting_text_and_cuts_the_rest() {
        // the third argument is the enforced limit (budget - reserve)
        assert_eq!(cap_newest("short", 100, 30720), "short");
        let text = format!("{}{}", "a".repeat(5000), "b".repeat(5000));
        let capped = cap_newest(&text, 200, 30720);
        // keep = 200 - 48 = 152 tokens -> 608 chars -> 304 head + 304 tail
        let want = format!(
            "{}\n\n{}\n\n{}",
            "a".repeat(304),
            prompt::cap_note(9392, 30720),
            "b".repeat(304)
        );
        assert_eq!(capped, want);
        // no room at all still keeps 128 tokens: 256 chars of head and of tail
        let starved = cap_newest(&text, -500, 30720);
        let want = format!(
            "{}\n\n{}\n\n{}",
            "a".repeat(256),
            prompt::cap_note(9488, 30720),
            "b".repeat(256)
        );
        assert_eq!(starved, want);
    }

    #[test]
    fn a_limit_equal_to_the_rollback_margin_goes_straight_to_final() {
        // limit = 4096 - 2048 = 2048 = retry_margin: the guard is `limit > margin`
        let edge = Cfg {
            budget: 4096,
            ..clm()
        };
        assert_eq!(edge.limit(), edge.retry_margin);
        let mut ctl = Ctl::default();
        assert!(matches!(
            evaluate(&edge, &mut ctl, 3000, 0, 4, H),
            Decision::Final(_)
        ));
        assert_eq!((ctl.n_retry, ctl.consec), (0, 0));
        // one token of limit more and the same step rolls back
        let above = Cfg {
            budget: 4097,
            ..clm()
        };
        assert_eq!(
            evaluate(&above, &mut Ctl::default(), 3000, 0, 4, H),
            Decision::Rollback
        );
    }

    #[test]
    fn cap_newest_honours_the_room_for_multibyte_text() {
        let room = 200;
        let cases = [
            ("é", 5000),  // 2 bytes
            ("日", 5000), // 3 bytes
            ("🦀", 5000), // 4 bytes
        ];
        for (unit, n) in cases {
            let text = unit.repeat(n);
            let out = cap_newest(&text, room, 30720);
            assert!(out.len() < text.len(), "{unit}: not cut");
            // the same bound the fit check uses, the cap note included
            assert!(
                (out.len() as i64 + 1) / 4 <= room,
                "{unit}: {} bytes exceed the room",
                out.len()
            );
            assert!(out.contains("characters removed"), "{unit}: no cap note");
            assert!(out.starts_with(unit) && out.ends_with(unit));
        }
        // mixed widths cut mid-sequence of the byte target still land on boundaries
        let mixed: String = "aé日🦀".repeat(2000);
        let out = cap_newest(&mixed, room, 30720);
        assert!((out.len() as i64 + 1) / 4 <= room);
        // the starved floor (512 bytes of text) holds for wide chars too
        let out = cap_newest(&"日".repeat(5000), -500, 30720);
        assert!(out.len() <= 512 + 4 + prompt::cap_note(15000, 30720).len());
    }
}
