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

pub async fn embed_texts(
    iii: &IIIClient,
    model: &str,
    texts: &[String],
    timeout_ms: u64,
) -> Option<Vec<Vec<f32>>> {
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
    let embeddings: Vec<Vec<f32>> =
        serde_json::from_value(reply.get("embeddings").cloned().unwrap_or(Value::Null)).ok()?;
    if embeddings.len() == texts.len() {
        Some(embeddings)
    } else {
        None
    }
}

/// Embed the query for one recall. `None` (disabled, router absent, no
/// embed provider, timeout) means lexical-only recall this turn.
pub async fn query_vector(deps: &Deps, query: &str) -> Option<Vec<f32>> {
    let cfg = deps.config().await;
    if !cfg.embeddings_enabled {
        return None;
    }
    embed_texts(
        &deps.iii,
        &cfg.embedding_model,
        &[query.to_string()],
        QUERY_EMBED_TIMEOUT_MS,
    )
    .await
    .and_then(|mut v| v.pop())
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
    let model = cfg.embedding_model.clone();
    let total = backfill_pass(&bank, &model, |texts| {
        let (iii, model) = (deps.iii.clone(), model.clone());
        async move { embed_texts(&iii, &model, &texts, BACKFILL_TIMEOUT_MS).await }
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

/// Per `bank\0id`, how often that memory's text failed to embed. A text the
/// provider rejects outright (one over a llama.cpp server's batch size, say)
/// would otherwise head every window and fail every batch it is in, so no
/// memory in the bank ever got a vector. Known failures go to the back of
/// the window instead, and a failed batch is retried one text at a time.
fn failures() -> &'static Mutex<HashMap<String, u32>> {
    static FAILURES: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    FAILURES.get_or_init(Default::default)
}

fn failure_key(bank: &str, id: &str) -> String {
    format!("{bank}\0{id}")
}

/// The next window of backfill work: unembedded memories, those that failed
/// before last (only re-sorted when this bank has any).
async fn next_window(bank: &Bank, model: &str) -> Vec<Memory> {
    let prefix = failure_key(&bank.name, "");
    let failed: HashMap<String, u32> = failures()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter_map(|(k, n)| k.strip_prefix(&prefix).map(|id| (id.to_string(), *n)))
        .collect();
    if failed.is_empty() {
        return bank.unembedded(BACKFILL_BATCH, model).await;
    }
    let mut pending = bank.unembedded(usize::MAX, model).await;
    pending.sort_by_key(|m| (failed.get(&m.id).copied().unwrap_or(0), m.id.clone()));
    pending.truncate(BACKFILL_BATCH);
    pending
}

fn note_failure(bank: &Bank, memory: &Memory) {
    *failures()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(failure_key(&bank.name, &memory.id))
        .or_default() += 1;
}

/// Store one embedding; `Err` when the sidecar write failed (stop the pass).
async fn store(bank: &Bank, model: &str, memory: &Memory, v: Vec<f32>) -> Result<bool, ()> {
    match bank.set_vector(&memory.id, model, &memory.text, v).await {
        Ok(stored) => {
            let key = failure_key(&bank.name, &memory.id);
            failures()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&key);
            Ok(stored)
        }
        Err(e) => {
            tracing::warn!(bank = %bank.name, error = %e, "vector write failed");
            Err(())
        }
    }
}

/// The backfill loop with the embedder injected (`router::embed` in
/// production). Returns how many vectors it stored.
async fn backfill_pass<F, Fut>(bank: &Bank, model: &str, mut embed: F) -> usize
where
    F: FnMut(Vec<String>) -> Fut,
    Fut: Future<Output = Option<Vec<Vec<f32>>>>,
{
    let lock = bank_lock(&bank.name);
    let _pass = lock.lock().await;
    let mut total = 0usize;
    for _ in 0..BACKFILL_ROUNDS {
        let pending = next_window(bank, model).await;
        if pending.is_empty() {
            break;
        }
        let texts: Vec<String> = pending.iter().map(|f| f.text.clone()).collect();
        if let Some(vectors) = embed(texts).await {
            for (memory, v) in pending.iter().zip(vectors) {
                let Ok(stored) = store(bank, model, memory, v).await else {
                    return total;
                };
                total += usize::from(stored);
            }
            continue;
        }
        // The batch failed: the provider is down, or it rejects one of the
        // texts. One at a time tells them apart: two failures before any
        // success reads as "down" (a later pass retries); otherwise only the
        // rejected texts miss out, and go last in later windows.
        if pending.len() == 1 {
            note_failure(bank, &pending[0]);
            break;
        }
        let (mut ok, mut failed) = (0usize, 0usize);
        for memory in &pending {
            match embed(vec![memory.text.clone()]).await {
                Some(mut v) if v.len() == 1 => {
                    let Ok(stored) = store(bank, model, memory, v.remove(0)).await else {
                        return total;
                    };
                    total += usize::from(stored);
                    ok += 1;
                }
                _ => {
                    note_failure(bank, memory);
                    failed += 1;
                    // ponytail: "down" costs 3 calls per pass (batch + 2
                    // singles); classify router errors if that ever matters.
                    if ok == 0 && failed >= 2 {
                        return total;
                    }
                }
            }
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
/// bank and no router call; with no embed provider installed, one
/// fast-failing `router::embed` per bank per minute.
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
            async move { Some(v) }
        };

        assert_eq!(backfill_pass(&bank, "m", embed).await, 1);
        let hits = bank.semantic(&[1.0, 0.0], "m", 5, false).await;
        assert_eq!(hits[0].0.id, saved.id);

        // The edit drops the old text's vector; the next pass embeds the new.
        let mut edited = saved.clone();
        edited.text = "user lives in porto".into();
        edited.revision += 1;
        bank.commit(edited).await.unwrap();
        assert_eq!(backfill_pass(&bank, "m", embed).await, 1);
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
                Some(texts.iter().map(|_| vec![1.0, 0.0]).collect())
            }
        };

        let (a, b) = tokio::join!(
            backfill_pass(&bank, "m", embed),
            backfill_pass(&bank, "m", embed)
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
                Some(texts.iter().map(axis).collect())
            }
        };

        // The stale "lisbon" vector is discarded; the next round embeds porto.
        assert_eq!(backfill_pass(&bank, "m", embed).await, 1);
        assert!(bank.semantic(&[1.0, 0.0], "m", 5, false).await.is_empty());
        assert_eq!(bank.semantic(&[0.0, 1.0], "m", 5, false).await.len(), 1);
    }

    #[tokio::test]
    async fn a_failed_embed_leaves_the_memory_for_the_next_pass() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-failed").await;
        bank.commit(memory("user owns a bike")).await.unwrap();

        assert_eq!(backfill_pass(&bank, "m", |_| async { None }).await, 0);
        assert_eq!(bank.unembedded(10, "m").await.len(), 1);
    }

    /// A provider that rejects any batch holding a "REJECTED" text.
    fn picky(
        calls: &AtomicUsize,
    ) -> impl FnMut(Vec<String>) -> std::future::Ready<Option<Vec<Vec<f32>>>> + '_ {
        move |texts: Vec<String>| {
            calls.fetch_add(1, Ordering::SeqCst);
            let ok = !texts.iter().any(|t| t.contains("REJECTED"));
            std::future::ready(ok.then(|| texts.iter().map(|_| vec![1.0, 0.0]).collect()))
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
        assert_eq!(backfill_pass(&bank, "m", picky(&calls)).await, 2);

        // A later save: the known failure goes last, the new text lands.
        bank.commit(memory("user plays chess")).await.unwrap();
        assert_eq!(backfill_pass(&bank, "m", picky(&calls)).await, 1);
        let left = bank.unembedded(10, "m").await;
        assert_eq!(left.len(), 1);
        assert!(left[0].text.starts_with("REJECTED"));
    }

    #[tokio::test]
    async fn two_rejected_texts_up_front_cost_at_most_one_pass() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-rejected-two").await;
        for t in [
            "REJECTED one",
            "REJECTED two",
            "user owns a bike",
            "user likes tea",
        ] {
            bank.commit(memory(t)).await.unwrap();
        }
        let calls = AtomicUsize::new(0);
        // Both rejected texts may come first and read as "provider down";
        // the next pass tries them last and embeds the rest.
        let first = backfill_pass(&bank, "m", picky(&calls)).await;
        let second = backfill_pass(&bank, "m", picky(&calls)).await;
        assert_eq!(first + second, 2);
        assert_eq!(bank.unembedded(10, "m").await.len(), 2);
    }

    #[tokio::test]
    async fn a_down_provider_costs_one_batch_and_two_singles() {
        let dir = tempfile::tempdir().unwrap();
        let bank = bank(&dir, "embed-down").await;
        for t in ["user owns a bike", "user likes tea", "user plays chess"] {
            bank.commit(memory(t)).await.unwrap();
        }
        let calls = AtomicUsize::new(0);
        let down = |_: Vec<String>| {
            calls.fetch_add(1, Ordering::SeqCst);
            async { None }
        };
        assert_eq!(backfill_pass(&bank, "m", down).await, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}
