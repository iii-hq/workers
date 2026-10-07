//! Judge-driven navigation: breadth-first from the walk root, two levels
//! per round, admitting directories and files the judge scores above 0.5.
//!
//! Ported from dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit
//! 82ef1fd: `packages/core/src/retrieve.ts` `score` (79-155), `discover`
//! (297-417) and `parallel` (450-464).
//!
//! Deviations: concurrency is the worker-wide judge slots, not 32 stage
//! workers; one ask deadline bounds every call; an outage stops the walk
//! (jevgrep keeps asking a failing provider); the level reads the snapshot
//! it previews instead of re-reading before chunking; answers are cached in
//! memory only ([`super::AnswerCache`]).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use iii_helpers::observability::opentelemetry::trace::FutureExt as _;
use iii_helpers::observability::opentelemetry::Context;
use judge_contract::Evaluation;
use tokio::task::JoinSet;

use super::prompts::{self, FilePreview, Kind, NavigationItem};
use super::units::MAX_PARSE_BYTES;
use super::walk::{self, Snap, Snapshot, Tree};
use crate::code::judge::{Evaluator, JudgeError, Scores};

/// jevgrep's per-request navigation caps.
pub const MAX_ITEMS: usize = 128;
pub const MAX_REQUEST_BYTES: usize = 38_000;
/// Chunk size for a preview too large for one request.
const CHUNK_BYTES: usize = 12_000;
/// Admission threshold for directories and files (strict).
pub const ADMIT: f64 = 0.5;

/// Why an ask stopped scheduling judge work.
#[derive(Debug, Clone, PartialEq)]
pub enum Stop {
    Deadline,
    /// The ask spent its judge token budget.
    Budget,
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: String,
    /// Hash of the bytes the judge admitted; later passes re-check it.
    pub content_hash: String,
    pub score: f64,
}

#[derive(Debug, Default)]
pub struct State {
    pub issues: BTreeMap<String, u64>,
    pub stop: Option<Stop>,
    /// Calls the judge replied to (a deadline or a local pause refusal
    /// is not a reply), and their questions.
    pub judge_calls: u64,
    pub questions: u64,
    pub input_tokens: u64,
    /// Answers served from the in-memory cache instead of a call.
    pub cache_hits: u64,
    pub candidates: HashMap<String, Candidate>,
    /// Candidate paths in first-admission order (jevgrep's `Map` order).
    admitted: Vec<String>,
    visited: HashSet<String>,
    /// Discovery previews by path, reused by the file assessment.
    pub previews: HashMap<String, FilePreview>,
    /// Content hash of each file as previewed.
    hashes: HashMap<String, String>,
    /// Entries of the directories discovery listed (jevgrep `entriesSeen`).
    entries_seen: usize,
}

/// One ask's shared discovery state.
pub struct Run {
    pub query: String,
    pub tree: Tree,
    pub evaluate: Evaluator,
    pub deadline: Instant,
    /// Navigation request byte cap: jevgrep's, or twice a small window.
    pub cap: usize,
    /// Evidence state byte cap, likewise.
    pub state_cap: usize,
    /// Twice a known window, for requests jevgrep does not cap.
    pub window_cap: usize,
    /// Answer-cache namespace (the judge provider and its listed models);
    /// `None` bypasses the cache.
    pub cache: Option<String>,
    /// Judge calls this ask schedules at once (the worker's slot count).
    pub slots: usize,
    /// Judge input tokens the ask may spend; 0 = unlimited.
    pub token_budget: u64,
    pub state: Mutex<State>,
}

impl Run {
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn issue(&self, kind: &str) {
        *self.state().issues.entry(kind.to_string()).or_default() += 1;
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

    /// One judge call under the ask deadline, counted in the stats. A
    /// missed deadline, a call the judge timed out or failed, or a rejected
    /// request is an issue and an outage halts the ask; `TooLarge` is left
    /// to the caller, which may split.
    pub async fn call(&self, request: Evaluation) -> Result<Scores, JudgeError> {
        let cache_key = self
            .cache
            .as_deref()
            .and_then(|namespace| super::cache_key(namespace, &request));
        if let Some(scores) = cache_key
            .as_ref()
            .and_then(|key| super::cached(key, &request.questions))
        {
            self.state().cache_hits += 1;
            return Ok(scores);
        }
        // A spent budget refuses new calls like a passed deadline, without
        // counting another deadline issue.
        if self.state().stop == Some(Stop::Budget) {
            return Err(JudgeError::Deadline);
        }
        let questions = request.questions.len() as u64;
        let request_ids: Vec<String> = request.questions.keys().cloned().collect();
        let outcome = tokio::time::timeout_at(
            self.deadline.into(),
            (self.evaluate)(request, self.deadline),
        )
        .await
        .unwrap_or(Err(JudgeError::Deadline))
        // evaluator.ts: every question answered in [0, 1], or the reply is
        // invalid (`judge::classify` checks this too; the seam may not).
        .and_then(|(scores, tokens)| {
            let valid = request_ids
                .iter()
                .all(|id| scores.get(id).is_some_and(|p| (0.0..=1.0).contains(p)));
            if valid {
                Ok((scores, tokens))
            } else {
                Err(JudgeError::Unavailable("invalid_response".into()))
            }
        });
        let mut state = self.state();
        if !matches!(outcome, Err(JudgeError::Deadline | JudgeError::Paused)) {
            state.judge_calls += 1;
            state.questions += questions;
        }
        match outcome {
            Ok((scores, tokens)) => {
                state.input_tokens += tokens;
                // Calls already in flight still land; nothing new starts.
                if self.token_budget > 0
                    && state.input_tokens >= self.token_budget
                    && state.stop.is_none()
                {
                    state.stop = Some(Stop::Budget);
                    *state.issues.entry("token_budget".into()).or_default() += 1;
                }
                drop(state);
                if let Some(key) = cache_key {
                    super::remember(key, &scores);
                }
                return Ok(scores);
            }
            Err(JudgeError::TooLarge) => {}
            Err(JudgeError::Deadline) => {
                // Before the ask's deadline the judge gave up on this call.
                let kind = if Instant::now() < self.deadline {
                    "judge_call_timeout"
                } else {
                    "deadline"
                };
                *state.issues.entry(kind.into()).or_default() += 1
            }
            Err(JudgeError::Invalid) => {
                *state.issues.entry("invalid_response".into()).or_default() += 1
            }
            Err(JudgeError::Rejected(_)) => {
                *state.issues.entry("invalid_request".into()).or_default() += 1
            }
            Err(ref error @ (JudgeError::Unavailable(_) | JudgeError::Paused)) => {
                *state.issues.entry("provider".into()).or_default() += 1;
                state.stop.get_or_insert(Stop::Unavailable(error.reason()));
            }
        }
        outcome.map(|(scores, _)| scores)
    }

    /// Admitted files, best first (retrieve.ts `sortedCandidates`).
    pub fn sorted_candidates(&self) -> Vec<Candidate> {
        let mut candidates: Vec<Candidate> = self.state().candidates.values().cloned().collect();
        candidates.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| walk::locale_cmp(&a.path, &b.path))
        });
        candidates
    }

    /// Admitted files in first-admission order (retrieve.ts `ordered`).
    pub fn admitted(&self) -> Vec<Candidate> {
        let state = self.state();
        state
            .admitted
            .iter()
            .map(|path| state.candidates[path].clone())
            .collect()
    }

    /// retrieve.ts `unchanged`: re-read `candidate`; `None`, and an issue,
    /// once it is no longer the file the judge admitted. Blocking.
    pub fn unchanged(&self, candidate: &Candidate) -> Option<Snapshot> {
        match walk::read(&self.tree, &candidate.path) {
            Snap::Ok(snapshot) if snapshot.content_hash == candidate.content_hash => Some(snapshot),
            Snap::Issue(kind) => {
                self.issue(kind);
                None
            }
            _ => {
                self.issue("changed");
                None
            }
        }
    }

    /// retrieve.ts `parallel`: `work` over `items`, one judge slot each at
    /// most, until the ask stops. Results arrive in completion order.
    pub async fn parallel<T, R, Fut>(
        self: &Arc<Self>,
        items: Vec<T>,
        work: impl Fn(Arc<Self>, T) -> Fut,
    ) -> Vec<R>
    where
        Fut: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        let mut queue = items.into_iter();
        let (mut running, mut results) = (JoinSet::new(), Vec::new());
        loop {
            while running.len() < self.slots && !self.stopped() {
                let Some(item) = queue.next() else {
                    break;
                };
                running.spawn(work(self.clone(), item).with_context(Context::current()));
            }
            let Some(joined) = running.join_next().await else {
                break;
            };
            match joined {
                Ok(result) => results.push(result),
                Err(_) => self.issue("unreadable"),
            }
        }
        results
    }

    /// retrieve.ts `score`: batch `items`, ask the judge, split a batch the
    /// judge found too large and requeue its halves. Only a failed leaf
    /// records an issue.
    pub async fn score(self: &Arc<Self>, items: Vec<NavigationItem>) -> Vec<(NavigationItem, f64)> {
        let (mut batches, oversize) = plan_batches(&self.query, items, self.cap);
        for _ in 0..oversize {
            self.issue("request-size");
        }
        let mut results = Vec::new();
        let mut running = JoinSet::new();
        loop {
            while running.len() < self.slots && !self.stopped() {
                let Some(group) = batches.pop_front() else {
                    break;
                };
                let request = prompts::navigation(&self.query, &group);
                let run = self.clone();
                running.spawn(
                    async move { (group, run.call(request).await) }
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
                Ok(scores) => {
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
                Err(_) => {} // recorded by `call`
            }
        }
        results
    }

    /// retrieve.ts `discover`: breadth-first from `seeds`, two levels per
    /// round. Admitted directories seed the next round; files above the
    /// threshold become candidates, keeping their best score.
    pub async fn discover(self: &Arc<Self>, seeds: Vec<String>) {
        let mut directories = seeds;
        while !directories.is_empty()
            && !self.stopped()
            && self.state().entries_seen < walk::MAX_ENTRIES
        {
            let level = std::mem::take(&mut directories);
            let run = self.clone();
            let (items, chunks) = tokio::task::spawn_blocking(move || run.build_level(level))
                .await
                .unwrap_or_else(|_| {
                    self.issue("unreadable");
                    Default::default()
                });
            let mut classified = self.score(items).await;
            classified.extend(self.score(chunks).await);
            let mut state = self.state();
            for (item, p) in classified {
                match item.kind {
                    Kind::Directory if p > ADMIT => directories.push(item.path),
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
                        if state
                            .candidates
                            .insert(item.path.clone(), candidate)
                            .is_none()
                        {
                            state.admitted.push(item.path);
                        }
                    }
                    _ => {}
                }
            }
        }
        // As in jevgrep, directories left unexplored at a stop count too.
        if !directories.is_empty() {
            self.issue("resource_limit");
        }
    }

    /// Read and preview one round: the `directories`, their child
    /// directories, and every eligible file in both. Returns the items to
    /// score and the chunked items of files whose preview alone is too big
    /// for one request. Every listed entry counts toward [`walk::MAX_ENTRIES`]
    /// (retrieve.ts 303-336).
    fn build_level(&self, directories: Vec<String>) -> (Vec<NavigationItem>, Vec<NavigationItem>) {
        let mut level: Vec<(String, u8)> = directories.into_iter().map(|d| (d, 0)).collect();
        let (mut items, mut chunks) = (Vec::new(), Vec::new());
        let mut index = 0;
        while index < level.len() && !self.stopped() {
            let (path, depth) = level[index].clone();
            index += 1;
            let left = walk::MAX_ENTRIES.saturating_sub(self.state().entries_seen);
            if left == 0 {
                self.issue("resource_limit");
                break;
            }
            if !self.state().visited.insert(path.clone()) {
                continue;
            }
            let listing = self.tree.list(&path, left + 1);
            if listing.unreadable {
                self.issue("unreadable");
            }
            if listing.entries.len() > left {
                self.issue("resource_limit");
            }
            for entry in &listing.entries {
                if self.stopped() {
                    break;
                }
                {
                    let mut state = self.state();
                    if state.entries_seen >= walk::MAX_ENTRIES {
                        *state.issues.entry("resource_limit".into()).or_default() += 1;
                        break;
                    }
                    state.entries_seen += 1;
                }
                let child = walk::join(&path, &entry.name);
                if entry.is_dir {
                    if depth == 0 {
                        level.push((child, 1));
                    } else {
                        let Some(child_preview) = walk::preview_directory(&self.tree, &child)
                        else {
                            self.issue("unreadable");
                            continue;
                        };
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
                let file_preview = walk::preview_file(&snapshot, &self.query);
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
    cap: usize,
) -> (VecDeque<Vec<NavigationItem>>, usize) {
    let bytes =
        |batch: &[NavigationItem]| prompts::request_bytes(&prompts::navigation(query, batch));
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
