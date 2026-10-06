//! laya's checkpoint in llama.cpp (crates/llama-native): the encoder and the
//! decision head in one graph, which scores every token for each question
//! type (choice, score, noul); a row's option scores are its question type's
//! column at its `[MASK]` markers. It runs on llama-runtime's `Runtime`: one
//! thread owns the model and a context sized for a full pass (embeddings, no
//! pooling, no KV cache) and runs the passes in arrival order.
use anyhow::{ensure, Result};
use iii_llama_native::{Batch, Context, ContextParams, Pooling};
use iii_llama_runtime::Runtime;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::sync::oneshot;

/// Tokens one pass holds at most: rows per pass are capped to fill it at the
/// checkpoint's window (16 rows of 512 tokens, 8 of 1024).
const PASS_TOKENS: usize = 8192;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub threads: usize,
    /// None offloads every layer when a GPU device exists.
    pub gpu_layers: Option<u32>,
    /// Rows scored together in one forward (capped by `PASS_TOKENS`).
    pub batch_rows: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            threads: std::thread::available_parallelism().map_or(8, |n| n.get().min(8)),
            gpu_layers: None,
            batch_rows: 16,
        }
    }
}

/// One row: its token ids, the positions of its `[MASK]` markers and its
/// question type (`encode::QType`, the score column).
pub type Row = (Vec<u32>, Vec<usize>, u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Deadline,
    Cancelled,
    Failed,
}

/// Handle to the engine thread; clones share it, and the thread frees the
/// model once the last one is dropped and its queued passes have run.
#[derive(Clone)]
pub struct Engine {
    runtime: Runtime,
    pub batch_rows: usize,
    pub device: Arc<str>,
}

impl Engine {
    /// Load the GGUF (blocking, seconds); `window` is the longest row in tokens.
    pub fn spawn(gguf: &Path, window: usize, options: Options) -> Result<Self> {
        let batch_rows = options.batch_rows.min(PASS_TOKENS / window.max(1)).max(1);
        // A non-causal pass is one micro-batch: n_ubatch holds every token.
        let tokens = u32::try_from(window * batch_rows)?;
        let runtime = Runtime::spawn(
            gguf,
            iii_llama_runtime::Options {
                threads: options.threads,
                gpu_layers: options.gpu_layers,
            },
            move |params| ContextParams {
                n_ctx: tokens,
                n_batch: tokens,
                n_ubatch: tokens,
                n_seq_max: batch_rows as u32,
                embeddings: true,
                // The scores are read per token, whatever a GGUF says.
                pooling: Pooling::None,
                ..params
            },
        )?;
        // An encoder-only GGUF loads too, and would answer with hidden states.
        let head = runtime.run(|session| session.model.meta("modern-bert.decision.type"))?;
        ensure!(
            head.as_deref() == Some("laya"),
            "{}: decision head {head:?}, not laya",
            gguf.display()
        );
        Ok(Self {
            device: runtime.device().into(),
            runtime,
            batch_rows,
        })
    }

    /// The option scores of up to `batch_rows` rows. Dropping the receiver
    /// does not stop the work; set `cancel` or let the deadline pass for that.
    pub fn scores(
        &self,
        rows: Vec<Row>,
        deadline: Instant,
        cancel: Arc<AtomicBool>,
    ) -> oneshot::Receiver<Result<Vec<Vec<f32>>, Stop>> {
        self.runtime
            .submit(move |session| scores(&mut session.ctx, &rows, deadline, &cancel))
    }
}

/// One pass over `rows`, one sequence each. An embeddings pass outputs every
/// token anyway (llama.cpp overrides partial output flags, with a warning),
/// so all are flagged; the head's columns are read at the markers.
fn scores(
    ctx: &mut Context<'_>,
    rows: &[Row],
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<Vec<Vec<f32>>, Stop> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Stop::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(Stop::Deadline);
    }
    let mut batch = Batch::default();
    let mut starts = Vec::with_capacity(rows.len());
    let mut start = 0;
    for (seq, (ids, _, _)) in rows.iter().enumerate() {
        for (pos, &id) in ids.iter().enumerate() {
            batch.add(id as i32, pos as i32, seq as i32, true);
        }
        starts.push(start);
        start += ids.len();
    }
    if let Err(error) = ctx.encode(&batch) {
        tracing::warn!(%error, "laya forward failed");
        return Err(Stop::Failed);
    }
    rows.iter()
        .zip(starts)
        .map(|((_, markers, qtype), start)| {
            markers
                .iter()
                .map(|&marker| {
                    ctx.embeddings_ith((start + marker) as i32)
                        .and_then(|scores| scores.get(*qtype as usize).copied())
                        .ok_or(Stop::Failed)
                })
                .collect()
        })
        .collect()
}
