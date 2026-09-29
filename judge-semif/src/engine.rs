//! SemIf on the shared option-label scorer (`iii_llama_runtime::scorer`): one
//! thread owns the model context, so a single forward runs on the hardware at
//! a time. Each question is one whole chat prompt read over the letters
//! `A`–`P`; SemIf puts the evidence first, so the questions about one state
//! share its prefill and decode their suffixes in parallel (SemIf's parallel
//! suffixes).
use crate::{download::Checkpoint, prompt};
use anyhow::Result;
pub use iii_llama_runtime::scorer::{Options, Stop};
use iii_llama_runtime::{
    llama_cpp_2::token::LlamaToken,
    scorer::{softmax, Render, Scorer, Vocab},
};
use std::{
    future::Future,
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};
use tokio::sync::oneshot::error::RecvError;

/// One evaluation: questions (criterion, option descriptions) about one state.
pub struct Evaluation {
    pub state: serde_json::Value,
    pub questions: Vec<(String, Vec<String>)>,
}

impl Render for Evaluation {
    fn prompts(&self) -> usize {
        self.questions.len()
    }
    fn options(&self, i: usize) -> usize {
        self.questions[i].1.len()
    }
    fn tokens(&self, vocab: &Vocab<'_>, i: usize) -> Result<Vec<LlamaToken>, Stop> {
        let (criterion, options) = &self.questions[i];
        vocab.tokenize(&prompt::render(&self.state, criterion, options))
    }
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

#[derive(Clone)]
pub struct Engine {
    scorer: Scorer,
    pub device: Arc<str>,
    pub context_tokens: u32,
}

impl Engine {
    /// Load the GGUF on the runtime thread and return once it answers (or failed).
    pub fn spawn(checkpoint: &Checkpoint, options: Options) -> Result<Self> {
        let scorer = Scorer::spawn(&checkpoint.gguf, options)?;
        Ok(Self {
            device: scorer.device.clone(),
            context_tokens: scorer.context_tokens,
            scorer,
        })
    }

    /// Queue a job. Dropping the returned future does not stop the work; set
    /// `cancel` or let the deadline pass for that.
    pub fn submit(
        &self,
        evaluations: Vec<Evaluation>,
        deadline: Instant,
        cancel: Arc<AtomicBool>,
    ) -> impl Future<Output = Result<Result<Outcome, Stop>, RecvError>> {
        let evaluations = evaluations
            .into_iter()
            .map(|evaluation| Box::new(evaluation) as Box<dyn Render>)
            .collect();
        let reply = self.scorer.submit(evaluations, deadline, cancel);
        async move {
            reply.await.map(|outcome| {
                let outcome = outcome?;
                let scores = outcome
                    .scores
                    .iter()
                    .map(|prompts| {
                        prompts
                            .iter()
                            .map(|z| softmax(z, 1.0).map(|probabilities| Scored { probabilities }))
                            .collect::<Option<Vec<_>>>()
                            .ok_or(Stop::Failed)
                    })
                    .collect::<Result<_, _>>()?;
                Ok(Outcome {
                    scores,
                    tokens: outcome.tokens,
                })
            })
        }
    }
}
