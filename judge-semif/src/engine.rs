//! llama.cpp scoring on a dedicated thread. `LlamaContext` is not `Send`, so
//! one thread owns the model context and serves jobs in arrival order; that
//! also keeps a single forward on the hardware at a time.
//!
//! Each evaluation shares one state across its questions, and SemIf's prompt
//! puts the evidence first, so the engine prefills that prefix once, snapshots
//! the sequence (`state_seq_get`; Qwen3.5's hybrid memory cannot copy
//! sequences), and restores it before decoding each question's suffix.
use crate::{download::Checkpoint, prompt::LETTERS};
use anyhow::{anyhow, bail, Result};
use llama_cpp_2::{
    context::{params::LlamaContextParams, session::LlamaStateSeqFlags, LlamaContext},
    llama_backend::LlamaBackend,
    llama_batch::LlamaBatch,
    model::{params::LlamaModelParams, AddBos, LlamaModel},
    token::LlamaToken,
};
use std::{
    num::NonZeroU32,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, OnceLock,
    },
    time::Instant,
};
use tokio::sync::oneshot;

/// Tokens per `llama_decode`; deadlines and cancellation are observed between chunks.
const CHUNK: usize = 512;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub threads: usize,
    /// None offloads every layer when a GPU device exists.
    pub gpu_layers: Option<u32>,
    pub context_tokens: u32,
}

/// One evaluation: its shared-state prefix and one prompt per question.
pub struct Evaluation {
    pub prefix: String,
    pub prompts: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    /// Softmax over the option letters, in option order.
    pub probabilities: Vec<f64>,
}

#[derive(Debug)]
pub struct Outcome {
    pub scores: Vec<Vec<Scored>>,
    /// Tokens decoded per evaluation (shared prefix counted once).
    pub tokens: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Deadline,
    Cancelled,
    /// A prompt does not fit the context window.
    TooLong,
    Failed,
}

struct Job {
    evaluations: Vec<Evaluation>,
    deadline: Instant,
    cancel: Arc<AtomicBool>,
    reply: oneshot::Sender<Result<Outcome, Stop>>,
}

#[derive(Clone)]
pub struct Engine {
    jobs: mpsc::Sender<Job>,
    pub device: Arc<str>,
    pub context_tokens: u32,
}

/// llama.cpp's backend is process-global and initializes once.
fn backend() -> Result<&'static LlamaBackend> {
    static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| LlamaBackend::init().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| anyhow!("llama.cpp init: {e}"))
}

impl Engine {
    /// Load the GGUF on a new thread and return once it answers (or failed).
    pub fn spawn(checkpoint: &Checkpoint, options: Options) -> Result<Self> {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (ready_tx, ready) = mpsc::channel();
        let gguf = checkpoint.gguf.clone();
        std::thread::Builder::new()
            .name("semif-engine".into())
            .spawn(move || {
                let loaded = (|| -> Result<_> {
                    let backend = backend()?;
                    let gpu = llama_cpp_2::list_llama_ggml_backend_devices()
                        .into_iter()
                        .find(|d| !d.backend.eq_ignore_ascii_case("cpu"));
                    let layers =
                        options
                            .gpu_layers
                            .unwrap_or(if gpu.is_some() { u32::MAX } else { 0 });
                    let device = match (&gpu, layers) {
                        (Some(d), n) if n > 0 => format!("{} ({})", d.description, d.backend),
                        _ => "CPU".into(),
                    };
                    // The context borrows the model for the thread's lifetime.
                    let model: &'static LlamaModel = Box::leak(Box::new(
                        LlamaModel::load_from_file(
                            backend,
                            &gguf,
                            &LlamaModelParams::default().with_n_gpu_layers(layers),
                        )
                        .map_err(|e| anyhow!("load {}: {e}", gguf.display()))?,
                    ));
                    let threads = i32::try_from(options.threads).unwrap_or(8);
                    let ctx = model
                        .new_context(
                            backend,
                            LlamaContextParams::default()
                                .with_n_ctx(NonZeroU32::new(options.context_tokens))
                                .with_n_batch(CHUNK as u32)
                                .with_n_ubatch(CHUNK as u32)
                                .with_n_seq_max(1)
                                .with_n_threads(threads)
                                .with_n_threads_batch(threads),
                        )
                        .map_err(|e| anyhow!("context: {e}"))?;
                    let letters = letter_tokens(model)?;
                    Ok((
                        Scorer {
                            model,
                            ctx,
                            letters,
                        },
                        device,
                    ))
                })();
                let mut scorer = match loaded {
                    Ok((scorer, device)) => {
                        let _ = ready_tx.send(Ok(device));
                        scorer
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                for job in inbox {
                    let result = scorer.run(&job);
                    let _ = job.reply.send(result);
                }
            })?;
        let device = ready
            .recv()
            .map_err(|_| anyhow!("SemIf engine thread exited during load"))??;
        Ok(Self {
            jobs,
            device: device.into(),
            context_tokens: options.context_tokens,
        })
    }

    /// Queue a job. Dropping the returned receiver does not stop the work;
    /// set `cancel` or let the deadline pass for that.
    pub fn submit(
        &self,
        evaluations: Vec<Evaluation>,
        deadline: Instant,
        cancel: Arc<AtomicBool>,
    ) -> oneshot::Receiver<Result<Outcome, Stop>> {
        let (reply, receiver) = oneshot::channel();
        // A send error means the thread died: the dropped reply reports it.
        let _ = self.jobs.send(Job {
            evaluations,
            deadline,
            cancel,
            reply,
        });
        receiver
    }
}

/// Each option letter must be exactly one token of the model's vocabulary.
fn letter_tokens(model: &LlamaModel) -> Result<Vec<LlamaToken>> {
    LETTERS
        .chars()
        .map(|letter| {
            let tokens = model
                .str_to_token(&letter.to_string(), AddBos::Never)
                .map_err(|e| anyhow!("tokenize {letter}: {e}"))?;
            match tokens.as_slice() {
                [token] => Ok(*token),
                _ => bail!("option letter {letter} is not a single token in this vocabulary"),
            }
        })
        .collect()
}

struct Scorer {
    model: &'static LlamaModel,
    ctx: LlamaContext<'static>,
    letters: Vec<LlamaToken>,
}

impl Scorer {
    fn run(&mut self, job: &Job) -> Result<Outcome, Stop> {
        let check = || {
            if job.cancel.load(Ordering::Relaxed) {
                Err(Stop::Cancelled)
            } else if Instant::now() >= job.deadline {
                Err(Stop::Deadline)
            } else {
                Ok(())
            }
        };
        let n_ctx = self.ctx.n_ctx() as usize;
        let mut outcome = Outcome {
            scores: Vec::with_capacity(job.evaluations.len()),
            tokens: Vec::with_capacity(job.evaluations.len()),
        };
        for evaluation in &job.evaluations {
            check()?;
            let tokenize = |text: &str| {
                self.model
                    .str_to_token(text, AddBos::Never)
                    .map_err(|_| Stop::Failed)
            };
            let prompts = evaluation
                .prompts
                .iter()
                .map(|(text, options)| Ok((tokenize(text)?, *options)))
                .collect::<Result<Vec<_>, Stop>>()?;
            if prompts.iter().any(|(ids, _)| ids.len() > n_ctx) {
                return Err(Stop::TooLong);
            }
            let mut prefix = tokenize(&evaluation.prefix)?;
            prefix.pop();
            // Reuse pays off from the second question on, and only when every
            // prompt really starts with the prefix tokens.
            let shared = prompts.len() > 1
                && !prefix.is_empty()
                && prompts
                    .iter()
                    .all(|(ids, _)| ids.len() > prefix.len() && ids.starts_with(&prefix));
            let mut tokens = 0u64;
            let mut scores = Vec::with_capacity(prompts.len());
            let saved = if shared {
                self.ctx.clear_kv_cache();
                self.decode(&prefix, 0, false, &check)?;
                tokens += prefix.len() as u64;
                Some(
                    self.ctx
                        .state_seq_get(0, LlamaStateSeqFlags::default())
                        .map_err(|_| Stop::Failed)?,
                )
            } else {
                None
            };
            for (ids, options) in &prompts {
                check()?;
                let logits = match &saved {
                    Some(state) => {
                        self.ctx
                            .clear_kv_cache_seq(Some(0), None, None)
                            .map_err(|_| Stop::Failed)?;
                        self.ctx.state_seq_set(state, 0).map_err(|_| Stop::Failed)?;
                        tokens += (ids.len() - prefix.len()) as u64;
                        self.decode(&ids[prefix.len()..], prefix.len(), true, &check)?
                    }
                    None => {
                        self.ctx.clear_kv_cache();
                        tokens += ids.len() as u64;
                        self.decode(ids, 0, true, &check)?
                    }
                };
                let z: Vec<f32> = self.letters[..*options]
                    .iter()
                    .map(|t| logits[t.0 as usize])
                    .collect();
                scores.push(Scored {
                    probabilities: softmax(&z).ok_or(Stop::Failed)?,
                });
            }
            outcome.scores.push(scores);
            outcome.tokens.push(tokens);
        }
        Ok(outcome)
    }

    /// Decode `tokens` at positions `start..`; with `want`, return the logits
    /// of the last position.
    fn decode(
        &mut self,
        tokens: &[LlamaToken],
        start: usize,
        want: bool,
        check: &dyn Fn() -> Result<(), Stop>,
    ) -> Result<Vec<f32>, Stop> {
        let mut last = 0;
        for (index, chunk) in tokens.chunks(CHUNK).enumerate() {
            check()?;
            let final_chunk = (index + 1) * CHUNK >= tokens.len();
            let mut batch = LlamaBatch::new(chunk.len(), 1);
            for (offset, token) in chunk.iter().enumerate() {
                let position = (start + index * CHUNK + offset) as i32;
                let logits = want && final_chunk && offset + 1 == chunk.len();
                batch
                    .add(*token, position, &[0], logits)
                    .map_err(|_| Stop::Failed)?;
            }
            self.ctx.decode(&mut batch).map_err(|_| Stop::Failed)?;
            last = batch.n_tokens() - 1;
        }
        if !want {
            return Ok(Vec::new());
        }
        let vocab = self.model.n_vocab() as usize;
        Ok(self.ctx.get_logits_ith(last)[..vocab].to_vec())
    }
}

pub fn softmax(z: &[f32]) -> Option<Vec<f64>> {
    if z.len() < 2 || z.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let max = z
        .iter()
        .fold(f64::NEG_INFINITY, |m, &v| m.max(f64::from(v)));
    let exp: Vec<f64> = z.iter().map(|&v| (f64::from(v) - max).exp()).collect();
    let sum: f64 = exp.iter().sum();
    Some(exp.iter().map(|v| v / sum).collect())
}
