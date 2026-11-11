//! Codex's usage, from what Codex records (agent-accounts D13): each turn
//! leaves a `token_count` event in the session's
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl` with the account's
//! `rate_limits`. Windows are told apart by `window_minutes` (300 for five
//! hours, 10080 for the week), never by `primary`/`secondary`: on a Pro plan
//! the primary one is the week. `resets_at` is in seconds since the epoch
//! (Codex 0.13x, seen on this host's records); older Codex wrote
//! `resets_in_seconds`, from the event's time. Read-only.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::time::{json_time_ms, parse_rfc3339_ms};
use crate::model::{CodexCredits, CodexUsage, Limit};

/// How many of the newest rollouts are looked through for an event.
const ROLLOUTS_TRIED: usize = 6;
/// How far back each one is read.
const TAIL_LIMIT: u64 = 8 * 1024 * 1024;
const CHUNK: u64 = 256 * 1024;

/// One `token_count` line, if it carries rate limits.
pub fn parse_event(line: &str) -> Option<CodexUsage> {
    if !line.contains("\"rate_limits\"") {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let limits = payload.get("rate_limits").filter(|v| v.is_object())?;
    let as_of_ms = value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_rfc3339_ms)?;
    let mut usage = CodexUsage {
        plan: limits
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        five_hour: None,
        week: None,
        credits: limits.get("credits").and_then(|credits| {
            if credits.get("has_credits").and_then(Value::as_bool) == Some(false)
                && credits.get("unlimited").and_then(Value::as_bool) != Some(true)
            {
                return None;
            }
            Some(CodexCredits {
                balance: match credits.get("balance")? {
                    Value::String(text) => text.clone(),
                    Value::Number(number) => number.to_string(),
                    _ => return None,
                },
                unlimited: credits
                    .get("unlimited")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        }),
        as_of_ms,
    };
    for key in ["primary", "secondary"] {
        let Some(window) = limits.get(key).filter(|v| v.is_object()) else {
            continue;
        };
        let Some(used) = window.get("used_percent").and_then(Value::as_f64) else {
            continue;
        };
        let resets_at_ms = window.get("resets_at").and_then(json_time_ms).or_else(|| {
            let seconds = window.get("resets_in_seconds")?.as_f64()?;
            Some(as_of_ms + (seconds.max(0.0) * 1000.0) as u64)
        });
        let limit = Limit {
            used_percent: used.max(0.0),
            resets_at_ms,
            reset_since_read: false,
        };
        match window.get("window_minutes").and_then(Value::as_u64) {
            Some(300) => usage.five_hour = Some(limit),
            Some(10_080) => usage.week = Some(limit),
            _ => {}
        }
    }
    Some(usage)
}

/// A window whose reset passed after Codex recorded it is empty now.
pub fn settle(usage: &mut CodexUsage, now_ms: u64) {
    for limit in [&mut usage.five_hour, &mut usage.week]
        .into_iter()
        .flatten()
    {
        if limit.resets_at_ms.is_some_and(|at| at <= now_ms) {
            limit.used_percent = 0.0;
            limit.reset_since_read = true;
            limit.resets_at_ms = None;
        }
    }
}

/// The last event with rate limits in `file` (its last `TAIL_LIMIT` bytes).
fn last_event(path: &Path) -> Option<CodexUsage> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut end = len;
    let mut carry: Vec<u8> = Vec::new();
    while end > 0 && len - end < TAIL_LIMIT {
        let start = end.saturating_sub(CHUNK);
        let mut buffer = vec![0; (end - start) as usize];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut buffer).ok()?;
        buffer.extend_from_slice(&carry);
        let first_break = if start == 0 {
            0
        } else {
            match buffer.iter().position(|b| *b == b'\n') {
                Some(at) => at + 1,
                None => {
                    carry = buffer;
                    end = start;
                    continue;
                }
            }
        };
        for line in buffer[first_break..].rsplit(|b| *b == b'\n') {
            if let Some(usage) = std::str::from_utf8(line).ok().and_then(parse_event) {
                return Some(usage);
            }
        }
        carry = buffer[..first_break.saturating_sub(1)].to_vec();
        end = start;
    }
    None
}

/// Sub-folders (or files) of `dir`, newest name first.
fn entries_desc(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    out.sort();
    out.reverse();
    out
}

/// The newest rollouts, by their date folders and then their names (which
/// start with the time they were made); at most `ROLLOUTS_TRIED`.
fn newest_rollouts(sessions: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for year in entries_desc(sessions) {
        for month in entries_desc(&year) {
            for day in entries_desc(&month) {
                let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries_desc(&day)
                    .into_iter()
                    .filter(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| {
                                name.starts_with("rollout-") && name.ends_with(".jsonl")
                            })
                    })
                    .filter_map(|path| {
                        Some((std::fs::metadata(&path).ok()?.modified().ok()?, path))
                    })
                    .collect();
                files.sort_by_key(|file| std::cmp::Reverse(file.0));
                found.extend(files.into_iter().map(|(_, path)| path));
                if found.len() >= ROLLOUTS_TRIED {
                    found.truncate(ROLLOUTS_TRIED);
                    return found;
                }
            }
        }
    }
    found
}

/// Codex's latest usage under `codex_home`. Blocking.
pub fn read(codex_home: &Path) -> Option<CodexUsage> {
    newest_rollouts(&codex_home.join("sessions"))
        .iter()
        .filter_map(|path| last_event(path))
        .max_by_key(|usage| usage.as_of_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line as Codex wrote it on this host (2026-10-23), ids and numbers
    /// changed.
    const PRO_LINE: &str = r#"{"timestamp":"2026-10-23T14:26:30.073Z","ordinal":41,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3},"model_context_window":475000},"rate_limits":{"limit_id":"codex","limit_name":null,"primary":{"used_percent":16.0,"window_minutes":10080,"resets_at":1793303337},"secondary":null,"credits":{"has_credits":true,"unlimited":false,"balance":"62500.0000000000"},"individual_limit":null,"spend_control_reached":null,"plan_type":"pro","rate_limit_reached_type":null}}}"#;

    #[test]
    fn a_pro_plans_week_is_its_primary_window() {
        let usage = parse_event(PRO_LINE).unwrap();
        assert_eq!(usage.plan.as_deref(), Some("pro"));
        assert!(usage.five_hour.is_none());
        let week = usage.week.as_ref().unwrap();
        assert_eq!(week.used_percent, 16.0);
        assert_eq!(week.resets_at_ms, Some(1_793_303_337_000));
        assert_eq!(
            usage.credits,
            Some(CodexCredits {
                balance: "62500.0000000000".into(),
                unlimited: false
            })
        );
        assert_eq!(usage.as_of_ms, 1_792_765_590_073);
    }

    #[test]
    fn windows_go_by_their_length_and_older_fields_read() {
        let line = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":40,"window_minutes":300,"resets_in_seconds":600},"secondary":{"used_percent":9.5,"window_minutes":10080,"resets_at":1767830400},"credits":{"has_credits":false,"unlimited":false,"balance":"0"}}}}"#;
        let usage = parse_event(line).unwrap();
        let five = usage.five_hour.as_ref().unwrap();
        assert_eq!(five.used_percent, 40.0);
        assert_eq!(five.resets_at_ms, Some(1_767_225_600_000 + 600_000));
        assert_eq!(usage.week.as_ref().unwrap().used_percent, 9.5);
        assert!(usage.credits.is_none());
        assert!(parse_event(
            r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":null}}"#
        )
        .is_none());
        assert!(parse_event(r#"{"timestamp":"2026-01-01T00:00:00Z","payload":{"type":"agent_message","rate_limits":{}}}"#).is_none());
    }

    #[test]
    fn the_newest_rollout_with_an_event_wins() {
        let home =
            std::env::temp_dir().join(format!("tmux-codex-{}", uuid::Uuid::new_v4().simple()));
        let day = home.join("sessions/2026/10/23");
        std::fs::create_dir_all(&day).unwrap();
        let older = home.join("sessions/2026/09/30");
        std::fs::create_dir_all(&older).unwrap();
        std::fs::write(
            older.join("rollout-2026-09-30T10-00-00-a.jsonl"),
            PRO_LINE
                .replace("16.0", "3.0")
                .replace("2026-10-23T14:26:30.073Z", "2026-09-30T10:00:00Z")
                + "\n",
        )
        .unwrap();
        let mut body = String::from("{\"type\":\"session_meta\",\"payload\":{\"id\":\"x\"}}\n");
        body.push_str(PRO_LINE);
        body.push('\n');
        body.push_str(r#"{"timestamp":"2026-10-23T14:27:00Z","type":"event_msg","payload":{"type":"agent_message","message":"hi"}}"#);
        body.push('\n');
        std::fs::write(day.join("rollout-2026-10-23T11-25-28-b.jsonl"), body).unwrap();
        // The newest file has no event yet.
        std::fs::write(
            day.join("rollout-2026-10-23T12-00-00-c.jsonl"),
            "{\"type\":\"session_meta\"}\n",
        )
        .unwrap();
        let usage = read(&home).unwrap();
        assert_eq!(usage.week.as_ref().unwrap().used_percent, 16.0);
        let mut settled = usage.clone();
        settle(&mut settled, 1_793_303_337_000);
        assert!(settled.week.as_ref().unwrap().reset_since_read);
        assert!(read(&home.join("nothing")).is_none());
        let _ = std::fs::remove_dir_all(home);
    }
}
