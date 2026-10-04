//! llama.cpp runtime shared by the in-process judge providers: device
//! selection, model loading, and one thread that owns the llama.cpp `Context`
//! (not `Send`) and serves jobs in arrival order, so a single forward runs on
//! the hardware at a time. llama.cpp itself is `iii_llama_native`. What a job
//! does with the model is the provider's business; `scorer` is the job of the
//! providers that read option labels (judge-decider, judge-semif), and
//! `lifecycle` loads a provider's model on first use and releases it when idle
//! (every local provider; judge-laya and judge-clef drive `iii_llama_native`
//! on their own thread).
pub use iii_llama_native;
pub mod lifecycle;
pub mod scorer;

pub use lifecycle::{ModelSlot, Unloaded, IDLE_RELEASE};

use anyhow::{anyhow, Result};
use iii_llama_native::{Context, ContextParams, Model};
use std::{
    path::Path,
    sync::{mpsc, Arc},
};
use tokio::sync::oneshot;

/// Why a forward stopped short of its scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Deadline,
    Cancelled,
    /// A prompt does not fit the context window.
    TooLong,
    Failed,
}

/// Softmax over the option logits at `temperature`; `None` for no options or
/// a non-finite logit. One option is certain (`[1.0]`).
pub fn softmax(z: &[f32], temperature: f64) -> Option<Vec<f64>> {
    if z.is_empty() || z.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let z: Vec<f64> = z.iter().map(|&v| f64::from(v) / temperature).collect();
    let max = z.iter().fold(f64::NEG_INFINITY, |m, &v| m.max(v));
    let exp: Vec<f64> = z.iter().map(|&v| (v - max).exp()).collect();
    let sum: f64 = exp.iter().sum();
    Some(exp.iter().map(|v| v / sum).collect())
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub threads: usize,
    /// None offloads every layer when a GPU device exists.
    pub gpu_layers: Option<u32>,
}

/// The loaded model and its context, owned by the runtime thread; both are
/// freed when the last `Runtime` handle is dropped and the thread ends.
pub struct Session<'a> {
    pub model: &'a Model,
    pub ctx: Context<'a>,
}

type Job = Box<dyn for<'a> FnOnce(&mut Session<'a>) + Send>;

/// Handle to the runtime thread; clones share it.
#[derive(Clone)]
pub struct Runtime {
    jobs: mpsc::Sender<Job>,
    device: Arc<str>,
}

impl Runtime {
    /// Load `gguf` on a new thread and return once it answers (or failed).
    /// `configure` sets the context parameters the provider needs (window,
    /// batch sizes, sequences, pooling); the thread count comes from `options`.
    pub fn spawn(
        gguf: &Path,
        options: Options,
        configure: impl FnOnce(ContextParams) -> ContextParams + Send + 'static,
    ) -> Result<Self> {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (ready_tx, ready) = mpsc::channel();
        let gguf = gguf.to_path_buf();
        std::thread::Builder::new()
            .name("llama-runtime".into())
            .spawn(move || {
                // gpu_layers 0 keeps the GPU devices visible, so llama.cpp
                // still hands them large batch matmuls.
                let model = match Model::load(&gguf, options.gpu_layers, options.threads, true) {
                    Ok(model) => model,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let params = configure(ContextParams {
                    threads: i32::try_from(options.threads).unwrap_or(8),
                    ..ContextParams::default()
                });
                // The context borrows the model; both live until the jobs end.
                let ctx = match model.new_context(params) {
                    Ok(ctx) => ctx,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(model.device.clone()));
                let mut session = Session { model: &model, ctx };
                for job in inbox {
                    job(&mut session);
                }
            })?;
        let device: String = ready
            .recv()
            .map_err(|_| anyhow!("llama.cpp runtime thread exited during load"))??;
        tracing::info!(device, "selected inference device");
        Ok(Self {
            jobs,
            device: device.into(),
        })
    }

    /// The device running the model ("CPU" or the GPU's description).
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Queue `job`; the receiver yields its result. Dropping the receiver does
    /// not stop the work: jobs observe their own deadline or cancel flag.
    pub fn submit<R: Send + 'static>(
        &self,
        job: impl for<'a> FnOnce(&mut Session<'a>) -> R + Send + 'static,
    ) -> oneshot::Receiver<R> {
        let (reply, receiver) = oneshot::channel();
        // A send error means the thread died: the dropped reply reports it.
        let _ = self.jobs.send(Box::new(move |session: &mut Session<'_>| {
            let _ = reply.send(job(session));
        }));
        receiver
    }

    /// Run `job` and wait for it (load-time setup; blocks the calling thread).
    pub fn run<R: Send + 'static>(
        &self,
        job: impl for<'a> FnOnce(&mut Session<'a>) -> R + Send + 'static,
    ) -> Result<R> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.jobs
            .send(Box::new(move |session: &mut Session<'_>| {
                let _ = reply.send(job(session));
            }))
            .map_err(|_| anyhow!("llama.cpp runtime thread exited"))?;
        receiver
            .recv()
            .map_err(|_| anyhow!("llama.cpp runtime thread exited"))
    }
}
