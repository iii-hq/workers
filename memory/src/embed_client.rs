//! Fail-soft client for `router::embed` plus the vector backfill task.
//! Embeddings are derived data: every failure here degrades recall to
//! BM25 + entities (and recall responses say so), never blocks a turn,
//! and never touches memory integrity.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::store::Bank;
use crate::types::Memory;

/// Query-path budget: the pre-generate hook runs inside a 3s harness
/// timeout, so the embed call must fail fast and fail open.
const QUERY_EMBED_TIMEOUT_MS: u64 = 2_000;
/// Backfill batches are off every hot path; give the provider room.
const BACKFILL_TIMEOUT_MS: u64 = 20_000;
const BACKFILL_BATCH: usize = 64;
/// Backfill rounds per pass — caps one pass at ~640 embeddings so a huge
/// legacy bank fills over several passes instead of one giant burst.
const BACKFILL_ROUNDS: usize = 10;

/// One `router::embed` reply: the model the provider REPORTS (what the
/// vectors' space actually is, whatever was requested) and one vector per
/// input, in input order.
pub type Embedded = (String, Vec<Vec<f32>>);

pub async fn embed_texts(
    iii: &IIIClient,
    model: &str,
    texts: &[String],
    timeout_ms: u64,
) -> Option<Embedded> {
    let mut payload = json!({ "input": texts });
    if !model.trim().is_empty() {
        payload["model"] = json!(model);
    }
    let reply = iii
        .trigger(TriggerRequest {
            function_id: "router::embed".into(),
            payload,
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await
        .inspect_err(|e| tracing::debug!(error = %e, "router::embed failed"))
        .ok()?;
    // The router always names the model; a reply without one cannot be
    // placed in a space, so it is not used.
    let reported = reply
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())?
        .to_string();
    let embeddings: Vec<Vec<f32>> =
        serde_json::from_value(reply.get("embeddings").cloned().unwrap_or(Value::Null)).ok()?;
    (embeddings.len() == texts.len()).then_some((reported, embeddings))
}

/// Per configured `embedding_model` ("" = provider default), the model the
/// provider last reported for it: the space every stored vector must be in.
/// Learned from every embed reply (queries, backfill, canaries), so a
/// provider whose default changes is noticed on its next reply.
fn current_models() -> &'static Mutex<HashMap<String, String>> {
    static CURRENT: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CURRENT.get_or_init(Default::default)
}

fn learn_model(configured: &str, reported: &str) {
    let mut map = current_models().lock().unwrap_or_else(|e| e.into_inner());
    if map.get(configured).map(String::as_str) != Some(reported) {
        map.insert(configured.to_string(), reported.to_string());
    }
}

fn current_model(configured: &str) -> Option<String> {
    current_models()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(configured)
        .cloned()
}

/// A query embedding and the model the provider reported for it: recall
/// compares it only with vectors stored under exactly that model.
pub struct QueryVector {
    pub model: String,
    pub v: Vec<f32>,
}

impl QueryVector {
    /// `(vector, model)` as `Bank::recall` takes them; `(None, "")` when the
    /// query was not embedded.
    pub fn split(query: Option<&QueryVector>) -> (Option<&[f32]>, &str) {
        query.map_or((None, ""), |q| (Some(q.v.as_slice()), q.model.as_str()))
    }
}

/// Embed the query for one recall. `None` (disabled, router absent, no
/// embed provider, timeout) means lexical-only recall this turn.
pub async fn query_vector(deps: &Deps, query: &str) -> Option<QueryVector> {
    let cfg = deps.config().await;
    if !cfg.embeddings_enabled {
        return None;
    }
    let (model, mut vectors) = embed_texts(
        &deps.iii,
        &cfg.embedding_model,
        &[query.to_string()],
        QUERY_EMBED_TIMEOUT_MS,
    )
    .await?;
    learn_model(&cfg.embedding_model, &model);
    Some(QueryVector {
        model,
        v: vectors.pop()?,
    })
}

/// Embed a bank's new text off the caller's path — after extraction, an
/// explicit save, or a text edit — so it is semantically recallable (and a
/// semantic consolidation candidate) within seconds, not after a restart.
pub fn spawn_backfill(deps: &Deps, bank_name: &str) {
    tokio::spawn(backfill_bank(Arc::new(deps.clone()), bank_name.to_string()));
}

/// One backfill pass over a single bank: embed live memories that have no
/// vector yet, in bounded batches. Spawn-and-forget.
pub async fn backfill_bank(deps: Arc<Deps>, bank_name: String) {
    let cfg = deps.config().await;
    if !cfg.embeddings_enabled {
        return;
    }
    let store = deps.store().await;
    let Ok(bank) = store.bank(&bank_name).await else {
        return;
    };
    let configured = cfg.embedding_model.clone();
    let known = current_model(&configured);
    let total = backfill_pass(&bank, known, |texts| {
        let (iii, configured) = (deps.iii.clone(), configured.clone());
        async move {
            let reply = embed_texts(&iii, &configured, &texts, BACKFILL_TIMEOUT_MS).await;
            if let Some((reported, _)) = &reply {
                learn_model(&configured, reported);
            }
            reply
        }
    })
    .await;
    if total > 0 {
        tracing::info!(bank = %bank_name, embedded = total, "vector backfill pass complete");
    }
}

/// One mutex per bank name. Parallel saves, extraction, and the boot
/// catch-up each spawn a pass; unserialized, every pass would list and embed
/// the same pending memories. Serialized, a later pass finds them embedded.
fn bank_lock(bank: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(bank.to_string())
        .or_default()
        .clone()
}

/// A text the provider rejects outright (one over a llama.cpp server's batch
/// size, say) fails every batch it is in, so it would otherwise keep every
/// memory of its bank from getting a vector. Such texts are found one at a
/// time and sit out windows for [`REJECT_COOLDOWN`], keyed by memory AND
/// text: an edit that fixes the text is embedded at once.
const REJECT_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(600);
/// Embedded alone after a failed batch to tell "the provider is down"
/// (this fails too) from "the provider rejects a text in the batch".
const CANARY: &str = "ok";

fn rejected() -> &'static Mutex<HashMap<String, std::time::Instant>> {
    static REJECTED: OnceLock<Mutex<HashMap<String, std::time::Instant>>> = OnceLock::new();
    REJECTED.get_or_init(Default::default)
}

fn reject_key(bank: &str, memory: &Memory) -> String {
    format!("{bank}\0{}\0{}", memory.id, memory.text)
}

fn reject(bank: &Bank, memories: &[&Memory]) {
    let mut map = rejected().lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    for memory in memories {
        map.insert(reject_key(&bank.name, memory), now);
    }
}

/// The next window of backfill work: unembedded memories, minus texts the
/// provider rejected within the cooldown (only filtered when this bank has
/// any, so the common case stays a bounded scan).
async fn next_window(bank: &Bank, model: &str) -> Vec<Memory> {
    let fresh: std::collections::HashSet<String> = {
        let mut map = rejected().lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, at| at.elapsed() < REJECT_COOLDOWN);
        let prefix = format!("{}\0", bank.name);
        map.keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect()
    };
    if fresh.is_empty() {
        return bank.unembedded(BACKFILL_BATCH, model).await;
    }
    let mut pending = bank.unembedded(usize::MAX, model).await;
    pending.retain(|m| !fresh.contains(&reject_key(&bank.name, m)));
    pending.truncate(BACKFILL_BATCH);
    pending
}

/// Store one embedding; `Err` when the sidecar write failed (stop the pass).
async fn store(bank: &Bank, model: &str, memory: &Memory, v: Vec<f32>) -> Result<bool, ()> {
    bank.set_vector(&memory.id, model, &memory.text, v)
        .await
        .map_err(|e| tracing::warn!(bank = %bank.name, error = %e, "vector write failed"))
}

/// The backfill loop with the embedder injected (`router::embed` in
/// production). `known` is the model the provider last reported (`None`
/// before any reply): vectors stored under any other model are in another
/// space and get re-embedded. Returns how many vectors it stored.
async fn backfill_pass<F, Fut>(bank: &Bank, known: Option<String>, mut embed: F) -> usize
where
    F: FnMut(Vec<String>) -> Fut,
    Fut: Future<Output = Option<Embedded>>,
{
    let lock = bank_lock(&bank.name);
    let _pass = lock.lock().await;
    // "" = space not known yet: only vectorless memories are work. With
    // vectors stored, ask once which space they must be in.
    let mut current = known.unwrap_or_default();
    if current.is_empty() && bank.vector_count().await > 0 {
        match embed(vec![CANARY.to_string()]).await {
            Some((reported, _)) => current = reported,
            None => return 0,
        }
    }
    let mut total = 0usize;
    for _ in 0..BACKFILL_ROUNDS {
        let pending = next_window(bank, &current).await;
        if pending.is_empty() {
            break;
        }
        let texts: Vec<String> = pending.iter().map(|f| f.text.clone()).collect();
        if let Some((reported, vectors)) = embed(texts).await {
            // A new space (the provider's default changed mid-pass): the
            // next round re-lists everything stored under the old one.
            current = reported;
            for (memory, v) in pending.iter().zip(vectors) {
                let Ok(stored) = store(bank, &current, memory, v).await else {
                    return total;
                };
                total += usize::from(stored);
            }
            continue;
        }
        // The batch failed. A failing canary means the provider is down: a
        // later pass retries, and no text is blamed for it.
        match embed(vec![CANARY.to_string()]).await {
            Some((reported, _)) => current = reported,
            None => break,
        }
        // The provider answers, so it rejects some text here: embed one at a
        // time. A failure is blamed on its text only once the provider is
        // seen answering after it (a later success, or a canary at the end
        // or after three failures in a row), so dying mid-loop blames none.
        let mut suspects: Vec<&Memory> = Vec::new();
        for memory in &pending {
            match embed(vec![memory.text.clone()]).await {
                Some((reported, mut v)) if v.len() == 1 => {
                    reject(bank, &suspects);
                    suspects.clear();
                    current = reported;
                    let Ok(stored) = store(bank, &current, memory, v.remove(0)).await else {
                        return total;
                    };
                    total += usize::from(stored);
                }
                _ => {
                    suspects.push(memory);
                    if suspects.len() == 3 {
                        if embed(vec![CANARY.to_string()]).await.is_none() {
                            return total;
                        }
                        reject(bank, &suspects);
                        suspects.clear();
                    }
                }
            }
        }
        if !suspects.is_empty() {
            if embed(vec![CANARY.to_string()]).await.is_none() {
                return total;
            }
            reject(bank, &suspects);
        }
    }
    total
}

/// Backfill sweep across every bank: first 20 s after boot (so llm-router
/// and its providers finish their own boot), then every minute. Event
/// passes (save, edit, extraction) are the fast path; the sweep is what
/// finishes work they leave behind — a failed embed in a bank that then
/// goes quiet, a provider up late, the per-pass cap, a newly pinned
/// `embedding_model`, a reload. With nothing pending it is one RAM scan per
/// bank and no router call; with no embed provider installed, two
/// fast-failing `router::embed` calls (batch and canary) per bank per minute.
pub fn backfill_all(deps: Arc<Deps>) {
    tokio::spawn(async move {
        let mut wait = std::time::Duration::from_secs(20);
        loop {
            tokio::time::sleep(wait).await;
            // ponytail: fixed 1-min retry, no per-bank backoff; add backoff
            // if a persistently failing provider makes the minutely retry
            // costly.
            wait = std::time::Duration::from_secs(60);
            for bank in deps.store().await.bank_names().await {
                backfill_bank(deps.clone(), bank).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::types::{fingerprint, now_ms, Confidence, Memory};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn memory(text: &str) -> Memory {
        Memory {
            id: fingerprint(text),
            text: text.into(),
            entities: vec![],
            tags: vec![],
            confidence: Confidence::Stated,
            corroboration: 0,
            pinned: false,
            source: None,
            created_at: now_ms(),
            updated_at: now_ms(),
            invalid_at: None,
            superseded_by: None,
            revision: 0,
            stated_at: None,
        }
    }

    async fn bank(dir: &tempfile::TempDir, name: &str) -> Arc<Bank> {
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        store.ensure_bank(name, None).await.unwrap().0
    }

    #[tokio::test]
    async fn a_save_and_a_text_edit_become_semantically_recallable() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-save").await;
        let saved = memory("user lives in lisbon");
        bank.commit(saved.clone()).await.unwrap();
        let seen = Mutex::new(Vec::new());
        let embed = |texts: Vec<String>| {
            let v = texts.iter().map(|_| vec![1.0, 0.0]).collect();
            seen.lock().unwrap().extend(texts);
            async move { Some(("m".to_string(), v)) }
        };

        assert_eq!(backfill_pass(&bank, Some("m".into()), embed).await, 1);
        let hits = bank.semantic(&[1.0, 0.0], "m", 5, false).await;
        assert_eq!(hits[0].0.id, saved.id);

        // The edit drops the old text's vector; the next pass embeds the new.
        let mut edited = saved.clone();
        edited.text = "user lives in porto".into();
        edited.revision += 1;
        bank.commit(edited).await.unwrap();
        assert_eq!(backfill_pass(&bank, Some("m".into()), embed).await, 1);
        assert_eq!(
            *seen.lock().unwrap(),
            ["user lives in lisbon", "user lives in porto"]
        );
    }

    #[tokio::test]
    async fn concurrent_passes_embed_each_memory_once() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-concurrent").await;
        bank.commit(memory("user prefers tea")).await.unwrap();
        let embedded = AtomicUsize::new(0);
        let embed = |texts: Vec<String>| {
            embedded.fetch_add(texts.len(), Ordering::SeqCst);
            async move {
                // Yield mid-embed so the other pass runs while this one
                // waits on the "provider".
                tokio::task::yield_now().await;
                Some((
                    "m".to_string(),
                    texts.iter().map(|_| vec![1.0, 0.0]).collect(),
                ))
            }
        };

        let (a, b) = tokio::join!(
            backfill_pass(&bank, Some("m".into()), embed),
            backfill_pass(&bank, Some("m".into()), embed)
        );
        assert_eq!(a + b, 1);
        assert_eq!(embedded.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_edit_during_the_embed_discards_the_old_texts_vector() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-race").await;
        let saved = memory("user lives in lisbon");
        bank.commit(saved.clone()).await.unwrap();
        let mut edited = saved.clone();
        edited.text = "user lives in porto".into();
        edited.revision += 1;
        let edit_pending = std::sync::atomic::AtomicBool::new(true);
        // One axis per city, so the stored vector tells which text it embeds.
        // The edit lands while the provider is still embedding "lisbon".
        let embed = |texts: Vec<String>| {
            let edit = edit_pending
                .swap(false, Ordering::SeqCst)
                .then(|| (bank.clone(), edited.clone()));
            async move {
                if let Some((bank, edited)) = edit {
                    bank.commit(edited).await.unwrap();
                }
                let axis = |t: &String| match t.contains("porto") {
                    true => vec![0.0, 1.0],
                    false => vec![1.0, 0.0],
                };
                Some(("m".to_string(), texts.iter().map(axis).collect()))
            }
        };

        // The stale "lisbon" vector is discarded; the next round embeds porto.
        assert_eq!(backfill_pass(&bank, Some("m".into()), embed).await, 1);
        assert!(bank.semantic(&[1.0, 0.0], "m", 5, false).await.is_empty());
        assert_eq!(bank.semantic(&[0.0, 1.0], "m", 5, false).await.len(), 1);
    }

    #[tokio::test]
    async fn a_failed_embed_leaves_the_memory_for_the_next_pass() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-failed").await;
        bank.commit(memory("user owns a bike")).await.unwrap();

        assert_eq!(
            backfill_pass(&bank, Some("m".into()), |_| async { None }).await,
            0
        );
        assert_eq!(bank.unembedded(10, "m").await.len(), 1);
    }

    /// A provider that rejects any batch holding a "REJECTED" text.
    fn picky(
        calls: &AtomicUsize,
    ) -> impl FnMut(Vec<String>) -> std::future::Ready<Option<Embedded>> + '_ {
        move |texts: Vec<String>| {
            calls.fetch_add(1, Ordering::SeqCst);
            let ok = !texts.iter().any(|t| t.contains("REJECTED"));
            std::future::ready(ok.then(|| {
                (
                    "m".to_string(),
                    texts.iter().map(|_| vec![1.0, 0.0]).collect(),
                )
            }))
        }
    }

    #[tokio::test]
    async fn a_rejected_text_does_not_block_the_rest_of_the_bank() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-rejected").await;
        for t in [
            "REJECTED oversized text",
            "user owns a bike",
            "user likes tea",
        ] {
            bank.commit(memory(t)).await.unwrap();
        }
        let calls = AtomicUsize::new(0);
        assert_eq!(
            backfill_pass(&bank, Some("m".into()), picky(&calls)).await,
            2
        );

        // A later save: the rejected text sits out, the batch succeeds.
        bank.commit(memory("user plays chess")).await.unwrap();
        calls.store(0, Ordering::SeqCst);
        assert_eq!(
            backfill_pass(&bank, Some("m".into()), picky(&calls)).await,
            1
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one clean batch");
        let left = bank.unembedded(10, "m").await;
        assert_eq!(left.len(), 1);
        assert!(left[0].text.starts_with("REJECTED"));
    }

    #[tokio::test]
    async fn rejected_texts_up_front_cost_no_extra_pass() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-rejected-two").await;
        for t in [
            "REJECTED one",
            "REJECTED two",
            "REJECTED three",
            "user owns a bike",
        ] {
            bank.commit(memory(t)).await.unwrap();
        }
        let calls = AtomicUsize::new(0);
        assert_eq!(
            backfill_pass(&bank, Some("m".into()), picky(&calls)).await,
            1
        );
        assert_eq!(bank.unembedded(10, "m").await.len(), 3);
    }

    #[tokio::test]
    async fn an_edit_that_fixes_a_rejected_text_is_embedded_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-rejected-edit").await;
        let mut bad = memory("REJECTED oversized text");
        bank.commit(bad.clone()).await.unwrap();
        bank.commit(memory("user owns a bike")).await.unwrap();
        let calls = AtomicUsize::new(0);
        assert_eq!(
            backfill_pass(&bank, Some("m".into()), picky(&calls)).await,
            1
        );

        bad.text = "user keeps it short".into();
        bad.revision += 1;
        bank.commit(bad).await.unwrap();
        assert_eq!(
            backfill_pass(&bank, Some("m".into()), picky(&calls)).await,
            1
        );
        assert!(bank.unembedded(10, "m").await.is_empty());
    }

    #[tokio::test]
    async fn a_lone_rejected_text_sits_out_the_next_pass() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-rejected-lone").await;
        bank.commit(memory("REJECTED alone")).await.unwrap();
        let calls = AtomicUsize::new(0);
        assert_eq!(
            backfill_pass(&bank, Some("m".into()), picky(&calls)).await,
            0
        );
        // Batch, canary, the text alone (a second chance), canary.
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        calls.store(0, Ordering::SeqCst);
        assert_eq!(
            backfill_pass(&bank, Some("m".into()), picky(&calls)).await,
            0
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "sits out the cooldown");
    }

    #[tokio::test]
    async fn a_lone_text_survives_one_transient_failure() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-lone-blip").await;
        bank.commit(memory("user owns a bike")).await.unwrap();
        let calls = AtomicUsize::new(0);
        // The batch call blips; everything after it answers.
        let blip = |texts: Vec<String>| {
            let first = calls.fetch_add(1, Ordering::SeqCst) == 0;
            let v = (!first).then(|| {
                (
                    "m".to_string(),
                    texts.iter().map(|_| vec![1.0, 0.0]).collect(),
                )
            });
            async move { v }
        };
        assert_eq!(backfill_pass(&bank, Some("m".into()), blip).await, 1);
    }

    #[tokio::test]
    async fn a_provider_dying_mid_loop_blames_no_text() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-dies").await;
        for t in [
            "user owns a bike",
            "user likes tea",
            "user plays chess",
            "user rows on weekends",
        ] {
            bank.commit(memory(t)).await.unwrap();
        }
        let calls = AtomicUsize::new(0);
        // The batch fails, the canary answers, the provider is gone for the
        // next four calls, then back. Three failed singles must re-check the
        // provider (and stop) rather than blame them on a later success.
        let dying = |texts: Vec<String>| {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            let up = n == 1 || n >= 6;
            let v = up.then(|| {
                (
                    "m".to_string(),
                    texts.iter().map(|_| vec![1.0, 0.0]).collect(),
                )
            });
            async move { v }
        };
        assert_eq!(backfill_pass(&bank, Some("m".into()), dying).await, 0);

        // Back up: none of the four sits out, one batch embeds them all.
        let healthy = AtomicUsize::new(0);
        assert_eq!(
            backfill_pass(&bank, Some("m".into()), picky(&healthy)).await,
            4
        );
        assert_eq!(healthy.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_outage_blames_no_text() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-outage").await;
        bank.commit(memory("user owns a bike")).await.unwrap();
        let calls = AtomicUsize::new(0);
        let down = |_: Vec<String>| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { None }
        };
        for _ in 0..5 {
            assert_eq!(backfill_pass(&bank, Some("m".into()), down).await, 0);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 10, "batch + canary per pass");

        // Back up, with two rejected saves arriving: the good text lands.
        for t in ["REJECTED one", "REJECTED two"] {
            bank.commit(memory(t)).await.unwrap();
        }
        let picky_calls = AtomicUsize::new(0);
        assert_eq!(
            backfill_pass(&bank, Some("m".into()), picky(&picky_calls)).await,
            1
        );
    }

    /// A provider whose every reply reports `model`.
    fn reporting<'a>(
        model: &'static str,
        calls: &'a AtomicUsize,
    ) -> impl FnMut(Vec<String>) -> std::future::Ready<Option<Embedded>> + 'a {
        move |texts: Vec<String>| {
            calls.fetch_add(1, Ordering::SeqCst);
            let v = texts.iter().map(|_| vec![0.0, 1.0]).collect();
            std::future::ready(Some((model.to_string(), v)))
        }
    }

    #[tokio::test]
    async fn a_default_model_change_re_embeds_into_the_new_space() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-default-change").await;
        let m = memory("user owns a bike");
        bank.commit(m.clone()).await.unwrap();
        // Embedded while the provider's default was "old-default" (the
        // configured embedding_model was "" both times).
        bank.set_vector(&m.id, "old-default", &m.text, vec![1.0, 0.0])
            .await
            .unwrap();

        // A fresh process: nothing known yet, so one canary learns the space.
        let calls = AtomicUsize::new(0);
        assert_eq!(
            backfill_pass(&bank, None, reporting("new-default", &calls)).await,
            1
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2, "canary + one batch");
        assert!(bank
            .semantic(&[1.0, 0.0], "new-default", 5, false)
            .await
            .is_empty());
        assert_eq!(
            bank.semantic(&[0.0, 1.0], "new-default", 5, false)
                .await
                .len(),
            1
        );
        assert!(bank
            .semantic(&[1.0, 0.0], "old-default", 5, false)
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn a_switch_from_a_pinned_model_to_the_default_re_embeds() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-pinned-to-default").await;
        let m = memory("user likes tea");
        bank.commit(m.clone()).await.unwrap();
        // Embedded under the pinned model (and tagged with what the provider
        // reported for it).
        bank.set_vector(&m.id, "text-embedding-3-large", &m.text, vec![1.0, 0.0])
            .await
            .unwrap();
        // embedding_model is now "": the provider default reports another model.
        let calls = AtomicUsize::new(0);
        let known = Some("text-embedding-3-small".to_string());
        assert_eq!(
            backfill_pass(&bank, known, reporting("text-embedding-3-small", &calls)).await,
            1
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1, "space known: no canary");
        let hits = bank
            .semantic(&[0.0, 1.0], "text-embedding-3-small", 5, false)
            .await;
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn vectors_in_the_current_space_cost_no_call() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-current").await;
        let m = memory("user plays chess");
        bank.commit(m.clone()).await.unwrap();
        bank.set_vector(&m.id, "m", &m.text, vec![1.0, 0.0])
            .await
            .unwrap();
        let calls = AtomicUsize::new(0);
        assert_eq!(
            backfill_pass(&bank, Some("m".into()), reporting("m", &calls)).await,
            0
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_space_change_mid_pass_re_lists_the_old_space() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-mid-pass").await;
        let old = memory("user rows on weekends");
        bank.commit(old.clone()).await.unwrap();
        bank.set_vector(&old.id, "m1", &old.text, vec![1.0, 0.0])
            .await
            .unwrap();
        bank.commit(memory("user collects stamps")).await.unwrap(); // vectorless
                                                                    // The pass believes the space is m1; the provider now answers m2.
        let calls = AtomicUsize::new(0);
        assert_eq!(
            backfill_pass(&bank, Some("m1".into()), reporting("m2", &calls)).await,
            2
        );
        assert_eq!(bank.semantic(&[0.0, 1.0], "m2", 5, false).await.len(), 2);
    }

    #[test]
    fn the_current_space_is_learned_per_configured_model() {
        learn_model("pinned-x", "pinned-x-reported");
        learn_model("", "default-reported");
        assert_eq!(
            current_model("pinned-x").as_deref(),
            Some("pinned-x-reported")
        );
        assert_eq!(current_model("").as_deref(), Some("default-reported"));
        learn_model("", "default-changed");
        assert_eq!(current_model("").as_deref(), Some("default-changed"));
        assert_eq!(current_model("never-seen"), None);
    }
}
