//! Evidence selection: ask the judge about each admitted file's
//! declarations and keep the verbatim source it selects.
//!
//! Ported from dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit
//! 82ef1fd: `packages/core/src/selection.ts` `selectFile` and the first
//! selection pass of `retrieve.ts` (518-594), over every file.
//!
//! Deviations: one read per file, re-hashed before its excerpts are
//! emitted, instead of a freshness check before every judge attempt; a
//! group the judge finds too large, or over a known window, is halved like
//! one over the state cap; files run concurrently up to the worker's judge
//! slots.

use std::collections::HashMap;
use std::sync::Arc;

use super::navigate::{Candidate, Run};
use super::prompts::{self, Declaration, SourceRange};
use super::units::{self, Inspection, MAX_PARSE_BYTES};
use super::walk::{self, Snapshot, Unit};
use super::{ByteSpan, CallLead, Excerpt, Lead};
use crate::code::judge::JudgeError;

/// Units, context lines and owner headers above this are never shown whole.
const SOURCE_UNIT_BYTES: usize = 24_000;
/// Chunk size when the file has no declarations.
const TEXT_UNIT_BYTES: usize = 3_000;
const BLOCK_LINES: usize = 16;
const GROUP_UNITS: usize = 128;
const GROUP_BYTES: usize = 42_000;
/// Files up to this size go to the judge whole as context.
const WHOLE_FILE_BYTES: usize = 16_000;
const OPENING_LINES: usize = 20;
const CONTEXT_LINES: usize = 8;
const EXCERPT_LINES: usize = 3;
/// jevgrep's evidence state cap.
pub const MAX_STATE_BYTES: usize = 80_000;
/// Selection, lead and presentation thresholds (strict).
const SELECT: f64 = 0.5;
const LEAD: f64 = 0.25;
const PRESENT: f64 = 0.7;

/// What selection found in one file (jevgrep `FileEvidence`).
#[derive(Debug, Default, Clone)]
pub struct Selected {
    /// Presentation: the selected source of decisions above 0.7.
    pub excerpts: Vec<Excerpt>,
    /// All selected source; a test file shows this instead.
    pub selected_excerpts: Vec<Excerpt>,
    pub leads: Vec<Lead>,
    /// Possible local calls from the shown source (Python).
    pub call_leads: Vec<CallLead>,
    pub source_omitted: bool,
    /// The whole-line selected ranges (jevgrep `selected` without byte
    /// spans), for the Python call pass.
    pub selected_lines: Vec<SourceRange>,
}

impl Selected {
    pub fn omitted() -> Self {
        Self {
            source_omitted: true,
            ..Self::default()
        }
    }
}

/// A byte span of the source.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    start: usize,
    end: usize,
}

/// jevgrep `EvidenceRange`: lines, plus the byte span when it does not
/// cover whole lines.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Range {
    start_line: usize,
    end_line: usize,
    bytes: Option<Span>,
}

fn merge(mut spans: Vec<Span>) -> Vec<Span> {
    spans.retain(|s| s.end > s.start);
    spans.sort_by_key(|s| (s.start, s.end));
    let mut merged: Vec<Span> = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
            _ => merged.push(span),
        }
    }
    merged
}

/// One rendered excerpt.
struct Block {
    range: Range,
    text: String,
}

impl Block {
    fn excerpt(&self) -> Excerpt {
        Excerpt {
            line_from: self.range.start_line as u32,
            line_to: self.range.end_line as u32,
            text: self.text.clone(),
            partial: self.range.bytes.map(|span| ByteSpan {
                byte_from: span.start as u32,
                byte_to: span.end as u32,
            }),
        }
    }
}

/// One file's lines and syntax, in the byte coordinates of its snapshot.
struct Source {
    snapshot: Snapshot,
    /// Each line's bytes, without its `\n`.
    lines: Vec<Span>,
    /// Line start offsets clamped to the source length; `lines + 1` long.
    offsets: Vec<usize>,
    syntax: Inspection,
    giant_line: bool,
}

impl Source {
    fn new(snapshot: Snapshot) -> Self {
        let len = snapshot.source.len();
        let (mut lines, mut offsets) = (Vec::new(), vec![0]);
        for line in snapshot.source.split('\n') {
            let start = offsets[offsets.len() - 1];
            lines.push(Span {
                start,
                end: start + line.len(),
            });
            offsets.push(len.min(start + line.len() + 1));
        }
        let giant_line = lines.iter().any(|l| l.end - l.start > SOURCE_UNIT_BYTES);
        let max_unit_bytes = if giant_line {
            SOURCE_UNIT_BYTES
        } else {
            SOURCE_UNIT_BYTES.max(len)
        };
        let syntax = units::inspect(
            &snapshot.path,
            &snapshot.source,
            max_unit_bytes,
            MAX_PARSE_BYTES,
        );
        Self {
            snapshot,
            lines,
            offsets,
            syntax,
            giant_line,
        }
    }

    fn len(&self) -> usize {
        self.snapshot.source.len()
    }

    fn offset(&self, index: usize) -> usize {
        self.offsets.get(index).copied().unwrap_or(self.len())
    }

    /// `lines.slice(start - 1, end).join("\n")`.
    fn lines_text(&self, start: usize, end: usize) -> &str {
        let end = end.min(self.lines.len());
        if start == 0 || start > end {
            return "";
        }
        &self.snapshot.source[self.lines[start - 1].start..self.lines[end - 1].end]
    }

    fn line_len(&self, line: usize) -> usize {
        self.lines[line - 1].end - self.lines[line - 1].start
    }

    fn line_at(&self, byte: usize) -> usize {
        let (mut low, mut high) = (0, self.lines.len());
        while low + 1 < high {
            let middle = (low + high) / 2;
            if self.offsets[middle] <= byte {
                low = middle;
            } else {
                high = middle;
            }
        }
        low + 1
    }

    fn range_for(&self, span: Span) -> Range {
        let start_line = self.line_at(span.start);
        let end_line = self.line_at(span.start.max(span.end.saturating_sub(1)));
        let whole = span.start == self.offset(start_line - 1) && span.end == self.offset(end_line);
        Range {
            start_line,
            end_line,
            bytes: (!whole).then_some(span),
        }
    }

    fn partial_line(&self, unit: &Unit) -> bool {
        unit.byte_start != self.offset(unit.start_line - 1)
            || unit.byte_end != self.offset(unit.end_line)
    }

    /// The units the judge is asked about: `source` chunks when nothing
    /// parsed, and 16-line blocks of any unit over [`SOURCE_UNIT_BYTES`].
    /// Giant lines keep the parser's byte-bounded spans.
    fn units(&self) -> Vec<Unit> {
        let mut units = self.syntax.units.clone();
        if self.giant_line {
            return units;
        }
        if self.syntax.text || units.iter().all(|u| u.partial) {
            units = walk::split_source(&self.snapshot.source, TEXT_UNIT_BYTES);
            for unit in &mut units {
                unit.end_line = self.line_at(unit.byte_start.max(unit.byte_end.saturating_sub(1)));
            }
        }
        units
            .into_iter()
            .flat_map(|unit| {
                if self.lines_text(unit.start_line, unit.end_line).len() <= SOURCE_UNIT_BYTES {
                    return vec![unit];
                }
                (unit.start_line..=unit.end_line)
                    .step_by(BLOCK_LINES)
                    .map(|start| {
                        let end = unit.end_line.min(start + BLOCK_LINES - 1);
                        Unit {
                            name: unit.name.clone(),
                            start_line: start,
                            end_line: end,
                            byte_start: self.offset(start - 1),
                            byte_end: self.offset(end),
                            partial: true,
                            owner_headers: Vec::new(),
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn groups(&self, units: Vec<Unit>) -> Vec<Vec<Unit>> {
        let (mut groups, mut pending) = (Vec::new(), Vec::<Unit>::new());
        for unit in units {
            if !pending.is_empty()
                && (pending.len() >= GROUP_UNITS
                    || self.lines_text(pending[0].start_line, unit.end_line).len() > GROUP_BYTES)
            {
                groups.push(std::mem::take(&mut pending));
            }
            pending.push(unit);
        }
        if !pending.is_empty() {
            groups.push(pending);
        }
        groups
    }

    /// The source the judge reads for `group`: the whole file when small,
    /// else the opening lines and the group's lines ±8; a partial or giant
    /// line sends only the units' own byte spans.
    fn context(&self, group: &[Unit]) -> String {
        let first = group[0].start_line.saturating_sub(CONTEXT_LINES).max(1);
        let last = self
            .lines
            .len()
            .min(group[group.len() - 1].end_line + CONTEXT_LINES);
        let giant = |from: usize, to: usize| {
            (from..=to.min(self.lines.len())).any(|l| self.line_len(l) > SOURCE_UNIT_BYTES)
        };
        if group.iter().any(|u| self.partial_line(u))
            || giant(1, OPENING_LINES)
            || giant(first, last)
        {
            return group
                .iter()
                .map(|u| {
                    format!(
                        "Source lines {}-{}; source bytes {}-{}:\n{}",
                        u.start_line,
                        u.end_line,
                        u.byte_start,
                        u.byte_end,
                        String::from_utf8_lossy(
                            &self.snapshot.source.as_bytes()[u.byte_start..u.byte_end]
                        )
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        if self.len() <= WHOLE_FILE_BYTES {
            return self.snapshot.source.clone();
        }
        format!(
            "Opening context:\n{}\nSource lines {first}-{last}:\n{}",
            self.lines_text(1, OPENING_LINES),
            self.lines_text(first, last)
        )
    }

    /// selection.ts `excerptsFor`'s windows: each range ±3 lines, grown
    /// over every comment it overlaps or that only blank lines separate
    /// from it, until none is left. jevgrep rescans all comments until
    /// nothing changes, quadratic over a run of comments; joining comments
    /// into blocks first reaches the same fixed point in one step.
    fn windows(&self, ranges: &[SourceRange]) -> Vec<SourceRange> {
        let line_count = self.lines.len();
        let code = |l: usize| !self.lines_text(l, l).trim().is_empty();
        // The nearest non-blank line at or above each line (0: none), and
        // at or below it (usize::MAX: none); lines past the end are blank.
        let mut above = vec![0; line_count + 1];
        for l in 1..=line_count {
            above[l] = if code(l) { l } else { above[l - 1] };
        }
        let mut below = vec![usize::MAX; line_count + 2];
        for l in (1..=line_count).rev() {
            below[l] = if code(l) { l } else { below[l + 1] };
        }
        let above = |l: usize| above[l.min(line_count)];
        let below = |l: usize| below.get(l).copied().unwrap_or(usize::MAX);
        // Comments (sorted by start) that overlap or only blank lines part
        // form a block: a window that takes one comment takes its block.
        let mut blocks: Vec<SourceRange> = Vec::new();
        for comment in &self.syntax.comments {
            match blocks.last_mut() {
                Some(block) if comment.start_line <= below(block.end_line + 1) => {
                    block.end_line = block.end_line.max(comment.end_line)
                }
                _ => blocks.push(*comment),
            }
        }
        ranges
            .iter()
            .map(|r| {
                let start = r.start_line.saturating_sub(EXCERPT_LINES).max(1);
                let end = line_count.min(r.end_line + EXCERPT_LINES);
                // A comment joins iff it reaches the nearest non-blank line
                // above or below the window. Blocks lie apart by a non-blank
                // line, so the grown window reaches no further block.
                let (up, down) = (above(start - 1), below(end + 1));
                let first = blocks.partition_point(|b| b.end_line < up);
                let last = blocks.partition_point(|b| b.start_line <= down);
                if first < last {
                    SourceRange {
                        start_line: start.min(blocks[first].start_line),
                        end_line: end.max(blocks[last - 1].end_line),
                    }
                } else {
                    SourceRange {
                        start_line: start,
                        end_line: end,
                    }
                }
            })
            .collect()
    }

    /// selection.ts `excerptsFor`: the [`Self::windows`], never through an
    /// unselected giant line; `rendered` byte spans join as they are.
    fn excerpts_for(
        &self,
        ranges: Vec<SourceRange>,
        mut rendered: Vec<Span>,
        chosen: &[Span],
    ) -> Vec<Block> {
        let line_count = self.lines.len();
        let mut windows = self.windows(&ranges);
        let to_last_line = windows.iter().any(|w| w.end_line == line_count);
        // Overlapping or touching windows render as one (their spans merge
        // below either way), so each line is visited once.
        windows.sort_by_key(|w| w.start_line);
        let mut joined: Vec<SourceRange> = Vec::new();
        for window in windows {
            match joined.last_mut() {
                Some(last) if window.start_line <= last.end_line + 1 => {
                    last.end_line = last.end_line.max(window.end_line)
                }
                _ => joined.push(window),
            }
        }
        for window in &joined {
            let mut segment = self.offset(window.start_line - 1);
            for line in window.start_line..=window.end_line {
                let (start, end) = (self.offset(line - 1), self.offset(line));
                // An adjacent selected declaration must not pull in an
                // unselected giant line.
                if end - start > SOURCE_UNIT_BYTES {
                    rendered.push(Span {
                        start: segment,
                        end: start,
                    });
                    for span in chosen {
                        if span.start < end && span.end > start {
                            rendered.push(Span {
                                start: span.start.max(start),
                                end: span.end.min(end),
                            });
                        }
                    }
                    segment = end;
                }
            }
            rendered.push(Span {
                start: segment,
                end: self.offset(window.end_line),
            });
        }
        merge(rendered)
            .into_iter()
            .map(|span| {
                let mut range = self.range_for(span);
                // A trailing empty line has no bytes but belongs to a
                // line-based window.
                if range.bytes.is_none() && span.end == self.len() && to_last_line {
                    range.end_line = line_count;
                }
                let text = match range.bytes {
                    Some(span) => String::from_utf8_lossy(
                        &self.snapshot.source.as_bytes()[span.start..span.end],
                    )
                    .into_owned(),
                    None => self
                        .lines_text(range.start_line, range.end_line)
                        .to_string(),
                };
                Block { range, text }
            })
            .collect()
    }

    /// selection.ts `presentationFor`: `spans` (merged: sorted and apart)
    /// plus the headers (≤ 24 000 bytes) of the declarations they touch.
    fn presentation(&self, spans: &[Span], chosen: &[Span]) -> Vec<Excerpt> {
        let mut headers: Vec<SourceRange> = Vec::new();
        for unit in &self.syntax.units {
            let next = spans.partition_point(|s| s.end <= unit.byte_start);
            if spans.get(next).is_none_or(|s| s.start >= unit.byte_end) {
                continue;
            }
            for header in &unit.owner_headers {
                let bytes = self.offset(header.end_line) - self.offset(header.start_line - 1);
                if bytes <= SOURCE_UNIT_BYTES && !headers.contains(header) {
                    headers.push(*header);
                }
            }
        }
        let (mut whole, mut partial) = (Vec::new(), Vec::new());
        for span in spans {
            let range = self.range_for(*span);
            match range.bytes {
                Some(_) => partial.push(*span),
                None => whole.push(SourceRange {
                    start_line: range.start_line,
                    end_line: range.end_line,
                }),
            }
        }
        whole.extend(headers);
        self.excerpts_for(whole, partial, chosen)
            .iter()
            .map(Block::excerpt)
            .collect()
    }
}

/// retrieve.ts `selectEvidence`'s first pass over every candidate.
pub async fn select_evidence(run: &Arc<Run>) -> HashMap<String, Selected> {
    run.parallel(run.admitted(), select_file)
        .await
        .into_iter()
        .filter_map(|(path, file)| Some((path, file?)))
        .collect()
}

/// Re-read `candidate` and check it is still the file the judge admitted.
async fn fresh(run: &Arc<Run>, candidate: &Candidate) -> Option<Snapshot> {
    let (reader, candidate) = (run.clone(), candidate.clone());
    tokio::task::spawn_blocking(move || reader.unchanged(&candidate))
        .await
        .unwrap_or_else(|_| {
            run.issue("unreadable");
            None
        })
}

/// selection.ts `selectFile`: each declaration scores `min(q, scope)`;
/// `None` leaves the file without evidence.
pub async fn select_file(run: Arc<Run>, candidate: Candidate) -> (String, Option<Selected>) {
    let path = candidate.path.clone();
    let Some(snapshot) = fresh(&run, &candidate).await else {
        return (path, Some(Selected::omitted()));
    };
    if snapshot.source.len() > MAX_PARSE_BYTES {
        run.issue("source_inspection_limit");
        return (path, None);
    }
    let Ok((source, mut groups)) = tokio::task::spawn_blocking(move || {
        let source = Source::new(snapshot);
        let groups = source.groups(source.units());
        (source, groups)
    })
    .await
    else {
        run.issue("unreadable");
        return (path, None);
    };

    let mut selected: Vec<Span> = Vec::new();
    let mut decisions: Vec<(Span, f64)> = Vec::new();
    let mut leads: Vec<(String, Range, f64)> = Vec::new();
    let mut index = 0;
    while index < groups.len() && !run.stopped() {
        let group = &groups[index];
        let declarations: Vec<Declaration> = group
            .iter()
            .map(|u| Declaration {
                name: u.name.clone(),
                start_line: u.start_line,
                end_line: u.end_line,
            })
            .collect();
        let request = prompts::evidence(&run.query, &path, &source.context(group), &declarations);
        let halve = |groups: &mut Vec<Vec<Unit>>| {
            let mut first = groups.remove(index);
            let second = first.split_off(first.len().div_ceil(2));
            groups.insert(index, second);
            groups.insert(index, first);
        };
        // The state cap is jevgrep's; a known window also counts the
        // questions, two per declaration.
        if group.len() > 1
            && (prompts::state_text(&request).len() > run.state_cap
                || !prompts::fits(&request, usize::MAX, run.window))
        {
            halve(&mut groups);
            continue;
        }
        let scores = match run.call(request).await {
            Ok(scores) => scores,
            Err(JudgeError::TooLarge) if group.len() > 1 => {
                halve(&mut groups);
                continue;
            }
            // selection.ts warns and moves on to the next group (a 413 or a
            // timeout is a "provider" failure there); an outage or the ask
            // deadline stops the loop through `run.stopped()`.
            Err(error) => {
                if matches!(error, JudgeError::TooLarge) {
                    run.issue("request_size");
                }
                index += 1;
                continue;
            }
        };
        for (i, unit) in group.iter().enumerate() {
            let answer =
                |prefix: &str| scores.get(&prompts::key(prefix, i)).copied().unwrap_or(0.0);
            let value = answer("q").min(answer("scope"));
            let span = Span {
                start: unit.byte_start,
                end: unit.byte_end,
            };
            match decisions.iter_mut().find(|(s, _)| *s == span) {
                Some(decision) => decision.1 = value,
                None => decisions.push((span, value)),
            }
            if value > SELECT {
                selected.push(span);
            }
            if value > LEAD && !unit.name.ends_with(".context") {
                let range = if source.partial_line(unit) {
                    source.range_for(span)
                } else {
                    Range {
                        start_line: unit.start_line,
                        end_line: unit.end_line,
                        bytes: None,
                    }
                };
                match leads
                    .iter_mut()
                    .find(|(name, r, _)| *name == unit.name && *r == range)
                {
                    Some(lead) => lead.2 = value,
                    None => leads.push((unit.name.clone(), range, value)),
                }
            }
        }
        index += 1;
    }

    // The rest is linear in the file (bounded by MAX_PARSE_BYTES) but can
    // still take a while on a large one: keep it off the async runtime.
    let built = tokio::task::spawn_blocking(move || {
        let chosen = merge(selected);
        // Presentation is stricter than selection: decisions above 0.7
        // within the selected source (`chosen` is merged: sorted, apart).
        let displayed = merge(
            decisions
                .iter()
                .filter(|(_, score)| *score > PRESENT)
                .flat_map(|(span, _)| {
                    let first = chosen.partition_point(|s| s.end <= span.start);
                    chosen[first..]
                        .iter()
                        .take_while(|s| s.start < span.end)
                        .map(|s| Span {
                            start: span.start.max(s.start),
                            end: span.end.min(s.end),
                        })
                })
                .collect(),
        );
        let excerpts = source.presentation(&displayed, &chosen);
        let selected_excerpts = source.presentation(&chosen, &chosen);
        let selected_lines = chosen
            .iter()
            .map(|span| source.range_for(*span))
            .filter(|range| range.bytes.is_none())
            .map(|range| SourceRange {
                start_line: range.start_line,
                end_line: range.end_line,
            })
            .collect();
        let mut lead_list: Vec<Lead> = leads
            .iter()
            .map(|(name, range, score)| Lead {
                name: name.clone(),
                line_from: range.start_line as u32,
                line_to: range.end_line as u32,
                score: *score,
            })
            .collect();
        lead_list.sort_by_key(|lead| lead.line_from);
        Selected {
            excerpts,
            selected_excerpts,
            leads: lead_list,
            call_leads: Vec::new(),
            source_omitted: false,
            selected_lines,
        }
    })
    .await;
    let Ok(selected) = built else {
        run.issue("unreadable");
        return (path, None);
    };
    // The file may have changed while the judge read it.
    if fresh(&run, &candidate).await.is_none() {
        return (path, Some(Selected::omitted()));
    }
    (path, Some(selected))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use judge_contract::Evaluation;
    use serde_json::Value;

    use super::walk::Snap;
    use super::*;
    use crate::code::judge::{Evaluator, Scores};

    /// A run over `dir` whose judge scores each declaration by `score` and
    /// every scope question 1.
    fn run_over(
        dir: &Path,
        state_cap: usize,
        score: impl Fn(&Value, &Evaluation) -> Result<f64, JudgeError> + Send + Sync + 'static,
    ) -> (Arc<Run>, Arc<Mutex<Vec<Evaluation>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sent = log.clone();
        let evaluate: Evaluator = Arc::new(move |ev: Evaluation, _| {
            let ev = prompts::decoded(ev);
            sent.lock().unwrap().push(ev.clone());
            let mut scores = Scores::new();
            let outcome = ev.state["declarations"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .try_for_each(|(i, d)| {
                    scores.insert(prompts::key("q", i), score(d, &ev)?);
                    scores.insert(prompts::key("scope", i), 1.0);
                    Ok(())
                })
                .map(|()| (scores, 1));
            Box::pin(async move { outcome })
        });
        let root = dir.canonicalize().unwrap();
        let cfg = crate::code::config::CoderConfig {
            base_paths: vec![root.clone()],
            ..Default::default()
        };
        let resolver = Arc::new(crate::code::path::PathResolver::new(&cfg).unwrap());
        let tree = walk::Tree::new(&resolver, &root, None, &root, 1 << 24);
        let run = Arc::new(Run {
            query: "q".into(),
            tree,
            evaluate,
            deadline: Instant::now() + Duration::from_secs(60),
            state_cap,
            window: None,
            cache: None,
            slots: crate::code::judge::DEFAULT_SLOTS,
            token_budget: 0,
            state: Mutex::new(Default::default()),
        });
        (run, log)
    }

    async fn select(
        source: &str,
        path: &str,
        state_cap: usize,
        score: impl Fn(&Value, &Evaluation) -> Result<f64, JudgeError> + Send + Sync + 'static,
    ) -> (Selected, Arc<Run>, Vec<Evaluation>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(path), source).unwrap();
        let (run, log) = run_over(dir.path(), state_cap, score);
        let candidate = candidate(&run, path);
        let (_, selected) = select_file(run.clone(), candidate).await;
        let sent = log.lock().unwrap().clone();
        (selected.unwrap(), run, sent)
    }

    fn candidate(run: &Run, path: &str) -> Candidate {
        let Snap::Ok(snapshot) = walk::read(&run.tree, path) else {
            panic!("fixture unreadable");
        };
        Candidate {
            path: path.into(),
            content_hash: snapshot.content_hash,
            score: 0.9,
        }
    }

    fn named(d: &Value, name: &str) -> bool {
        d["name"].as_str().unwrap().ends_with(name)
    }

    fn ranges(selected: &Selected) -> Vec<(u32, u32)> {
        selected
            .excerpts
            .iter()
            .map(|e| (e.line_from, e.line_to))
            .collect()
    }

    fn issues(run: &Run) -> BTreeMap<String, u64> {
        run.state().issues.clone()
    }

    #[tokio::test]
    async fn thresholds_split_excerpts_leads_and_nothing() {
        let source = "function first() { return 1; }\n\n\n\n\n\n\n\n\nfunction second() { return 2; }\n\n\n\n\n\n\n\n\nfunction third() { return 3; }\n\n\n\n\n\n\n\n\nfunction fourth() { return 4; }\n";
        let (selected, run, sent) = select(source, "a.ts", MAX_STATE_BYTES, |d, _| {
            Ok(match d["name"].as_str().unwrap() {
                "first" => 0.9,
                "second" => 0.6,
                "third" => 0.3,
                _ => 0.1,
            })
        })
        .await;
        assert!(issues(&run).is_empty());
        assert_eq!(sent.len(), 1);
        // only 0.9 is presented (±3 lines); 0.6 is selected but not shown
        assert_eq!(ranges(&selected), [(1, 4)]);
        assert_eq!(
            selected.excerpts[0].text,
            "function first() { return 1; }\n\n\n"
        );
        let leads: Vec<_> = selected
            .leads
            .iter()
            .map(|l| (l.name.as_str(), l.line_from, l.score))
            .collect();
        assert_eq!(
            leads,
            [("first", 1, 0.9), ("second", 10, 0.6), ("third", 19, 0.3)]
        );
        assert!(!selected.source_omitted);
    }

    #[tokio::test]
    async fn the_score_is_the_smaller_of_relevance_and_scope() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
        let (mut run, _) = run_over(dir.path(), MAX_STATE_BYTES, |_, _| Ok(0.0));
        Arc::get_mut(&mut run).unwrap().evaluate = Arc::new(|_, _| {
            let scores = Scores::from([("q000".into(), 0.9), ("scope000".into(), 0.4)]);
            Box::pin(async move { Ok((scores, 1)) })
        });
        let candidate = candidate(&run, "a.rs");
        let (_, selected) = select_file(run, candidate).await;
        let selected = selected.unwrap();
        assert!(selected.excerpts.is_empty());
        assert_eq!(selected.leads[0].score, 0.4);
    }

    #[tokio::test]
    async fn excerpts_grow_over_adjacent_comments_and_merge() {
        // selected methods bring their class header along
        let source = "// explains the class\n\nclass Box {\n  alpha() {\n    return 1;\n  }\n\n\n\n\n\n  beta() {\n    return 2;\n  }\n\n\n\n\n\n\n\n\n\n  // far away\n  gamma() {\n    return 3;\n  }\n}\n";
        let (selected, _, _) = select(source, "box.ts", MAX_STATE_BYTES, |d, _| {
            Ok(if named(d, "alpha") || named(d, "beta") {
                0.9
            } else {
                0.0
            })
        })
        .await;
        // header 3 and alpha 4-6 (±3) reach the comment on line 1, beta
        // 12-14 (±3) the one on line 24 over blank lines, and all merge;
        // gamma stays out
        assert_eq!(ranges(&selected), [(1, 24)]);
        assert!(selected.excerpts[0]
            .text
            .starts_with("// explains the class\n"));
        assert!(selected.excerpts[0].text.ends_with("  // far away"));
    }

    #[tokio::test]
    async fn a_group_over_the_state_cap_is_halved() {
        let source: String = (0..8)
            .map(|i| format!("function f{i}() {{ return {}; }}\n", "x".repeat(300)))
            .collect();
        let (selected, run, sent) = select(&source, "a.ts", 2_000, |_, _| Ok(0.9)).await;
        // every state holds the whole file: 8 → 4 + 4 → … → eight singles,
        // each sent although still over the cap
        let asked: Vec<usize> = sent
            .iter()
            .map(|ev| ev.state["declarations"].as_array().unwrap().len())
            .collect();
        assert_eq!(asked, [1; 8]);
        assert_eq!(run.state().judge_calls, sent.len() as u64);
        assert_eq!(ranges(&selected), [(1, 9)]);
    }

    #[tokio::test]
    async fn a_group_over_a_known_window_is_halved() {
        // 53 declarations in one group: their 106 questions and the state
        // are over a 16 384-token window, though the state is far under the
        // state cap
        let source: String = (0..53)
            .map(|i| format!("function f{i:02}() {{ return {i}; }}\n"))
            .collect();
        for (window, asked) in [(None, vec![53]), (Some(16_384), vec![27, 26])] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("a.ts"), &source).unwrap();
            let (mut run, log) = run_over(dir.path(), MAX_STATE_BYTES, |_, _| Ok(0.9));
            Arc::get_mut(&mut run).unwrap().window = window;
            let candidate = candidate(&run, "a.ts");
            select_file(run.clone(), candidate).await;
            let sent: Vec<usize> = log
                .lock()
                .unwrap()
                .iter()
                .map(|ev| ev.state["declarations"].as_array().unwrap().len())
                .collect();
            assert_eq!(sent, asked, "window {window:?}");
            assert!(issues(&run).is_empty());
        }
    }

    #[tokio::test]
    async fn judge_failures_halve_or_skip_the_group_and_later_groups_are_asked() {
        let source: String = (0..4)
            .map(|i| format!("function f{i}() {{ return {i}; }}\n\n\n\n\n\n\n\n\n"))
            .collect();
        let (selected, run, sent) = select(&source, "a.ts", MAX_STATE_BYTES, |d, ev| {
            if ev.state["declarations"].as_array().unwrap().len() > 1 {
                return Err(JudgeError::TooLarge);
            }
            match d["name"].as_str().unwrap() {
                "f0" => Err(JudgeError::TooLarge),
                "f1" => Err(JudgeError::Deadline),
                _ => Ok(0.9),
            }
        })
        .await;
        let asked: Vec<usize> = sent
            .iter()
            .map(|ev| ev.state["declarations"].as_array().unwrap().len())
            .collect();
        assert_eq!(asked, [4, 2, 1, 1, 2, 1, 1]);
        let issues = issues(&run);
        assert_eq!(issues.get("request_size"), Some(&1));
        assert_eq!(issues.get("judge_call_timeout"), Some(&1));
        // f2 (line 19) and f3 (line 28) are still selected
        assert_eq!(ranges(&selected), [(16, 22), (25, 31)]);
    }

    #[tokio::test]
    async fn a_giant_line_is_listed_by_bytes_and_never_pulled_into_an_excerpt() {
        let source = format!(
            "function target() {{\n  return 1;\n}}\nconst big = \"{}\";\n",
            "x".repeat(25_000)
        );
        let (selected, _, sent) = select(&source, "a.ts", MAX_STATE_BYTES, |d, _| {
            Ok(if named(d, "target") { 0.9 } else { 0.0 })
        })
        .await;
        let context = sent[0].state["source"].as_str().unwrap();
        assert!(context.starts_with("Source lines 1-3; source bytes 0-"));
        assert!(context.contains("\nSource lines 4-4; source bytes "));
        // line 4 is within ±3 lines of the selection but stays out
        assert_eq!(ranges(&selected), [(1, 3)]);
        assert!(!selected.excerpts[0].text.contains('x'));
    }

    #[tokio::test]
    async fn a_unit_over_24000_bytes_is_asked_in_16_line_blocks() {
        let body: String = (0..38)
            .map(|i| format!("  const v{i:02} = \"{}\";\n", "y".repeat(680)))
            .collect();
        let source = format!("function big() {{\n{body}}}\n");
        let (selected, _, sent) = select(&source, "a.ts", MAX_STATE_BYTES, |_, _| Ok(0.3)).await;
        let asked: Vec<(u64, u64)> = sent
            .iter()
            .flat_map(|ev| ev.state["declarations"].as_array().unwrap())
            .map(|d| {
                (
                    d["startLine"].as_u64().unwrap(),
                    d["endLine"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(asked, [(1, 16), (17, 32), (33, 40)]);
        let leads: Vec<_> = selected
            .leads
            .iter()
            .map(|l| (l.name.as_str(), l.line_from, l.line_to))
            .collect();
        assert_eq!(leads, [("big", 1, 16), ("big", 17, 32), ("big", 33, 40)]);
    }

    #[tokio::test]
    async fn a_file_changed_while_judged_is_omitted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "fn a() {}\n").unwrap();
        let (run, _) = run_over(dir.path(), MAX_STATE_BYTES, move |_, _| {
            std::fs::write(&file, "fn b() {}\n").unwrap();
            Ok(0.9)
        });
        let candidate = candidate(&run, "a.rs");
        let (_, selected) = select_file(run.clone(), candidate).await;
        let selected = selected.unwrap();
        assert!(selected.source_omitted);
        assert!(selected.excerpts.is_empty() && selected.leads.is_empty());
        assert_eq!(issues(&run).get("changed"), Some(&1));
    }

    #[tokio::test]
    async fn text_files_are_asked_in_chunks_and_large_ones_in_windows() {
        let source: String = (1..=600)
            .map(|i| format!("line {i:04} {}\n", "y".repeat(40)))
            .collect();
        let (_, _, sent) = select(&source, "notes.txt", MAX_STATE_BYTES, |_, _| Ok(0.0)).await;
        let declarations: Vec<&Value> = sent
            .iter()
            .flat_map(|ev| ev.state["declarations"].as_array().unwrap())
            .collect();
        assert!(declarations.iter().all(|d| d["name"] == "source"));
        assert_eq!(declarations[0]["startLine"], 1);
        assert_eq!(declarations.last().unwrap()["endLine"], 600);
        let context = sent[0].state["source"].as_str().unwrap();
        assert!(context.starts_with("Opening context:\nline 0001"));
        assert!(context.contains("\nSource lines 1-"));
    }

    #[tokio::test]
    async fn an_excerpt_inside_a_giant_line_is_a_marked_byte_span() {
        // every declaration on the one line is asked about as the same two
        // 24 000-byte chunks; only the first is selected
        let source = format!(
            "const pad = \"{}\"; function target() {{ return 1; }}\n",
            "x".repeat(25_000)
        );
        let asked = std::sync::atomic::AtomicUsize::new(0);
        let (selected, _, _) = select(&source, "a.ts", MAX_STATE_BYTES, move |_, _| {
            let first = asked
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                .is_multiple_of(2);
            Ok(if first { 0.9 } else { 0.0 })
        })
        .await;
        let excerpt = &selected.excerpts[0];
        let span = excerpt.partial.expect("a byte span");
        assert_eq!((excerpt.line_from, excerpt.line_to), (1, 1));
        assert_eq!((span.byte_from, span.byte_to), (0, 24_000));
        assert_eq!(excerpt.text, source[..24_000]);
        let wire = serde_json::to_value(excerpt).unwrap();
        assert_eq!(
            wire["partial"],
            serde_json::json!({"byte_from": 0, "byte_to": 24_000})
        );
        // whole-line excerpts carry no `partial`
        let whole = Excerpt {
            partial: None,
            ..excerpt.clone()
        };
        assert!(serde_json::to_value(&whole)
            .unwrap()
            .get("partial")
            .is_none());
    }

    fn text_source(text: String) -> Source {
        Source::new(Snapshot {
            path: "a.txt".into(),
            source: text,
            content_hash: String::new(),
        })
    }

    /// selection.ts's own fixed point: rescan every comment until no window
    /// grows (quadratic over a run of comments).
    fn windows_by_rescanning(source: &Source, ranges: &[SourceRange]) -> Vec<SourceRange> {
        let line_count = source.lines.len();
        let blank =
            |from: usize, to: usize| (from..=to).all(|l| source.lines_text(l, l).trim().is_empty());
        ranges
            .iter()
            .map(|r| {
                let mut window = SourceRange {
                    start_line: r.start_line.saturating_sub(EXCERPT_LINES).max(1),
                    end_line: line_count.min(r.end_line + EXCERPT_LINES),
                };
                let mut changed = true;
                while changed {
                    changed = false;
                    for comment in &source.syntax.comments {
                        let before = comment.end_line < window.start_line
                            && blank(comment.end_line + 1, window.start_line - 1);
                        let after = comment.start_line > window.end_line
                            && blank(window.end_line + 1, comment.start_line - 1);
                        let overlap = comment.start_line <= window.end_line
                            && comment.end_line >= window.start_line;
                        if overlap || before || after {
                            let grown = SourceRange {
                                start_line: window.start_line.min(comment.start_line),
                                end_line: window.end_line.max(comment.end_line),
                            };
                            changed |= grown != window;
                            window = grown;
                        }
                    }
                }
                window
            })
            .collect()
    }

    #[test]
    fn comment_growth_reaches_the_rescanning_fixed_point() {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for _ in 0..2_000 {
            let line_count = 1 + next(40);
            let text: String = (0..line_count)
                .map(|_| ["\n", "  \n", "code\n"][next(3)])
                .collect();
            let mut source = text_source(text);
            // overlapping, nested and past-the-end comments included
            let count = next(12);
            let mut comments: Vec<SourceRange> = (0..count)
                .map(|_| {
                    let start = 1 + next(line_count + 2);
                    SourceRange {
                        start_line: start,
                        end_line: start + next(4),
                    }
                })
                .collect();
            comments.sort_by_key(|c| c.start_line);
            source.syntax.comments = comments;
            let count = 1 + next(4);
            let ranges: Vec<SourceRange> = (0..count)
                .map(|_| {
                    let start = 1 + next(line_count);
                    SourceRange {
                        start_line: start,
                        end_line: start + next(3),
                    }
                })
                .collect();
            assert_eq!(
                source.windows(&ranges),
                windows_by_rescanning(&source, &ranges),
                "comments {:?}, ranges {:?}",
                source.syntax.comments,
                ranges
            );
        }
    }

    #[test]
    fn a_long_run_of_comments_grows_a_window_in_linear_time() {
        // 300 000 comments apart by blank lines above the selection: the
        // rescanning fixed point takes one pass over them per comment
        let text = format!("{}def target():\n    return 1\n", "#\n\n".repeat(300_000));
        let mut source = text_source(text);
        source.syntax.comments = (0..300_000)
            .map(|i| SourceRange {
                start_line: 2 * i + 1,
                end_line: 2 * i + 1,
            })
            .collect();
        let started = Instant::now();
        let target = SourceRange {
            start_line: 600_001,
            end_line: 600_002,
        };
        let blocks = source.excerpts_for(vec![target], Vec::new(), &[]);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            (blocks[0].range.start_line, blocks[0].range.end_line),
            (1, 600_003)
        );
        assert_eq!(blocks[0].text, source.snapshot.source);
    }
}
