//! Typed evaluations over the in-process Clef checkpoint (backbone engine and
//! joint schema head), with the judge contract's deadlines, atomic results,
//! usage accounting and caller-scoped cancellation.
use crate::{
    cancellation::CancellationRegistry,
    download::Checkpoint,
    encode::{self, Encoded},
    engine::{self, Engine, Stop},
    head::{Head, Lexicon},
};
use anyhow::{anyhow, ensure, Result};
use iii_llama_runtime::scorer::softmax;
use judge_contract::{
    confidence, encode_evaluation_with_limits, validate_answer, validate_request_with_limits,
    Answer, CancelRequest, CancelResponse, ErrorCode, EvaluateRequest, EvaluateResponse,
    EvaluationResult, ModelCard, ModelsRequest, ModelsResponse, Question, Stats, Usage,
    DEFAULT_MAX_REQUEST_BYTES, DEFAULT_MAX_TIMEOUT_MS,
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokenizers::Tokenizer;
use tokio::time::{timeout_at, Instant};

/// Operator limits on the request itself; the context window bounds each prompt.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_request_bytes: usize,
    pub max_timeout_ms: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
            max_timeout_ms: DEFAULT_MAX_TIMEOUT_MS,
        }
    }
}
impl Limits {
    fn validate(self) -> Result<(), ErrorCode> {
        if self.max_request_bytes == 0
            || self.max_timeout_ms == 0
            || i64::try_from(self.max_timeout_ms).is_err()
        {
            return Err(ErrorCode::InvalidRequest);
        }
        Ok(())
    }
}

/// Clone per handler; clones share the engine, the head and the cancellation
/// registry.
#[derive(Clone)]
pub struct ClefClient {
    engine: Engine,
    head: Arc<Head>,
    lexicon: Arc<Lexicon>,
    tokenizer: Arc<Tokenizer>,
    name: Arc<str>,
    revision: Arc<str>,
    limits: Limits,
    calls: Arc<CancellationRegistry>,
    caller_id: Option<Arc<str>>,
}

/// Cancels the engine job when the evaluation ends or is dropped.
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl ClefClient {
    /// Load the checkpoint (seconds; the GGUF is mmapped). The GGUF header,
    /// the head and the tokenizer are checked before the backbone takes any
    /// GPU memory.
    pub fn load(checkpoint: &Checkpoint, options: engine::Options) -> Result<Self> {
        let lexicon = Lexicon::open(&checkpoint.gguf)?;
        let head = Head::load(&checkpoint.head, &checkpoint.head_config, lexicon.hidden())?;
        let tokenizer = Tokenizer::from_file(&checkpoint.tokenizer)
            .map_err(|e| anyhow!("tokenizer {}: {e}", checkpoint.tokenizer.display()))?;
        let engine = Engine::spawn(&checkpoint.gguf, options, engine::BATCH_TOKENS)?;
        ensure!(
            engine.hidden == lexicon.hidden(),
            "the backbone's hidden size {} differs from its output.weight's {}",
            engine.hidden,
            lexicon.hidden()
        );
        Ok(Self {
            engine,
            head: Arc::new(head),
            lexicon: Arc::new(lexicon),
            tokenizer: Arc::new(tokenizer),
            name: checkpoint.model.as_str().into(),
            revision: checkpoint.revision.as_str().into(),
            limits: Limits::default(),
            calls: Arc::new(CancellationRegistry::default()),
            caller_id: Some(Arc::from("local")),
        })
    }

    pub fn with_limits(&self, limits: Limits) -> Self {
        Self {
            limits,
            ..self.clone()
        }
    }

    pub fn with_caller_id(&self, caller_id: Option<&str>) -> Self {
        Self {
            caller_id: caller_id.map(Arc::from),
            ..self.clone()
        }
    }

    pub fn model_name(&self) -> &str {
        &self.name
    }

    pub fn device(&self) -> &str {
        &self.engine.device
    }

    pub fn cancel(&self, request: CancelRequest) -> CancelResponse {
        match self
            .calls
            .cancel(self.caller_id.as_deref(), &request.request_id)
        {
            Ok(cancelled) => CancelResponse::Ok { cancelled },
            Err(code) => CancelResponse::Error { code },
        }
    }

    pub async fn evaluate(&self, request: EvaluateRequest) -> EvaluateResponse {
        let started = Instant::now();
        let mut stats = Stats::default();
        let failure = |code, stats| EvaluateResponse::Error {
            code,
            http_status: None,
            provider_error: None,
            retry_after_ms: None,
            stats,
        };
        let deadline = match self.deadline(started, request.timeout_ms, request.expires_at_unix_ms)
        {
            Ok(deadline) => deadline,
            Err(code) => return failure(code, stats),
        };
        let mut guard = match self
            .calls
            .start(self.caller_id.as_deref(), request.request_id.as_deref())
        {
            Ok(guard) => guard,
            Err(code) => return failure(code, stats),
        };
        if request
            .model
            .as_deref()
            .is_some_and(|model| model != self.name.as_ref())
        {
            return failure(ErrorCode::InvalidRequest, stats);
        }
        let max_bytes = self.limits.max_request_bytes;
        if let Err(code) = self
            .limits
            .validate()
            .and_then(|_| validate_request_with_limits(&request, max_bytes))
            // Each evaluation is bounded like a provider request body.
            .and_then(|_| {
                request.evaluations.iter().try_for_each(|evaluation| {
                    encode_evaluation_with_limits(&self.name, evaluation, max_bytes).map(|_| ())
                })
            })
        {
            return failure(code, stats);
        }
        // Every evaluation is encoded first, so a bad one fails before any inference.
        let tokenize = |text: &str| {
            self.tokenizer
                .encode_fast(text, false)
                .map(|encoding| encoding.get_ids().to_vec())
                .map_err(|_| ErrorCode::InvalidRequest)
        };
        let window = self.engine.context_tokens as usize;
        let mut prompts = Vec::with_capacity(request.evaluations.len());
        for evaluation in &request.evaluations {
            match encode::encode(evaluation, window, tokenize) {
                Ok(encoded) => prompts.push(encoded),
                Err(code) => return failure(code, stats),
            }
        }
        let truncated = prompts.iter().filter(|p| p.dropped > 0).count();
        if truncated > 0 {
            // The model answers about state it never saw: callers sending big
            // states need a larger `context_tokens`.
            tracing::warn!(
                evaluations = prompts.len(),
                truncated,
                dropped_tokens = prompts.iter().map(|p| p.dropped).sum::<usize>(),
                "state truncated to the context window"
            );
        }
        let mut results = BTreeMap::new();
        for (evaluation, encoded) in request.evaluations.iter().zip(prompts) {
            let tokens = encoded.ids.len() as u64;
            // One attempt per backbone forward, as judge-laya counts its passes.
            stats.attempts += 1;
            let logits = tokio::select! {
                biased;
                _ = guard.cancelled() => Err(ErrorCode::Cancelled),
                joined = timeout_at(deadline, self.forward(encoded, deadline)) => {
                    joined.unwrap_or(Err(ErrorCode::Deadline))
                }
            };
            stats.elapsed_ms = started.elapsed().as_millis() as u64;
            let logits = match logits {
                Ok(logits) if logits.len() == evaluation.questions.len() => logits,
                Ok(_) => return failure(ErrorCode::InvalidResponse, stats),
                Err(code) => return failure(code, stats),
            };
            let answers = evaluation
                .questions
                .iter()
                .zip(logits)
                .map(|((qid, question), z)| {
                    answer(question, &z)
                        .filter(|answer| validate_answer(question, answer).is_ok())
                        .map(|answer| (qid.clone(), answer))
                })
                .collect::<Option<BTreeMap<_, _>>>();
            let Some(answers) = answers else {
                return failure(ErrorCode::InvalidResponse, stats);
            };
            stats.requests += 1;
            stats.questions += answers.len();
            stats.input_tokens += tokens;
            results.insert(
                evaluation.id.clone(),
                EvaluationResult {
                    answers,
                    usage: Some(Usage {
                        input_tokens: Some(tokens),
                        output_tokens: Some(0),
                    }),
                },
            );
        }
        stats.usage_complete = true;
        EvaluateResponse::Ok {
            model: self.name.to_string(),
            results,
            stats,
        }
    }

    /// One evaluation's option logits: the backbone states, then the joint
    /// head on the blocking pool. The engine job stops between chunks when
    /// this future ends or is dropped.
    async fn forward(
        &self,
        encoded: Encoded,
        deadline: Instant,
    ) -> Result<Vec<Vec<f32>>, ErrorCode> {
        let cancel = Arc::new(AtomicBool::new(false));
        let _stop = CancelOnDrop(cancel.clone());
        let states = match self
            .engine
            .states(encoded.ids.clone(), deadline.into_std(), cancel)
            .await
        {
            Err(_) => return Err(ErrorCode::Transport),
            Ok(Ok(states)) => states,
            Ok(Err(stop)) => {
                return Err(match stop {
                    Stop::Deadline => ErrorCode::Deadline,
                    Stop::Cancelled => ErrorCode::Cancelled,
                    Stop::TooLong => ErrorCode::PayloadTooLarge,
                    Stop::Failed => ErrorCode::InvalidResponse,
                })
            }
        };
        let (head, lexicon) = (self.head.clone(), self.lexicon.clone());
        tokio::task::spawn_blocking(move || head.logits(&lexicon, states, &encoded))
            .await
            .map_err(|_| ErrorCode::Transport)?
            .map_err(|_| ErrorCode::InvalidResponse)
    }

    /// The loaded model is the whole catalog.
    pub async fn list_models(&self, request: ModelsRequest) -> ModelsResponse {
        let started = Instant::now();
        let stats = |ok: bool| Stats {
            elapsed_ms: started.elapsed().as_millis() as u64,
            usage_complete: ok,
            ..Stats::default()
        };
        if let Err(code) = self.deadline(started, request.timeout_ms, request.expires_at_unix_ms) {
            return ModelsResponse::Error {
                code,
                http_status: None,
                provider_error: None,
                retry_after_ms: None,
                stats: stats(false),
            };
        }
        let description =
            crate::download::model(&self.name).map_or("local checkpoint", |m| m.description);
        ModelsResponse::Ok {
            models: vec![ModelCard {
                name: self.name.to_string(),
                description: format!(
                    "{description}; running in-process on {} with a {}-token window",
                    self.engine.device, self.engine.context_tokens
                ),
                release_date: self.revision.to_string(),
                context_window: Some(self.engine.context_tokens),
                // The contract's own bound: every option is a span of the prompt.
                max_options: Some(255),
            }],
            stats: stats(true),
        }
    }

    fn deadline(
        &self,
        started: Instant,
        timeout_ms: u64,
        expiry: Option<u64>,
    ) -> Result<Instant, ErrorCode> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let remaining_ms = expiry
            .map(|expiry| u128::from(expiry).saturating_sub(now_ms))
            .unwrap_or(u128::MAX);
        if remaining_ms == 0 {
            return Err(ErrorCode::Deadline);
        }
        self.limits.validate()?;
        if !(1..=self.limits.max_timeout_ms).contains(&timeout_ms) {
            return Err(ErrorCode::InvalidRequest);
        }
        let budget_ms = u64::try_from(remaining_ms.min(u128::from(timeout_ms)))
            .map_err(|_| ErrorCode::InvalidRequest)?;
        started
            .checked_add(Duration::from_millis(budget_ms))
            .ok_or(ErrorCode::InvalidRequest)
    }
}

/// A question's answer from its option logits, in `encode::options` order: a
/// softmax at temperature 1 as in the reference, with TypeSafe's confidences
/// (`judge_contract::confidence`).
fn answer(question: &Question, z: &[f32]) -> Option<Answer> {
    let keys: Vec<String> = encode::options(question)
        .1
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    let p = softmax(z, 1.0).filter(|p| p.len() == keys.len())?;
    // The first of equal maxima, as Python's max() picks it.
    let best = (0..p.len()).fold(0, |best, i| if p[i] > p[best] { i } else { best });
    let probabilities: BTreeMap<String, f64> =
        keys.iter().cloned().zip(p.iter().cloned()).collect();
    Some(match question {
        // Options `true` then `false`.
        Question::Noul { .. } => Answer::Noul { noul: p[0] },
        Question::Choice { .. } => Answer::Choice {
            choice: keys[best].clone(),
            probabilities,
            confidence: confidence::choice(&p),
        },
        Question::Score { criteria, .. } => Answer::Score {
            score: p.iter().enumerate().map(|(i, x)| i as f64 * x).sum(),
            probabilities,
            confidence: confidence::score(&p),
            legend: keys.into_iter().zip(criteria.iter().cloned()).collect(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn question(value: serde_json::Value) -> Question {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn answers_read_the_logits_in_prompt_order() {
        // Clef lists `true` first.
        let noul = answer(&question(json!({"type": "noul"})), &[2.0, 0.0]);
        assert!(matches!(noul, Some(Answer::Noul { noul }) if noul > 0.88));
        let score = question(json!({"type": "score", "criteria": ["low", "mid", "high"]}));
        let Some(Answer::Score { score, legend, .. }) = answer(&score, &[0.0, 0.0, 30.0]) else {
            panic!("a score answer");
        };
        assert!((score - 2.0).abs() < 1e-9 && legend.len() == 3);
        let choice = question(json!({"type": "choice", "criteria": {"b": null, "a": null}}));
        assert!(
            matches!(answer(&choice, &[0.0, 1.0]), Some(Answer::Choice { choice, .. }) if choice == "b")
        );
        // One logit per option, or no answer.
        assert!(answer(&choice, &[0.0]).is_none());
    }
}
