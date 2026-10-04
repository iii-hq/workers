//! The loaded Clef model (crates/llama-native) for async callers: one forward
//! at a time in arrival order, each on its own thread so the executor never
//! waits on llama.cpp.
use crate::encode::Encoded;
use anyhow::{ensure, Result};
use iii_llama_native::Model;
pub use iii_llama_runtime::Stop;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::sync::{oneshot, Mutex};

/// Run `work` on a thread of its own. Not tokio's blocking pool: handlers run
/// on the SDK's connection runtime, and the SDK's shutdown joins that thread,
/// whose runtime waits for every blocking task (a whole pass) before exiting.
/// Errs when `work` panicked.
pub(crate) fn detached<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> oneshot::Receiver<T> {
    let (done, result) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = done.send(work());
    });
    result
}

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
        // A context per call starts ggml-vulkan's submit sizing from zero
        // flops, so each pass submits its graph in 100-node pieces; at 16k
        // tokens one nears amdgpu's 2 s job timeout, which loses the device
        // and aborts the process. 10-node pieces cost at most 0.5% (RX 6900
        // XT). An operator's value wins.
        iii_llama_native::init_backends(Some(10));
        let model = Model::load(gguf, options.gpu_layers, options.threads, false)?;
        // A qwen35 GGUF of the backbone loads too, without the decision head.
        let arch = model.architecture();
        ensure!(
            arch.as_deref() == Some("clef"),
            "{}: architecture {arch:?}, not clef",
            gguf.display()
        );
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
        detached(move || {
            if cancel.load(Ordering::Relaxed) {
                return Err(Stop::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(Stop::Deadline);
            }
            match decide(&model, &encoded) {
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

/// One forward over the prompt: the score of every option of every field, in
/// prompt order (softmax per field).
fn decide(model: &Model, encoded: &Encoded) -> Result<Vec<f32>> {
    // llama_decision_order: question noul 1, choice 2, score 3; option 4.
    let mut orders = vec![0u8; encoded.ids.len()];
    for field in &encoded.fields {
        orders[field.span.clone()].fill(field.kind as u8 + 1);
        for option in &field.options {
            orders[option.clone()].fill(4);
        }
    }
    let ids: Vec<i32> = encoded.ids.iter().map(|&id| id as i32).collect();
    let n_scores = encoded.fields.iter().map(|f| f.options.len()).sum();
    model.decide(&ids, &orders, n_scores)
}
