//! Evidence selection: ask the judge about each admitted file's
//! declarations and keep the verbatim source it selects.
//!
//! Ported from dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit
//! 82ef1fd: `packages/core/src/selection.ts` `selectFile` (first pass) and
//! the selection loop of `retrieve.ts` (518-564).
//!
//! Deviations: one read per file, re-hashed before its excerpts are
//! emitted, instead of a freshness check before every judge attempt; a
//! group the judge finds too large is halved like one over the state cap;
//! files run concurrently up to the worker's judge slots.

use std::collections::HashMap;
use std::sync::Arc;

use iii_helpers::observability::opentelemetry::trace::FutureExt as _;
use iii_helpers::observability::opentelemetry::Context;
use tokio::task::JoinSet;

use super::navigate::{Candidate, Run};
use super::prompts::{self, Declaration, SourceRange};
use super::units::{self, Inspection, MAX_PARSE_BYTES};
use super::walk::{self, Snap, Snapshot, Unit};
use super::{Excerpt, Lead};
use crate::code::judge::{JudgeError, SLOT_COUNT};

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
/// jevgrep's evidence state cap (`Run::state_cap` may lower it).
pub const MAX_STATE_BYTES: usize = 80_000;
/// Selection, lead and presentation thresholds (strict).
const SELECT: f64 = 0.5;
const LEAD: f64 = 0.25;
const PRESENT: f64 = 0.7;

/// What selection found in one file.
#[derive(Debug, Default)]
pub struct Selected {
    pub excerpts: Vec<Excerpt>,
    pub leads: Vec<Lead>,
    pub source_omitted: bool,
}

impl Selected {
    fn omitted() -> Self {
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

    fn blank(&self, from: usize, to: usize) -> bool {
        (from..=to).all(|l| self.lines_text(l, l).trim().is_empty())
    }

    /// selection.ts `excerptsFor`: each range ±3 lines, grown over comments
    /// separated from it only by blank lines, never through an unselected
    /// giant line; `rendered` byte spans join as they are.
    fn excerpts_for(
        &self,
        ranges: Vec<SourceRange>,
        mut rendered: Vec<Span>,
        chosen: &[Span],
    ) -> Vec<Excerpt> {
        let line_count = self.lines.len();
        let mut windows: Vec<SourceRange> = ranges
            .iter()
            .map(|r| SourceRange {
                start_line: r.start_line.saturating_sub(EXCERPT_LINES).max(1),
                end_line: line_count.min(r.end_line + EXCERPT_LINES),
            })
            .collect();
        for window in &mut windows {
            let mut changed = true;
            while changed {
                changed = false;
                for comment in &self.syntax.comments {
                    let before = comment.end_line < window.start_line
                        && self.blank(comment.end_line + 1, window.start_line - 1);
                    let after = comment.start_line > window.end_line
                        && self.blank(window.end_line + 1, comment.start_line - 1);
                    let overlap = comment.start_line <= window.end_line
                        && comment.end_line >= window.start_line;
                    if overlap || before || after {
                        let start = window.start_line.min(comment.start_line);
                        let end = window.end_line.max(comment.end_line);
                        if (start, end) != (window.start_line, window.end_line) {
                            *window = SourceRange {
                                start_line: start,
                                end_line: end,
                            };
                            changed = true;
                        }
                    }
                }
            }
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
                if range.bytes.is_none()
                    && span.end == self.len()
                    && windows.iter().any(|w| w.end_line == line_count)
                {
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
                Excerpt {
                    line_from: range.start_line as u32,
                    line_to: range.end_line as u32,
                    text,
                }
            })
            .collect()
    }

    /// selection.ts `presentationFor`: `spans` plus the headers (≤ 24 000
    /// bytes) of the declarations they touch.
    fn presentation(&self, spans: &[Span], chosen: &[Span]) -> Vec<Excerpt> {
        let mut headers: Vec<SourceRange> = Vec::new();
        for unit in &self.syntax.units {
            if !spans
                .iter()
                .any(|s| s.start < unit.byte_end && s.end > unit.byte_start)
            {
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
    }
}

/// Select every candidate, at most one file per judge slot at a time.
pub async fn select_all(run: &Arc<Run>) -> HashMap<String, Selected> {
    let mut candidates: Vec<Candidate> = run.state().candidates.values().cloned().collect();
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| walk::locale_cmp(&a.path, &b.path))
    });
    let mut queue = candidates.into_iter();
    let (mut running, mut selected) = (JoinSet::new(), HashMap::new());
    loop {
        while running.len() < SLOT_COUNT && !run.stopped() {
            let Some(candidate) = queue.next() else {
                break;
            };
            running.spawn(select_file(run.clone(), candidate).with_context(Context::current()));
        }
        let Some(joined) = running.join_next().await else {
            break;
        };
        match joined {
            Ok((path, Some(file))) => {
                selected.insert(path, file);
            }
            Ok((_, None)) => {}
            Err(_) => run.issue("unreadable"),
        }
    }
    selected
}

/// Re-read `candidate` and check it is still the file the judge admitted.
async fn fresh(run: &Arc<Run>, candidate: &Candidate) -> Option<Snapshot> {
    let (reader, path) = (run.clone(), candidate.path.clone());
    let snap = tokio::task::spawn_blocking(move || walk::read(&reader.tree, &path))
        .await
        .unwrap_or(Snap::Issue("unreadable"));
    match snap {
        Snap::Ok(snapshot) if snapshot.content_hash == candidate.content_hash => Some(snapshot),
        Snap::Issue(kind) => {
            run.issue(kind);
            None
        }
        _ => {
            run.issue("changed");
            None
        }
    }
}

/// selection.ts `selectFile`, first pass: `None` leaves the file as
/// navigation found it.
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
        let request = prompts::evidence(
            &run.query,
            &path,
            &source.context(group),
            &declarations,
            None,
        );
        let halve = |groups: &mut Vec<Vec<Unit>>| {
            let mut first = groups.remove(index);
            let second = first.split_off(first.len().div_ceil(2));
            groups.insert(index, second);
            groups.insert(index, first);
        };
        if group.len() > 1 && walk::json_len(&request.state) > run.state_cap {
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
                    run.issue("request-size");
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

    let chosen = merge(selected);
    // Presentation is stricter than selection: decisions above 0.7 within
    // the selected source.
    let displayed = merge(
        decisions
            .iter()
            .filter(|(_, score)| *score > PRESENT)
            .flat_map(|(span, _)| {
                chosen.iter().filter_map(|s| {
                    let (start, end) = (span.start.max(s.start), span.end.min(s.end));
                    (start < end).then_some(Span { start, end })
                })
            })
            .collect(),
    );
    let excerpts = source.presentation(&displayed, &chosen);
    let mut leads: Vec<Lead> = leads
        .into_iter()
        .map(|(name, range, score)| Lead {
            name,
            line_from: range.start_line as u32,
            line_to: range.end_line as u32,
            score,
        })
        .collect();
    leads.sort_by_key(|lead| lead.line_from);

    // The file may have changed while the judge read it.
    if fresh(&run, &candidate).await.is_none() {
        return (path, Some(Selected::omitted()));
    }
    (
        path,
        Some(Selected {
            excerpts,
            leads,
            source_omitted: false,
        }),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use judge_contract::Evaluation;
    use serde_json::Value;

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
        let tree = walk::Tree {
            root: dir.canonicalize().unwrap(),
            children: HashMap::new(),
            truncated: false,
            unreadable: 0,
            max_file_bytes: 1 << 24,
        };
        let run = Arc::new(Run {
            query: "q".into(),
            tree,
            evaluate,
            deadline: Instant::now() + Duration::from_secs(60),
            cap: usize::MAX,
            state_cap,
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
        assert_eq!(issues.get("request-size"), Some(&1));
        assert_eq!(issues.get("deadline"), Some(&1));
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
}
