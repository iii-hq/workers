//! The loaded Clef model (src/llama.rs) for async callers: one forward at a
//! time in arrival order, each on the blocking pool so the executor never
//! waits on llama.cpp.
use crate::{encode::Encoded, llama::Model};
use anyhow::Result;
pub use iii_llama_runtime::Stop;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub threads: usize,
    /// None offloads the whole model when a GPU device exists; Some(0) keeps
    /// it on the CPU.
    pub gpu_layers: Option<u32>,
    /// The longest prompt, in tokens.
    pub context_tokens: u32,
}

#[derive(Clone)]
pub struct Engine {
    /// tokio's Mutex is fair: forwards run in arrival order.
    model: Arc<Mutex<Model>>,
    pub device: Arc<str>,
    pub context_tokens: u32,
}

impl Engine {
    /// Load the GGUF (blocking, seconds).
    pub fn load(gguf: &Path, options: Options) -> Result<Self> {
        let model = Model::load(gguf, options.gpu_layers, options.threads)?;
        Ok(Self {
            device: model.device.as_str().into(),
            model: Arc::new(Mutex::new(model)),
            context_tokens: options.context_tokens,
        })
    }

    /// Resolves once no pass holds the model (a detached one has ended).
    pub async fn idle(&self) {
        drop(self.model.lock().await);
    }

    /// The score of every option of every field, in prompt order. `cancel`
    /// and `deadline` are checked once the model is free, before the pass.
    /// ponytail: the pass itself cannot be stopped (no abort callback on
    /// Vulkan); a cancelled or late evaluation still holds the model until its
    /// pass ends, up to about 15 s at 16384 tokens on a GPU.
    pub async fn decide(
        &self,
        encoded: Encoded,
        deadline: Instant,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<f32>, Stop> {
        // The pass's buffers grow with the prompt: the window is the memory guard.
        if encoded.ids.len() > self.context_tokens as usize {
            return Err(Stop::TooLong);
        }
        let model = self.model.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            if cancel.load(Ordering::Relaxed) {
                return Err(Stop::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(Stop::Deadline);
            }
            match model.decide(&encoded) {
                Ok(scores) if scores.iter().all(|s| s.is_finite()) => Ok(scores),
                Ok(_) => {
                    tracing::warn!("clef forward gave a non-finite score");
                    Err(Stop::Failed)
                }
                Err(error) => {
                    tracing::warn!(%error, "clef forward failed");
                    Err(Stop::Failed)
                }
            }
        })
        .await
        .map_err(|_| Stop::Failed)?
    }
}
