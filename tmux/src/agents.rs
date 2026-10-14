//! Which agent CLI runs in a pane, and what it is doing.
//!
//! Presence comes from the pane's process tree. State comes from the pane's
//! bottom lines (`capture-pane`): a waiting pattern (a permission or choice
//! prompt) wins, then a working pattern (Claude Code prints `esc to
//! interrupt` only while a turn runs); an agent without working patterns
//! counts as working while its screen keeps changing.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use regex::{Regex, RegexBuilder};

use crate::config::AgentMatcher;
use crate::model::AgentState;
use crate::procs::ProcTable;

/// How deep below the pane's process an agent is looked for (`zsh → node → codex`).
const SEARCH_DEPTH: usize = 6;
/// Bottom lines of the pane the patterns are matched against.
const TAIL_LINES: usize = 12;

pub fn compile_pattern(pattern: &str) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(1 << 20)
        .build()
        .map_err(|error| format!("invalid pattern '{pattern}': {error}"))
}

#[derive(Debug)]
pub struct CompiledAgent {
    pub id: String,
    pub label: String,
    process: Vec<String>,
    working: Vec<Regex>,
    waiting: Vec<Regex>,
}

impl CompiledAgent {
    /// The agent's own names (matcher id, label, process names), which as a
    /// pane title say nothing about the session.
    fn is_own_name(&self, text: &str) -> bool {
        text.eq_ignore_ascii_case(&self.id)
            || text.eq_ignore_ascii_case(&self.label)
            || self
                .process
                .iter()
                .any(|name| text.eq_ignore_ascii_case(name))
    }
}

/// A status glyph agents put in front of their title: Claude Code's ✳ when
/// idle and a braille spinner while working, or any other non-ASCII symbol,
/// `*` or whitespace.
fn is_status_glyph(c: char) -> bool {
    c.is_whitespace() || c == '*' || (!c.is_ascii() && !c.is_alphanumeric())
}

/// The session title an agent set on its pane, or `None` when the pane only
/// shows tmux's default title (one of `host_names`) or the agent's own name.
pub fn session_title(
    agent: &CompiledAgent,
    pane_title: &str,
    host_names: &[String],
) -> Option<String> {
    let title = pane_title.trim_start_matches(is_status_glyph).trim_end();
    if title.is_empty()
        || agent.is_own_name(title)
        || host_names
            .iter()
            .any(|host| title.eq_ignore_ascii_case(host))
    {
        return None;
    }
    Some(title.to_string())
}

/// This machine's host name and its first label (`box.lan` → `box`): tmux's
/// default pane title is `#{host}`.
pub fn host_names() -> &'static [String] {
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        let mut buffer = [0_u8; 256];
        // SAFETY: gethostname writes at most `len` bytes into the buffer.
        let ok = unsafe {
            libc::gethostname(buffer.as_mut_ptr().cast::<libc::c_char>(), buffer.len()) == 0
        };
        if !ok {
            return Vec::new();
        }
        let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
        let full = String::from_utf8_lossy(&buffer[..end]).trim().to_string();
        let mut names = Vec::new();
        if !full.is_empty() {
            if let Some((short, _)) = full.split_once('.') {
                names.push(short.to_string());
            }
            names.push(full);
        }
        names
    })
}

#[derive(Debug, Default)]
pub struct AgentSet {
    agents: Vec<CompiledAgent>,
}

impl AgentSet {
    pub fn compile(matchers: &[AgentMatcher]) -> Result<Self, String> {
        let compile_all = |patterns: &[String]| -> Result<Vec<Regex>, String> {
            patterns
                .iter()
                .map(|pattern| compile_pattern(pattern))
                .collect()
        };
        let agents = matchers
            .iter()
            .map(|matcher| {
                Ok(CompiledAgent {
                    id: matcher.id.clone(),
                    label: matcher.label.clone(),
                    process: matcher
                        .process
                        .iter()
                        .map(|name| name.trim().to_string())
                        .filter(|name| !name.is_empty())
                        .collect(),
                    working: compile_all(&matcher.working_patterns)?,
                    waiting: compile_all(&matcher.waiting_patterns)?,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Self { agents })
    }

    pub fn get(&self, index: usize) -> Option<&CompiledAgent> {
        self.agents.get(index)
    }

    /// The shallowest process under `pane_pid` that an agent matcher claims:
    /// `(matcher index, pid)`.
    pub fn find(&self, table: &ProcTable, pane_pid: u32) -> Option<(usize, u32)> {
        for (pid, _) in table.subtree(pane_pid, SEARCH_DEPTH) {
            let Some(proc) = table.get(pid) else {
                continue;
            };
            for (index, agent) in self.agents.iter().enumerate() {
                if proc
                    .names()
                    .any(|name| agent.process.iter().any(|wanted| wanted == name))
                {
                    return Some((index, pid));
                }
            }
        }
        None
    }
}

fn tail(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let keep = lines.len().saturating_sub(TAIL_LINES);
    lines.drain(..keep);
    lines
}

/// The state the screen shows, before the "screen keeps changing" fallback.
pub fn classify(agent: &CompiledAgent, text: &str, changed_recently: bool) -> AgentState {
    let lines = tail(text);
    let any = |patterns: &[Regex]| {
        lines
            .iter()
            .any(|line| patterns.iter().any(|re| re.is_match(line)))
    };
    if any(&agent.waiting) {
        AgentState::Waiting
    } else if any(&agent.working) || (agent.working.is_empty() && changed_recently) {
        AgentState::Working
    } else {
        AgentState::Idle
    }
}

#[derive(Debug, Clone)]
struct Track {
    agent_id: String,
    pid: u32,
    hash: u64,
    changed_at_ms: u64,
    state: AgentState,
    since_ms: u64,
}

/// Per-pane memory between polls: the screen hash (to see change) and when
/// the current state began.
#[derive(Debug, Default)]
pub struct AgentTracker {
    panes: HashMap<String, Track>,
}

impl AgentTracker {
    /// Feed one capture of `pane_id`; returns the state and since when it holds.
    pub fn observe(
        &mut self,
        pane_id: &str,
        agent: &CompiledAgent,
        pid: u32,
        text: &str,
        now_ms: u64,
        busy_window_ms: u64,
    ) -> (AgentState, u64) {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();

        let previous = self
            .panes
            .get(pane_id)
            .filter(|track| track.agent_id == agent.id && track.pid == pid)
            .cloned();
        let changed_at_ms = match &previous {
            Some(track) if track.hash == hash => track.changed_at_ms,
            Some(_) => now_ms,
            // First sight: no evidence of change yet.
            None => 0,
        };
        let changed_recently =
            changed_at_ms != 0 && now_ms.saturating_sub(changed_at_ms) < busy_window_ms;
        let state = classify(agent, text, changed_recently);
        let since_ms = match &previous {
            Some(track) if track.state == state => track.since_ms,
            _ => now_ms,
        };
        self.panes.insert(
            pane_id.to_string(),
            Track {
                agent_id: agent.id.clone(),
                pid,
                hash,
                changed_at_ms,
                state,
                since_ms,
            },
        );
        (state, since_ms)
    }

    /// Forget panes that no longer run an agent.
    pub fn retain(&mut self, live: &HashSet<String>) {
        self.panes.retain(|pane_id, _| live.contains(pane_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_agents;
    use crate::procs::ProcInfo;

    const CLAUDE_WORKING: &str = include_str!("../tests/fixtures/claude-working.txt");
    const CLAUDE_IDLE: &str = include_str!("../tests/fixtures/claude-idle.txt");
    const CLAUDE_PERMISSION: &str = include_str!("../tests/fixtures/claude-permission.txt");

    fn agents() -> AgentSet {
        AgentSet::compile(&default_agents()).unwrap()
    }

    fn proc(pid: u32, ppid: u32, name: &str, argv: &[&str]) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: Some(ppid),
            name: name.to_string(),
            argv: argv.iter().map(|a| a.to_string()).collect(),
            cwd: None,
        }
    }

    #[test]
    fn claude_screens_classify() {
        let set = agents();
        let claude = set.get(0).unwrap();
        assert_eq!(claude.id, "claude");
        assert_eq!(classify(claude, CLAUDE_WORKING, false), AgentState::Working);
        assert_eq!(
            classify(claude, CLAUDE_IDLE, true),
            AgentState::Idle,
            "typing is not working"
        );
        assert_eq!(
            classify(claude, CLAUDE_PERMISSION, false),
            AgentState::Waiting
        );
    }

    /// A phone attached to the session shrinks its panes and cuts the status line.
    fn narrow(text: &str, cols: usize) -> String {
        text.lines()
            .map(|line| line.chars().take(cols).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn narrow_panes_still_classify() {
        let set = agents();
        let claude = set.get(0).unwrap();
        assert_eq!(
            classify(claude, &narrow(CLAUDE_WORKING, 40), false),
            AgentState::Working
        );
        assert_eq!(
            classify(claude, &narrow(CLAUDE_IDLE, 40), false),
            AgentState::Idle
        );
        let codex = set.get(1).unwrap();
        assert_eq!(codex.id, "codex");
        let screen = "• Ran\n  └ pnpm tsc --noEmit · no\n◦ Working (93s • esc to int\n› Ask Codex to do anything";
        assert_eq!(classify(codex, screen, false), AgentState::Working);
        assert_eq!(
            classify(codex, "› Ask Codex to do anything", false),
            AgentState::Idle
        );
    }

    #[test]
    fn finds_the_shallowest_agent_in_the_pane() {
        let table = ProcTable::from_procs([
            proc(10, 1, "zsh", &["-zsh"]),
            proc(
                11,
                10,
                "node",
                &["node", "/usr/lib/node_modules/@openai/codex/bin/codex.js"],
            ),
            proc(12, 11, "codex", &["codex"]),
            proc(20, 1, "zsh", &["-zsh"]),
            proc(
                21,
                20,
                "claude",
                &["claude", "--dangerously-skip-permissions"],
            ),
            proc(30, 1, "zsh", &["-zsh"]),
            proc(31, 30, "vim", &["vim"]),
        ]);
        let set = agents();
        let (index, pid) = set.find(&table, 10).unwrap();
        assert_eq!((set.get(index).unwrap().id.as_str(), pid), ("codex", 11));
        let (index, pid) = set.find(&table, 20).unwrap();
        assert_eq!((set.get(index).unwrap().id.as_str(), pid), ("claude", 21));
        assert!(set.find(&table, 30).is_none());
    }

    #[test]
    fn tracker_keeps_since_while_state_holds() {
        let set = agents();
        let claude = set.get(0).unwrap();
        let mut tracker = AgentTracker::default();
        assert_eq!(
            tracker.observe("%1", claude, 21, CLAUDE_WORKING, 1_000, 2_000),
            (AgentState::Working, 1_000)
        );
        assert_eq!(
            tracker.observe("%1", claude, 21, CLAUDE_WORKING, 2_000, 2_000),
            (AgentState::Working, 1_000)
        );
        assert_eq!(
            tracker.observe("%1", claude, 21, CLAUDE_IDLE, 3_000, 2_000),
            (AgentState::Idle, 3_000)
        );
        // A different agent process in the same pane starts fresh.
        assert_eq!(
            tracker.observe("%1", claude, 99, CLAUDE_IDLE, 4_000, 2_000),
            (AgentState::Idle, 4_000)
        );
    }

    #[test]
    fn agents_without_working_patterns_use_screen_changes() {
        let set = AgentSet::compile(&[AgentMatcher {
            id: "aider".into(),
            label: "Aider".into(),
            process: vec!["aider".into()],
            working_patterns: vec![],
            waiting_patterns: vec![],
        }])
        .unwrap();
        let aider = set.get(0).unwrap();
        let mut tracker = AgentTracker::default();
        assert_eq!(
            tracker.observe("%2", aider, 5, "a", 1_000, 2_000).0,
            AgentState::Idle
        );
        assert_eq!(
            tracker.observe("%2", aider, 5, "b", 2_000, 2_000).0,
            AgentState::Working
        );
        assert_eq!(
            tracker.observe("%2", aider, 5, "b", 3_500, 2_000).0,
            AgentState::Working
        );
        assert_eq!(
            tracker.observe("%2", aider, 5, "b", 4_500, 2_000).0,
            AgentState::Idle
        );
    }

    #[test]
    fn session_titles_drop_status_glyphs_and_defaults() {
        let set = agents();
        let claude = set.get(0).unwrap();
        let hosts = vec!["dev-workstation".to_string()];
        let title = |raw: &str| session_title(claude, raw, &hosts);
        assert_eq!(
            title("✳ Add cursor pagination").as_deref(),
            Some("Add cursor pagination")
        );
        assert_eq!(title("⠂ Fix bug").as_deref(), Some("Fix bug"));
        assert_eq!(
            title("  ✻  Deploy notes  ").as_deref(),
            Some("Deploy notes")
        );
        assert_eq!(
            title("[WIP] keep brackets").as_deref(),
            Some("[WIP] keep brackets")
        );
        assert_eq!(title("dev-workstation"), None);
        assert_eq!(title("DEV-Workstation"), None);
        assert_eq!(title("claude"), None);
        assert_eq!(title("✳ Claude Code"), None);
        assert_eq!(title("✳ "), None);
        assert_eq!(title(""), None);
    }
}
