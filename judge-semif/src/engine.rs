//! llama.cpp scoring through the shared runtime: one thread owns the model
//! context, so a single forward runs on the hardware at a time.
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
use iii_llama_runtime::{
    llama_cpp_2::{
        context::{session::LlamaStateSeqFlags, LlamaContext},
        llama_batch::LlamaBatch,
        model::{AddBos, LlamaModel},
        token::LlamaToken,
    },
    Runtime, Session,
};
use std::{
    num::NonZeroU32,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
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

#[derive(Clone)]
pub struct Engine {
    runtime: Runtime,
    letters: Arc<Vec<LlamaToken>>,
    parallel: usize,
    pub device: Arc<str>,
    pub context_tokens: u32,
}

impl Engine {
    /// Load the GGUF on the runtime thread and return once it answers (or failed).
    pub fn spawn(checkpoint: &Checkpoint, options: Options) -> Result<Self> {
        let parallel = options.parallel.max(1);
        let runtime = Runtime::spawn(
            &checkpoint.gguf,
            iii_llama_runtime::Options {
                threads: options.threads,
                gpu_layers: options.gpu_layers,
            },
            move |params| {
                params
                    .with_n_ctx(NonZeroU32::new(options.context_tokens))
                    .with_n_batch(CHUNK as u32)
                    .with_n_ubatch(CHUNK as u32)
                    .with_n_seq_max(parallel as u32)
                    // One KV pool shared by the parallel sequences, so a long
                    // prompt can still use the whole window.
                    .with_kv_unified(true)
            },
        )?;
        let letters = runtime.run(|session| letter_tokens(session.model))??;
        Ok(Self {
            device: runtime.device().into(),
            runtime,
            letters: Arc::new(letters),
            parallel,
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
        let (letters, parallel) = (self.letters.clone(), self.parallel);
        self.runtime.submit(move |session| {
            score(session, &letters, parallel, &evaluations, deadline, &cancel)
        })
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

fn score(
    session: &mut Session,
    letters: &[LlamaToken],
    parallel: usize,
    evaluations: &[Evaluation],
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<Outcome, Stop> {
    let check = || {
        if cancel.load(Ordering::Relaxed) {
            Err(Stop::Cancelled)
        } else if Instant::now() >= deadline {
            Err(Stop::Deadline)
        } else {
            Ok(())
        }
    };
    let Session { model, ctx } = session;
    let n_ctx = ctx.n_ctx() as usize;
    let mut outcome = Outcome {
        scores: Vec::with_capacity(evaluations.len()),
        tokens: Vec::with_capacity(evaluations.len()),
    };
    for evaluation in evaluations {
        check()?;
        let tokenize = |text: &str| {
            model
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
            ctx.clear_kv_cache();
            decode(ctx, letters, &[(prefix, 0, None)], &check)?;
            tokens += prefix.len() as u64;
            Some(
                ctx.state_seq_get(0, LlamaStateSeqFlags::default())
                    .map_err(|_| Stop::Failed)?,
            )
        } else {
            None
        };
        let skip = if saved.is_some() { prefix.len() } else { 0 };
        // The unified KV pool holds every branch of a group: the prefix
        // once per sequence plus the suffixes.
        let group = parallel.min((n_ctx / longest).max(1));
        for chunk in tails.chunks(group) {
            check()?;
            ctx.clear_kv_cache();
            if let Some(state) = &saved {
                for seq in 0..chunk.len() {
                    ctx.state_seq_set(state, seq as i32)
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
            for z in decode(ctx, letters, &spans, &check)? {
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
    ctx: &mut LlamaContext<'static>,
    letters: &[LlamaToken],
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
        ctx.decode(&mut batch).map_err(|_| Stop::Failed)?;
        for (seq, row) in wanted {
            let logits = ctx.get_logits_ith(row);
            let options = spans[seq].2.unwrap_or(0);
            out[seq] = Some(
                letters[..options]
                    .iter()
                    .map(|t| logits[t.0 as usize])
                    .collect(),
            );
        }
    }
    Ok(out.into_iter().flatten().collect())
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
