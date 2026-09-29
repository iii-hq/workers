//! Option-label logits for the providers that read a decision from the next
//! token (judge-semif, judge-decider): each prompt ends where the model would
//! write the label of its answer, and the scorer returns the logits of the
//! option labels there, in option order. No text is generated.
//!
//! Every prompt of one evaluation is about one state, and the providers put
//! the state first, so the scorer prefills the prompts' longest common token
//! run once, snapshots the sequence (`state_seq_get`; Qwen3.5's hybrid memory
//! cannot copy sequences), restores it into up to `parallel` sequences and
//! decodes their suffixes together in one batch.
use crate::{Runtime, Session};
use anyhow::{anyhow, bail, Result};
use llama_cpp_2::{
    context::{session::LlamaStateSeqFlags, LlamaContext},
    llama_batch::LlamaBatch,
    model::{AddBos, LlamaModel},
    token::LlamaToken,
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

/// Tokens per `llama_decode` (and llama.cpp's physical batch): large batches
/// keep a GPU busy on prompt processing. Deadlines and cancellation are
/// observed between chunks.
const CHUNK: usize = 2048;

/// The widest label table: `A`–`Z`, then the two-letter labels that are one
/// token each (decider's `label_table`).
pub const MAX_LABELS: usize = 255;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub threads: usize,
    /// None offloads every layer when a GPU device exists.
    pub gpu_layers: Option<u32>,
    pub context_tokens: u32,
    /// Prompts decoded together in one batch (1 = one at a time).
    pub parallel: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Deadline,
    Cancelled,
    /// A prompt does not fit the context window.
    TooLong,
    Failed,
}

#[derive(Debug)]
pub struct Outcome {
    /// Per evaluation, per prompt: the option labels' logits, in option order.
    pub scores: Vec<Vec<Vec<f32>>>,
    /// Tokens decoded per evaluation (shared prefix counted once).
    pub tokens: Vec<u64>,
}

/// The model's vocabulary as the prompts need it: text to tokens, the option
/// labels (`A`–`Z`, then the two-letter labels that are one token, up to
/// `MAX_LABELS`) and the `\n(` that opens a label in decider's wide layout.
pub struct Vocab<'a> {
    model: &'a LlamaModel,
    pub labels: &'a [LlamaToken],
    pub open: &'a [LlamaToken],
}

impl Vocab<'_> {
    pub fn tokenize(&self, text: &str) -> Result<Vec<LlamaToken>, Stop> {
        self.model
            .str_to_token(text, AddBos::Never)
            .map_err(|_| Stop::Failed)
    }
}

/// One evaluation's prompts, tokenized one at a time on the runtime thread:
/// memory grows with the prompts, not with copies of the state.
pub trait Render: Send + 'static {
    fn prompts(&self) -> usize;
    /// Options offered by prompt `i`: its answer is read over that many labels.
    fn options(&self, i: usize) -> usize;
    fn tokens(&self, vocab: &Vocab<'_>, i: usize) -> Result<Vec<LlamaToken>, Stop>;
}

struct Labels {
    ids: Vec<LlamaToken>,
    open: Vec<LlamaToken>,
}

/// A model loaded for option-label scoring; clones share it.
#[derive(Clone)]
pub struct Scorer {
    runtime: Runtime,
    labels: Arc<Labels>,
    parallel: usize,
    pub device: Arc<str>,
    pub context_tokens: u32,
}

impl Scorer {
    /// Load `gguf` on the runtime thread and return once it answers (or failed).
    pub fn spawn(gguf: &Path, options: Options) -> Result<Self> {
        let parallel = options.parallel.max(1);
        let runtime = Runtime::spawn(
            gguf,
            crate::Options {
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
        let labels = runtime.run(|session| label_tokens(session.model))??;
        Ok(Self {
            device: runtime.device().into(),
            runtime,
            labels: Arc::new(labels),
            parallel,
            context_tokens: options.context_tokens,
        })
    }

    /// One-token option labels this vocabulary has (`A`–`Z` at least).
    pub fn labels(&self) -> usize {
        self.labels.ids.len()
    }

    /// Queue a job. Dropping the returned receiver does not stop the work;
    /// set `cancel` or let the deadline pass for that.
    pub fn submit(
        &self,
        evaluations: Vec<Box<dyn Render>>,
        deadline: Instant,
        cancel: Arc<AtomicBool>,
    ) -> oneshot::Receiver<Result<Outcome, Stop>> {
        let (labels, parallel) = (self.labels.clone(), self.parallel);
        self.runtime.submit(move |session| {
            score(session, &labels, parallel, &evaluations, deadline, &cancel)
        })
    }

    /// The token ids of `evaluation`'s prompts (tests compare them with the
    /// reference tokenizers).
    pub fn tokens(&self, evaluation: Box<dyn Render>) -> Result<Vec<Vec<i32>>> {
        let labels = self.labels.clone();
        self.runtime.run(move |session| {
            let vocab = Vocab {
                model: session.model,
                labels: &labels.ids,
                open: &labels.open,
            };
            (0..evaluation.prompts())
                .map(|i| {
                    let ids = evaluation
                        .tokens(&vocab, i)
                        .map_err(|_| anyhow!("tokenization failed"))?;
                    Ok(ids.into_iter().map(|t| t.0).collect())
                })
                .collect()
        })?
    }
}

/// Every letter `A`–`Z` must be one token of the model's vocabulary; the
/// two-letter labels that are one token follow, up to `MAX_LABELS`.
fn label_tokens(model: &LlamaModel) -> Result<Labels> {
    let tokenize = |text: &str| {
        model
            .str_to_token(text, AddBos::Never)
            .map_err(|e| anyhow!("tokenize {text:?}: {e}"))
    };
    let letters: Vec<char> = ('A'..='Z').collect();
    let mut ids = Vec::with_capacity(MAX_LABELS);
    for letter in &letters {
        match tokenize(&letter.to_string())?.as_slice() {
            [token] => ids.push(*token),
            _ => bail!("option letter {letter} is not a single token in this vocabulary"),
        }
    }
    for pair in letters
        .iter()
        .flat_map(|a| letters.iter().map(move |b| format!("{a}{b}")))
    {
        if ids.len() == MAX_LABELS {
            break;
        }
        if let [token] = tokenize(&pair)?.as_slice() {
            ids.push(*token);
        }
    }
    Ok(Labels {
        ids,
        open: tokenize("\n(")?,
    })
}

fn score(
    session: &mut Session<'_>,
    labels: &Labels,
    parallel: usize,
    evaluations: &[Box<dyn Render>],
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
    let vocab = Vocab {
        model,
        labels: &labels.ids,
        open: &labels.open,
    };
    let n_ctx = ctx.n_ctx() as usize;
    let mut outcome = Outcome {
        scores: Vec::with_capacity(evaluations.len()),
        tokens: Vec::with_capacity(evaluations.len()),
    };
    for evaluation in evaluations {
        check()?;
        // Keep the first prompt whole and, of the others, only what follows
        // their common run with it. Prompt `i` is `first[..lcp_i]` then
        // `tails[i]`.
        let mut first = Vec::new();
        let mut common = 0;
        let (mut shortest, mut longest) = (usize::MAX, 1);
        let mut tails: Vec<(usize, Vec<LlamaToken>, usize)> =
            Vec::with_capacity(evaluation.prompts());
        for i in 0..evaluation.prompts() {
            check()?;
            let options = evaluation.options(i);
            if options > labels.ids.len() {
                return Err(Stop::Failed);
            }
            let ids = evaluation.tokens(&vocab, i)?;
            if ids.len() > n_ctx {
                return Err(Stop::TooLong);
            }
            (shortest, longest) = (shortest.min(ids.len()), longest.max(ids.len()));
            // The shared prefix is the longest common run of prompt tokens,
            // not a re-tokenized text prefix: the state's closing bytes can
            // merge with what follows it (`{}}` vs `{}`) more than one token back.
            if tails.is_empty() {
                common = ids.len();
                tails.push((common, Vec::new(), options));
                first = ids;
            } else {
                // At most `common`, so `common` stays the running minimum.
                common = first[..common]
                    .iter()
                    .zip(&ids)
                    .take_while(|(a, b)| a == b)
                    .count();
                tails.push((common, ids[common..].to_vec(), options));
            }
        }
        // Every prompt keeps at least its last token to decode.
        let common = common.min(shortest.saturating_sub(1));
        let prefix = &first[..common];
        let shared = tails.len() > 1 && !prefix.is_empty();
        let mut tokens = 0u64;
        let mut scores = Vec::with_capacity(tails.len());
        let saved = if shared {
            ctx.clear_kv_cache();
            decode(ctx, &labels.ids, &[(prefix, 0, None)], &check)?;
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
            scores.extend(decode(ctx, &labels.ids, &spans, &check)?);
        }
        outcome.scores.push(scores);
        outcome.tokens.push(tokens);
    }
    Ok(outcome)
}

/// Decode each span `(tokens, start, options)` into sequence `i` at
/// positions `start..`, packed into batches of `CHUNK` tokens. For spans
/// with `Some(options)`, return the option labels' logits at their last
/// position, in span order.
#[allow(clippy::type_complexity)]
fn decode(
    ctx: &mut LlamaContext<'_>,
    labels: &[LlamaToken],
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
                labels[..options]
                    .iter()
                    .map(|t| logits[t.0 as usize])
                    .collect(),
            );
        }
    }
    Ok(out.into_iter().flatten().collect())
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

#[cfg(test)]
mod tests {
    use super::softmax;

    #[test]
    fn softmax_is_a_distribution_at_any_temperature() {
        assert_eq!(softmax(&[3.0], 1.0), Some(vec![1.0]));
        assert_eq!(softmax(&[], 1.0), None);
        assert_eq!(softmax(&[f32::NAN, 1.0], 1.0), None);
        let hot = softmax(&[2.0, 0.0], 1.0).unwrap();
        let cool = softmax(&[2.0, 0.0], 2.0).unwrap();
        assert!((hot.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!(cool[0] < hot[0] && cool[0] > 0.5);
    }
}
