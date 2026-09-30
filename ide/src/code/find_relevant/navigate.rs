//! Judge-driven navigation: breadth-first over the walked tree, two levels
//! per round, admitting directories and files the judge scores above 0.5.
//!
//! Ported from dzhng/jevgrep (MIT, Copyright (c) 2026 David Zhang), commit
//! 82ef1fd: `packages/core/src/retrieve.ts` `score` (79-155),
//! `withDirectoryContent` (197-241), `discover` (297-417), `parallel`
//! (450-464) and the relationship pass (468-505).
//!
//! Deviations: concurrency is the worker-wide judge slots, not 32 stage
//! workers; one ask deadline bounds every call; an outage stops the walk
//! (jevgrep keeps asking a failing provider); the level reads the snapshot
//! it previews instead of re-reading before chunking, and content samples
//! are not re-hashed before each judge attempt; answers are cached in
//! memory only ([`super::AnswerCache`]).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use iii_helpers::observability::opentelemetry::trace::FutureExt as _;
use iii_helpers::observability::opentelemetry::Context;
use judge_contract::Evaluation;
use tokio::task::JoinSet;

use super::prompts::{self, ContentSample, FilePreview, Kind, NavigationItem, RelationAnchor};
use super::units::{self, MAX_PARSE_BYTES};
use super::walk::{self, Snap, Snapshot, Tree};
use crate::code::judge::{Evaluator, JudgeError, Scores, SLOT_COUNT};

/// jevgrep's per-request navigation caps.
pub const MAX_ITEMS: usize = 128;
pub const MAX_REQUEST_BYTES: usize = 38_000;
/// Chunk size for a preview too large for one request.
const CHUNK_BYTES: usize = 12_000;
/// Admission threshold for directories and files (strict).
pub const ADMIT: f64 = 0.5;
/// Content samples of one pruned directory: a budget split across its
/// files, a floor per file, and the preview's JSON ceiling.
const SAMPLE_BYTES: usize = 16_000;
const SAMPLE_FLOOR: usize = 80;
const SAMPLE_JSON_BYTES: usize = 28_000;
/// An anchor's class list stays under this many JSON bytes.
const ANCHOR_CLASSES_BYTES: usize = 4_000;

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
    /// Evidence state byte cap, likewise.
    pub state_cap: usize,
    /// Twice a known window, for requests jevgrep does not cap.
    pub window_cap: usize,
    /// Answer-cache namespace (the judge provider, `""` for the hub's
    /// default); `None` bypasses the cache.
    pub cache: Option<String>,
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
    /// missed deadline or a rejected request is an issue and an outage
    /// halts the ask; `TooLarge` is left to the caller, which may split.
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
                drop(state);
                if let Some(key) = cache_key {
                    super::remember(key, &scores);
                }
                return Ok(scores);
            }
            Err(JudgeError::TooLarge) => {}
            Err(JudgeError::Deadline) => *state.issues.entry("deadline".into()).or_default() += 1,
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
            while running.len() < SLOT_COUNT && !self.stopped() {
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
    /// threshold become candidates, keeping their best score. Under an
    /// `anchor` every directory is judged on its content samples, and a
    /// pruned one is not kept again.
    pub async fn discover(self: &Arc<Self>, seeds: Vec<String>, anchor: Option<&RelationAnchor>) {
        let mut directories = seeds;
        while !directories.is_empty() && !self.stopped() {
            let level = std::mem::take(&mut directories);
            let (run, owned) = (self.clone(), anchor.cloned());
            let (items, chunks) =
                tokio::task::spawn_blocking(move || run.build_level(level, owned.as_ref()))
                    .await
                    .unwrap_or_else(|_| {
                        self.issue("unreadable");
                        Default::default()
                    });
            let mut classified = self.score(items, anchor).await;
            classified.extend(self.score(chunks, anchor).await);
            let mut state = self.state();
            for (item, p) in classified {
                match item.kind {
                    Kind::Directory if p > ADMIT => directories.push(item.path),
                    Kind::Directory if anchor.is_some() => {}
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
                        if state
                            .candidates
                            .insert(item.path.clone(), candidate)
                            .is_none()
                        {
                            state.admitted.push(item.path);
                        }
                    }
                    Kind::File => {}
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
    /// for one request.
    fn build_level(
        &self,
        directories: Vec<String>,
        anchor: Option<&RelationAnchor>,
    ) -> (Vec<NavigationItem>, Vec<NavigationItem>) {
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
                        let item = NavigationItem {
                            path: child,
                            kind: Kind::Directory,
                            source_range: None,
                            file_preview: None,
                            child_preview: Some(child_preview),
                        };
                        items.push(match anchor {
                            Some(_) => self.with_directory_content(item),
                            None => item,
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
                        anchor,
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

    /// retrieve.ts `withDirectoryContent`: add head, middle and tail
    /// samples of the directory's previewed files, shrunk until the
    /// preview's JSON fits. Offsets and lengths count UTF-16 code units, as
    /// in JavaScript. Blocking.
    pub(super) fn with_directory_content(&self, mut item: NavigationItem) -> NavigationItem {
        let Some(preview) = item.child_preview.as_mut() else {
            return item;
        };
        let files: Vec<String> = preview
            .entries
            .iter()
            .filter(|entry| entry.kind == Kind::File)
            .map(|entry| entry.name.clone())
            .collect();
        let per_file = SAMPLE_FLOOR.max(SAMPLE_BYTES / files.len().max(1));
        let part = per_file / 3;
        let mut samples = Vec::new();
        for name in files {
            if self.stopped() {
                break;
            }
            let snapshot = match walk::read(&self.tree, &walk::join(&item.path, &name)) {
                Snap::Ok(snapshot) => snapshot,
                Snap::Excluded => continue,
                Snap::Issue(kind) => {
                    self.issue(kind);
                    continue;
                }
            };
            if snapshot.source.len() > MAX_PARSE_BYTES {
                continue;
            }
            let units: Vec<u16> = snapshot.source.encode_utf16().collect();
            let length = units.len();
            let source = if length <= per_file {
                snapshot.source
            } else {
                [
                    0,
                    (length / 2).saturating_sub(part / 2),
                    length.saturating_sub(part),
                ]
                .iter()
                .map(|&start| {
                    let end = length.min(start + part);
                    format!(
                        "[character offset {start}]\n{}",
                        String::from_utf16_lossy(&units[start..end])
                    )
                })
                .collect::<Vec<_>>()
                .join("\n...\n")
            };
            samples.push(ContentSample {
                name,
                source,
                truncated: length > per_file,
            });
        }
        preview.content_samples = Some(samples);
        let utf16_len = |text: &str| text.encode_utf16().count();
        while walk::json_len(&*preview) > SAMPLE_JSON_BYTES
            && preview
                .content_samples
                .iter()
                .flatten()
                .any(|s| utf16_len(&s.source) > SAMPLE_FLOOR)
        {
            for sample in preview.content_samples.iter_mut().flatten() {
                let units: Vec<u16> = sample.source.encode_utf16().collect();
                let keep = units.len().min(SAMPLE_FLOOR.max(units.len() * 4 / 5));
                // A cut through a surrogate pair becomes U+FFFD, one unit
                // like the lone surrogate JavaScript keeps.
                sample.source = String::from_utf16_lossy(&units[..keep]);
                sample.truncated = true;
            }
        }
        item
    }

    /// retrieve.ts 468-505, run once after the first discovery: anchor on
    /// the best candidate that declares classes, re-judge every pruned
    /// directory on its content samples for a code relationship to them,
    /// and discover from the directories that have one.
    pub async fn relate(self: &Arc<Self>) {
        let run = self.clone();
        let Ok(Some(anchor)) = tokio::task::spawn_blocking(move || run.anchor()).await else {
            return;
        };
        if self.stopped() {
            return;
        }
        let pruned = self.state().pruned.clone();
        let run = self.clone();
        let items = tokio::task::spawn_blocking(move || {
            let mut items = Vec::new();
            for item in pruned {
                if run.stopped() {
                    break;
                }
                items.push(run.with_directory_content(item));
            }
            items
        })
        .await
        .unwrap_or_default();
        let seeds = self
            .score(items, Some(&anchor))
            .await
            .into_iter()
            .filter(|(_, p)| *p > ADMIT)
            .map(|(item, _)| item.path)
            .collect();
        self.discover(seeds, Some(&anchor)).await;
    }

    /// The first candidate, best first, whose whole-source units name
    /// classes (`Class.context`) listed in under 4000 JSON bytes. Blocking.
    fn anchor(&self) -> Option<RelationAnchor> {
        for candidate in self.sorted_candidates() {
            if candidate.score <= ADMIT || self.stopped() {
                break;
            }
            let Some(snapshot) = self.unchanged(&candidate) else {
                continue;
            };
            let size = snapshot.source.len();
            let syntax = units::inspect(&snapshot.path, &snapshot.source, size.max(4), size.max(1));
            let mut classes: Vec<String> = Vec::new();
            for unit in syntax.units {
                let Some(class) = unit
                    .name
                    .ends_with(".context")
                    .then(|| unit.name.split('.').next().unwrap_or_default())
                else {
                    continue;
                };
                if !classes.iter().any(|known| known == class) {
                    classes.push(class.to_string());
                }
            }
            if !classes.is_empty() && walk::json_len(&classes) < ANCHOR_CLASSES_BYTES {
                return Some(RelationAnchor {
                    path: candidate.path,
                    classes,
                });
            }
        }
        None
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
