//! Clef's backbone on the shared llama.cpp runtime: the final-norm hidden
//! state of every prompt token (embeddings mode, no pooling), read by the
//! joint head in candle.
//!
//! ponytail: llama.cpp 0.1.156's qwen35 graph still computes the vocab logits
//! of every token (an unused LM head: 2 GFLOP per token on Clef-Flash, 485 MiB
//! of the 0.5-0.6 GiB compute buffer at batch 512), and its host output buffer
//! holds `n_vocab + hidden` floats per token of batch (0.96 MiB each, about
//! 0.5 GiB at 512). Dropping both needs a llama.cpp with the `clef` arch
//! (PR #29831), which no published llama-cpp-2 has yet.
use anyhow::Result;
pub use iii_llama_runtime::scorer::Stop;
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
    runtime: Runtime,
    /// The backbone's hidden size (`n_embd_out`).
    pub hidden: usize,
    pub device: Arc<str>,
    pub context_tokens: u32,
}

impl Engine {
    /// Load the GGUF on the runtime thread; `batch_tokens` is one forward.
    pub fn spawn(gguf: &Path, options: Options, batch_tokens: u32) -> Result<Self> {
        let Options {
            threads,
            gpu_layers,
            context_tokens,
        } = options;
        let runtime = Runtime::spawn(
            gguf,
            iii_llama_runtime::Options {
                threads,
                gpu_layers,
            },
            move |params| {
                params
                    .with_n_ctx(NonZeroU32::new(context_tokens))
                    .with_n_batch(batch_tokens)
                    .with_n_ubatch(batch_tokens)
                    .with_embeddings(true)
                    .with_pooling_type(LlamaPoolingType::None)
                    // LLAMA_FLASH_ATTN_TYPE_DISABLED. Flash attention's CPU
                    // kernel changes below 64 queries, so a prompt's last chunk
                    // drifted (min cos 0.995); without it chunking is
                    // bit-exact, Vulkan prefill is 13% faster at 16k tokens and
                    // the reference |Δp| is unchanged (CPU) or lower (Vulkan).
                    // The price is an n_ctx x n_ubatch x heads f32 KQ: +79 MiB
                    // VRAM at 16k tokens (it shares the vocab logits' compute
                    // buffer), +1.7 GiB at 65536.
                    .with_flash_attention_policy(0)
            },
        )?;
        // The width embeddings_ith returns (n_embd unless the GGUF sets an
        // output width).
        let hidden = runtime.run(|session| session.model.n_embd_out() as usize)?;
        Ok(Self {
            device: runtime.device().into(),
            runtime,
            hidden,
            context_tokens,
        })
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
        let hidden = self.hidden;
        self.runtime
            .submit(move |session| prefill(session, hidden, &ids, deadline, &cancel))
    }
}

/// Decode `ids` in chunks of n_batch at consecutive positions of sequence 0.
/// The memory (attention KV plus Qwen3.5's recurrent state) carries between
/// chunks and is cleared first, so requests never see each other.
fn prefill(
    session: &mut Session<'_>,
    hidden: usize,
    ids: &[u32],
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<Vec<f32>, Stop> {
    let ctx = &mut session.ctx;
    if ids.len() > ctx.n_ctx() as usize {
        return Err(Stop::TooLong);
    }
    // A decode larger than n_batch aborts the process.
    let chunk = ctx.n_batch() as usize;
    ctx.clear_kv_cache();
    let mut batch = LlamaBatch::new(chunk, 1);
    let mut states = Vec::with_capacity(ids.len() * hidden);
    for (start, part) in (0..).step_by(chunk).zip(ids.chunks(chunk)) {
        if cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(Stop::Deadline);
        }
        batch.clear();
        for (pos, &id) in (start..).zip(part) {
            batch
                .add(LlamaToken(id as i32), pos, &[0], true)
                .map_err(|_| Stop::Failed)?;
        }
        ctx.decode(&mut batch).map_err(|_| Stop::Failed)?;
        for i in 0..part.len() as i32 {
            states.extend_from_slice(ctx.embeddings_ith(i).map_err(|_| Stop::Failed)?);
        }
    }
    Ok(states)
}
