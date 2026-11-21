//! Building blocks of the stdio runtime: the child's environment, the
//! stderr ring and the restart policy.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;
use tokio::time::Instant;

use crate::config::StdioEntry;
use crate::refs::Resolved;
use crate::server::{Backoff, Timings};
use crate::translate::scrub;

/// The only names a stdio child inherits from the worker's environment.
#[cfg(unix)]
pub const BASE_ENV: &[&str] = &[
    "HOME", "LOGNAME", "PATH", "SHELL", "TERM", "USER", "LANG", "LC_ALL", "TMPDIR",
];
#[cfg(windows)]
pub const BASE_ENV: &[&str] = &[
    "APPDATA",
    "HOMEDRIVE",
    "HOMEPATH",
    "LOCALAPPDATA",
    "PATH",
    "PROCESSOR_ARCHITECTURE",
    "SYSTEMDRIVE",
    "SYSTEMROOT",
    "TEMP",
    "USERNAME",
    "USERPROFILE",
];

/// The child's whole environment, sorted by name: the `BASE_ENV` names the
/// parent has, then the entry's own `env` over them. A name the entry sets
/// itself is passed as written, `III_*` included.
pub fn child_env(
    parent: &dyn Fn(&str) -> Option<String>,
    entry: &StdioEntry,
    resolved: &Resolved,
) -> Vec<(String, String)> {
    let mut env: BTreeMap<String, String> = BASE_ENV
        .iter()
        .filter_map(|k| parent(k).map(|v| (k.to_string(), v)))
        .collect();
    for (k, v) in &entry.env {
        env.insert(k.clone(), resolved.text(v));
    }
    env.into_iter().collect()
}

const RING_LINES: usize = 500;
const LINE_BYTES: usize = 4096;

/// One stderr line, already redacted when stored.
#[derive(Debug, Serialize, JsonSchema, Clone)]
pub struct LogLine {
    pub ts: u64,
    pub text: String,
}

pub struct StderrRing {
    lines: VecDeque<LogLine>,
    dropped: u64,
}

impl Default for StderrRing {
    fn default() -> Self {
        Self::new()
    }
}

impl StderrRing {
    pub fn new() -> Self {
        StderrRing {
            lines: VecDeque::with_capacity(RING_LINES),
            dropped: 0,
        }
    }

    /// Redacts first: a cut can leave the head of a secret that no longer matches.
    pub fn push(&mut self, ts: u64, line: &str, secrets: &[String]) {
        let mut text = scrub(line, secrets);
        text.truncate(text.floor_char_boundary(LINE_BYTES));
        if self.lines.len() == RING_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(LogLine { ts, text });
    }

    pub fn snapshot(&self) -> (Vec<LogLine>, u64) {
        (self.lines.iter().cloned().collect(), self.dropped)
    }

    pub fn last(&self) -> Option<String> {
        self.lines.back().map(|l| l.text.clone())
    }
}

pub struct RestartPolicy {
    backoff: Backoff,
    failed: VecDeque<Instant>,
    budget_starts: u32,
    budget_window: Duration,
    cooldown: Duration,
}

impl RestartPolicy {
    pub fn new(t: &Timings) -> Self {
        RestartPolicy {
            backoff: Backoff::new(t.backoff_min, t.backoff_max),
            failed: VecDeque::new(),
            budget_starts: t.budget_starts,
            budget_window: t.budget_window,
            cooldown: t.cooldown,
        }
    }

    /// The wait before the next start: the backoff while the budget holds,
    /// the cooldown once `budget_starts` failed starts fall inside the window.
    pub fn on_failed_start(&mut self, now: Instant) -> Duration {
        while self
            .failed
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) > self.budget_window)
        {
            self.failed.pop_front();
        }
        self.failed.push_back(now);
        let wait = self.backoff.next();
        if self.failed.len() >= self.budget_starts as usize {
            self.cooldown
        } else {
            wait
        }
    }

    pub fn on_stable(&mut self) {
        self.reset();
    }

    pub fn reset(&mut self) {
        self.backoff.reset();
        self.failed.clear();
    }

    pub fn restarts(&self) -> u32 {
        (self.failed.len() as u32).min(self.budget())
    }

    pub fn budget(&self) -> u32 {
        self.budget_starts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StdioEntry;
    use crate::refs::{Resolved, Text};
    use crate::server::Timings;
    use std::time::Duration;
    use tokio::time::Instant;

    fn entry(env: &[(&str, Text)]) -> StdioEntry {
        StdioEntry {
            command: "x".into(),
            args: vec![],
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            cwd: None,
        }
    }

    fn resolved(pairs: &[(&str, &str)]) -> Resolved {
        Resolved {
            values: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn child_env_is_base_then_entry_and_inherits_nothing_else() {
        let parent = |k: &str| match k {
            "PATH" => Some("/usr/bin".to_string()),
            "HOME" => Some("/home/u".to_string()),
            "III_URL" => Some("ws://127.0.0.1:49134".to_string()),
            "AWS_SECRET_ACCESS_KEY" => Some("leak".to_string()),
            _ => None,
        };
        let e = entry(&[
            ("FOO", Text::Literal("bar".into())),
            ("GITHUB_TOKEN", Text::parse("secret://GH")),
            ("PATH", Text::Literal("/opt/bin".into())),
            ("III_FOO", Text::Literal("1".into())),
        ]);
        let got = child_env(&parent, &e, &resolved(&[("secret://GH", "tok")]));
        let want: Vec<(String, String)> = [
            ("FOO", "bar"),
            ("GITHUB_TOKEN", "tok"),
            ("HOME", "/home/u"),
            ("III_FOO", "1"),
            ("PATH", "/opt/bin"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn ring_keeps_the_last_500_lines_and_counts_the_dropped() {
        let mut r = StderrRing::new();
        assert_eq!(r.last(), None);
        for i in 0..503u64 {
            r.push(i, &format!("l{i}"), &[]);
        }
        let (lines, dropped) = r.snapshot();
        assert_eq!((lines.len(), dropped), (500, 3));
        assert_eq!((lines[0].ts, lines[0].text.as_str()), (3, "l3"));
        assert_eq!(r.last(), Some("l502".to_string()));
    }

    #[test]
    fn ring_redacts_before_it_cuts() {
        let mut r = StderrRing::new();
        let line = format!("{}s3cr3t-value", "a".repeat(4090));
        r.push(0, &line, &["s3cr3t-value".to_string()]);
        let last = r.last().unwrap();
        assert!(!last.contains("s3cr3t"));
        assert!(last.len() <= 4096);
        assert!(last.ends_with("[redac"));
    }

    #[test]
    fn ring_cuts_on_a_char_boundary() {
        let mut r = StderrRing::new();
        r.push(0, &format!("xx{}", "€".repeat(2000)), &[]);
        let last = r.last().unwrap();
        assert_eq!(last.len(), 4094);
        assert_eq!(last.chars().count(), 1366);
    }

    #[test]
    fn ring_redaction_edge_cases() {
        let mut r = StderrRing::new();
        r.push(0, "abc", &[String::new()]);
        assert_eq!(r.last().as_deref(), Some("abc"));
        r.push(
            0,
            "x abcdef y abc",
            &["abc".to_string(), "abcdef".to_string()],
        );
        assert_eq!(r.last().as_deref(), Some("x [redacted] y [redacted]"));
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn budget_of_five_then_cooldown() {
        let mut p = RestartPolicy::new(&Timings::default());
        let t0 = Instant::now();
        let got: Vec<_> = (0..4).map(|n| p.on_failed_start(t0 + secs(n))).collect();
        assert_eq!(got, [secs(1), secs(2), secs(4), secs(8)]);
        assert_eq!((p.restarts(), p.budget()), (4, 5));
        assert_eq!(p.on_failed_start(t0 + secs(4)), secs(300));
        assert_eq!(p.on_failed_start(t0 + secs(304)), secs(300));
        assert_eq!(p.restarts(), 5);
    }

    #[test]
    fn failures_older_than_the_window_stop_counting() {
        let mut p = RestartPolicy::new(&Timings::default());
        let t0 = Instant::now();
        for n in 0..4 {
            p.on_failed_start(t0 + secs(n));
        }
        assert_eq!(p.on_failed_start(t0 + secs(604)), secs(16));
        assert_eq!(p.restarts(), 1);
    }

    #[test]
    fn backoff_doubles_to_sixty_seconds() {
        let t = Timings {
            budget_starts: 100,
            ..Timings::default()
        };
        let mut p = RestartPolicy::new(&t);
        let t0 = Instant::now();
        let got: Vec<_> = (0..8).map(|n| p.on_failed_start(t0 + secs(n))).collect();
        let want: Vec<_> = [1, 2, 4, 8, 16, 32, 60, 60].map(secs).into();
        assert_eq!(got, want);
    }

    #[test]
    fn stable_and_reset_restore_backoff_and_budget() {
        let calls: [fn(&mut RestartPolicy); 2] = [RestartPolicy::on_stable, RestartPolicy::reset];
        for call in calls {
            let mut p = RestartPolicy::new(&Timings::default());
            let t0 = Instant::now();
            for n in 0..5 {
                p.on_failed_start(t0 + secs(n));
            }
            call(&mut p);
            assert_eq!(p.restarts(), 0);
            assert_eq!(p.on_failed_start(t0 + secs(6)), secs(1));
        }
    }
}
