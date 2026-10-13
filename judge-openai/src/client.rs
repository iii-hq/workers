//! Atomic OpenAI Decisions calls with worker-wide concurrency and cancellation.
use crate::decisions::{self, Decoded};
use judge_contract::{
    validate_request_with_limits, CancelRequest, CancelResponse, ErrorCode, EvaluateRequest,
    EvaluateResponse, Evaluation, EvaluationResult, ModelsRequest, ModelsResponse, ProviderError,
    RequestOptions, Stats, Usage,
};
use judge_provider::{
    cancellation::CancellationRegistry,
    transport::{self, check_deadline, ExecutionLimits, Failure, RetryPolicy, DEFAULT_RETRY},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::{
    sync::Semaphore,
    task::JoinSet,
    time::{timeout_at, Instant},
};

const ENDPOINT: &str = "https://api.openai.com/v1/decisions";
const CONCURRENCY: usize = 4;

/// Clone or use `with_api_key` for every handler; constructing another client
/// creates another worker transport and concurrency pool. Credentials are never
/// read from evaluation payloads or from the environment during a call.
#[derive(Clone)]
pub struct DecisionsClient {
    http: reqwest::Client,
    endpoint: Arc<str>,
    models_endpoint: Arc<str>,
    api_key: Option<Arc<str>>,
    permits: Arc<Semaphore>,
    limits: ExecutionLimits,
    retry: RetryPolicy,
    calls: Arc<CancellationRegistry>,
    caller_id: Option<Arc<str>>,
}
impl std::fmt::Debug for DecisionsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecisionsClient")
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish_non_exhaustive()
    }
}
struct Accepted {
    id: String,
    decoded: Decoded,
}
struct CallContext {
    deadline: Instant,
    attempts: AtomicUsize,
    unknown_usage: AtomicBool,
    options: RequestOptions,
}
impl CallContext {
    fn new(deadline: Instant, options: RequestOptions) -> Self {
        Self {
            deadline,
            options,
            attempts: AtomicUsize::new(0),
            unknown_usage: AtomicBool::new(false),
        }
    }
}
#[derive(Default)]
struct Outcome {
    model: Option<String>,
    results: BTreeMap<String, EvaluationResult>,
    stats: Stats,
    missing_usage: bool,
}

impl DecisionsClient {
    /// Construct the production client with the credential captured at boot.
    pub fn new(api_key: Option<String>) -> Self {
        Self::with_endpoint(api_key, ENDPOINT.into())
    }

    /// Dependency injection for isolated tests. Never expose this endpoint
    /// through worker configuration or the public evaluation request. The
    /// model catalog is `/v1/models` on the same origin.
    #[doc(hidden)]
    pub fn with_endpoint(api_key: Option<String>, endpoint: String) -> Self {
        let models_endpoint = reqwest::Url::parse(&endpoint)
            .map(|mut url| {
                url.set_path("/v1/models");
                url.set_query(None);
                url.set_fragment(None);
                url.to_string()
            })
            .unwrap_or_default();
        Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .build()
                .expect("OpenAI HTTP client initializes"),
            endpoint: Arc::from(endpoint),
            models_endpoint: Arc::from(models_endpoint),
            api_key: normalized_key(api_key.as_deref()),
            permits: Arc::new(Semaphore::new(CONCURRENCY)),
            limits: ExecutionLimits::default(),
            retry: DEFAULT_RETRY,
            calls: Arc::new(CancellationRegistry::default()),
            caller_id: Some(Arc::from("local")),
        }
    }

    /// Bind a configuration snapshot, sharing the transport and permit pool.
    /// A nonblank configured key takes precedence over this client's boot key.
    pub fn with_api_key(&self, configured: Option<&str>) -> Self {
        Self {
            api_key: normalized_key(configured).or_else(|| self.api_key.clone()),
            ..self.clone()
        }
    }

    /// Bind operator limits without replacing the shared transport/permit pool.
    pub fn with_limits(&self, limits: ExecutionLimits) -> Self {
        Self {
            limits,
            ..self.clone()
        }
    }

    /// Test hook: faster or single-attempt policies. Production keeps `DEFAULT_RETRY`.
    #[doc(hidden)]
    pub fn with_retry(&self, retry: RetryPolicy) -> Self {
        Self {
            retry,
            ..self.clone()
        }
    }

    /// Scope cancellation to the identity supplied by the trusted bus transport.
    pub fn with_caller_id(&self, caller_id: Option<&str>) -> Self {
        Self {
            caller_id: caller_id.map(Arc::from),
            ..self.clone()
        }
    }

    /// Signal a currently active call belonging to this caller. This does not
    /// roll back provider work or claim that a remote request was unbilled.
    pub fn cancel(&self, request: CancelRequest) -> CancelResponse {
        match self
            .calls
            .cancel(self.caller_id.as_deref(), &request.request_id)
        {
            Ok(cancelled) => CancelResponse::Ok { cancelled },
            Err(code) => CancelResponse::Error { code },
        }
    }

    /// Execute a generic batch atomically, one Decisions call per evaluation.
    /// The deadline includes validation, permit waits, headers and complete
    /// bodies; absolute expiry also accounts for time spent queued on the bus
    /// before this function was entered.
    pub async fn evaluate(
        &self,
        request: EvaluateRequest,
        default_model: &str,
    ) -> EvaluateResponse {
        let started = Instant::now();
        let deadline = match transport::deadline(
            self.limits,
            started,
            request.timeout_ms,
            request.expires_at_unix_ms,
        ) {
            Ok(deadline) => deadline,
            Err(code) => {
                return EvaluateResponse::Error {
                    code,
                    http_status: None,
                    provider_error: None,
                    retry_after_ms: None,
                    stats: Stats::default(),
                }
            }
        };
        let call = Arc::new(CallContext::new(deadline, request.options.clone()));
        let mut guard = match self
            .calls
            .start(self.caller_id.as_deref(), request.request_id.as_deref())
        {
            Ok(guard) => guard,
            Err(code) => {
                return EvaluateResponse::Error {
                    code,
                    http_status: None,
                    provider_error: None,
                    retry_after_ms: None,
                    stats: Stats::default(),
                }
            }
        };
        // Replies echo the model; a batch answered without HTTP reports this one.
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| default_model.to_owned());
        let mut outcome = Outcome::default();
        let mut pending = JoinSet::new();
        let result = tokio::select! {
            biased;
            _ = guard.cancelled() => Err(ErrorCode::Cancelled.into()),
            result = timeout_at(deadline, self.evaluate_until(request, &model, &call, &mut pending, &mut outcome)) =>
                result.unwrap_or_else(|_| Err(ErrorCode::Deadline.into())),
        };
        // Drain aborted tasks before reading attempts: a task on another runtime
        // thread must not start HTTP after this call has returned its accounting.
        pending.abort_all();
        while let Some(completed) = pending.join_next().await {
            if let Ok(Ok(accepted)) = completed {
                // Aborting cannot erase completed responses. Keep their known
                // usage when compatible, but never change the original error
                // or collect partial answers during failure cleanup.
                let _ = record_usage(&mut outcome, &accepted.decoded);
            }
        }
        outcome.stats.attempts = call.attempts.load(Ordering::SeqCst);
        outcome.stats.elapsed_ms = started.elapsed().as_millis() as u64;
        outcome.stats.usage_complete =
            result.is_ok() && !outcome.missing_usage && !call.unknown_usage.load(Ordering::SeqCst);
        match result {
            Ok(()) => EvaluateResponse::Ok {
                model: outcome.model.unwrap_or(model),
                results: outcome.results,
                stats: outcome.stats,
            },
            Err(Failure {
                code,
                http_status,
                provider_error,
                retry_after_ms,
            }) => EvaluateResponse::Error {
                code,
                http_status,
                provider_error,
                retry_after_ms,
                stats: outcome.stats,
            },
        }
    }

    async fn evaluate_until(
        &self,
        request: EvaluateRequest,
        model: &str,
        call: &Arc<CallContext>,
        pending: &mut JoinSet<Result<Accepted, Failure>>,
        outcome: &mut Outcome,
    ) -> Result<(), Failure> {
        let deadline = call.deadline;
        check_deadline(deadline)?;
        transport::validate_options(&call.options, self.limits)?;
        let validation = validate_request_with_limits(&request, self.limits.max_request_bytes);
        check_deadline(deadline)?;
        validation?;
        if !crate::SUPPORTED_MODELS.contains(&model) {
            return Err(ErrorCode::InvalidRequest.into());
        }
        // Preflight ALL evaluations before spawning any HTTP work, retaining no
        // bodies: each task re-encodes after acquiring one of the HTTP permits,
        // which bounds live buffers to the worker's concurrency.
        for evaluation in &request.evaluations {
            check_deadline(deadline)?;
            if decisions::remote(evaluation) {
                decisions::encode(model, evaluation, self.limits.max_request_bytes)?;
            }
            // Up to 512 bodies of up to 8 MiB: give cancellation and other
            // tasks a turn between encodings.
            tokio::task::yield_now().await;
        }
        // Also before local answers: a keyless worker fails every request alike.
        self.api_key.as_ref().ok_or(ErrorCode::MissingKey)?;
        let (remote, local): (Vec<_>, Vec<_>) =
            request.evaluations.into_iter().partition(decisions::remote);
        for evaluation in local {
            let answers = decisions::local_answers(&evaluation);
            outcome.stats.questions += answers.len();
            outcome.results.insert(
                evaluation.id,
                EvaluationResult {
                    answers,
                    usage: Some(Usage {
                        input_tokens: Some(0),
                        output_tokens: Some(0),
                    }),
                },
            );
        }
        let model: Arc<str> = Arc::from(model);
        // Single-attempt policies keep unsent work unspawned so one failure
        // aborts the batch after at most a permit's worth of extra requests.
        // Retrying policies spawn everything: backoff releases the HTTP permit,
        // and the rest of the batch should use it instead of waiting in line.
        let window = if self.retry.max_retries == 0 {
            CONCURRENCY
        } else {
            remote.len()
        };
        let mut evaluations = remote.into_iter();
        for evaluation in evaluations.by_ref().take(window) {
            self.spawn(pending, evaluation, model.clone(), call);
        }
        while let Some(result) = pending.join_next().await {
            let accepted = result.map_err(|_| ErrorCode::Transport)??;
            merge(outcome, accepted)?;
            check_deadline(deadline)?;
            if let Some(evaluation) = evaluations.next() {
                self.spawn(pending, evaluation, model.clone(), call);
            }
        }
        check_deadline(deadline)?;
        Ok(())
    }

    fn spawn(
        &self,
        pending: &mut JoinSet<Result<Accepted, Failure>>,
        evaluation: Evaluation,
        model: Arc<str>,
        call: &Arc<CallContext>,
    ) {
        let client = self.clone();
        let call = call.clone();
        pending.spawn(async move { client.send(evaluation, model, call).await });
    }

    async fn send(
        &self,
        evaluation: Evaluation,
        model: Arc<str>,
        call: Arc<CallContext>,
    ) -> Result<Accepted, Failure> {
        let deadline = call.deadline;
        timeout_at(deadline, async {
            let bytes = self
                .send_http(&call, || {
                    let body =
                        decisions::encode(&model, &evaluation, self.limits.max_request_bytes)?;
                    Ok(self
                        .http
                        .post(self.endpoint.as_ref())
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(body))
                })
                .await?;
            let decoded = decisions::decode(&evaluation, &bytes)?;
            Ok(Accepted {
                id: evaluation.id,
                decoded,
            })
        })
        .await
        .unwrap_or_else(|_| Err(ErrorCode::Deadline.into()))
    }

    /// List the supported Decisions models this key can see, using the same
    /// credential snapshot and HTTP permits. Empty when none is visible.
    pub async fn list_models(&self, request: ModelsRequest) -> ModelsResponse {
        let started = Instant::now();
        let deadline = match transport::deadline(
            self.limits,
            started,
            request.timeout_ms,
            request.expires_at_unix_ms,
        ) {
            Ok(deadline) => deadline,
            Err(code) => {
                return ModelsResponse::Error {
                    code,
                    http_status: None,
                    provider_error: None,
                    retry_after_ms: None,
                    stats: Stats::default(),
                }
            }
        };
        let call = CallContext::new(deadline, request.options);
        let mut guard = match self
            .calls
            .start(self.caller_id.as_deref(), request.request_id.as_deref())
        {
            Ok(guard) => guard,
            Err(code) => {
                return ModelsResponse::Error {
                    code,
                    http_status: None,
                    provider_error: None,
                    retry_after_ms: None,
                    stats: Stats::default(),
                }
            }
        };
        let result = tokio::select! {
            biased;
            _ = guard.cancelled() => Err(ErrorCode::Cancelled.into()),
            result = timeout_at(deadline, async {
                transport::validate_options(&call.options, self.limits)?;
                let bytes = self.send_http(&call, || Ok(self.http.get(self.models_endpoint.as_ref()))).await?;
                let models = decisions::model_cards(&bytes)?;
                check_deadline(deadline)?;
                Ok::<_, Failure>(models)
            }) => result.unwrap_or_else(|_| Err(ErrorCode::Deadline.into())),
        };
        // Listing models performs no inference and claims no token usage.
        let stats = Stats {
            attempts: call.attempts.load(Ordering::SeqCst),
            requests: usize::from(result.is_ok()),
            elapsed_ms: started.elapsed().as_millis() as u64,
            usage_complete: result.is_ok(),
            ..Stats::default()
        };
        match result {
            Ok(models) => ModelsResponse::Ok { models, stats },
            Err(Failure {
                code,
                http_status,
                provider_error,
                retry_after_ms,
            }) => ModelsResponse::Error {
                code,
                http_status,
                provider_error,
                retry_after_ms,
                stats,
            },
        }
    }

    async fn send_http(
        &self,
        call: &CallContext,
        build: impl Fn() -> Result<reqwest::RequestBuilder, Failure>,
    ) -> Result<Vec<u8>, Failure> {
        let key = self.api_key.as_deref().ok_or(ErrorCode::MissingKey)?;
        transport::send_http(
            &self.http,
            &self.permits,
            key,
            self.limits,
            &call.options,
            self.retry,
            call.deadline,
            &call.attempts,
            &call.unknown_usage,
            build,
        )
        .await
        .map_err(without_auth_message)
    }
}
fn normalized_key(key: Option<&str>) -> Option<Arc<str>> {
    key.map(str::trim)
        .filter(|key| !key.is_empty())
        .map(Arc::from)
}
/// OpenAI's 401/403 messages can quote a masked fragment of the key: keep only
/// the error's `type` and `code` (none for a non-JSON body).
fn without_auth_message(mut failure: Failure) -> Failure {
    if matches!(failure.http_status, Some(401 | 403)) {
        failure.provider_error = failure.provider_error.map(|error| ProviderError {
            detail: error
                .detail
                .as_ref()
                .and_then(|detail| detail.get("error"))
                .map(|error| json!({"error": {"type": error.get("type"), "code": error.get("code")}})),
            message: None,
            truncated: error.truncated,
        });
    }
    failure
}
fn merge(outcome: &mut Outcome, accepted: Accepted) -> Result<(), ErrorCode> {
    record_usage(outcome, &accepted.decoded)?;
    outcome.results.insert(
        accepted.id,
        EvaluationResult {
            answers: accepted.decoded.answers,
            usage: Some(accepted.decoded.usage),
        },
    );
    Ok(())
}

fn record_usage(outcome: &mut Outcome, decoded: &Decoded) -> Result<(), ErrorCode> {
    if outcome
        .model
        .as_ref()
        .is_some_and(|model| model != &decoded.model)
    {
        return Err(ErrorCode::InvalidResponse);
    }
    let input_tokens = outcome
        .stats
        .input_tokens
        .checked_add(decoded.usage.input_tokens.unwrap_or(0))
        .ok_or(ErrorCode::InvalidResponse)?;
    let output_tokens = outcome
        .stats
        .output_tokens
        .checked_add(decoded.usage.output_tokens.unwrap_or(0))
        .ok_or(ErrorCode::InvalidResponse)?;
    outcome.model.get_or_insert_with(|| decoded.model.clone());
    outcome.stats.requests += 1;
    outcome.stats.questions += decoded.answers.len();
    outcome.stats.input_tokens = input_tokens;
    outcome.stats.output_tokens = output_tokens;
    outcome.missing_usage |=
        decoded.usage.input_tokens.is_none() || decoded.usage.output_tokens.is_none();
    Ok(())
}
