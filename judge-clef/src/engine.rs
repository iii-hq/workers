//! Clef's backbone on the shared llama.cpp runtime: the final-norm hidden
//! state of every prompt token (embeddings mode, no pooling). STUB: the
//! interface is fixed; the body is implemented in its own change.
use anyhow::Result;
pub use iii_llama_runtime::scorer::Stop;
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};
use tokio::sync::oneshot;

/// Tokens per forward (llama.cpp's n_batch = n_ubatch).
pub const BATCH_TOKENS: u32 = 512;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub threads: usize,
    /// None offloads every layer when a GPU device exists.
    pub gpu_layers: Option<u32>,
    /// The longest prompt, in tokens.
    pub context_tokens: u32,
}

#[derive(Clone)]
pub struct Engine {
    /// The backbone's hidden size (`n_embd_out`).
    pub hidden: usize,
    pub device: Arc<str>,
    pub context_tokens: u32,
}

impl Engine {
    /// Load the GGUF on the runtime thread; `batch_tokens` is one forward.
    pub fn spawn(gguf: &Path, options: Options, batch_tokens: u32) -> Result<Self> {
        let _ = (gguf, options, batch_tokens);
        todo!("engine::spawn")
    }

    /// Hidden states `[ids.len() × hidden]`, row-major, memory cleared first.
    /// Dropping the receiver does not stop the work; set `cancel` or let the
    /// deadline pass.
    pub fn states(
        &self,
        ids: Vec<u32>,
        deadline: Instant,
        cancel: Arc<AtomicBool>,
    ) -> oneshot::Receiver<Result<Vec<f32>, Stop>> {
        let _ = (ids, deadline, cancel);
        todo!("engine::states")
    }
}
