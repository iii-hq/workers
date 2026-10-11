//! Budget controller: the token ruler, tier/persistent nudges, rollback of
//! the newest steps, the final turn and the stop, plus the caps applied to
//! tool output. Behaviour follows the CLM spike (`clm/src/controller.ts`);
//! every model-facing string comes from `crate::prompt`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{ClmConfig, EditGate, PersistentNudge};
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
    pub fn from_config(c: &ClmConfig, budget_override: Option<u64>) -> Cfg {
        Cfg {
            budget: budget_override.unwrap_or(c.budget_tokens),
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
}
