//! decider on the shared option-label scorer (`iii_llama_runtime::scorer`):
//! one thread owns the model context, so a single forward runs on the
//! hardware at a time. Every prompt of an evaluation starts with the same
//! `Context:` tokens, so the prompts share its prefill and decode their
//! question blocks in parallel.
use crate::{
    download::Checkpoint,
    prompt::{self, Piece},
};
use anyhow::Result;
pub use iii_llama_runtime::scorer::{Options, Outcome, Stop};
use iii_llama_runtime::{
    llama_cpp_2::token::LlamaToken,
    scorer::{Render, Scorer, Vocab},
};
use std::{
    cell::OnceCell,
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};
use tokio::sync::oneshot;

/// One decision the model reads: its question text and option texts.
pub type Prompt = (String, Vec<String>);

/// One evaluation: prompts about one state.
pub struct Evaluation {
    pub state: serde_json::Value,
    pub prompts: Vec<Prompt>,
    /// `Context:\n<state>` tokenized once, as `prompt_fast.build_rows` does.
    context: OnceCell<Vec<LlamaToken>>,
}

impl Evaluation {
    pub fn new(state: serde_json::Value, prompts: Vec<Prompt>) -> Self {
        Self {
            state,
            prompts,
            context: OnceCell::new(),
        }
    }
}

impl Render for Evaluation {
    fn prompts(&self) -> usize {
        self.prompts.len()
    }
    fn options(&self, i: usize) -> usize {
        self.prompts[i].1.len()
    }
    fn tokens(&self, vocab: &Vocab<'_>, i: usize) -> Result<Vec<LlamaToken>, Stop> {
        let context = match self.context.get() {
            Some(context) => context,
            None => {
                let text = format!("Context:\n{}", prompt::render_state(&self.state));
                let ids = vocab.tokenize(&text)?;
                self.context.get_or_init(|| ids)
            }
        };
        let (question, options) = &self.prompts[i];
        let mut ids = context.clone();
        match prompt::question_piece(question, options) {
            Piece::Whole(text) => ids.extend(vocab.tokenize(&text)?),
            Piece::Wide {
                head,
                options,
                tail,
            } => {
                ids.extend(vocab.tokenize(&head)?);
                for (option, label) in options.iter().zip(vocab.labels) {
                    ids.extend(vocab.open);
                    ids.push(*label);
                    ids.extend(vocab.tokenize(option)?);
                }
                ids.extend(vocab.tokenize(tail)?);
            }
        }
        Ok(ids)
    }
}

#[derive(Clone)]
pub struct Engine {
    scorer: Scorer,
    pub device: Arc<str>,
    pub context_tokens: u32,
    /// Most options one prompt can offer: the vocabulary's one-token labels.
    pub max_options: usize,
}

impl Engine {
    /// Load the GGUF on the runtime thread and return once it answers (or failed).
    pub fn spawn(checkpoint: &Checkpoint, options: Options) -> Result<Self> {
        let scorer = Scorer::spawn(&checkpoint.gguf, options)?;
        Ok(Self {
            device: scorer.device.clone(),
            context_tokens: scorer.context_tokens,
            max_options: scorer.labels().min(prompt::MAX_OPTIONS),
            scorer,
        })
    }

    /// Queue a job; the outcome holds each prompt's label logits. Dropping
    /// the receiver does not stop the work; set `cancel` or let the deadline
    /// pass for that.
    pub fn submit(
        &self,
        evaluations: Vec<Evaluation>,
        deadline: Instant,
        cancel: Arc<AtomicBool>,
    ) -> oneshot::Receiver<Result<Outcome, Stop>> {
        let evaluations = evaluations
            .into_iter()
            .map(|evaluation| Box::new(evaluation) as Box<dyn Render>)
            .collect();
        self.scorer.submit(evaluations, deadline, cancel)
    }

    /// The token ids of `prompts` about `state` (tests compare them with
    /// decider's tokenizer).
    pub fn prompt_tokens(
        &self,
        state: serde_json::Value,
        prompts: Vec<Prompt>,
    ) -> Result<Vec<Vec<i32>>> {
        self.scorer
            .tokens(Box::new(Evaluation::new(state, prompts)))
    }
}
