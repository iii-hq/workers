//! laya's encoder through the shared llama.cpp runtime: the final states of
//! every token in a batch of rows (embeddings mode, no pooling, no KV cache).
//! The decision head reads them in candle (`model::LayaModel`).
use anyhow::Result;
use iii_llama_runtime::{
    llama_cpp_2::{context::params::LlamaPoolingType, llama_batch::LlamaBatch, token::LlamaToken},
    Runtime, Session,
};
use std::{
    num::NonZeroU32,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub threads: usize,
    /// None offloads every layer when a GPU device exists.
    pub gpu_layers: Option<u32>,
    /// Rows encoded together in one forward.
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

/// Final encoder states of a batch: `rows × width × hidden`, rows
/// right-padded with zeros.
pub struct States {
    pub flat: Vec<f32>,
    pub rows: usize,
    pub width: usize,
    pub hidden: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Deadline,
    Cancelled,
    Failed,
}

#[derive(Clone)]
pub struct Engine {
    runtime: Runtime,
    pub hidden: usize,
    pub batch_rows: usize,
    pub device: Arc<str>,
}

impl Engine {
    /// Load the encoder GGUF; `window` is the longest row in tokens.
    pub fn spawn(gguf: &Path, window: usize, options: Options) -> Result<Self> {
        let batch_rows = options.batch_rows.max(1);
        // Non-causal models decode a whole batch in one micro-batch.
        let tokens = (window * batch_rows) as u32;
        let runtime = Runtime::spawn(
            gguf,
            iii_llama_runtime::Options {
                threads: options.threads,
                gpu_layers: options.gpu_layers,
            },
            move |params| {
                params
                    .with_n_ctx(NonZeroU32::new(tokens))
                    .with_n_batch(tokens)
                    .with_n_ubatch(tokens)
                    .with_n_seq_max(batch_rows as u32)
                    .with_embeddings(true)
                    .with_pooling_type(LlamaPoolingType::None)
            },
        )?;
        let hidden = runtime.run(|session| session.model.n_embd() as usize)?;
        Ok(Self {
            device: runtime.device().into(),
            runtime,
            hidden,
            batch_rows,
        })
    }

    /// Encode up to `batch_rows` rows. Dropping the receiver does not stop
    /// the work; set `cancel` or let the deadline pass for that.
    pub fn states(
        &self,
        rows: Vec<Vec<u32>>,
        deadline: Instant,
        cancel: Arc<AtomicBool>,
    ) -> oneshot::Receiver<Result<States, Stop>> {
        let hidden = self.hidden;
        self.runtime
            .submit(move |session| encode(session, hidden, &rows, deadline, &cancel))
    }
}

fn encode(
    session: &mut Session,
    hidden: usize,
    rows: &[Vec<u32>],
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<States, Stop> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Stop::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(Stop::Deadline);
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let total: usize = rows.iter().map(Vec::len).sum();
    let mut batch = LlamaBatch::new(total.max(1), 1);
    for (seq, ids) in rows.iter().enumerate() {
        for (pos, &id) in ids.iter().enumerate() {
            batch
                .add(LlamaToken(id as i32), pos as i32, &[seq as i32], true)
                .map_err(|_| Stop::Failed)?;
        }
    }
    session.ctx.decode(&mut batch).map_err(|_| Stop::Failed)?;
    let mut flat = vec![0f32; rows.len() * width * hidden];
    let mut output = 0i32;
    for (seq, ids) in rows.iter().enumerate() {
        for pos in 0..ids.len() {
            let state = session
                .ctx
                .embeddings_ith(output)
                .map_err(|_| Stop::Failed)?;
            let at = (seq * width + pos) * hidden;
            flat[at..at + hidden].copy_from_slice(state);
            output += 1;
        }
    }
    Ok(States {
        flat,
        rows: rows.len(),
        width,
        hidden,
    })
}
