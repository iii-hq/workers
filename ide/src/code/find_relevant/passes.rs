//! The passes around evidence selection: file roles and read-early
//! priority (judged beside it), the presentation passes after it, and the
//! Python preview sampler discovery uses.
//!
//! Ported from dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit
//! 82ef1fd: `packages/core/src/retrieve.ts` `assessFiles` (596-622) and the
//! presentation passes (623-695), `call-context.ts`, `parser-preview.mjs`,
//! and the `previewMatches` and `calls` helpers of `parser-helpers.mjs`.
//!
//! Deviations: the assessment reads the discovery preview without re-reading
//! the file; its roles attach only where the evidence re-hash held (the
//! caller's check, retrieve.ts 643-650); an assessment request over
//! `Run::window_cap` is not sent (jevgrep has no cap there). Python names
//! and query tokens are not NFKC normalized, and parser columns are UTF-8
//! bytes, not UTF-16 units. Of the checks jevgrep runs on its parser
//! process's output only the call ranges' remain (a class header on one
//! line is empty and drops the pass, as there). A Python file is read once
//! for the call pass; the caller's final re-hash covers later changes. An
//! inheritance chain deeper than 256 drops the call pass, which stops at
//! the ask deadline (jevgrep's abort kills its parser process).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use once_cell::sync::Lazy;
use tree_sitter::Node;

use super::navigate::Run;
use super::prompts::{self, SourceRange};
use super::select::Selected;
use super::units::{
    self, named, py_body, py_definition, py_end_line, py_start_line, range, text, unparenthesized,
    MAX_PARSE_BYTES,
};
use super::walk::Snapshot;
use super::{CallLead, Excerpt};
use crate::code::judge::{JudgeError, Scores};

/// Roles and priority above this count (strict).
const ROLE: f64 = 0.5;
/// Larger call targets are never shown whole.
const SOURCE_UNIT_BYTES: usize = 24_000;
/// Deeper inheritance chains drop the call pass. jevgrep's recursion runs in
/// a parser process, where a stack overflow is a failed parse; here it would
/// abort the worker.
const MAX_MRO_DEPTH: usize = 256;

/// parser-helpers.mjs's query identifiers
/// (`/[_\p{ID_Start}][_\p{ID_Continue}]*/u`).
static IDENTIFIER: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(r"[_\p{ID_Start}][_\p{ID_Continue}]*").expect("identifier regex")
});

#[derive(Debug, Clone, PartialEq)]
pub struct Assessment {
    /// In jevgrep's role order.
    pub roles: Vec<String>,
    pub priority: f64,
}

impl Assessment {
    fn from_scores(scores: &Scores) -> Self {
        Self {
            roles: prompts::ROLES
                .iter()
                .filter(|(role, _)| scores.get(*role).is_some_and(|p| *p > ROLE))
                .map(|(role, _)| role.to_string())
                .collect(),
            priority: scores.get("priority").copied().unwrap_or(0.0),
        }
    }
}

/// One file-assessment request per candidate, reusing its discovery
/// preview; a failed request leaves the file unassessed.
pub async fn assess_files(run: &Arc<Run>) -> HashMap<String, Assessment> {
    run.parallel(run.admitted(), |run, candidate| async move {
        let preview = run.state().previews.get(&candidate.path).cloned()?;
        let request = prompts::file_assessment(&run.query, &candidate.path, &preview);
        if prompts::request_bytes(&request) > run.window_cap {
            run.issue("request-size");
            return None;
        }
        match run.call(request).await {
            Ok(scores) => Some((candidate.path, Assessment::from_scores(&scores))),
            Err(JudgeError::TooLarge) => {
                run.issue("request-size");
                None
            }
            Err(_) => None, // recorded by `call`
        }
    })
    .await
    .into_iter()
    .flatten()
    .collect()
}

/// retrieve.ts 623-695: a test file shows all its selected source, and a
/// Python file's shown source gains the local methods its selected code
/// may call (and their class headers). Omitted files are left alone.
pub async fn present(
    run: &Arc<Run>,
    files: &mut HashMap<String, Selected>,
    assessments: &HashMap<String, Assessment>,
) {
    // retrieve.ts runs these passes in `parallel`, which starts nothing
    // once the ask stopped.
    if run.stopped() {
        return;
    }
    let tested = |path: &str| {
        assessments
            .get(path)
            .is_some_and(|a| a.roles.iter().any(|role| role == "test"))
    };
    let mut python = Vec::new();
    for candidate in run.admitted() {
        let Some(file) = files
            .get_mut(&candidate.path)
            .filter(|file| !file.source_omitted)
        else {
            continue;
        };
        if tested(&candidate.path) {
            file.excerpts = file.selected_excerpts.clone();
        }
        if units::is_python(&candidate.path) {
            python.push((candidate, file.clone()));
        }
    }
    if python.is_empty() {
        return;
    }
    let reader = run.clone();
    let Ok(read) = tokio::task::spawn_blocking(move || {
        python
            .into_iter()
            .filter_map(|(candidate, mut file)| {
                let snapshot = reader.unchanged(&candidate)?;
                let mut widened = file.clone();
                let context = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    local_call_context(&snapshot, &mut widened, reader.deadline)
                }));
                match context {
                    Ok(()) => file = widened,
                    // Optional structural context never discards selected
                    // evidence.
                    Err(_) => reader.issue("local-call-context"),
                }
                Some((candidate.path, file))
            })
            .collect::<Vec<_>>()
    })
    .await
    else {
        run.issue("local-call-context");
        return;
    };
    files.extend(read);
}

/// call-context.ts `localCallContext`: with whole-line shown source only,
/// add each possible local call target (at most 24 000 bytes) and its
/// class header to the shown source, merging touching ranges, and list the
/// calls as leads.
fn local_call_context(snapshot: &Snapshot, file: &mut Selected, deadline: Instant) {
    if snapshot.source.len() > MAX_PARSE_BYTES
        || file.source_omitted
        || file.excerpts.iter().any(|e| e.partial.is_some())
        || file.selected_lines.is_empty()
    {
        return;
    }
    let found = calls(&snapshot.source, &file.selected_lines, deadline);
    if found.is_empty() {
        return;
    }
    let lines: Vec<&str> = snapshot.source.split('\n').collect();
    let valid = |r: &SourceRange| {
        r.start_line >= 1 && r.end_line >= r.start_line && r.end_line <= lines.len()
    };
    if found
        .iter()
        .any(|c| !valid(&c.range) || !valid(&c.owner_header))
    {
        return;
    }
    let source = |r: &SourceRange| lines[r.start_line - 1..r.end_line].join("\n");
    let kept: Vec<Call> = found
        .into_iter()
        .filter(|c| source(&c.range).len() <= SOURCE_UNIT_BYTES)
        .collect();
    if kept.is_empty() {
        return;
    }
    let mut ranges: Vec<SourceRange> = file
        .excerpts
        .iter()
        .map(|e| range(e.line_from as usize, e.line_to as usize))
        .chain(kept.iter().flat_map(|c| [c.range, c.owner_header]))
        .collect();
    ranges.sort_by_key(|r| (r.start_line, r.end_line));
    let mut merged: Vec<SourceRange> = Vec::new();
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start_line <= last.end_line + 1 => {
                last.end_line = last.end_line.max(r.end_line)
            }
            _ => merged.push(r),
        }
    }
    file.excerpts = merged
        .iter()
        .map(|r| Excerpt {
            line_from: r.start_line as u32,
            line_to: r.end_line as u32,
            text: source(r),
            partial: None,
        })
        .collect();
    file.call_leads = kept
        .into_iter()
        .map(|c| CallLead {
            caller: c.caller,
            name: c.name,
            line_from: c.range.start_line as u32,
            line_to: c.range.end_line as u32,
            unknown_earlier_bases: c.unknown_earlier_bases,
        })
        .collect();
}

/// parser-helpers.mjs `walk`: every named node, depth-first preorder.
fn preorder(root: Node<'_>) -> Vec<Node<'_>> {
    let (mut nodes, mut stack) = (Vec::new(), vec![root]);
    while let Some(node) = stack.pop() {
        nodes.push(node);
        stack.extend(named(node).into_iter().rev());
    }
    nodes
}

fn py_range(node: Node<'_>) -> SourceRange {
    range(py_start_line(node), py_end_line(node))
}

fn py_name<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    node.child_by_field_name("name")
        .map_or("", |name| text(name, source))
}

/// The decorated definition wrapping `node`, or `node`.
fn wrapper(node: Node<'_>) -> Node<'_> {
    node.parent()
        .filter(|parent| parent.kind() == "decorated_definition")
        .unwrap_or(node)
}

/// A class or function the query names (parser-helpers.mjs
/// `previewMatches`), in 1-based lines and byte columns.
#[derive(Debug, Clone, PartialEq)]
struct PreviewMatch {
    start: usize,
    end: usize,
    /// The enclosing class up to its first statement.
    context: Option<SourceRange>,
    /// The line before the first body statement.
    header_end: usize,
    header_line: usize,
    /// Where the first statement starts on the header line, or 0.
    header_column_end: usize,
    /// The first statement past a docstring.
    body_start: usize,
    /// Its column when it shares the docstring's line, or 0.
    body_column: usize,
}

/// `/^[rub]*[fb]/i`: an f-string or bytes literal is not a docstring.
fn formatted_or_bytes(literal: &str) -> bool {
    for c in literal.chars().map(|c| c.to_ascii_lowercase()) {
        match c {
            'f' | 'b' => return true,
            'r' | 'u' => {}
            _ => return false,
        }
    }
    false
}

fn preview_matches(root: Node<'_>, source: &str, tokens: &HashSet<&str>) -> Vec<PreviewMatch> {
    let mut matches = Vec::new();
    for n in preorder(root) {
        if !matches!(n.kind(), "class_definition" | "function_definition")
            || !tokens.contains(py_name(n, source))
        {
            continue;
        }
        let statements = py_body(n);
        let Some(&first) = statements.first() else {
            continue;
        };
        let expression = unparenthesized(
            (first.kind() == "expression_statement")
                .then(|| first.named_child(0))
                .flatten(),
        );
        let strings: Vec<Option<Node<'_>>> = match expression {
            Some(e) if e.kind() == "concatenated_string" => named(e)
                .into_iter()
                .filter(|c| c.kind() == "string")
                .map(Some)
                .collect(),
            _ => vec![expression],
        };
        let docstring = !strings.is_empty()
            && strings.iter().all(|s| {
                s.is_some_and(|s| s.kind() == "string" && !formatted_or_bytes(text(s, source)))
            });
        let implementation = if docstring && statements.len() > 1 {
            statements[1]
        } else {
            first
        };
        let mut owner = n.parent();
        while let Some(node) = owner.filter(|o| o.kind() != "class_definition") {
            owner = node.parent();
        }
        let context = owner.map(|owner| {
            range(
                py_start_line(wrapper(owner)),
                py_body(owner).first().map_or(0, |b| b.start_position().row),
            )
        });
        let (row, first_row) = (n.start_position().row, first.start_position().row);
        let body = implementation.start_position();
        matches.push(PreviewMatch {
            start: py_start_line(wrapper(n)),
            end: py_end_line(n),
            context,
            header_end: first_row,
            header_line: row + 1,
            header_column_end: if first_row == row {
                first.start_position().column
            } else {
                0
            },
            body_start: body.row + 1,
            body_column: if implementation.id() != first.id() && body.row == first_row {
                body.column
            } else {
                0
            },
        });
    }
    matches.sort_by_key(|m| (m.start, m.end));
    matches
}

/// parser-preview.mjs `clip`: at most `limit` bytes of `raw` (its tail
/// when `from_end`), never splitting a character.
fn clip(raw: &str, limit: usize, from_end: bool) -> &str {
    let bytes = raw.as_bytes();
    let len = bytes.len();
    let (mut start, mut end) = if from_end {
        (len.saturating_sub(limit), len)
    } else {
        (0, len.min(limit))
    };
    while start < end && (bytes[start] & 0xc0) == 0x80 {
        start += 1;
    }
    while end < len && end > start && (bytes[end] & 0xc0) == 0x80 {
        end -= 1;
    }
    &raw[start..end]
}

/// parser-preview.mjs `preview`'s window writer.
struct Sampler<'a> {
    lines: Vec<&'a str>,
    budget: i64,
    used: i64,
    text: String,
    /// Windows already written: `(None, start, end, body)` for lines,
    /// `(Some(line), from, to, body)` for a partial line.
    seen: HashSet<(Option<usize>, usize, usize, String)>,
}

impl Sampler<'_> {
    fn write(&mut self, rendered: String) {
        self.used += rendered.len() as i64;
        self.text.push_str(&rendered);
    }

    /// `add`: lines `start..=end` (from the end when `from_end`) within
    /// `allowance` bytes, clipping a first line that alone is too long.
    fn add(
        &mut self,
        start: usize,
        end: usize,
        allowance: i64,
        basis: &str,
        from_end: bool,
    ) -> bool {
        if start > end {
            return false;
        }
        let (start, end) = (start.max(1), end.min(self.lines.len()));
        let allowance = allowance.min(self.budget - self.used);
        let header = format!("--- source lines {start}-{end}; {basis}; may be clipped ---\n");
        let remaining = allowance - header.len() as i64 - 1;
        if remaining <= 0 {
            return false;
        }
        let mut candidates: Vec<&str> = if start <= end {
            self.lines[start - 1..end].to_vec()
        } else {
            Vec::new()
        };
        if from_end {
            candidates.reverse();
        }
        let (mut selected, mut size, mut partial) = (Vec::new(), 0i64, false);
        for line in candidates {
            let cost = line.len() as i64 + i64::from(!selected.is_empty());
            if size + cost > remaining {
                if selected.is_empty() {
                    let clipped = clip(line, remaining as usize, from_end);
                    if !clipped.is_empty() {
                        selected.push(clipped);
                        partial = true;
                    }
                }
                break;
            }
            selected.push(line);
            size += cost;
        }
        if selected.is_empty() {
            return false;
        }
        let mut start = start;
        if from_end {
            selected.reverse();
            start = end + 1 - selected.len();
        }
        let actual_end = start + selected.len() - 1;
        let identity = (None, start, actual_end, selected.join("\n"));
        if self.seen.contains(&identity) {
            return true;
        }
        let rendered = format!(
            "--- source lines {start}-{actual_end}; {basis}{} ---\n{}\n",
            if partial { "; partial line" } else { "" },
            identity.3
        );
        if rendered.len() as i64 > allowance {
            return false;
        }
        self.seen.insert(identity);
        self.write(rendered);
        true
    }

    /// `inline`: bytes `from..to` of one line within `allowance`.
    fn inline(&mut self, line: usize, from: usize, to: usize, allowance: i64, basis: &str) -> bool {
        let raw = self.lines[line - 1];
        let (from, to) = (from.min(raw.len()), to.min(raw.len()));
        let allowance = allowance.min(self.budget - self.used);
        let longest =
            format!("--- source line {line}, bytes {from}-{to}; {basis}; partial line ---\n");
        let room = allowance - longest.len() as i64 - 1;
        let Some(part) = raw.get(from..to).filter(|_| room > 0) else {
            return false;
        };
        let body = clip(part, room as usize, false);
        if body.is_empty() {
            return false;
        }
        let actual_end = from + body.len();
        let identity = (Some(line), from, actual_end, body.to_string());
        if self.seen.contains(&identity) {
            return true;
        }
        let rendered = format!(
            "--- source line {line}, bytes {from}-{actual_end}; {basis}; partial line ---\n{body}\n"
        );
        self.seen.insert(identity);
        self.write(rendered);
        true
    }
}

/// retrieve.ts 265-277 via source.ts `pythonPreview` and parser-preview.mjs
/// `preview`: a query-aware sample of a Python source over `budget` bytes.
/// A quarter goes to the opening lines; each class or function the query
/// names shares three quarters of the rest (its enclosing class header,
/// its own header, then its implementation past any docstring); what is
/// left samples 32-line windows at a third, two thirds and the end. `None`
/// when the source fits the budget.
pub fn python_preview(source: &str, query: &str, budget: usize) -> Option<String> {
    if source.len() <= budget {
        return None;
    }
    let tokens: HashSet<&str> = IDENTIFIER.find_iter(query).map(|m| m.as_str()).collect();
    let matches = units::parse_python(source)
        .map(|tree| preview_matches(tree.root_node(), source, &tokens))
        .unwrap_or_default();
    let mut s = Sampler {
        lines: source.split('\n').collect(),
        budget: budget as i64,
        used: 0,
        text: String::new(),
        seen: HashSet::new(),
    };
    let line_count = s.lines.len();
    s.add(1, line_count, s.budget / 4, "opening context", false);
    if !matches.is_empty() {
        let per_match = ((s.budget - s.used) * 3 / 4 / matches.len() as i64).max(1);
        for m in &matches {
            let mut context_cost = 0;
            if let Some(context) = m.context {
                let before = s.used;
                s.add(
                    context.start_line,
                    context.end_line,
                    (per_match / 3).min(512),
                    "enclosing class context",
                    false,
                );
                context_cost = s.used - before;
            }
            let header = ((per_match - context_cost) / 3).min(512);
            let before = s.used;
            s.add(
                m.start,
                m.header_end,
                header,
                "query-named declaration header",
                false,
            );
            if m.header_column_end > 0 {
                s.inline(
                    m.header_line,
                    0,
                    m.header_column_end,
                    header,
                    "query-named declaration header",
                );
            }
            let remaining = per_match - context_cost - (s.used - before);
            if m.body_column > 0 {
                let before = s.used;
                let line_end = s.lines[m.body_start - 1].len();
                s.inline(
                    m.body_start,
                    m.body_column,
                    line_end,
                    remaining,
                    "query-named implementation",
                );
                if m.end > m.body_start {
                    s.add(
                        m.body_start + 1,
                        m.end,
                        remaining - (s.used - before),
                        "query-named implementation continuation",
                        false,
                    );
                }
            } else {
                s.add(
                    m.body_start,
                    m.end,
                    remaining,
                    "query-named implementation",
                    false,
                );
            }
        }
    }
    let mut positions = vec![
        line_count / 3 + 1,
        2 * line_count / 3 + 1,
        line_count.saturating_sub(31).max(1),
    ];
    positions.sort_unstable();
    positions.dedup();
    let count = positions.len();
    for (i, start) in positions.into_iter().enumerate() {
        let allowance = (s.budget - s.used) / (count - i) as i64;
        s.add(
            start,
            (start + 31).min(line_count),
            allowance,
            "distributed context",
            i == count - 1,
        );
    }
    Some(s.text)
}

/// A possible local call (parser-helpers.mjs `calls`).
#[derive(Debug, Clone, PartialEq)]
struct Call {
    caller: String,
    name: String,
    range: SourceRange,
    unknown_earlier_bases: Vec<String>,
    owner_header: SourceRange,
}

/// parser-helpers.mjs `uniqueDefinitions`: definitions by name in
/// first-seen order, without names defined twice.
fn unique<'t>(nodes: Vec<Node<'t>>, source: &str) -> Vec<(String, Node<'t>)> {
    let (mut found, mut index, mut duplicates) = (Vec::new(), HashMap::new(), HashSet::new());
    for node in nodes {
        let key = py_definition(node).map_or("", |d| py_name(d, source));
        match index.get(key) {
            Some(&i) => {
                duplicates.insert(key);
                found[i] = (key.to_string(), node);
            }
            None => {
                index.insert(key, found.len());
                found.push((key.to_string(), node));
            }
        }
    }
    found.retain(|(key, _)| !duplicates.contains(key.as_str()));
    found
}

/// The C3 linearization of `key` over `bases`; a name that is not a local
/// class linearizes to itself. `None` on a cycle, an inconsistent
/// hierarchy or once `deadline` passes (the merge is quadratic in the
/// bases; jevgrep's abort kills its parser process instead).
fn mro(
    key: &str,
    seen: &[String],
    bases: &HashMap<String, Vec<String>>,
    memo: &mut HashMap<String, Vec<String>>,
    deadline: Instant,
) -> Option<Vec<String>> {
    if seen.iter().any(|s| s == key) || seen.len() > MAX_MRO_DEPTH || Instant::now() >= deadline {
        return None;
    }
    if let Some(order) = memo.get(key) {
        return Some(order.clone());
    }
    let parents = bases.get(key).cloned().unwrap_or_default();
    let mut next = seen.to_vec();
    next.push(key.to_string());
    let mut sequences: Vec<VecDeque<String>> = Vec::new();
    for parent in &parents {
        sequences.push(mro(parent, &next, bases, memo, deadline)?.into());
    }
    sequences.push(parents.into());
    let mut order = vec![key.to_string()];
    loop {
        let active: Vec<usize> = (0..sequences.len())
            .filter(|&i| !sequences[i].is_empty())
            .collect();
        if active.is_empty() {
            break;
        }
        let mut head = None;
        for &i in &active {
            if Instant::now() >= deadline {
                return None;
            }
            let candidate = &sequences[i][0];
            if !active
                .iter()
                .any(|&j| sequences[j].iter().skip(1).any(|x| x == candidate))
            {
                head = Some(candidate.clone());
                break;
            }
        }
        let head = head?;
        for &i in &active {
            if sequences[i][0] == head {
                sequences[i].pop_front();
            }
        }
        order.push(head);
    }
    memo.insert(key.to_string(), order.clone());
    Some(order)
}

/// parser-helpers.mjs `bodyNodes`: `function` and everything inside it
/// outside nested definitions and lambdas, each with whether it lies inside
/// a match-case pattern.
fn body_nodes(function: Node<'_>) -> Vec<(Node<'_>, bool)> {
    let (mut nodes, mut stack) = (Vec::new(), vec![(function, false)]);
    while let Some((node, in_pattern)) = stack.pop() {
        nodes.push((node, in_pattern));
        let inside = in_pattern || node.kind() == "case_pattern";
        stack.extend(
            named(node)
                .into_iter()
                .rev()
                .filter(|c| {
                    !matches!(
                        c.kind(),
                        "function_definition"
                            | "class_definition"
                            | "lambda"
                            | "decorated_definition"
                    )
                })
                .map(|c| (c, inside)),
        );
    }
    nodes
}

/// parser-helpers.mjs `assignsSelf`: `self` among the names `target` binds.
fn assigns_self(target: Option<Node<'_>>, source: &str) -> bool {
    let mut stack: Vec<Node<'_>> = target.into_iter().collect();
    while let Some(target) = stack.pop() {
        match target.kind() {
            "identifier" if text(target, source) == "self" => return true,
            "identifier" | "attribute" | "subscript" => {}
            _ => stack.extend(named(target)),
        }
    }
    false
}

/// A node that may rebind `self`, including by destructuring, a match
/// pattern (a `self` anywhere in one: `in_pattern`, from [`body_nodes`],
/// so nested patterns are not rescanned) or an import.
fn rebinds_self(n: Node<'_>, in_pattern: bool, source: &str) -> bool {
    match n.kind() {
        "identifier" => in_pattern && text(n, source) == "self",
        "import_statement" => named(n).into_iter().any(|c| {
            c.kind() == "dotted_name" && text(c, source).split('.').next() == Some("self")
        }),
        "import_from_statement" => {
            let mut cursor = n.walk();
            let found = n
                .children_by_field_name("name", &mut cursor)
                .any(|c| c.kind() == "dotted_name" && text(c, source) == "self");
            found
        }
        "assignment"
        | "augmented_assignment"
        | "named_expression"
        | "for_statement"
        | "for_in_clause"
        | "as_pattern"
        | "aliased_import" => assigns_self(
            n.child_by_field_name("alias")
                .or_else(|| n.child_by_field_name("left"))
                .or_else(|| n.child_by_field_name("name")),
            source,
        ),
        _ => false,
    }
}

/// parser-helpers.mjs `calls`: the `self.m()` calls inside `ranges` made by
/// undecorated methods of top-level classes that never rebind `self`, each
/// resolved along its class's C3 order to the first local class defining
/// `m` (earlier bases that are not local classes are reported). A call
/// into its own class, or to a target already inside `ranges`, is not a
/// lead. Empty when the source does not parse or a hierarchy is cyclic or
/// inconsistent, or once `deadline` passes.
fn calls(source: &str, ranges: &[SourceRange], deadline: Instant) -> Vec<Call> {
    let Some(tree) = units::parse_python(source) else {
        return Vec::new();
    };
    let classes = unique(
        named(tree.root_node())
            .into_iter()
            .filter(|n| py_definition(*n).is_some_and(|d| d.kind() == "class_definition"))
            .collect(),
        source,
    );
    let mut methods: HashMap<String, Vec<(String, Node<'_>)>> = HashMap::new();
    let mut bases: HashMap<String, Vec<String>> = HashMap::new();
    let mut class_of: HashMap<String, Node<'_>> = HashMap::new();
    for (key, wrapped) in &classes {
        let Some(class) = py_definition(*wrapped) else {
            continue;
        };
        let definitions = py_body(class)
            .into_iter()
            .filter(|c| py_definition(*c).is_some_and(|d| d.kind() == "function_definition"))
            .collect();
        methods.insert(key.clone(), unique(definitions, source));
        // Only bare names can resolve to local classes; any other base
        // stays explicitly unknown.
        let superclasses = class
            .child_by_field_name("superclasses")
            .map(named)
            .unwrap_or_default();
        bases.insert(
            key.clone(),
            superclasses
                .into_iter()
                .filter(|c| c.kind() != "keyword_argument" && c.kind() != "comment")
                .map(|c| text(unparenthesized(Some(c)).unwrap_or(c), source).to_string())
                .collect(),
        );
        class_of.insert(key.clone(), class);
    }
    let lines: Vec<&str> = source.split('\n').collect();
    let (mut memo, mut seen, mut result) = (HashMap::new(), HashSet::new(), Vec::new());
    for (owner, _) in &classes {
        for (method, wrapped) in &methods[owner] {
            let Some(function) = py_definition(*wrapped) else {
                continue;
            };
            let parameters: Vec<Node<'_>> = function
                .child_by_field_name("parameters")
                .map(named)
                .unwrap_or_default()
                .into_iter()
                .filter(|n| n.kind() != "comment")
                .collect();
            let first = parameters.first().copied();
            let parameter = match first {
                Some(first) if first.kind() == "identifier" => Some(first),
                _ => first
                    .unwrap_or(function)
                    .child_by_field_name("name")
                    .or_else(|| {
                        first.and_then(|f| named(f).into_iter().find(|c| c.kind() == "identifier"))
                    }),
            };
            if wrapped.kind() == "decorated_definition"
                || parameter.map(|p| text(p, source)) != Some("self")
                || first.is_some_and(|f| text(f, source).starts_with('*'))
            {
                continue;
            }
            let nodes = body_nodes(function);
            // Conservatively no leads once this scope may rebind self.
            let mut rebinds = false;
            for (n, in_pattern) in &nodes {
                if Instant::now() >= deadline {
                    return Vec::new();
                }
                if rebinds_self(*n, *in_pattern, source) {
                    rebinds = true;
                    break;
                }
            }
            if rebinds {
                continue;
            }
            for (call, _) in nodes.iter().filter(|(n, _)| n.kind() == "call") {
                if Instant::now() >= deadline {
                    return Vec::new();
                }
                let Some(callee) = unparenthesized(call.child_by_field_name("function"))
                    .filter(|f| f.kind() == "attribute")
                else {
                    continue;
                };
                if unparenthesized(callee.child_by_field_name("object")).map(|o| text(o, source))
                    != Some("self")
                {
                    continue;
                }
                let row = call.start_position().row + 1;
                if !ranges
                    .iter()
                    .any(|r| r.start_line <= row && r.end_line >= row)
                {
                    continue;
                }
                let Some(order) = mro(owner, &[], &bases, &mut memo, deadline) else {
                    return Vec::new();
                };
                let attribute = callee
                    .child_by_field_name("attribute")
                    .map(|a| text(a, source));
                let mut unknown = Vec::new();
                for ancestor in order {
                    let Some(definitions) = methods.get(&ancestor) else {
                        unknown.push(ancestor);
                        continue;
                    };
                    let Some((target_name, target)) = definitions
                        .iter()
                        .find(|(name, _)| Some(name.as_str()) == attribute)
                    else {
                        continue;
                    };
                    if ancestor == *owner {
                        break;
                    }
                    let mut target_range = py_range(*target);
                    if ranges.iter().any(|r| {
                        r.start_line <= target_range.start_line
                            && r.end_line >= target_range.end_line
                    }) {
                        break;
                    }
                    let identity = (
                        owner.clone(),
                        method.clone(),
                        ancestor.clone(),
                        target_name.clone(),
                    );
                    if !seen.insert(identity) {
                        break;
                    }
                    while target_range.start_line > 1
                        && lines[target_range.start_line - 2]
                            .trim_start()
                            .starts_with('#')
                    {
                        target_range.start_line -= 1;
                    }
                    let class = class_of[&ancestor];
                    result.push(Call {
                        caller: format!("{owner}.{method}"),
                        name: format!("{ancestor}.{target_name}"),
                        range: target_range,
                        unknown_earlier_bases: unknown.clone(),
                        owner_header: range(
                            class.start_position().row + 1,
                            py_body(class).first().map_or(0, |b| b.start_position().row),
                        ),
                    });
                    break;
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn later() -> Instant {
        Instant::now() + std::time::Duration::from_secs(60)
    }

    #[test]
    fn query_identifiers_are_unicode_words() {
        let tokens: Vec<&str> = IDENTIFIER
            .find_iter("how does _parse_café(x) call 2nd Löader?")
            .map(|m| m.as_str())
            .collect();
        assert_eq!(
            tokens,
            ["how", "does", "_parse_café", "x", "call", "nd", "Löader"]
        );
    }

    /// 600 two-line filler functions around a documented `target_function`.
    fn big_module() -> String {
        let mut source = String::from("import os\n\n");
        for i in 0..600 {
            if i == 300 {
                source.push_str("def target_function(value):\n    \"\"\"Docs.\"\"\"\n    total = value * 2\n    return total\n\n\n");
            }
            source.push_str(&format!("def filler_{i:03}(x):\n    return x + {i}\n\n\n"));
        }
        source
    }

    fn headers(text: &str) -> Vec<&str> {
        text.split('\n')
            .filter(|line| line.starts_with("--- source line"))
            .collect()
    }

    #[test]
    fn the_sampler_splits_its_budget_around_a_query_named_def() {
        let source = big_module();
        let text = python_preview(&source, "what does target_function return?", 16_384).unwrap();
        assert!(text.len() <= 16_384);
        let target = source
            .lines()
            .position(|l| l.starts_with("def target_function"))
            .unwrap()
            + 1;
        let count = source.split('\n').count();
        let found = headers(&text);
        assert!(found[0].starts_with("--- source lines 1-"));
        assert!(found[0].ends_with("; opening context ---"));
        // the opening context gets a quarter of the budget
        let opening = text.split("\n--- ").next().unwrap();
        assert!(opening.len() < 4096 && opening.len() > 3_900);
        let (third, two_thirds) = (count / 3 + 1, 2 * count / 3 + 1);
        assert_eq!(
            found[1..],
            [
                format!("--- source lines {target}-{target}; query-named declaration header ---"),
                // past the docstring
                format!(
                    "--- source lines {}-{}; query-named implementation ---",
                    target + 2,
                    target + 3
                ),
                format!(
                    "--- source lines {third}-{}; distributed context ---",
                    third + 31
                ),
                format!(
                    "--- source lines {two_thirds}-{}; distributed context ---",
                    two_thirds + 31
                ),
                format!(
                    "--- source lines {}-{count}; distributed context ---",
                    count - 31
                ),
            ]
        );
        assert!(text.contains("\n    total = value * 2\n    return total\n"));
        // nothing named: the opening and the distributed windows only
        let text = python_preview(&source, "how are numbers added?", 16_384).unwrap();
        assert_eq!(headers(&text).len(), 4);
        assert!(!text.contains("query-named"));
        assert!(python_preview("def a():\n    pass\n", "a", 16_384).is_none());
    }

    #[test]
    fn a_named_method_brings_its_class_header_and_a_one_line_def_is_cut_by_column() {
        let mut source = String::from(
            "class Holder(Base):\n    \"\"\"Holds.\"\"\"\n    kind = 1\n\n    def target(self): return self.kind\n",
        );
        source.push_str(&"# padding line for the sampler budget\n".repeat(600));
        let text = python_preview(&source, "target", 16_384).unwrap();
        assert!(text
            .contains("--- source lines 1-1; enclosing class context ---\nclass Holder(Base):\n"));
        assert!(text.contains(
            "--- source line 5, bytes 0-22; query-named declaration header; partial line ---\n    def target(self): \n"
        ));
        assert!(text.contains(
            "--- source lines 5-5; query-named implementation ---\n    def target(self): return self.kind\n"
        ));
    }

    #[test]
    fn clip_never_splits_a_character() {
        assert_eq!(clip("aé", 2, false), "a");
        assert_eq!(clip("éa", 2, true), "a");
        assert_eq!(clip("abc", 5, false), "abc");
    }

    const HIERARCHY: &str = "class Base:
    def run(self):
        return 0

class Left(Base):
    def helper(self):
        return 1

class Right(Base):
    def helper(self):
        return 2

    # runs the right way
    def run(self):
        return 3

class Child(Left, External, Right):
    def own(self):
        return 4

    def go(self):
        self.helper()
        self.run()
        self.own()
        return self.missing()
";

    #[test]
    fn c3_resolves_a_diamond_and_reports_unknown_earlier_bases() {
        let bases = HashMap::from([
            ("Left".to_string(), vec!["Base".to_string()]),
            ("Right".to_string(), vec!["Base".to_string()]),
            (
                "Child".to_string(),
                vec!["Left".into(), "External".into(), "Right".into()],
            ),
        ]);
        let order = mro("Child", &[], &bases, &mut HashMap::new(), later()).unwrap();
        assert_eq!(order, ["Child", "Left", "External", "Right", "Base"]);
        // inconsistent: D wants B before A, A derives from B
        let bad = HashMap::from([
            ("A".to_string(), vec!["B".to_string()]),
            ("D".to_string(), vec!["B".into(), "A".into()]),
        ]);
        assert!(mro("D", &[], &bad, &mut HashMap::new(), later()).is_none());
        let cyclic = HashMap::from([
            ("A".to_string(), vec!["B".to_string()]),
            ("B".to_string(), vec!["A".to_string()]),
        ]);
        assert!(mro("A", &[], &cyclic, &mut HashMap::new(), later()).is_none());

        let found = calls(HIERARCHY, &[range(21, 25)], later());
        let summary: Vec<_> = found
            .iter()
            .map(|c| {
                (
                    c.caller.as_str(),
                    c.name.as_str(),
                    (c.range.start_line, c.range.end_line),
                    c.unknown_earlier_bases.clone(),
                    (c.owner_header.start_line, c.owner_header.end_line),
                )
            })
            .collect();
        // helper: Left's; run: past the unknown External to Right's, whose
        // comment joins it; own: the class's own; missing: nowhere
        assert_eq!(
            summary,
            [
                ("Child.go", "Left.helper", (6, 7), vec![], (5, 5)),
                (
                    "Child.go",
                    "Right.run",
                    (13, 15),
                    vec!["External".to_string()],
                    (9, 9)
                ),
            ]
        );
        // a target already selected is no lead
        let found = calls(HIERARCHY, &[range(5, 8), range(21, 25)], later());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Right.run");
    }

    #[test]
    fn a_method_that_may_rebind_self_or_a_cyclic_hierarchy_gives_no_calls() {
        let rebinding = "class A(B):\n    def go(self):\n        self = other\n        self.run()\n\nclass B:\n    def run(self):\n        pass\n";
        assert!(calls(rebinding, &[range(1, 8)], later()).is_empty());
        let cyclic = "class A(B):\n    def go(self):\n        self.run()\n\nclass B(A):\n    def run(self):\n        pass\n";
        assert!(calls(cyclic, &[range(2, 3)], later()).is_empty());
    }

    #[test]
    fn nested_match_patterns_are_scanned_once() {
        // rescanning every nested pattern's subtree took seconds here
        let method = |capture: &str| {
            let depth = 6_000;
            format!(
                "class A(B):\n    def go(self, v):\n        match v:\n            case {}{capture}{}:\n                pass\n        self.run()\n\nclass B:\n    def run(self):\n        pass\n",
                "[".repeat(depth),
                "]".repeat(depth)
            )
        };
        let started = Instant::now();
        // a `self` deep in the pattern may rebind it: no lead
        assert!(calls(&method("self"), &[range(2, 6)], later()).is_empty());
        let found = calls(&method("other"), &[range(2, 6)], later());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "B.run");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn a_passed_deadline_stops_the_structural_passes() {
        let past = Instant::now();
        assert!(calls(HIERARCHY, &[range(21, 25)], past).is_empty());
        let bases = HashMap::from([("A".to_string(), vec!["B".to_string()])]);
        assert!(mro("A", &[], &bases, &mut HashMap::new(), past).is_none());
    }
}
