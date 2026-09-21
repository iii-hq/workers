//! Atomic JEV calls with worker-wide concurrency and cancellation.
use crate::{
    cancellation::CancellationRegistry,
    transport::{self, Failure},
};
use jev_contract::{
    encode_evaluation_with_limits, validate_answer, validate_request_with_limits, Answer,
    CancelRequest, CancelResponse, EncodingLimits, ErrorCode, EvaluateRequest, EvaluateResponse,
    Evaluation, EvaluationResult, ModelCard, ModelsRequest, ModelsResponse, RequestOptions, Stats,
    Usage, DEFAULT_MAX_REQUEST_BYTES, DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_MAX_TIMEOUT_MS,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::Semaphore,
    task::JoinSet,
    time::{timeout_at, Instant},
};

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const CONCURRENCY: usize = 4;
/// Independent request/response byte guards and a whole-call deadline ceiling.
#[derive(Clone, Copy, Debug)]
pub struct ExecutionLimits {
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_timeout_ms: u64,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_request_bytes: DEFAULT_MAX_REQUEST_BYTES,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_timeout_ms: DEFAULT_MAX_TIMEOUT_MS,
        }
    }
}

impl ExecutionLimits {
    pub(crate) fn validate(self) -> Result<(), ErrorCode> {
        if self.max_request_bytes == 0
            || self.max_response_bytes == 0
            || self.max_request_bytes > isize::MAX as usize
            || self.max_response_bytes > isize::MAX as usize
            || self.max_timeout_ms == 0
            || i64::try_from(self.max_timeout_ms).is_err()
            || Instant::now()
                .checked_add(Duration::from_millis(self.max_timeout_ms))
                .is_none()
        {
            return Err(ErrorCode::InvalidRequest);
        }
        Ok(())
    }

    fn encoding(self) -> EncodingLimits {
        EncodingLimits {
            max_body_bytes: self.max_request_bytes,
            max_state_question_bytes: None,
        }
    }
}

/// Clone or use `with_api_key` for every handler; constructing another client
/// creates another worker transport and concurrency pool. Credentials are never
/// read from evaluation payloads or from the environment during a call.
#[derive(Clone)]
pub struct JevClient {
    http: reqwest::Client,
    endpoint: Arc<str>,
    models_endpoint: Arc<str>,
    api_key: Option<Arc<str>>,
    permits: Arc<Semaphore>,
    limits: ExecutionLimits,
    calls: Arc<CancellationRegistry>,
    caller_id: Option<Arc<str>>,
}
impl std::fmt::Debug for JevClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JevClient")
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish_non_exhaustive()
    }
}
#[derive(Deserialize)]
struct ProviderResponse {
    model: String,
    #[serde(deserialize_with = "unique_answers")]
    answers: BTreeMap<String, Answer>,
    usage: Usage,
}
#[derive(Deserialize)]
struct ProviderModels {
    models: Vec<ModelCard>,
}
struct Accepted {
    id: String,
    response: ProviderResponse,
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

impl JevClient {
    /// Construct the production client with the credential captured at boot.
    pub fn new(api_key: Option<String>) -> Self {
        Self::with_endpoint(api_key, ENDPOINT.into())
    }

    /// Dependency injection for isolated integration tests. Never expose this
    /// endpoint through worker configuration or the public evaluation request.
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
                .expect("JEV HTTP client initializes"),
            endpoint: Arc::from(endpoint),
            models_endpoint: Arc::from(models_endpoint),
            api_key: normalized_key(api_key.as_deref()),
            permits: Arc::new(Semaphore::new(CONCURRENCY)),
            limits: ExecutionLimits::default(),
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

    /// Execute a generic batch atomically. The deadline includes validation,
    /// permit waits, headers and complete bodies; absolute expiry also accounts
    /// for time spent queued on the bus before this function was entered.
    pub async fn evaluate(
        &self,
        request: EvaluateRequest,
        default_model: &str,
    ) -> EvaluateResponse {
        let started = Instant::now();
        let deadline = match self.deadline(started, request.timeout_ms, request.expires_at_unix_ms)
        {
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
        let mut outcome = Outcome::default();
        let mut pending = JoinSet::new();
        let result = tokio::select! {
            biased;
            _ = guard.cancelled() => Err(ErrorCode::Cancelled.into()),
            result = timeout_at(deadline, self.evaluate_until(request, default_model, &call, &mut pending, &mut outcome)) =>
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
                let _ = record_usage(&mut outcome, &accepted.response);
            }
        }
        outcome.stats.attempts = call.attempts.load(Ordering::SeqCst);
        outcome.stats.elapsed_ms = started.elapsed().as_millis() as u64;
        outcome.stats.usage_complete =
            result.is_ok() && !outcome.missing_usage && !call.unknown_usage.load(Ordering::SeqCst);
        match result {
            Ok(()) => EvaluateResponse::Ok {
                model: outcome.model.expect("nonempty validated batch"),
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
        default_model: &str,
        call: &Arc<CallContext>,
        pending: &mut JoinSet<Result<Accepted, Failure>>,
        outcome: &mut Outcome,
    ) -> Result<(), Failure> {
        let deadline = call.deadline;
        check_deadline(deadline)?;
        transport::validate_options(&call.options, self.limits)?;
        let validation = validate_request_with_limits(&request, self.limits.encoding());
        check_deadline(deadline)?;
        validation?;
        let model = request.model.as_deref().unwrap_or(default_model);
        // Preflight ALL evaluations before spawning any HTTP work.
        for evaluation in &request.evaluations {
            check_deadline(deadline)?;
            let body = encode_evaluation_with_limits(model, evaluation, self.limits.encoding());
            check_deadline(deadline)?;
            // Preflight the whole batch atomically, retaining no encoded bodies.
            // Encoding again after acquiring a permit bounds duplicate buffers
            // to the worker's HTTP concurrency, even for 512 large evaluations.
            drop(body?);
        }
        self.api_key.as_ref().ok_or(ErrorCode::MissingKey)?;
        let model: Arc<str> = Arc::from(model);
        // Backoff releases HTTP permits. Let the rest of a retrying batch use
        // them instead of occupying every scheduling slot with sleeping calls.
        // Validation caps the batch at 512; encoding still occurs after permit
        // acquisition, bounding encoded buffers to the four HTTP slots.
        let initial_window = if call.options.retry.max_retries == 0 {
            CONCURRENCY
        } else {
            request.evaluations.len()
        };
        let mut evaluations = request.evaluations.into_iter();
        for evaluation in evaluations.by_ref().take(initial_window) {
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
                        encode_evaluation_with_limits(&model, &evaluation, self.limits.encoding())?;
                    Ok(self
                        .http
                        .post(self.endpoint.as_ref())
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(body))
                })
                .await?;
            let response: ProviderResponse =
                serde_json::from_slice(&bytes).map_err(|_| ErrorCode::InvalidResponse)?;
            if response.model.trim().is_empty()
                || !evaluation.questions.keys().eq(response.answers.keys())
                || evaluation.questions.iter().any(|(id, question)| {
                    response
                        .answers
                        .get(id)
                        .is_none_or(|answer| validate_answer(question, answer).is_err())
                })
            {
                return Err(ErrorCode::InvalidResponse.into());
            }
            Ok(Accepted {
                id: evaluation.id,
                response,
            })
        })
        .await
        .unwrap_or_else(|_| Err(ErrorCode::Deadline.into()))
    }

    /// List provider models using the same credential snapshot and HTTP permits.
    pub async fn list_models(&self, request: ModelsRequest) -> ModelsResponse {
        let started = Instant::now();
        let deadline = match self.deadline(started, request.timeout_ms, request.expires_at_unix_ms)
        {
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
                let response: ProviderModels = serde_json::from_slice(&bytes).map_err(|_| ErrorCode::InvalidResponse)?;
                check_deadline(deadline)?;
                Ok::<_, Failure>(response.models)
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
        // Absolute expiry wins, while invalid relative timeouts stay validation errors.
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
            call.deadline,
            &call.attempts,
            &call.unknown_usage,
            build,
        )
        .await
    }
}
fn normalized_key(key: Option<&str>) -> Option<Arc<str>> {
    key.map(str::trim)
        .filter(|key| !key.is_empty())
        .map(Arc::from)
}
fn check_deadline(deadline: Instant) -> Result<(), ErrorCode> {
    if Instant::now() >= deadline {
        Err(ErrorCode::Deadline)
    } else {
        Ok(())
    }
}
fn merge(outcome: &mut Outcome, accepted: Accepted) -> Result<(), ErrorCode> {
    let response = accepted.response;
    record_usage(outcome, &response)?;
    outcome.results.insert(
        accepted.id,
        EvaluationResult {
            answers: response.answers,
            usage: Some(response.usage),
        },
    );
    Ok(())
}

fn record_usage(outcome: &mut Outcome, response: &ProviderResponse) -> Result<(), ErrorCode> {
    if outcome
        .model
        .as_ref()
        .is_some_and(|model| model != &response.model)
    {
        return Err(ErrorCode::InvalidResponse);
    }
    let input_tokens = outcome
        .stats
        .input_tokens
        .checked_add(response.usage.input_tokens.unwrap_or(0))
        .ok_or(ErrorCode::InvalidResponse)?;
    let output_tokens = outcome
        .stats
        .output_tokens
        .checked_add(response.usage.output_tokens.unwrap_or(0))
        .ok_or(ErrorCode::InvalidResponse)?;
    outcome.model.get_or_insert_with(|| response.model.clone());
    outcome.stats.requests += 1;
    outcome.stats.questions += response.answers.len();
    outcome.stats.input_tokens = input_tokens;
    outcome.stats.output_tokens = output_tokens;
    outcome.missing_usage |=
        response.usage.input_tokens.is_none() || response.usage.output_tokens.is_none();
    Ok(())
}

fn unique_answers<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, Answer>, D::Error> {
    struct AnswersVisitor;
    impl<'de> serde::de::Visitor<'de> for AnswersVisitor {
        type Value = BTreeMap<String, Answer>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("unique answer IDs")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut answers = BTreeMap::new();
            while let Some((id, answer)) = map.next_entry()? {
                if answers.insert(id, answer).is_some() {
                    return Err(serde::de::Error::custom("duplicate answer ID"));
                }
            }
            Ok(answers)
        }
    }
    deserializer.deserialize_map(AnswersVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::poll;
    use serde_json::{json, Value};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, oneshot},
        time::timeout,
    };

    async fn read_request(stream: &mut TcpStream) -> Value {
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = stream.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0, "request body arrives before EOF");
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                }
            }
        }
    }

    /// Abort must retain already-completed observations. No sleep controls the
    /// race: the parent is polled once, then child completion is proven by the
    /// permits becoming available, one deliberately released response at a time.
    #[tokio::test]
    async fn failure_drain_keeps_completed_compatible_usage_without_replacing_error() {
        check_failure_drain(false).await;
    }

    #[tokio::test]
    async fn failure_drain_keeps_partial_usage_without_replacing_error() {
        check_failure_drain(true).await;
    }

    async fn check_failure_drain(partial_usage: bool) {
        timeout(Duration::from_secs(5), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let (ready_tx, mut ready_rx) = mpsc::unbounded_channel();
            let server = tokio::spawn(async move {
                let mut connections = JoinSet::new();
                for _ in 0..4 {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let ready_tx = ready_tx.clone();
                    connections.spawn(async move {
                        let request = read_request(&mut stream).await;
                        let index = request["state"]["index"].as_u64().unwrap() as usize;
                        let (release, reply) = oneshot::channel::<(u16, String)>();
                        ready_tx.send((index, release)).unwrap();
                        let (status, body) = reply.await.unwrap();
                        stream.write_all(format!("HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                    });
                }
                while let Some(result) = connections.join_next().await { result.unwrap(); }
            });
            let request: EvaluateRequest = serde_json::from_value(json!({
                "timeout_ms": 3000,
                "options": {"retry":{"max_retries":0}},
                "evaluations": (0..4).map(|index| json!({
                    "id": format!("ticket-{index}"), "state": {"index": index},
                    "questions": {"urgent": {"type": "noul", "instructions": "Is this urgent?"}}
                })).collect::<Vec<_>>()
            })).unwrap();
            let client = JevClient::with_endpoint(Some("test-credential".into()), endpoint);
            let evaluation = client.evaluate(request, jev_contract::DEFAULT_MODEL);
            tokio::pin!(evaluation);
            assert!(poll!(evaluation.as_mut()).is_pending());
            // No further evaluation poll occurs until all four child tasks
            // have completed and queued their outcomes in the JoinSet.
            let mut responses = BTreeMap::new();
            for _ in 0..4 {
                let (index, release) = ready_rx.recv().await.unwrap();
                responses.insert(index, release);
            }
            for index in 0..4 {
                let (status, model, input_tokens) = match index {
                    0 => (500, "jev-1.13.0", 0),
                    1 => (200, "jev-1.13.0", 11),
                    2 => (200, "incompatible-model", 100),
                    _ => (200, "jev-1.13.0", u64::MAX),
                };
                let output_tokens = if partial_usage && index == 1 { None } else { Some(3) };
                let body = json!({"model": model, "answers": {"urgent": {"type": "noul", "noul": 0.75}}, "usage": {"input_tokens": input_tokens, "output_tokens": output_tokens}}).to_string();
                responses.remove(&index).unwrap().send((status, body)).unwrap();
                // On a current-thread runtime the child's send future and its
                // owning task complete in the same poll that releases a permit.
                let completed = client.permits.acquire_many(index as u32 + 1).await.unwrap();
                drop(completed);
            }
            let response = evaluation.await;
            let wire = serde_json::to_value(&response).unwrap();
            assert!(wire.get("results").is_none());
            let EvaluateResponse::Error { code, http_status, stats, .. } = response else { panic!("partial answers must not become success") };
            assert_eq!(code, ErrorCode::Http);
            assert_eq!(http_status, Some(500));
            assert_eq!(stats.attempts, 4);
            assert_eq!((stats.requests, stats.questions, stats.input_tokens, stats.output_tokens), (1, 1, 11, if partial_usage { 0 } else { 3 }));
            assert!(!stats.usage_complete);
            server.await.unwrap();
        }).await.expect("controlled child completions finish");
    }
}
