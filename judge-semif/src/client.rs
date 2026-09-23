//! Typed evaluations over the in-process SemIf engine, with the judge
//! contract's deadlines, atomic results, usage accounting and caller-scoped
//! cancellation.
use crate::{
    cancellation::CancellationRegistry,
    download::Checkpoint,
    engine::{self, Engine, Stop},
    prompt::{self, MAX_OPTIONS},
};
use anyhow::Result;
use judge_contract::{
    encode_evaluation_with_limits, validate_answer, validate_request_with_limits, Answer,
    CancelRequest, CancelResponse, Content, ErrorCode, EvaluateRequest, EvaluateResponse,
    EvaluationResult, ModelCard, ModelsRequest, ModelsResponse, Question, ScoreLevel, Stats, Usage,
    DEFAULT_MAX_REQUEST_BYTES, DEFAULT_MAX_TIMEOUT_MS,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
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

/// Clone per handler; clones share the engine and the cancellation registry.
#[derive(Clone)]
pub struct SemifClient {
    engine: Engine,
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

/// How one contract question maps onto SemIf's lettered options.
struct Plan {
    keys: Vec<String>,
    kind: Kind,
}
enum Kind {
    /// Option A is "true", B is "false" (SemIf's yes-then-no order).
    Noul,
    Choice,
    Score(BTreeMap<String, ScoreLevel>),
}

impl SemifClient {
    /// Load the GGUF on the engine thread (seconds; the weights are mmapped).
    pub fn load(checkpoint: &Checkpoint, options: engine::Options) -> Result<Self> {
        Ok(Self {
            engine: Engine::spawn(checkpoint, options)?,
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
        let mut jobs = Vec::with_capacity(request.evaluations.len());
        let mut plans = Vec::with_capacity(request.evaluations.len());
        for evaluation in &request.evaluations {
            let mut questions = Vec::with_capacity(evaluation.questions.len());
            let mut evaluation_plans = Vec::with_capacity(evaluation.questions.len());
            for (qid, question) in &evaluation.questions {
                let (criterion, options, plan) = match plan(question) {
                    Ok(planned) => planned,
                    Err(code) => return failure(code, stats),
                };
                questions.push((criterion, options));
                evaluation_plans.push((qid.clone(), plan));
            }
            // The engine renders each prompt as it tokenizes it: one copy of
            // the state per evaluation, not one per question.
            jobs.push(engine::Evaluation {
                state: evaluation.state.clone(),
                questions,
            });
            plans.push(evaluation_plans);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        // Stops the engine between chunks rather than finishing unwanted work,
        // including when the caller drops this future mid-evaluation.
        let _stop = CancelOnDrop(cancel.clone());
        let reply = self.engine.submit(jobs, deadline.into_std(), cancel);
        stats.attempts = 1;
        let outcome = tokio::select! {
            biased;
            _ = guard.cancelled() => Err(Stop::Cancelled),
            joined = timeout_at(deadline, reply) => match joined {
                Err(_) => Err(Stop::Deadline),
                Ok(Err(_)) => Err(Stop::Failed),
                Ok(Ok(result)) => result,
            },
        };
        stats.elapsed_ms = started.elapsed().as_millis() as u64;
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(stop) => {
                let code = match stop {
                    Stop::Deadline => ErrorCode::Deadline,
                    Stop::Cancelled => ErrorCode::Cancelled,
                    Stop::TooLong => ErrorCode::PayloadTooLarge,
                    Stop::Failed => ErrorCode::InvalidResponse,
                };
                return failure(code, stats);
            }
        };
        let mut results = BTreeMap::new();
        for ((evaluation, evaluation_plans), (scores, tokens)) in request
            .evaluations
            .iter()
            .zip(plans)
            .zip(outcome.scores.into_iter().zip(outcome.tokens))
        {
            let mut answers = BTreeMap::new();
            for ((qid, plan), scored) in evaluation_plans.into_iter().zip(scores) {
                let answer = answer(&plan, &scored.probabilities);
                if validate_answer(&evaluation.questions[&qid], &answer).is_err() {
                    return failure(ErrorCode::InvalidResponse, stats);
                }
                answers.insert(qid, answer);
            }
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
            crate::download::model(&self.name).map_or("local GGUF", |m| m.description);
        ModelsResponse::Ok {
            models: vec![ModelCard {
                name: self.name.to_string(),
                description: format!(
                    "{description}; running in-process on {} with a {}-token window",
                    self.engine.device, self.engine.context_tokens
                ),
                release_date: self.revision.to_string(),
                context_window: Some(self.engine.context_tokens),
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

/// Text for prompts: strings as-is, structured values as Python-style JSON.
fn text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => prompt::python_json(other),
    }
}

fn content_text(content: &Content) -> String {
    text(&serde_json::to_value(content).unwrap_or_default())
}

/// A question's criterion text, option descriptions and answer mapping.
fn plan(question: &Question) -> Result<(String, Vec<String>, Plan), ErrorCode> {
    let (instructions, options, plan) = match question {
        Question::Noul {
            instructions,
            criteria,
        } => {
            let side = |key: &str, fallback: &str| {
                criteria
                    .as_ref()
                    .and_then(|c| c.get(key))
                    .map(content_text)
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| fallback.to_owned())
            };
            (
                instructions,
                vec![
                    side("true", "Yes, the statement holds."),
                    side("false", "No, the statement does not hold."),
                ],
                Plan {
                    keys: vec!["true".into(), "false".into()],
                    kind: Kind::Noul,
                },
            )
        }
        Question::Choice {
            instructions,
            criteria,
        } => (
            instructions,
            criteria
                .iter()
                .map(|(key, value)| match content_text(value) {
                    t if t.is_empty() => key.clone(),
                    t => format!("{key}: {t}"),
                })
                .collect(),
            Plan {
                keys: criteria.keys().cloned().collect(),
                kind: Kind::Choice,
            },
        ),
        Question::Score {
            instructions,
            criteria,
        } => (
            instructions,
            criteria
                .iter()
                .enumerate()
                .map(|(i, level)| {
                    format!(
                        "level {i}: {}",
                        text(&serde_json::to_value(level).unwrap_or_default())
                    )
                })
                .collect(),
            Plan {
                keys: (0..criteria.len()).map(|i| i.to_string()).collect(),
                kind: Kind::Score(
                    criteria
                        .iter()
                        .enumerate()
                        .map(|(i, level)| (i.to_string(), level.clone()))
                        .collect(),
                ),
            },
        ),
    };
    if options.len() > MAX_OPTIONS {
        return Err(ErrorCode::PayloadTooLarge);
    }
    // SemIf's criterion is never empty; an instruction-less question asks
    // which option holds.
    let criterion = match content_text(instructions) {
        t if t.is_empty() => "Which option applies?".to_owned(),
        t => t,
    };
    Ok((criterion, options, plan))
}

/// Readout: probabilities in option order, entropy confidence (1 - H/ln k).
fn answer(plan: &Plan, p: &[f64]) -> Answer {
    let k = p.len();
    let entropy: f64 = -p.iter().map(|&x| x * x.max(1e-12).ln()).sum::<f64>();
    let confidence = (1.0 - entropy / (k as f64).ln()).clamp(0.0, 1.0);
    let probabilities: BTreeMap<String, f64> =
        plan.keys.iter().cloned().zip(p.iter().cloned()).collect();
    let best = (0..k).max_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap_or(0);
    match &plan.kind {
        Kind::Noul => Answer::Noul { noul: p[0] },
        Kind::Choice => Answer::Choice {
            choice: plan.keys[best].clone(),
            probabilities,
            confidence,
        },
        Kind::Score(legend) => Answer::Score {
            score: p.iter().enumerate().map(|(i, x)| i as f64 * x).sum(),
            probabilities,
            confidence,
            legend: legend.clone(),
        },
    }
}
