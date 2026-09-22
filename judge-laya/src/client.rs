//! Typed evaluations over the in-process laya model, with the judge contract's
//! deadlines, atomic results, usage accounting and caller-scoped cancellation.
use crate::{
    cancellation::CancellationRegistry,
    download::Checkpoint,
    encode::{render_options, Encoder, QType, Question as Rendered},
    model::LayaModel,
};
use anyhow::{anyhow, Result};
use candle_core::Device;
use judge_contract::{
    validate_answer, validate_request_with_limits, Answer, CancelRequest, CancelResponse, Content,
    ErrorCode, EvaluateRequest, EvaluateResponse, EvaluationResult, ModelCard, ModelsRequest,
    ModelsResponse, Question, ScoreLevel, Stats, Usage, DEFAULT_MAX_REQUEST_BYTES,
    DEFAULT_MAX_TIMEOUT_MS,
};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::Semaphore,
    time::{timeout_at, Instant},
};

/// Operator limits. `max_request_bytes` bounds the encoded request; the model
/// itself truncates each sequence to its context window.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_request_bytes: usize,
    pub max_timeout_ms: u64,
    /// Questions per forward pass; cancellation is observed between batches.
    pub batch_questions: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
            max_timeout_ms: DEFAULT_MAX_TIMEOUT_MS,
            batch_questions: 16,
        }
    }
}
impl Limits {
    fn validate(self) -> Result<(), ErrorCode> {
        if self.max_request_bytes == 0
            || self.batch_questions == 0
            || self.max_timeout_ms == 0
            || i64::try_from(self.max_timeout_ms).is_err()
        {
            return Err(ErrorCode::InvalidRequest);
        }
        Ok(())
    }
}

/// Clone per handler; clones share the model, the forward permit and the
/// cancellation registry.
#[derive(Clone)]
pub struct LayaClient {
    model: Arc<LayaModel>,
    encoder: Arc<Encoder>,
    name: Arc<str>,
    revision: Arc<str>,
    permit: Arc<Semaphore>,
    limits: Limits,
    calls: Arc<CancellationRegistry>,
    caller_id: Option<Arc<str>>,
}

struct Row {
    evaluation: usize,
    qid: String,
    qtype: QType,
    keys: Vec<String>,
    legend: BTreeMap<String, ScoreLevel>,
    ids: Vec<u32>,
    markers: Vec<usize>,
}

impl LayaClient {
    /// Load the checkpoint synchronously (seconds: the weights are mmapped).
    pub fn load(checkpoint: &Checkpoint, device: Device) -> Result<Self> {
        let model = LayaModel::load(checkpoint, device)?;
        let tokenizer = tokenizers::Tokenizer::from_file(&checkpoint.tokenizer)
            .map_err(|e| anyhow!("tokenizer: {e}"))?;
        let encoder = Encoder::new(tokenizer, model.agent.max_len, model.agent.head_max_len)?;
        Ok(Self {
            model: Arc::new(model),
            encoder: Arc::new(encoder),
            name: checkpoint.model.as_str().into(),
            revision: checkpoint.revision.as_str().into(),
            // One forward at a time: candle already uses every core for a batch.
            permit: Arc::new(Semaphore::new(1)),
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
        if let Err(code) = self
            .limits
            .validate()
            .and_then(|_| validate_request_with_limits(&request, self.limits.max_request_bytes))
        {
            return failure(code, stats);
        }
        let rows = match self.rows(&request) {
            Ok(rows) => rows,
            Err(code) => return failure(code, stats),
        };
        let mut results: BTreeMap<String, EvaluationResult> = request
            .evaluations
            .iter()
            .map(|evaluation| {
                (
                    evaluation.id.clone(),
                    EvaluationResult {
                        answers: BTreeMap::new(),
                        usage: Some(Usage {
                            input_tokens: Some(0),
                            output_tokens: Some(0),
                        }),
                    },
                )
            })
            .collect();
        let outcome: Result<(), ErrorCode> = async {
            for batch in rows.chunks(self.limits.batch_questions) {
                let _permit = self
                    .permit
                    .acquire()
                    .await
                    .map_err(|_| ErrorCode::Transport)?;
                if Instant::now() >= deadline {
                    return Err(ErrorCode::Deadline);
                }
                let model = self.model.clone();
                let inputs: Vec<(Vec<u32>, Vec<usize>, u32)> = batch
                    .iter()
                    .map(|row| (row.ids.clone(), row.markers.clone(), row.qtype as u32))
                    .collect();
                stats.attempts += 1;
                let logits = tokio::select! {
                    biased;
                    _ = guard.cancelled() => return Err(ErrorCode::Cancelled),
                    joined = timeout_at(deadline, tokio::task::spawn_blocking(move || model.logits(&inputs))) => {
                        joined
                            .map_err(|_| ErrorCode::Deadline)?
                            .map_err(|_| ErrorCode::Transport)?
                            .map_err(|_| ErrorCode::InvalidResponse)?
                    }
                };
                stats.requests += 1;
                for (row, z) in batch.iter().zip(logits) {
                    let question = &request.evaluations[row.evaluation].questions[&row.qid];
                    let answer = self.answer(row, &z)?;
                    validate_answer(question, &answer).map_err(|_| ErrorCode::InvalidResponse)?;
                    let result = results
                        .get_mut(&request.evaluations[row.evaluation].id)
                        .expect("every evaluation has a result slot");
                    result.answers.insert(row.qid.clone(), answer);
                    if let Some(usage) = &mut result.usage {
                        usage.input_tokens = usage.input_tokens.map(|n| n + row.ids.len() as u64);
                    }
                    stats.questions += 1;
                    stats.input_tokens += row.ids.len() as u64;
                }
            }
            Ok(())
        }
        .await;
        stats.elapsed_ms = started.elapsed().as_millis() as u64;
        match outcome {
            Ok(()) => {
                stats.usage_complete = true;
                EvaluateResponse::Ok {
                    model: self.name.to_string(),
                    results,
                    stats,
                }
            }
            Err(code) => failure(code, stats),
        }
    }

    /// The loaded checkpoint is the whole catalog.
    pub async fn list_models(&self, request: ModelsRequest) -> ModelsResponse {
        let started = Instant::now();
        let stats = |ok: bool| Stats {
            attempts: 0,
            requests: 0,
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
        ModelsResponse::Ok {
            models: vec![ModelCard {
                name: self.name.to_string(),
                description: format!(
                    "laya {} checkpoint ({}), running in-process on the CPU",
                    self.name, self.model.agent.encoder
                ),
                release_date: self.revision.to_string(),
            }],
            stats: stats(true),
        }
    }

    fn rows(&self, request: &EvaluateRequest) -> Result<Vec<Row>, ErrorCode> {
        let mut rows = Vec::new();
        for (index, evaluation) in request.evaluations.iter().enumerate() {
            for (qid, question) in &evaluation.questions {
                let (qtype, instructions, criteria, keys, legend) = match question {
                    Question::Noul {
                        instructions,
                        criteria,
                    } => (
                        QType::Noul,
                        instructions,
                        criteria
                            .as_ref()
                            .map(|c| serde_json::to_value(c).unwrap_or_default()),
                        vec!["false".to_owned(), "true".to_owned()],
                        BTreeMap::new(),
                    ),
                    Question::Choice {
                        instructions,
                        criteria,
                    } => (
                        QType::Choice,
                        instructions,
                        Some(serde_json::to_value(criteria).unwrap_or_default()),
                        criteria.keys().cloned().collect(),
                        BTreeMap::new(),
                    ),
                    Question::Score {
                        instructions,
                        criteria,
                    } => (
                        QType::Score,
                        instructions,
                        Some(serde_json::to_value(criteria).unwrap_or_default()),
                        (0..criteria.len()).map(|i| i.to_string()).collect(),
                        criteria
                            .iter()
                            .enumerate()
                            .map(|(i, level)| (i.to_string(), level.clone()))
                            .collect(),
                    ),
                };
                let rendered = Rendered {
                    qtype,
                    instructions: match instructions {
                        Content::Text(text) => text.clone(),
                        Content::Null => String::new(),
                        other => crate::encode::python_json(
                            &serde_json::to_value(other).unwrap_or_default(),
                        ),
                    },
                    options: render_options(qtype, criteria.as_ref())
                        .map_err(|_| ErrorCode::InvalidRequest)?,
                };
                let sequence = self
                    .encoder
                    .build(&evaluation.state, &rendered)
                    .map_err(|_| ErrorCode::InvalidRequest)?;
                // Every option needs its marker inside the window, like laya's own check.
                if sequence.markers.len() != rendered.options.len() {
                    return Err(ErrorCode::PayloadTooLarge);
                }
                rows.push(Row {
                    evaluation: index,
                    qid: qid.clone(),
                    qtype,
                    keys,
                    legend,
                    ids: sequence.ids,
                    markers: sequence.markers,
                });
            }
        }
        Ok(rows)
    }

    /// laya's readout: temperature-scaled softmax, entropy confidence.
    fn answer(&self, row: &Row, z: &[f32]) -> Result<Answer, ErrorCode> {
        let k = row.keys.len();
        if z.len() != k || z.iter().any(|v| !v.is_finite()) {
            return Err(ErrorCode::InvalidResponse);
        }
        let t = self.model.temperature(row.qtype as u32, k);
        let max = z
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, |m, v| m.max(v as f64));
        let exp: Vec<f64> = z.iter().map(|&v| ((v as f64 - max) / t).exp()).collect();
        let sum: f64 = exp.iter().sum();
        let p: Vec<f64> = exp.iter().map(|v| v / sum).collect();
        let entropy: f64 = -p.iter().map(|&x| x * x.max(1e-12).ln()).sum::<f64>();
        let confidence = if k < 2 {
            1.0
        } else {
            (1.0 - entropy / (k as f64).ln()).clamp(0.0, 1.0)
        };
        let probabilities: BTreeMap<String, f64> =
            row.keys.iter().cloned().zip(p.iter().cloned()).collect();
        let best = (0..k).max_by(|&a, &b| p[a].total_cmp(&p[b])).unwrap_or(0);
        Ok(match row.qtype {
            QType::Noul => Answer::Noul { noul: p[1] },
            QType::Choice => Answer::Choice {
                choice: row.keys[best].clone(),
                probabilities,
                confidence,
            },
            QType::Score => Answer::Score {
                score: p.iter().enumerate().map(|(i, x)| i as f64 * x).sum(),
                probabilities,
                confidence,
                legend: row.legend.clone(),
            },
        })
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
