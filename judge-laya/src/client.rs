//! Typed evaluations over the in-process laya checkpoints, with the judge
//! contract's deadlines, atomic results, usage accounting and caller-scoped
//! cancellation. Each evaluation routes to a loaded checkpoint like laya's
//! `Router` (explicit `model`, typed-decisions workflow, state language).
use crate::{
    cancellation::{CallGuard, CancellationRegistry},
    download::Checkpoint,
    encode::{render_options, serialize_state, Encoder, QType, Question as Rendered},
    lang,
    model::LayaModel,
};
use anyhow::{anyhow, Result};
use candle_core::Device;
use judge_contract::{
    validate_answer, validate_request_with_limits, Answer, CancelRequest, CancelResponse, Content,
    ErrorCode, EvaluateRequest, EvaluateResponse, Evaluation, EvaluationResult, ModelCard,
    ModelsRequest, ModelsResponse, Question, ScoreLevel, Stats, Usage, DEFAULT_MAX_REQUEST_BYTES,
    DEFAULT_MAX_TIMEOUT_MS,
};
use serde_json::Value;
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

/// Which loaded checkpoint answers an evaluation without an explicit `model`,
/// and the opt-in option shortlist (laya's `Router` and `predict_shortlist`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Routing {
    pub auto_route: bool,
    pub auto_task_detection: bool,
    pub shortlist_k: Option<usize>,
}

/// Tokens per text when embedding for the shortlist (laya's default).
const SHORTLIST_MAX_TOKENS: usize = 512;

struct Loaded {
    name: Arc<str>,
    revision: Arc<str>,
    model: Arc<LayaModel>,
    encoder: Arc<Encoder>,
}

/// Clone per handler; clones share the checkpoints, the forward permit and the
/// cancellation registry.
#[derive(Clone)]
pub struct LayaClient {
    /// Loaded checkpoints; the first is the default.
    models: Arc<Vec<Loaded>>,
    permit: Arc<Semaphore>,
    limits: Limits,
    routing: Routing,
    calls: Arc<CancellationRegistry>,
    caller_id: Option<Arc<str>>,
}

struct Row {
    evaluation: usize,
    model: usize,
    qid: String,
    qtype: QType,
    keys: Vec<String>,
    /// Choice labels the shortlist dropped: they answer with probability 0.
    dropped: Vec<String>,
    legend: BTreeMap<String, ScoreLevel>,
    ids: Vec<u32>,
    markers: Vec<usize>,
}

impl LayaClient {
    /// Load checkpoints synchronously (seconds each: the weights are mmapped);
    /// the first one is the default.
    pub fn load(checkpoints: &[Checkpoint], device: Device) -> Result<Self> {
        let mut models = Vec::with_capacity(checkpoints.len());
        for checkpoint in checkpoints {
            let model = LayaModel::load(checkpoint, device.clone())?;
            let tokenizer = tokenizers::Tokenizer::from_file(&checkpoint.tokenizer)
                .map_err(|e| anyhow!("tokenizer: {e}"))?;
            let encoder = Encoder::new(tokenizer, model.agent.max_len, model.agent.head_max_len)?;
            models.push(Loaded {
                name: checkpoint.model.as_str().into(),
                revision: checkpoint.revision.as_str().into(),
                model: Arc::new(model),
                encoder: Arc::new(encoder),
            });
        }
        if models.is_empty() {
            return Err(anyhow!("no checkpoint to load"));
        }
        Ok(Self {
            models: Arc::new(models),
            // One forward at a time: candle already uses every core for a batch.
            permit: Arc::new(Semaphore::new(1)),
            limits: Limits::default(),
            routing: Routing::default(),
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

    pub fn with_routing(&self, routing: Routing) -> Self {
        Self {
            routing,
            ..self.clone()
        }
    }

    pub fn with_caller_id(&self, caller_id: Option<&str>) -> Self {
        Self {
            caller_id: caller_id.map(Arc::from),
            ..self.clone()
        }
    }

    /// The default checkpoint's name.
    pub fn model_name(&self) -> &str {
        &self.models[0].name
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.models.iter().position(|m| m.name.as_ref() == name)
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
        let explicit = match request.model.as_deref() {
            Some(name) => match self.find(name) {
                Some(index) => Some(index),
                None => return failure(ErrorCode::InvalidRequest, stats),
            },
            None => None,
        };
        if let Err(code) = self
            .limits
            .validate()
            .and_then(|_| validate_request_with_limits(&request, self.limits.max_request_bytes))
        {
            return failure(code, stats);
        }
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
        let outcome: Result<usize, ErrorCode> = async {
            let rows = self.rows(&request, explicit, deadline, &mut guard).await?;
            // The reported model: the one every row used, else the default.
            let reported = rows
                .first()
                .map(|row| row.model)
                .filter(|&model| rows.iter().all(|row| row.model == model))
                .unwrap_or(0);
            let mut per_row = Duration::ZERO;
            for group in rows.chunk_by(|a, b| a.model == b.model) {
                let loaded = &self.models[group[0].model];
                for batch in group.chunks(self.limits.batch_questions) {
                    let model = loaded.model.clone();
                    let inputs: Vec<(Vec<u32>, Vec<usize>, u32)> = batch
                        .iter()
                        .map(|row| (row.ids.clone(), row.markers.clone(), row.qtype as u32))
                        .collect();
                    stats.attempts += 1;
                    let batch_started = Instant::now();
                    let logits = self
                        .blocking(
                            deadline,
                            &mut guard,
                            per_row * batch.len() as u32,
                            move || model.logits(&inputs),
                        )
                        .await?;
                    per_row = batch_started.elapsed() / batch.len() as u32;
                    for (row, z) in batch.iter().zip(logits) {
                        let question = &request.evaluations[row.evaluation].questions[&row.qid];
                        let answer = Self::answer(&loaded.model, row, &z)?;
                        validate_answer(question, &answer)
                            .map_err(|_| ErrorCode::InvalidResponse)?;
                        let result = results
                            .get_mut(&request.evaluations[row.evaluation].id)
                            .expect("every evaluation has a result slot");
                        result.answers.insert(row.qid.clone(), answer);
                        if let Some(usage) = &mut result.usage {
                            usage.input_tokens =
                                usage.input_tokens.map(|n| n + row.ids.len() as u64);
                        }
                        // The contract's unit of a "request" is one evaluation (one
                        // upstream POST for hosted providers): count usage when an
                        // evaluation completes, so failures keep only whole ones.
                        let questions = request.evaluations[row.evaluation].questions.len();
                        if result.answers.len() == questions {
                            stats.requests += 1;
                            stats.questions += questions;
                            stats.input_tokens += result
                                .usage
                                .as_ref()
                                .and_then(|usage| usage.input_tokens)
                                .unwrap_or(0);
                        }
                    }
                }
            }
            Ok(reported)
        }
        .await;
        stats.elapsed_ms = started.elapsed().as_millis() as u64;
        match outcome {
            Ok(model) => {
                stats.usage_complete = true;
                EvaluateResponse::Ok {
                    model: self.models[model].name.to_string(),
                    results,
                    stats,
                }
            }
            Err(code) => failure(code, stats),
        }
    }

    /// Every loaded checkpoint, the default first.
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
            models: self
                .models
                .iter()
                .map(|loaded| ModelCard {
                    name: loaded.name.to_string(),
                    description: format!(
                        "laya {} checkpoint ({}), running in-process",
                        loaded.name, loaded.model.agent.encoder
                    ),
                    release_date: loaded.revision.to_string(),
                    context_window: Some(loaded.model.agent.max_len as u32),
                })
                .collect(),
            stats: stats(true),
        }
    }

    /// laya's `Router.route` over the loaded checkpoints: the explicit model,
    /// then the typed-decisions workflow signature, then the state's script
    /// and language; the default when nothing applies or nothing is loaded.
    fn route(&self, explicit: Option<usize>, evaluation: &Evaluation) -> usize {
        if let Some(index) = explicit {
            return index;
        }
        if self.routing.auto_task_detection {
            let workflow =
                lang::typed_decisions_workflow(evaluation.questions.keys().map(String::as_str));
            if let (Some(workflow), Some(index)) = (workflow, self.find("laya-typed-decisions")) {
                tracing::debug!(
                    evaluation = evaluation.id,
                    workflow,
                    "routed to laya-typed-decisions"
                );
                return index;
            }
        }
        if self.routing.auto_route {
            let detection = lang::analyse(&evaluation.state);
            let wanted = match (detection.script, detection.is_english) {
                ("unknown", _) => None,
                (_, true) => Some("laya"),
                (_, false) => Some("laya-multilingual"),
            };
            if let Some(index) = wanted.and_then(|name| self.find(name)) {
                tracing::debug!(
                    evaluation = evaluation.id,
                    script = detection.script,
                    language = detection.language,
                    model = %self.models[index].name,
                    "routed by state language"
                );
                return index;
            }
        }
        0
    }

    /// Run `work` on the blocking pool under the forward permit. The permit is
    /// acquired within the deadline and travels with the work, so a timed-out
    /// forward keeps the CPU but never overlaps the next caller's. Results are
    /// atomic: work predicted (`estimate`) to miss the deadline is skipped.
    async fn blocking<T: Send + 'static>(
        &self,
        deadline: Instant,
        guard: &mut CallGuard,
        estimate: Duration,
        work: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T, ErrorCode> {
        let permit = timeout_at(deadline, self.permit.clone().acquire_owned())
            .await
            .map_err(|_| ErrorCode::Deadline)?
            .map_err(|_| ErrorCode::Transport)?;
        if Instant::now() + estimate >= deadline {
            return Err(ErrorCode::Deadline);
        }
        tokio::select! {
            biased;
            _ = guard.cancelled() => Err(ErrorCode::Cancelled),
            joined = timeout_at(deadline, tokio::task::spawn_blocking(move || { let _permit = permit; work() })) => {
                joined
                    .map_err(|_| ErrorCode::Deadline)?
                    .map_err(|_| ErrorCode::Transport)?
                    .map_err(|_| ErrorCode::InvalidResponse)
            }
        }
    }

    /// laya's `predict_shortlist` ranking: cosine similarity between the
    /// mean-pooled encoder states of `instructions + state` and of each
    /// rendered option; the top `k` labels win, ties keeping label order.
    #[allow(clippy::too_many_arguments)]
    async fn shortlist(
        &self,
        loaded: &Loaded,
        state: &Value,
        instructions: &str,
        criteria: &Value,
        k: usize,
        deadline: Instant,
        guard: &mut CallGuard,
    ) -> Result<Vec<String>, ErrorCode> {
        let labels: Vec<String> = criteria
            .as_object()
            .map(|c| c.keys().cloned().collect())
            .unwrap_or_default();
        let options =
            render_options(QType::Choice, Some(criteria)).map_err(|_| ErrorCode::InvalidRequest)?;
        let body = serialize_state(state);
        let query = if instructions.is_empty() {
            body
        } else {
            format!("{instructions}\n{body}")
        };
        let texts = std::iter::once(query)
            .chain(options)
            .map(|text| loaded.encoder.encode_text(&text, SHORTLIST_MAX_TOKENS))
            .collect::<Result<Vec<_>>>()
            .map_err(|_| ErrorCode::InvalidRequest)?;
        let model = loaded.model.clone();
        let vectors = self
            .blocking(deadline, guard, Duration::ZERO, move || model.embed(&texts))
            .await?;
        let (query, docs) = vectors.split_first().ok_or(ErrorCode::InvalidResponse)?;
        let norm = |v: &[f32]| v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
        let qn = norm(query);
        let sims: Vec<f64> = docs
            .iter()
            .map(|doc| {
                let dn = norm(doc);
                if qn == 0.0 || dn == 0.0 {
                    0.0
                } else {
                    doc.iter()
                        .zip(query)
                        .map(|(a, b)| f64::from(*a) * f64::from(*b))
                        .sum::<f64>()
                        / (dn * qn)
                }
            })
            .collect();
        let mut order: Vec<usize> = (0..sims.len()).collect();
        order.sort_by(|&a, &b| sims[b].total_cmp(&sims[a]));
        Ok(order
            .into_iter()
            .take(k)
            .map(|i| labels[i].clone())
            .collect())
    }

    async fn rows(
        &self,
        request: &EvaluateRequest,
        explicit: Option<usize>,
        deadline: Instant,
        guard: &mut CallGuard,
    ) -> Result<Vec<Row>, ErrorCode> {
        let mut rows = Vec::new();
        let (mut truncated, mut dropped_tokens) = (0usize, 0usize);
        for (index, evaluation) in request.evaluations.iter().enumerate() {
            let model = self.route(explicit, evaluation);
            let loaded = &self.models[model];
            for (qid, question) in &evaluation.questions {
                let (qtype, instructions, mut criteria, mut keys, legend) = match question {
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
                let instructions = match instructions {
                    Content::Text(text) => text.clone(),
                    Content::Null => String::new(),
                    other => {
                        crate::encode::python_json(&serde_json::to_value(other).unwrap_or_default())
                    }
                };
                let mut dropped = Vec::new();
                if let (QType::Choice, Some(k), Some(all)) =
                    (qtype, self.routing.shortlist_k, criteria.as_ref())
                {
                    if keys.len() > k {
                        let kept = self
                            .shortlist(
                                loaded,
                                &evaluation.state,
                                &instructions,
                                all,
                                k,
                                deadline,
                                guard,
                            )
                            .await?;
                        // Kept options stay in the contract's key order (laya
                        // reorders them by rank); the rest answer 0.
                        dropped = keys
                            .iter()
                            .filter(|key| !kept.contains(key))
                            .cloned()
                            .collect();
                        keys.retain(|key| kept.contains(key));
                        criteria = all.as_object().map(|object| {
                            Value::Object(
                                object
                                    .iter()
                                    .filter(|(key, _)| kept.contains(key))
                                    .map(|(key, value)| (key.clone(), value.clone()))
                                    .collect(),
                            )
                        });
                    }
                }
                let rendered = Rendered {
                    qtype,
                    instructions,
                    options: render_options(qtype, criteria.as_ref())
                        .map_err(|_| ErrorCode::InvalidRequest)?,
                };
                let sequence = loaded
                    .encoder
                    .build(&evaluation.state, &rendered)
                    .map_err(|_| ErrorCode::InvalidRequest)?;
                // Every option needs its marker inside the window, like laya's own check.
                if sequence.markers.len() != rendered.options.len() {
                    return Err(ErrorCode::PayloadTooLarge);
                }
                truncated += usize::from(sequence.state_dropped > 0);
                dropped_tokens += sequence.state_dropped;
                rows.push(Row {
                    evaluation: index,
                    model,
                    qid: qid.clone(),
                    qtype,
                    keys,
                    dropped,
                    legend,
                    ids: sequence.ids,
                    markers: sequence.markers,
                });
            }
        }
        if truncated > 0 {
            // The model answers about state it never saw: callers sending big
            // states (registry searches) need a long-window provider instead.
            tracing::warn!(
                rows = rows.len(),
                truncated,
                dropped_tokens,
                "state truncated to the checkpoint window"
            );
        }
        // One forward serves one checkpoint: group rows by model, keeping the
        // request order within each group.
        rows.sort_by_key(|row| row.model);
        Ok(rows)
    }

    /// laya's readout: temperature-scaled softmax, entropy confidence.
    fn answer(model: &LayaModel, row: &Row, z: &[f32]) -> Result<Answer, ErrorCode> {
        let k = row.keys.len();
        if z.len() != k || z.iter().any(|v| !v.is_finite()) {
            return Err(ErrorCode::InvalidResponse);
        }
        let t = model.temperature(row.qtype as u32, k);
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
        let mut probabilities: BTreeMap<String, f64> =
            row.keys.iter().cloned().zip(p.iter().cloned()).collect();
        probabilities.extend(row.dropped.iter().map(|label| (label.clone(), 0.0)));
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
