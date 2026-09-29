//! Judge-driven navigation: breadth-first over the walked tree, two levels
//! per round, admitting directories and files the judge scores above 0.5.
//!
//! Ported from dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit
//! 82ef1fd: `packages/core/src/retrieve.ts` `score` (79-155) and `discover`
//! (297-417).
//!
//! Deviations: concurrency is the worker-wide judge slots, not 32 stage
//! workers; one ask deadline bounds every call; an outage stops the walk
//! (jevgrep keeps asking a failing provider); the level reads the snapshot
//! it previews instead of re-reading before chunking.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use iii_helpers::observability::opentelemetry::trace::FutureExt as _;
use iii_helpers::observability::opentelemetry::Context;
use tokio::task::JoinSet;

use super::prompts::{self, FilePreview, Kind, NavigationItem, RelationAnchor};
use super::walk::{self, Snap, Tree};
use crate::code::judge::{Evaluator, JudgeError, SLOT_COUNT};

/// jevgrep's per-request navigation caps.
pub const MAX_ITEMS: usize = 128;
pub const MAX_REQUEST_BYTES: usize = 38_000;
/// Files larger than this are admitted on their preview alone.
const MAX_PARSE_BYTES: usize = 1_000_000;
/// Chunk size for a preview too large for one request.
const CHUNK_BYTES: usize = 12_000;
/// Admission threshold for directories and files (strict).
pub const ADMIT: f64 = 0.5;

/// Why an ask stopped scheduling judge work.
#[derive(Debug, Clone, PartialEq)]
pub enum Stop {
    Deadline,
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: String,
    /// Hash of the bytes the judge admitted; later passes re-check it.
    #[allow(dead_code)]
    pub content_hash: String,
    pub score: f64,
}

#[derive(Debug, Default)]
pub struct State {
    pub issues: BTreeMap<String, u64>,
    pub stop: Option<Stop>,
    pub judge_calls: u64,
    pub questions: u64,
    pub input_tokens: u64,
    /// Calls that came back with valid answers.
    pub answered: u64,
    pub candidates: HashMap<String, Candidate>,
    visited: HashSet<String>,
    /// Directories scored ≤ 0.5, kept for the relationship pass.
    pub pruned: Vec<NavigationItem>,
    /// Discovery previews by path, reused by the file assessment.
    pub previews: HashMap<String, FilePreview>,
    /// Content hash of each file as previewed.
    hashes: HashMap<String, String>,
}

/// One ask's shared discovery state.
pub struct Run {
    pub query: String,
    pub tree: Tree,
    pub evaluate: Evaluator,
    pub deadline: Instant,
    /// Navigation request byte cap: jevgrep's, or twice a small window.
    pub cap: usize,
    pub state: Mutex<State>,
}

impl Run {
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn issue(&self, kind: &str) {
        *self.state().issues.entry(kind.to_string()).or_default() += 1;
    }

    fn halt(&self, stop: Stop) {
        self.state().stop.get_or_insert(stop);
    }

    /// True once the ask stopped; passing the deadline stops it.
    pub fn stopped(&self) -> bool {
        let mut state = self.state();
        if state.stop.is_none() && Instant::now() >= self.deadline {
            state.stop = Some(Stop::Deadline);
            *state.issues.entry("deadline".into()).or_default() += 1;
        }
        state.stop.is_some()
    }

    /// retrieve.ts `score`: batch `items`, ask the judge, split a batch the
    /// judge found too large and requeue its halves. Only a failed leaf
    /// records an issue.
    pub async fn score(
        self: &Arc<Self>,
        items: Vec<NavigationItem>,
        anchor: Option<&RelationAnchor>,
    ) -> Vec<(NavigationItem, f64)> {
        let (mut batches, oversize) = plan_batches(&self.query, items, anchor, self.cap);
        for _ in 0..oversize {
            self.issue("request-size");
        }
        let mut results = Vec::new();
        let mut running = JoinSet::new();
        loop {
            while running.len() < SLOT_COUNT && !self.stopped() {
                let Some(group) = batches.pop_front() else {
                    break;
                };
                let request = prompts::navigation(&self.query, &group, anchor);
                {
                    let mut state = self.state();
                    state.judge_calls += 1;
                    state.questions += group.len() as u64;
                }
                let (evaluate, deadline) = (self.evaluate.clone(), self.deadline);
                running.spawn(
                    async move {
                        let outcome =
                            tokio::time::timeout_at(deadline.into(), evaluate(request, deadline))
                                .await
                                .unwrap_or(Err(JudgeError::Deadline));
                        (group, outcome)
                    }
                    .with_context(Context::current()),
                );
            }
            let Some(joined) = running.join_next().await else {
                break;
            };
            let Ok((group, outcome)) = joined else {
                self.issue("provider");
                continue;
            };
            match outcome {
                Ok((scores, tokens)) => {
                    {
                        let mut state = self.state();
                        state.input_tokens += tokens;
                        state.answered += 1;
                    }
                    for (i, item) in group.into_iter().enumerate() {
                        let p = scores.get(&prompts::key("q", i)).copied().unwrap_or(0.0);
                        results.push((item, p));
                    }
                }
                Err(JudgeError::TooLarge) if group.len() > 1 => {
                    let mut first = group;
                    let second = first.split_off(first.len().div_ceil(2));
                    batches.push_back(first);
                    batches.push_back(second);
                }
                Err(JudgeError::TooLarge) => self.issue("request-size"),
                Err(JudgeError::Deadline) => self.issue("deadline"),
                Err(JudgeError::Rejected(_)) => self.issue("invalid_request"),
                Err(JudgeError::Unavailable(reason)) => {
                    self.issue("provider");
                    self.halt(Stop::Unavailable(reason));
                }
            }
        }
        results
    }

    /// retrieve.ts `discover`: breadth-first from `seeds`, two levels per
    /// round. Admitted directories seed the next round; files above the
    /// threshold become candidates, keeping their best score.
    pub async fn discover(self: &Arc<Self>, seeds: Vec<String>) {
        let mut directories = seeds;
        while !directories.is_empty() && !self.stopped() {
            let level = std::mem::take(&mut directories);
            let run = self.clone();
            let (items, chunks) = tokio::task::spawn_blocking(move || run.build_level(level))
                .await
                .unwrap_or_else(|_| {
                    self.issue("unreadable");
                    Default::default()
                });
            let mut classified = self.score(items, None).await;
            classified.extend(self.score(chunks, None).await);
            let mut state = self.state();
            for (item, p) in classified {
                match item.kind {
                    Kind::Directory if p > ADMIT => directories.push(item.path),
                    Kind::Directory => {
                        state.pruned.retain(|pruned| pruned.path != item.path);
                        state.pruned.push(item);
                    }
                    Kind::File if p > ADMIT => {
                        if state
                            .candidates
                            .get(&item.path)
                            .is_some_and(|c| c.score >= p)
                        {
                            continue;
                        }
                        let content_hash =
                            state.hashes.get(&item.path).cloned().unwrap_or_default();
                        let candidate = Candidate {
                            path: item.path.clone(),
                            content_hash,
                            score: p,
                        };
                        state.candidates.insert(item.path, candidate);
                    }
                    Kind::File => {}
                }
            }
        }
        if !directories.is_empty() && self.state().stop.is_none() {
            self.issue("resource_limit");
        }
    }

    /// Read and preview one round: the `directories`, their child
    /// directories, and every eligible file in both. Returns the items to
    /// score and the chunked items of files whose preview alone is too big
    /// for one request.
    fn build_level(&self, directories: Vec<String>) -> (Vec<NavigationItem>, Vec<NavigationItem>) {
        let mut level: Vec<(String, u8)> = directories.into_iter().map(|d| (d, 0)).collect();
        let (mut items, mut chunks) = (Vec::new(), Vec::new());
        let mut index = 0;
        while index < level.len() && !self.stopped() {
            let (path, depth) = level[index].clone();
            index += 1;
            if !self.state().visited.insert(path.clone()) {
                continue;
            }
            for entry in self.tree.children.get(&path).into_iter().flatten() {
                if self.stopped() {
                    break;
                }
                let child = walk::join(&path, &entry.name);
                if entry.is_dir {
                    if depth == 0 {
                        level.push((child, 1));
                    } else {
                        let child_preview = walk::preview_directory(&self.tree, &child);
                        items.push(NavigationItem {
                            path: child,
                            kind: Kind::Directory,
                            source_range: None,
                            file_preview: None,
                            child_preview: Some(child_preview),
                        });
                    }
                    continue;
                }
                let snapshot = match walk::read(&self.tree, &child) {
                    Snap::Ok(snapshot) => snapshot,
                    Snap::Excluded => continue,
                    Snap::Issue(kind) => {
                        self.issue(kind);
                        continue;
                    }
                };
                let file_preview = walk::preview_file(&snapshot);
                {
                    let mut state = self.state();
                    state.previews.insert(child.clone(), file_preview.clone());
                    state
                        .hashes
                        .insert(child.clone(), snapshot.content_hash.clone());
                }
                let size = snapshot.source.len();
                if size > MAX_PARSE_BYTES {
                    self.issue("resource_limit");
                }
                let item = NavigationItem {
                    path: child,
                    kind: Kind::File,
                    source_range: None,
                    file_preview: Some(file_preview),
                    child_preview: None,
                };
                let oversize = size <= MAX_PARSE_BYTES
                    && prompts::request_bytes(&prompts::navigation(
                        &self.query,
                        std::slice::from_ref(&item),
                        None,
                    )) > self.cap;
                if !oversize {
                    items.push(item);
                    continue;
                }
                // Too large for preview scoring: bounded chunks, each
                // scored on its own; the file keeps its best chunk.
                let preview = item.file_preview.as_ref().expect("file item");
                for unit in walk::split_source(&snapshot.source, CHUNK_BYTES) {
                    let text = snapshot.source[unit.byte_start..unit.byte_end].to_string();
                    chunks.push(NavigationItem {
                        path: item.path.clone(),
                        kind: Kind::File,
                        source_range: None,
                        file_preview: Some(FilePreview {
                            size_bytes: preview.size_bytes,
                            extension: preview.extension.clone(),
                            preview_bytes: text.len(),
                            text,
                            truncated: true,
                            range: "sampled source ranges".into(),
                            declarations: None,
                            declaration_index_truncated: None,
                        }),
                        child_preview: None,
                    });
                }
            }
        }
        (items, chunks)
    }
}

/// Group `items` into requests of at most [`MAX_ITEMS`] items and `cap`
/// bytes; an item over `cap` on its own is dropped and counted.
pub fn plan_batches(
    query: &str,
    items: Vec<NavigationItem>,
    anchor: Option<&RelationAnchor>,
    cap: usize,
) -> (VecDeque<Vec<NavigationItem>>, usize) {
    let bytes = |batch: &[NavigationItem]| {
        prompts::request_bytes(&prompts::navigation(query, batch, anchor))
    };
    let (mut batches, mut batch, mut oversize) = (VecDeque::new(), Vec::new(), 0);
    for item in items {
        if bytes(std::slice::from_ref(&item)) > cap {
            oversize += 1;
            continue;
        }
        if !batch.is_empty()
            && (batch.len() >= MAX_ITEMS || {
                batch.push(item.clone());
                let over = bytes(&batch) > cap;
                batch.pop();
                over
            })
        {
            batches.push_back(std::mem::take(&mut batch));
        }
        batch.push(item);
    }
    if !batch.is_empty() {
        batches.push_back(batch);
    }
    (batches, oversize)
}
