use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use context_manager::config::WorkerConfig;
use context_manager::functions::assemble::{self, AssembleOptions, AssembleRequest};
use context_manager::ports::{lease_cell, Deps};
use context_manager::types::{AgentMessage, ContentBlock, ModelInput, ModelLimits};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::common::fakes::{FakeClock, FakeModelResolver, FakeSummarizer, InMemoryLeaseStore};

pub struct ReplayHistory {
    entries: Vec<ReplayEntry>,
}

struct ReplayEntry {
    id: String,
    revision: u64,
    message: AgentMessage,
}

impl ReplayHistory {
    pub fn message_count(&self) -> usize {
        self.entries.len()
    }

    pub fn ids(&self) -> Vec<&str> {
        self.entries.iter().map(|entry| entry.id.as_str()).collect()
    }

    pub fn revisions(&self) -> Vec<u64> {
        self.entries.iter().map(|entry| entry.revision).collect()
    }

    pub fn text_at(&self, index: usize) -> Option<&str> {
        let entry = self.entries.get(index)?;
        let [ContentBlock::Text { text }] = entry.message.content() else {
            return None;
        };
        Some(text)
    }

    pub fn user_turn_endpoints(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                (entry.message.role() == context_manager::types::Role::User
                    && !entry.message.has_function_result_block())
                .then_some(index)
            })
            .collect()
    }

    fn prefix_at(&self, endpoint: usize) -> Vec<AgentMessage> {
        self.entries[..=endpoint]
            .iter()
            .map(|entry| entry.message.clone())
            .collect()
    }
}

#[derive(Clone, Copy)]
pub struct EndpointEstimate {
    pub baseline_tokens: u64,
    pub decay_tokens: u64,
}

pub struct ReplayComparison {
    estimates: Vec<EndpointEstimate>,
}

pub struct ReplayReport {
    sessions: Vec<ReplaySession>,
}

struct ReplaySession {
    name: String,
    messages: usize,
    comparison: ReplayComparison,
}

impl ReplayComparison {
    pub fn turn_count(&self) -> usize {
        self.estimates.len()
    }

    pub fn estimates(&self) -> &[EndpointEstimate] {
        &self.estimates
    }

    pub fn final_baseline_tokens(&self) -> u64 {
        self.estimates
            .last()
            .map(|estimate| estimate.baseline_tokens)
            .unwrap_or(0)
    }

    pub fn final_decay_tokens(&self) -> u64 {
        self.estimates
            .last()
            .map(|estimate| estimate.decay_tokens)
            .unwrap_or(0)
    }
}

pub fn parse_session_lines<'a>(
    path: &str,
    lines: impl IntoIterator<Item = &'a str>,
) -> Result<ReplayHistory, String> {
    let mut entries = Vec::new();
    let mut by_id = HashMap::new();

    for (offset, line) in lines.into_iter().enumerate() {
        let line_number = offset + 1;
        let record: Value = serde_json::from_str(line)
            .map_err(|_| format!("{path}:{line_number}: invalid JSON record"))?;
        let record = record
            .as_object()
            .ok_or_else(|| format!("{path}:{line_number}: record is not an object"))?;
        let record_type = required_string(record, "type", path, line_number)?;
        match record_type {
            "meta" | "leaf" => {}
            "entry" => {
                let entry = required_object(record, "entry", path, line_number)?;
                let kind = required_string(entry, "kind", path, line_number)?;
                if kind != "message" {
                    continue;
                }
                let id = required_string(entry, "id", path, line_number)?.to_owned();
                if id.is_empty() {
                    return Err(format!("{path}:{line_number}: message entry id is empty"));
                }
                let revision = entry
                    .get("revision")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        format!("{path}:{line_number}: message entry revision is invalid")
                    })?;
                let raw_message = entry.get("message").ok_or_else(|| {
                    format!("{path}:{line_number}: message entry message is missing")
                })?;
                let message = serde_json::from_value(raw_message.clone()).map_err(|_| {
                    format!("{path}:{line_number}: message entry message is invalid")
                })?;
                let next = ReplayEntry {
                    id: id.clone(),
                    revision,
                    message,
                };
                if let Some(index) = by_id.get(&id) {
                    entries[*index] = next;
                } else {
                    by_id.insert(id, entries.len());
                    entries.push(next);
                }
            }
            _ => return Err(format!("{path}:{line_number}: unsupported record type")),
        }
    }

    Ok(ReplayHistory { entries })
}

pub async fn compare_decay_four(history: &ReplayHistory) -> Result<ReplayComparison, String> {
    let mut estimates = Vec::new();
    for endpoint in history.user_turn_endpoints() {
        let original_prefix = history.prefix_at(endpoint);
        let baseline = assemble_prefix(original_prefix.clone(), 0).await?;
        let decay = assemble_prefix(original_prefix, 4).await?;
        estimates.push(EndpointEstimate {
            baseline_tokens: baseline.token_count,
            decay_tokens: decay.token_count,
        });
    }
    Ok(ReplayComparison { estimates })
}

pub async fn compare_directory(directory: &Path) -> Result<ReplayReport, String> {
    let mut paths = session_paths(directory)?;
    paths.sort();
    let mut sessions = Vec::with_capacity(paths.len());
    for path in paths {
        let history = read_history(&path)?;
        let comparison = compare_decay_four(&history).await?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{}: file name is not valid UTF-8", path.display()))?
            .to_owned();
        sessions.push(ReplaySession {
            name,
            messages: history.message_count(),
            comparison,
        });
    }
    Ok(ReplayReport { sessions })
}

impl ReplayReport {
    pub fn render(&self) -> String {
        let mut output = String::new();
        writeln!(
            output,
            "Offline context::assemble replay (assembled-history estimates)"
        )
        .unwrap();
        writeln!(output, "config: defaults except decay_user_turns=0 versus 4; inline 1000000/1 limits; no prompt/tools/overhead; compaction disabled").unwrap();
        writeln!(output, "session\tmessages\tuser_turns\tlast_quarter_endpoints\tfinal_0\tfinal_4\tfinal_saved\tfinal_saved_pct\tlast_quarter_mean_0\tlast_quarter_mean_4\tmean_saved\tmean_saved_pct").unwrap();

        let mut message_count = 0usize;
        let mut turn_count = 0usize;
        let mut sample_count = 0usize;
        let mut final_baseline = 0u64;
        let mut final_decay = 0u64;
        let mut sample_baseline = 0u64;
        let mut sample_decay = 0u64;
        for session in &self.sessions {
            let summary = summary(&session.comparison);
            message_count += session.messages;
            turn_count += summary.turn_count;
            sample_count += summary.sample_count;
            final_baseline = final_baseline.saturating_add(summary.final_baseline);
            final_decay = final_decay.saturating_add(summary.final_decay);
            sample_baseline = sample_baseline.saturating_add(summary.sample_baseline);
            sample_decay = sample_decay.saturating_add(summary.sample_decay);
            writeln!(
                output,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}",
                session.name,
                session.messages,
                summary.turn_count,
                summary.sample_count,
                summary.final_baseline,
                summary.final_decay,
                delta(summary.final_baseline, summary.final_decay),
                percentage(
                    delta(summary.final_baseline, summary.final_decay),
                    summary.final_baseline
                ),
                mean(summary.sample_baseline, summary.sample_count),
                mean(summary.sample_decay, summary.sample_count),
                mean_delta(
                    summary.sample_baseline,
                    summary.sample_decay,
                    summary.sample_count
                ),
                percentage(
                    delta(summary.sample_baseline, summary.sample_decay),
                    summary.sample_baseline
                ),
            )
            .unwrap();
        }
        writeln!(
            output,
            "aggregate_final\tfiles={} messages={} user_turns={}\tbaseline={}\tdecay4={}\tsaved={}\tsaved_pct={:.2}",
            self.sessions.len(),
            message_count,
            turn_count,
            final_baseline,
            final_decay,
            delta(final_baseline, final_decay),
            percentage(delta(final_baseline, final_decay), final_baseline),
        )
        .unwrap();
        writeln!(
            output,
            "aggregate_last_quarter\tendpoints={}\tbaseline_total={}\tdecay4_total={}\tsaved_total={}\tsaved_pct={:.2}\tmean_0={:.2}\tmean_4={:.2}",
            sample_count,
            sample_baseline,
            sample_decay,
            delta(sample_baseline, sample_decay),
            percentage(delta(sample_baseline, sample_decay), sample_baseline),
            mean(sample_baseline, sample_count),
            mean(sample_decay, sample_count),
        )
        .unwrap();
        output
    }
}

struct ComparisonSummary {
    turn_count: usize,
    sample_count: usize,
    final_baseline: u64,
    final_decay: u64,
    sample_baseline: u64,
    sample_decay: u64,
}

fn summary(comparison: &ReplayComparison) -> ComparisonSummary {
    let estimates = comparison.estimates();
    let sample_count = estimates.len().div_ceil(4);
    let sampled = if sample_count == 0 {
        &[]
    } else {
        &estimates[estimates.len() - sample_count..]
    };
    ComparisonSummary {
        turn_count: estimates.len(),
        sample_count,
        final_baseline: comparison.final_baseline_tokens(),
        final_decay: comparison.final_decay_tokens(),
        sample_baseline: sampled
            .iter()
            .map(|estimate| estimate.baseline_tokens)
            .sum(),
        sample_decay: sampled.iter().map(|estimate| estimate.decay_tokens).sum(),
    }
}

fn mean(total: u64, count: usize) -> f64 {
    if count == 0 {
        0.0
    } else {
        total as f64 / count as f64
    }
}

fn mean_delta(baseline: u64, decay: u64, count: usize) -> f64 {
    if count == 0 {
        0.0
    } else {
        delta(baseline, decay) as f64 / count as f64
    }
}

fn delta(baseline: u64, decay: u64) -> i128 {
    i128::from(baseline) - i128::from(decay)
}

fn percentage(saved: i128, baseline: u64) -> f64 {
    if baseline == 0 {
        0.0
    } else {
        saved as f64 * 100.0 / baseline as f64
    }
}

fn session_paths(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(directory)
        .map_err(|_| format!("{}: cannot read input directory", directory.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|_| format!("{}: cannot read directory entry", directory.display()))?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
        {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        return Err(format!(
            "{}: no JSONL session files found",
            directory.display()
        ));
    }
    Ok(paths)
}

fn read_history(path: &Path) -> Result<ReplayHistory, String> {
    let file =
        fs::File::open(path).map_err(|_| format!("{}: cannot open input file", path.display()))?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for (offset, line) in reader.lines().enumerate() {
        lines.push(
            line.map_err(|_| format!("{}:{}: cannot read input line", path.display(), offset + 1))?,
        );
    }
    parse_session_lines(
        path.to_string_lossy().as_ref(),
        lines.iter().map(String::as_str),
    )
}

async fn assemble_prefix(
    messages: Vec<AgentMessage>,
    decay_user_turns: usize,
) -> Result<assemble::AssembleResponse, String> {
    let config = WorkerConfig {
        decay_user_turns,
        ..WorkerConfig::default()
    };
    let deps = replay_deps(config);
    let response = assemble::handle(
        &deps,
        AssembleRequest {
            messages: Some(messages),
            model: ModelInput {
                id: "offline-replay".to_string(),
                provider: None,
                limits: Some(ModelLimits {
                    context_window: 1_000_000,
                    max_output_tokens: 1,
                    input_limit: None,
                }),
            },
            system_prompt: None,
            tools: None,
            parts: None,
            options: Some(AssembleOptions {
                allow_compaction: Some(false),
                ..AssembleOptions::default()
            }),
        },
    )
    .await
    .map_err(|error| format!("assemble failed: {error}"))?;

    if response.applied.compacted || response.applied.initial_token_count > response.usable {
        return Err("replay budget allowed compaction or emergency reduction".to_string());
    }
    Ok(response)
}

fn replay_deps(config: WorkerConfig) -> Deps {
    let leases = Arc::new(InMemoryLeaseStore::new());
    Deps {
        config: Arc::new(RwLock::new(Arc::new(config))),
        resolver: Arc::new(FakeModelResolver::new()),
        summarizer: Arc::new(FakeSummarizer::new()),
        leases: lease_cell(leases),
        clock: Arc::new(FakeClock::new()),
    }
}

fn required_object<'a>(
    value: &'a serde_json::Map<String, Value>,
    field: &str,
    path: &str,
    line: usize,
) -> Result<&'a serde_json::Map<String, Value>, String> {
    value
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{path}:{line}: {field} is missing or invalid"))
}

fn required_string<'a>(
    value: &'a serde_json::Map<String, Value>,
    field: &str,
    path: &str,
    line: usize,
) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{path}:{line}: {field} is missing or invalid"))
}
