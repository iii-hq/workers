//! llama.cpp scoring on a dedicated thread. `LlamaContext` is not `Send`, so
//! one thread owns the model context and serves jobs in arrival order; that
//! also keeps a single forward on the hardware at a time.
//!
//! Each evaluation shares one state across its questions, and SemIf's prompt
//! puts the evidence first, so the engine prefills that prefix once, snapshots
//! the sequence (`state_seq_get`; Qwen3.5's hybrid memory cannot copy
//! sequences), restores it into up to `parallel` sequences and decodes their
//! question suffixes together in one batch (SemIf's parallel suffixes).
use crate::{
    download::Checkpoint,
    prompt::{self, LETTERS},
};
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

/// Tokens per `llama_decode` (and llama.cpp's physical batch): large batches
/// keep a GPU busy on prompt processing. Deadlines and cancellation are
/// observed between chunks.
const CHUNK: usize = 2048;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub threads: usize,
    /// None offloads every layer when a GPU device exists.
    pub gpu_layers: Option<u32>,
    pub context_tokens: u32,
    /// Questions decoded together in one batch (1 = one at a time).
    pub parallel: usize,
}

/// One evaluation: questions (criterion, option descriptions) about one state.
pub struct Evaluation {
    pub state: serde_json::Value,
    pub questions: Vec<(String, Vec<String>)>,
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
        .get_or_init(|| {
            load_backend_modules();
            LlamaBackend::init().map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| anyhow!("llama.cpp init: {e}"))
}

/// Linux x86_64 and Windows ship llama.cpp's backends as modules (Cargo.toml):
/// load them from the executable's directory (the published layout), else from
/// the build's own output (tests). A module whose system library is missing,
/// such as the Vulkan loader, is skipped, and the CPU module runs the model.
#[cfg(any(
    all(target_os = "linux", target_arch = "x86_64"),
    target_os = "windows"
))]
fn load_backend_modules() {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf))
    {
        llama_cpp_2::llama_backend::load_backends_from_path(&dir);
    }
    if llama_cpp_2::list_llama_ggml_backend_devices().is_empty() {
        llama_cpp_2::llama_backend::load_backends();
    }
}

/// Elsewhere the backends are linked in (Metal on macOS, CPU otherwise).
#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    target_os = "windows"
)))]
fn load_backend_modules() {}

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
                                .with_n_seq_max(options.parallel.max(1) as u32)
                                // One KV pool shared by the parallel sequences,
                                // so a long prompt can still use the whole window.
                                .with_kv_unified(true)
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
                            parallel: options.parallel.max(1),
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
    parallel: usize,
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
            // Render and tokenize one question at a time, keeping the first
            // prompt whole and, of the others, only what follows their common
            // run with it: memory grows with the questions, not with copies of
            // the state. Question `i` is `first[..lcp_i]` then `tails[i]`.
            let mut first = Vec::new();
            let mut common = 0;
            let (mut shortest, mut longest) = (usize::MAX, 1);
            let mut tails: Vec<(usize, Vec<LlamaToken>, usize)> =
                Vec::with_capacity(evaluation.questions.len());
            for (criterion, options) in &evaluation.questions {
                check()?;
                let ids = tokenize(&prompt::render(&evaluation.state, criterion, options))?;
                if ids.len() > n_ctx {
                    return Err(Stop::TooLong);
                }
                (shortest, longest) = (shortest.min(ids.len()), longest.max(ids.len()));
                // The shared prefix is the longest common run of prompt tokens,
                // not a re-tokenized text prefix: the state's closing bytes can
                // merge with what follows it (`{}}` vs `{}`) more than one token back.
                if tails.is_empty() {
                    common = ids.len();
                    tails.push((common, Vec::new(), options.len()));
                    first = ids;
                } else {
                    // At most `common`, so `common` stays the running minimum.
                    common = first[..common]
                        .iter()
                        .zip(&ids)
                        .take_while(|(a, b)| a == b)
                        .count();
                    tails.push((common, ids[common..].to_vec(), options.len()));
                }
            }
            // Every question keeps at least its last token to decode.
            let common = common.min(shortest.saturating_sub(1));
            let prefix = &first[..common];
            let shared = tails.len() > 1 && !prefix.is_empty();
            let mut tokens = 0u64;
            let mut scores = Vec::with_capacity(tails.len());
            let saved = if shared {
                self.ctx.clear_kv_cache();
                self.decode(&[(prefix, 0, None)], &check)?;
                tokens += prefix.len() as u64;
                Some(
                    self.ctx
                        .state_seq_get(0, LlamaStateSeqFlags::default())
                        .map_err(|_| Stop::Failed)?,
                )
            } else {
                None
            };
            let skip = if saved.is_some() { prefix.len() } else { 0 };
            // The unified KV pool holds every branch of a group: the prefix
            // once per sequence plus the suffixes.
            let group = self.parallel.min((n_ctx / longest).max(1));
            for chunk in tails.chunks(group) {
                check()?;
                self.ctx.clear_kv_cache();
                if let Some(state) = &saved {
                    for seq in 0..chunk.len() {
                        self.ctx
                            .state_seq_set(state, seq as i32)
                            .map_err(|_| Stop::Failed)?;
                    }
                }
                let suffixes: Vec<(Vec<LlamaToken>, usize)> = chunk
                    .iter()
                    .map(|(lcp, tail, options)| {
                        let ids = first[skip..*lcp].iter().chain(tail).copied().collect();
                        (ids, *options)
                    })
                    .collect();
                let spans: Vec<(&[LlamaToken], usize, Option<usize>)> = suffixes
                    .iter()
                    .map(|(ids, options)| (&ids[..], skip, Some(*options)))
                    .collect();
                tokens += spans.iter().map(|s| s.0.len() as u64).sum::<u64>();
                for z in self.decode(&spans, &check)? {
                    scores.push(Scored {
                        probabilities: softmax(&z).ok_or(Stop::Failed)?,
                    });
                }
            }
            outcome.scores.push(scores);
            outcome.tokens.push(tokens);
        }
        Ok(outcome)
    }

    /// Decode each span `(tokens, start, options)` into sequence `i` at
    /// positions `start..`, packed into batches of `CHUNK` tokens. For spans
    /// with `Some(options)`, return the option-letter logits of their last
    /// position, in span order.
    #[allow(clippy::type_complexity)]
    fn decode(
        &mut self,
        spans: &[(&[LlamaToken], usize, Option<usize>)],
        check: &dyn Fn() -> Result<(), Stop>,
    ) -> Result<Vec<Vec<f32>>, Stop> {
        let flat: Vec<(usize, usize)> = spans
            .iter()
            .enumerate()
            .flat_map(|(seq, (tokens, _, _))| (0..tokens.len()).map(move |i| (seq, i)))
            .collect();
        let mut out: Vec<Option<Vec<f32>>> = vec![None; spans.len()];
        for chunk in flat.chunks(CHUNK) {
            check()?;
            let mut batch = LlamaBatch::new(chunk.len(), 1);
            let mut wanted = Vec::new();
            for (row, &(seq, i)) in chunk.iter().enumerate() {
                let (tokens, start, options) = spans[seq];
                let last = options.is_some() && i + 1 == tokens.len();
                batch
                    .add(tokens[i], (start + i) as i32, &[seq as i32], last)
                    .map_err(|_| Stop::Failed)?;
                if last {
                    wanted.push((seq, row as i32));
                }
            }
            self.ctx.decode(&mut batch).map_err(|_| Stop::Failed)?;
            for (seq, row) in wanted {
                let logits = self.ctx.get_logits_ith(row);
                let options = spans[seq].2.unwrap_or(0);
                out[seq] = Some(
                    self.letters[..options]
                        .iter()
                        .map(|t| logits[t.0 as usize])
                        .collect(),
                );
            }
        }
        Ok(out.into_iter().flatten().collect())
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
