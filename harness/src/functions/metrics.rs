//! `harness::metrics` — aggregate durable model usage and function outcomes
//! plus trace/span observability over a root-and-descendant session tree.
//! `complete=false` means the returned aggregates are a live partial snapshot.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

use futures_util::future::BoxFuture;
use futures_util::stream::{self, StreamExt};
use iii_sdk::protocol::TriggerRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::clients::session::{LoadedEntry, SessionClient};
use crate::context_snapshot::ContextSnapshotV1;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::content::ContentBlock;
use crate::types::message::AgentMessage;

use super::session_tree::{self, SessionTreeNodeV1};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetricsRequestV1 {
    /// Any session in the tree. `session_id` is accepted as an alias, and a
    /// child id is walked up to its root, so the id that addresses
    /// `harness::status` addresses the tree here too.
    #[serde(alias = "session_id")]
    pub root_session_id: String,
    /// Leave each session's context snapshot out of `by_session` (and skip
    /// reading it). For callers that only need the counters and cost.
    #[serde(default)]
    pub omit_context: bool,
    /// Report the given session and its descendants only, instead of walking
    /// up to the root first: the response's root is the given session (depth
    /// 0), which keeps its own `parent_session_id`. For a caller that shows
    /// one conversation, not the whole tree it belongs to.
    #[serde(default)]
    pub subtree: bool,
    /// Skip the trace/span aggregation (`traces` stays absent). It scans the
    /// engine's trace store and is the slowest part of a complete answer;
    /// usage counters do not need it.
    #[serde(default)]
    pub omit_traces: bool,
}

// Schemars 0.8 does not publish serde aliases. Advertise both spellings and
// match serde's rejection of a request that supplies both.
impl JsonSchema for SessionMetricsRequestV1 {
    fn schema_name() -> String {
        "SessionMetricsRequestV1".into()
    }

    fn json_schema(generator: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, ObjectValidation, SchemaObject, SubschemaValidation};

        let mut session_id = String::json_schema(generator).into_object();
        session_id.metadata().description = Some(
            "Any session in the tree. `session_id` is accepted as an alias, and a child id is \
             walked up to its root, so the id that addresses `harness::status` addresses the \
             tree here too."
                .into(),
        );
        let mut omit_context = bool::json_schema(generator).into_object();
        omit_context.metadata().description = Some(
            "Leave each session's context snapshot out of `by_session` (and skip reading it). \
             For callers that only need the counters and cost. Default false."
                .into(),
        );
        let mut subtree = bool::json_schema(generator).into_object();
        subtree.metadata().description = Some(
            "Report the given session and its descendants only, instead of walking up to the \
             root first: the response's root is the given session (depth 0), which keeps its \
             own `parent_session_id`. Default false."
                .into(),
        );
        let mut omit_traces = bool::json_schema(generator).into_object();
        omit_traces.metadata().description = Some(
            "Skip the trace/span aggregation (`traces` stays absent). It scans the engine's \
             trace store and is the slowest part of a complete answer. Default false."
                .into(),
        );
        SchemaObject {
            instance_type: Some(InstanceType::Object.into()),
            object: Some(Box::new(ObjectValidation {
                properties: [
                    ("root_session_id".into(), session_id.clone().into()),
                    ("session_id".into(), session_id.into()),
                    ("omit_context".into(), omit_context.into()),
                    ("subtree".into(), subtree.into()),
                    ("omit_traces".into(), omit_traces.into()),
                ]
                .into(),
                ..Default::default()
            })),
            subschemas: Some(Box::new(SubschemaValidation {
                one_of: Some(
                    ["root_session_id", "session_id"]
                        .into_iter()
                        .map(|field| {
                            SchemaObject {
                                object: Some(Box::new(ObjectValidation {
                                    required: [field.into()].into(),
                                    ..Default::default()
                                })),
                                ..Default::default()
                            }
                            .into()
                        })
                        .collect(),
                ),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionMetricsResponseV1 {
    pub root_session_id: String,
    pub complete: bool,
    pub totals: SessionUsageTotalsV1,
    pub by_session: Vec<SessionUsageV1>,
    /// Trace/span aggregates when the engine's in-memory observability exporter
    /// is available. Usage metrics remain available when it is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traces: Option<SessionTraceMetricsV1>,
}

// `schema_with` preserves nullability: schemars 0.8 `required` alone turns
// Option<T> into a non-nullable T schema. Usage counters need both contracts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionUsageTotalsV1 {
    pub sessions: u64,
    pub turns: u64,
    pub function_calls: u64,
    pub function_call_errors: u64,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<u64>::json_schema")]
    pub input_tokens: Option<u64>,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<u64>::json_schema")]
    pub output_tokens: Option<u64>,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<u64>::json_schema")]
    pub cache_read_tokens: Option<u64>,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<u64>::json_schema")]
    pub cache_write_tokens: Option<u64>,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<u64>::json_schema")]
    pub reasoning_tokens: Option<u64>,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<f64>::json_schema")]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionUsageV1 {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub depth: u32,
    pub turns: u64,
    pub function_calls: u64,
    pub function_call_errors: u64,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<u64>::json_schema")]
    pub input_tokens: Option<u64>,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<u64>::json_schema")]
    pub output_tokens: Option<u64>,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<u64>::json_schema")]
    pub cache_read_tokens: Option<u64>,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<u64>::json_schema")]
    pub cache_write_tokens: Option<u64>,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<u64>::json_schema")]
    pub reasoning_tokens: Option<u64>,
    /// Summed over every generation that reported it. A single generation
    /// without the counter (a stream the provider cut before usage came back)
    /// is left out of the sum. `null` means no generation reported it or more
    /// than one did not — never a measured zero, so the key is always present
    /// and a reader can tell the two apart.
    #[schemars(required, schema_with = "Option::<f64>::json_schema")]
    pub cost_usd: Option<f64>,
    /// The session's latest per-generation context snapshot (categories,
    /// budget, usage) — absent for sessions that have not generated since
    /// snapshots landed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<ContextSnapshotV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionTraceMetricsV1 {
    /// Distinct traces across the root session and all descendants.
    pub trace_count: u64,
    pub span_count: u64,
    pub error_span_count: u64,
    /// Elapsed window from the first observed span to the last observed span.
    pub duration_ms: u64,
    pub by_session: Vec<SessionTraceUsageV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionTraceUsageV1 {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub depth: u32,
    pub trace_count: u64,
    pub span_count: u64,
    pub error_span_count: u64,
    /// Elapsed window from the session's first observed span to its last.
    pub duration_ms: u64,
}

#[derive(Debug, Deserialize)]
struct TraceGroupByResponse {
    groups: Vec<TraceGroup>,
}

#[derive(Debug, Deserialize)]
struct TraceGroup {
    value: String,
    trace_ids: Vec<String>,
    span_count: u64,
    first_seen_ms: u64,
    last_seen_ms: u64,
    duration_ms: u64,
    error_count: u64,
}

/// Sessions read at once. Each is several round trips (metadata, turn
/// record, wake bindings, transcript pages); read one after another, a
/// 176-session tree spent ~21 s on them, most of it between calls rather
/// than inside them.
const SESSION_READ_CONCURRENCY: usize = 16;

/// What one session contributes to the response.
struct SessionRead {
    usage: UsageAccumulator,
    context: Option<ContextSnapshotV1>,
    /// The turn record carried no snapshot: read the durable row at the end.
    read_back: bool,
    /// The latest turn is terminal and nothing will wake the session.
    settled: bool,
}

/// Usage of a session as of its last entry, kept while the session is
/// settled: a settled session's transcript only grows past that entry (a new
/// turn appends), so the next read asks for what came after it instead of
/// re-reading every message. On a 179-session tree the transcripts are over
/// 100 MB of JSON, nearly all of it content the counters never look at.
#[derive(Debug, Clone)]
struct CachedUsage {
    last_entry_id: String,
    usage: UsageAccumulator,
}

// ponytail: in-process and unbounded, one small entry per settled session
// read (a few hundred bytes); a restart starts it empty. Make it durable or
// bounded if harness ever serves enough sessions for that to matter.
#[derive(Debug, Default)]
struct UsageCache {
    sessions: Mutex<HashMap<String, CachedUsage>>,
}

impl UsageCache {
    /// The cached usage, for a session that is settled now.
    fn settled(&self, session_id: &str, settled: bool) -> Option<CachedUsage> {
        if !settled {
            return None;
        }
        self.sessions.lock().ok()?.get(session_id).cloned()
    }

    /// Keep a settled session's usage; forget one that is running again,
    /// whose latest messages can still change (usage is stamped after a
    /// step).
    fn remember(
        &self,
        session_id: &str,
        settled: bool,
        last_entry_id: Option<String>,
        usage: &UsageAccumulator,
    ) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        match last_entry_id.filter(|_| settled) {
            Some(last_entry_id) => {
                sessions.insert(
                    session_id.to_string(),
                    CachedUsage {
                        last_entry_id,
                        usage: usage.clone(),
                    },
                );
            }
            None => {
                sessions.remove(session_id);
            }
        }
    }
}

fn usage_cache() -> &'static UsageCache {
    static CACHE: OnceLock<UsageCache> = OnceLock::new();
    CACHE.get_or_init(UsageCache::default)
}

/// Observe `entries` on top of `usage`; the last entry read becomes the new
/// watermark (unchanged when nothing new came).
fn fold_entries(
    mut usage: UsageAccumulator,
    mut last_entry_id: Option<String>,
    entries: Vec<LoadedEntry>,
) -> (UsageAccumulator, Option<String>) {
    for entry in entries {
        if let Some(message) = entry.message.as_ref() {
            usage.observe(message);
        }
        last_entry_id = Some(entry.entry_id);
    }
    (usage, last_entry_id)
}

/// One session's usage and lifecycle; `None` when the session is gone.
///
/// Boxed and fed owned values: run concurrently from a stream, the unboxed
/// future's type defeats rustc's `Send` check at the spawn sites that await
/// this handler ("implementation of `Send` is not general enough").
fn read_session(
    deps: &Deps,
    session: SessionClient,
    timeout_ms: u64,
    session_id: String,
    omit_context: bool,
    expects_wake: bool,
) -> BoxFuture<'_, Result<Option<SessionRead>, HarnessError>> {
    Box::pin(async move {
        let session_id = session_id.as_str();
        if !session.exists(session_id).await? {
            return Ok(None);
        }
        let mut read = SessionRead {
            usage: UsageAccumulator::default(),
            context: None,
            read_back: false,
            settled: false,
        };
        if let Some(turn) =
            crate::state::get_turn_unhydrated(&deps.iii, session_id, timeout_ms).await?
        {
            let (snapshot, read_back) =
                session_context(turn.context_snapshot.clone(), omit_context);
            read.context = snapshot;
            read.read_back = read_back;
            read.settled = terminal_snapshot(Some(turn.status), expects_wake);
        }
        // No turn record: the transcript is durable independently of it. Keep
        // aggregating it; the answer is partial (`settled` stays false) and the
        // lifecycle/context data unavailable.
        let cache = usage_cache();
        let (usage, last_entry_id) = match cache.settled(session_id, read.settled) {
            // Settled when last read: only what came after can be new.
            Some(hit) => match session
                .messages_from(session_id, Some(&hit.last_entry_id))
                .await
            {
                Ok(entries) => fold_entries(hit.usage, Some(hit.last_entry_id), entries),
                // A fork moved the path (`invalid_cursor`), or the read
                // failed: read the whole transcript.
                Err(_) => fold_entries(
                    UsageAccumulator::default(),
                    None,
                    session.messages_strict(session_id).await?,
                ),
            },
            None => fold_entries(
                UsageAccumulator::default(),
                None,
                session.messages_strict(session_id).await?,
            ),
        };
        cache.remember(session_id, read.settled, last_entry_id, &usage);
        read.usage = usage;
        Ok(Some(read))
    })
}

pub async fn handle(
    deps: &Deps,
    req: SessionMetricsRequestV1,
) -> Result<SessionMetricsResponseV1, HarnessError> {
    let session = deps.session().await;
    let root_session_id = if req.subtree {
        req.root_session_id.clone()
    } else {
        resolve_root(deps, &req.root_session_id).await?
    };
    let mut tree = session_tree::collect(deps, &root_session_id).await?;
    if req.subtree {
        // The tree is cut below the given session, not detached from its
        // parent: its row keeps the link a caller shows as "spawned by".
        if let Some(start) = tree.sessions.first_mut() {
            start.parent_session_id =
                session
                    .metadata_of(&root_session_id)
                    .await?
                    .and_then(|metadata| {
                        metadata
                            .get("parent_session_id")
                            .and_then(serde_json::Value::as_str)
                            .filter(|parent| !parent.is_empty())
                            .map(str::to_string)
                    });
        }
    }
    let cfg = deps.cfg().await;
    let mut complete = tree.complete;
    let mut total = UsageAccumulator::default();
    let mut by_session = Vec::with_capacity(tree.sessions.len());
    let mut missing_context = Vec::new();
    // Read concurrently, consumed in tree order: `buffered` keeps the order,
    // so the response is the one the sequential walk produced.
    let timeout_ms = cfg.session_timeout_ms;
    let omit_context = req.omit_context;
    // The binding store is listed once for the whole tree: asked per session
    // (`bindings::session_expects_wake`), it was listed again for each one.
    // Same rule, failing closed the same way: an unreadable store counts every
    // session as parked, so the answer is never reported complete.
    let now = AgentMessage::now_ms();
    let parked: Option<BTreeSet<String>> =
        deps.bindings().await.list().await.ok().map(|bindings| {
            bindings
                .iter()
                .filter(|binding| crate::bindings::is_armed_wake(binding, now))
                .map(|binding| binding.owner.session_id.clone())
                .collect()
        });
    let reads: Vec<_> = tree
        .sessions
        .iter()
        .map(|node| {
            let expects_wake = parked
                .as_ref()
                .is_none_or(|parked| parked.contains(&node.session_id));
            read_session(
                deps,
                session.clone(),
                timeout_ms,
                node.session_id.clone(),
                omit_context,
                expects_wake,
            )
        })
        .collect();
    let reads = stream::iter(reads)
        .buffered(SESSION_READ_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    for (node, read) in tree.sessions.iter().zip(reads) {
        let Some(read) = read? else {
            complete = false;
            continue;
        };
        // The turn record usually carries the session's latest snapshot for
        // free. A freshly seeded turn has not generated yet, so its record has
        // none and the durable `harness_context` row is the fallback — noted
        // here, read after the walk.
        if read.read_back {
            missing_context.push(by_session.len());
        }
        if !read.settled {
            complete = false;
        }
        total.merge(&read.usage);
        by_session.push(read.usage.finish_session(node, read.context));
    }
    // One state read per session whose record carried no snapshot — collected
    // for the final response only, so a large tree does not pay a round trip
    // per session on every poll (see the note below on partial responses).
    if complete && !missing_context.is_empty() {
        let reads: Vec<BoxFuture<'_, _>> = missing_context
            .iter()
            .map(|&index| {
                let session_id = by_session[index].session_id.clone();
                let read: BoxFuture<'_, _> = Box::pin(async move {
                    crate::context_snapshot::get(&deps.iii, &session_id, timeout_ms).await
                });
                read
            })
            .collect();
        let snapshots = stream::iter(reads)
            .buffered(SESSION_READ_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        for (&index, snapshot) in missing_context.iter().zip(snapshots) {
            if let Ok(snapshot) = snapshot {
                by_session[index].context = snapshot;
            }
        }
    }
    // Partial snapshots are polled as progress signals. Trace aggregation is
    // comparatively expensive and does not help the watchdog decide whether
    // work advanced, so collect it only for the final complete response, and
    // never for a caller that asked to skip it.
    let traces = if complete && !req.omit_traces {
        collect_trace_metrics(deps, &tree.sessions, &root_session_id).await
    } else {
        None
    };

    Ok(SessionMetricsResponseV1 {
        root_session_id,
        complete,
        totals: total.finish_totals(tree.sessions.len() as u64),
        by_session,
        traces,
    })
}

/// The context snapshot a session reports, and whether the durable
/// `harness_context` row has to be read back for it: nothing, and no read,
/// when the caller asked to omit context.
fn session_context<T>(snapshot: Option<T>, omit: bool) -> (Option<T>, bool) {
    if omit {
        (None, false)
    } else {
        let read_back = snapshot.is_none();
        (snapshot, read_back)
    }
}

/// Walk `session_id` up to the root of its tree. Every other `harness::*`
/// function takes the session a caller is holding, so this one accepts it too
/// and reports the whole tree that session belongs to rather than refusing the
/// id. A root (or an orphan) walks zero steps.
async fn resolve_root(deps: &Deps, session_id: &str) -> Result<String, HarnessError> {
    let session = deps.session().await;
    resolve_root_with_metadata(session_id, |id| {
        let session = session.clone();
        async move { session.metadata_of(&id).await }
    })
    .await
}

/// Resolve the parent chain using the supplied durable metadata reader.
async fn resolve_root_with_metadata<F, Fut>(
    session_id: &str,
    mut metadata_of: F,
) -> Result<String, HarnessError>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<
        Output = Result<Option<serde_json::Map<String, serde_json::Value>>, HarnessError>,
    >,
{
    /// Deeper than any legitimate sub-agent chain; a cycle in the durable
    /// metadata must end as an error, never as an unbounded walk.
    const MAX_ANCESTORS: usize = 64;
    let mut current = session_id.to_string();
    let mut seen = BTreeSet::from([current.clone()]);
    for _ in 0..=MAX_ANCESTORS {
        let Some(metadata) = metadata_of(current.clone()).await? else {
            return Ok(current);
        };
        let Some(parent) = metadata
            .get("parent_session_id")
            .and_then(serde_json::Value::as_str)
            .filter(|parent| !parent.is_empty())
        else {
            return Ok(current);
        };
        if !seen.insert(parent.to_string()) {
            return Err(HarnessError::InvalidRequest(format!(
                "session `{session_id}` has a cyclic parent chain at `{parent}`;                  pass the root session id"
            )));
        }
        current = parent.to_string();
    }
    Err(HarnessError::InvalidRequest(format!(
        "session `{session_id}` has more than {MAX_ANCESTORS} ancestors;          pass the root session id"
    )))
}

fn terminal_status(status: Option<crate::types::turn::TurnStatus>) -> bool {
    status.is_some_and(crate::types::turn::TurnStatus::is_terminal)
}

fn terminal_snapshot(status: Option<crate::types::turn::TurnStatus>, expects_wake: bool) -> bool {
    terminal_status(status) && !expects_wake
}

async fn collect_trace_metrics(
    deps: &Deps,
    sessions: &[SessionTreeNodeV1],
    root_session_id: &str,
) -> Option<SessionTraceMetricsV1> {
    let cfg = deps.cfg().await;
    let response = deps
        .iii
        .trigger(TriggerRequest {
            function_id: "engine::traces::group_by".into(),
            payload: json!({
                "attribute": "iii.session.id",
                "limit": u32::MAX,
                "include_internal": true,
            }),
            action: None,
            timeout_ms: Some(cfg.dispatch_timeout_ms),
        })
        .await;
    let value = match response {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                %root_session_id,
                %error,
                "harness::metrics: trace metrics unavailable"
            );
            return None;
        }
    };
    let response: TraceGroupByResponse = match serde_json::from_value(value) {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                %root_session_id,
                %error,
                "harness::metrics: invalid trace metrics response"
            );
            return None;
        }
    };
    Some(summarize_trace_groups(sessions, response.groups))
}

fn summarize_trace_groups(
    sessions: &[SessionTreeNodeV1],
    groups: Vec<TraceGroup>,
) -> SessionTraceMetricsV1 {
    let mut groups: HashMap<_, _> = groups
        .into_iter()
        .map(|group| (group.value.clone(), group))
        .collect();
    let mut trace_ids = BTreeSet::new();
    let mut span_count = 0u64;
    let mut error_span_count = 0u64;
    let mut first_seen_ms = None::<u64>;
    let mut last_seen_ms = None::<u64>;
    let mut by_session = Vec::with_capacity(sessions.len());

    for session in sessions {
        let group = groups.remove(&session.session_id);
        let (session_trace_count, session_span_count, session_errors, duration_ms) = match group {
            Some(group) => {
                trace_ids.extend(group.trace_ids.iter().cloned());
                span_count = span_count.saturating_add(group.span_count);
                error_span_count = error_span_count.saturating_add(group.error_count);
                first_seen_ms = Some(first_seen_ms.map_or(group.first_seen_ms, |current| {
                    current.min(group.first_seen_ms)
                }));
                last_seen_ms = Some(last_seen_ms.map_or(group.last_seen_ms, |current| {
                    current.max(group.last_seen_ms)
                }));
                (
                    group.trace_ids.len() as u64,
                    group.span_count,
                    group.error_count,
                    group.duration_ms,
                )
            }
            None => (0, 0, 0, 0),
        };
        by_session.push(SessionTraceUsageV1 {
            session_id: session.session_id.clone(),
            parent_session_id: session.parent_session_id.clone(),
            depth: session.depth,
            trace_count: session_trace_count,
            span_count: session_span_count,
            error_span_count: session_errors,
            duration_ms,
        });
    }

    SessionTraceMetricsV1 {
        trace_count: trace_ids.len() as u64,
        span_count,
        error_span_count,
        duration_ms: match (first_seen_ms, last_seen_ms) {
            (Some(first), Some(last)) => last.saturating_sub(first),
            _ => 0,
        },
        by_session,
    }
}

#[derive(Debug, Clone, Default)]
struct UsageAccumulator {
    turns: u64,
    function_calls: u64,
    function_call_errors: u64,
    input: OptionalU64Sum,
    output: OptionalU64Sum,
    cache_read: OptionalU64Sum,
    cache_write: OptionalU64Sum,
    reasoning: OptionalU64Sum,
    cost_usd: OptionalF64Sum,
}

impl UsageAccumulator {
    /// Fold another accumulator in: the same as having observed its messages
    /// here (every field is a sum, so the order does not matter).
    fn merge(&mut self, other: &UsageAccumulator) {
        self.turns += other.turns;
        self.function_calls += other.function_calls;
        self.function_call_errors += other.function_call_errors;
        self.input.merge(&other.input);
        self.output.merge(&other.output);
        self.cache_read.merge(&other.cache_read);
        self.cache_write.merge(&other.cache_write);
        self.reasoning.merge(&other.reasoning);
        self.cost_usd.merge(&other.cost_usd);
    }

    fn observe(&mut self, message: &AgentMessage) {
        match message {
            AgentMessage::Assistant(assistant) => {
                self.turns += 1;
                self.function_calls += assistant
                    .content
                    .iter()
                    .filter(|block| matches!(block, ContentBlock::FunctionCall { .. }))
                    .count() as u64;
                let usage = assistant.usage.as_ref();
                self.input.observe(usage.and_then(|value| value.input));
                self.output.observe(usage.and_then(|value| value.output));
                self.cache_read
                    .observe(usage.and_then(|value| value.cache_read));
                self.cache_write
                    .observe(usage.and_then(|value| value.cache_write));
                self.reasoning
                    .observe(usage.and_then(|value| value.reasoning));
                self.cost_usd
                    .observe(usage.and_then(|value| value.cost_usd));
            }
            AgentMessage::FunctionResult(result) if result.is_error => {
                self.function_call_errors += 1;
            }
            _ => {}
        }
    }

    fn finish_session(
        self,
        node: &SessionTreeNodeV1,
        context: Option<ContextSnapshotV1>,
    ) -> SessionUsageV1 {
        SessionUsageV1 {
            session_id: node.session_id.clone(),
            parent_session_id: node.parent_session_id.clone(),
            depth: node.depth,
            turns: self.turns,
            function_calls: self.function_calls,
            function_call_errors: self.function_call_errors,
            input_tokens: self.input.finish(),
            output_tokens: self.output.finish(),
            cache_read_tokens: self.cache_read.finish(),
            cache_write_tokens: self.cache_write.finish(),
            reasoning_tokens: self.reasoning.finish(),
            cost_usd: self.cost_usd.finish(),
            context,
        }
    }

    fn finish_totals(self, sessions: u64) -> SessionUsageTotalsV1 {
        SessionUsageTotalsV1 {
            sessions,
            turns: self.turns,
            function_calls: self.function_calls,
            function_call_errors: self.function_call_errors,
            input_tokens: self.input.finish(),
            output_tokens: self.output.finish(),
            cache_read_tokens: self.cache_read.finish(),
            cache_write_tokens: self.cache_write.finish(),
            reasoning_tokens: self.reasoning.finish(),
            cost_usd: self.cost_usd.finish(),
        }
    }
}

/// Generations a counter may miss and still be summed. A provider that fails
/// mid-stream leaves one partial generation without usage; the rest of the
/// session still measures it. More than one missing is a gap, not a sum.
const UNREPORTED_GENERATIONS_TOLERATED: u64 = 1;

#[derive(Debug, Clone, Default)]
struct OptionalU64Sum {
    value: u64,
    reported: u64,
    unreported: u64,
}

impl OptionalU64Sum {
    fn merge(&mut self, other: &Self) {
        self.value = self.value.saturating_add(other.value);
        self.reported += other.reported;
        self.unreported += other.unreported;
    }

    fn observe(&mut self, value: Option<u64>) {
        match value {
            Some(value) => {
                self.value = self.value.saturating_add(value);
                self.reported += 1;
            }
            None => self.unreported += 1,
        }
    }

    fn finish(self) -> Option<u64> {
        (self.reported > 0 && self.unreported <= UNREPORTED_GENERATIONS_TOLERATED)
            .then_some(self.value)
    }
}

#[derive(Debug, Clone, Default)]
struct OptionalF64Sum {
    value: f64,
    reported: u64,
    unreported: u64,
}

impl OptionalF64Sum {
    fn merge(&mut self, other: &Self) {
        self.value += other.value;
        self.reported += other.reported;
        self.unreported += other.unreported;
    }

    fn observe(&mut self, value: Option<f64>) {
        match value {
            Some(value) => {
                self.value += value;
                self.reported += 1;
            }
            None => self.unreported += 1,
        }
    }

    fn finish(self) -> Option<f64> {
        (self.reported > 0 && self.unreported <= UNREPORTED_GENERATIONS_TOLERATED)
            .then_some(self.value)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::types::turn::TurnStatus;

    fn message(value: serde_json::Value) -> AgentMessage {
        serde_json::from_value(value).unwrap()
    }

    fn session(id: &str, parent: Option<&str>, depth: u32) -> SessionTreeNodeV1 {
        SessionTreeNodeV1 {
            session_id: id.into(),
            parent_session_id: parent.map(str::to_string),
            parent_turn_id: None,
            depth,
        }
    }

    #[tokio::test]
    async fn root_resolution_accepts_64_parent_links_and_rejects_65() {
        for links in [0, 1, 63, 64, 65] {
            let result = resolve_root_with_metadata(&links.to_string(), |id| {
                let index: u32 = id.parse().unwrap();
                let metadata = match index.checked_sub(1) {
                    Some(parent) => json!({"parent_session_id": parent.to_string()}),
                    None => json!({}),
                };
                std::future::ready(Ok(Some(metadata.as_object().unwrap().clone())))
            })
            .await;
            if links <= 64 {
                assert_eq!(result.unwrap(), "0", "{links} parent links");
            } else {
                assert!(
                    matches!(result, Err(HarnessError::InvalidRequest(ref message))
                        if message.contains("more than 64 ancestors")),
                    "{result:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn root_resolution_rejects_cyclic_parent_chains() {
        for links in [1, 2, 64] {
            let result = resolve_root_with_metadata("0", |id| {
                let index: u32 = id.parse().unwrap();
                let parent = (index + 1) % links;
                let metadata = json!({"parent_session_id": parent.to_string()});
                std::future::ready(Ok(Some(metadata.as_object().unwrap().clone())))
            })
            .await;
            assert!(
                matches!(result, Err(HarnessError::InvalidRequest(ref message))
                    if message.contains("cyclic parent chain")),
                "{result:?}"
            );
        }
    }

    #[tokio::test]
    async fn root_resolution_preserves_orphans_and_metadata_errors() {
        assert_eq!(
            resolve_root_with_metadata("orphan", |_| std::future::ready(Ok(None)))
                .await
                .unwrap(),
            "orphan"
        );
        let error = HarnessError::Dependency("session::get unavailable".into());
        assert_eq!(
            resolve_root_with_metadata("s_child", |_| std::future::ready(Err(error.clone()))).await,
            Err(error)
        );
    }

    #[test]
    fn counts_generations_calls_errors_and_usage() {
        let mut usage = UsageAccumulator::default();
        usage.observe(&message(json!({
            "role": "assistant",
            "content": [{
                "type": "function_call",
                "id": "call-1",
                "function_id": "orders::refund",
                "arguments": {"order_id": 4512}
            }],
            "stop_reason": "function_call",
            "usage": {"input": 10, "output": 2, "cost_usd": 0.1},
            "model": "model",
            "provider": "provider",
            "timestamp": 1
        })));
        usage.observe(&message(json!({
            "role": "function_result",
            "function_call_id": "call-1",
            "function_id": "orders::refund",
            "content": [],
            "details": {},
            "is_error": true,
            "timestamp": 2
        })));
        let totals = usage.finish_totals(1);
        assert_eq!(totals.sessions, 1);
        assert_eq!(totals.turns, 1);
        assert_eq!(totals.function_calls, 1);
        assert_eq!(totals.function_call_errors, 1);
        assert_eq!(totals.input_tokens, Some(10));
        assert_eq!(totals.output_tokens, Some(2));
        assert_eq!(totals.cost_usd, Some(0.1));
        assert_eq!(totals.cache_read_tokens, None);
    }

    fn generation(usage: serde_json::Value) -> AgentMessage {
        message(json!({
            "role": "assistant",
            "content": [],
            "stop_reason": "end",
            "usage": usage,
            "model": "model",
            "provider": "provider",
            "timestamp": 1
        }))
    }

    #[test]
    fn one_generation_cut_before_usage_is_left_out_of_the_sum() {
        let mut usage = UsageAccumulator::default();
        usage.observe(&generation(
            json!({"input": 10, "output": 1, "cost_usd": 0.5}),
        ));
        // A stream the provider cut mid-response persists without any usage.
        usage.observe(&generation(serde_json::Value::Null));
        usage.observe(&generation(
            json!({"input": 5, "output": 2, "cost_usd": 0.25}),
        ));
        let totals = usage.finish_totals(1);
        assert_eq!(totals.turns, 3);
        assert_eq!(totals.input_tokens, Some(15));
        assert_eq!(totals.output_tokens, Some(3));
        assert_eq!(totals.cost_usd, Some(0.75));
        // A counter the provider never reports stays unreported.
        assert_eq!(totals.cache_read_tokens, None);
    }

    #[test]
    fn more_than_one_unreported_generation_is_a_gap_not_a_sum() {
        let mut usage = UsageAccumulator::default();
        for input in [Some(10), None, None] {
            usage.observe(&generation(json!({"input": input, "output": 1})));
        }
        let totals = usage.finish_totals(1);
        assert_eq!(totals.turns, 3);
        assert_eq!(totals.input_tokens, None);
        assert_eq!(totals.output_tokens, Some(3));
    }

    #[test]
    fn merged_sessions_total_what_one_walk_over_all_messages_totals() {
        // Sessions are read concurrently and folded into the total, where the
        // sequential walk observed every message into it directly. The two
        // must agree, the one-unreported-generation tolerance included: one
        // gap in each of two sessions is two gaps in the tree.
        let sessions = [
            vec![
                generation(json!({"input": 10, "output": 1, "cost_usd": 0.5})),
                generation(serde_json::Value::Null),
            ],
            vec![
                generation(json!({"input": 5, "output": 2, "cost_usd": 0.25})),
                generation(json!({"output": 3})),
            ],
        ];
        let mut walked = UsageAccumulator::default();
        let mut merged = UsageAccumulator::default();
        for messages in &sessions {
            let mut one = UsageAccumulator::default();
            for message in messages {
                walked.observe(message);
                one.observe(message);
            }
            merged.merge(&one);
        }
        let (walked, merged) = (walked.finish_totals(2), merged.finish_totals(2));
        assert_eq!(merged, walked);
        assert_eq!(merged.turns, 4);
        assert_eq!(merged.output_tokens, Some(6), "one gap is tolerated");
        assert_eq!(
            merged.input_tokens, None,
            "two gaps across sessions are not"
        );
        assert_eq!(merged.cost_usd, None);
    }

    fn entry(id: &str, message: Option<AgentMessage>) -> LoadedEntry {
        LoadedEntry {
            entry_id: id.into(),
            message,
            custom: None,
        }
    }

    #[test]
    fn a_settled_session_resumes_from_its_last_entry() {
        let cache = UsageCache::default();
        // First read: the whole transcript.
        let (usage, last) = fold_entries(
            UsageAccumulator::default(),
            None,
            vec![
                entry("e1", Some(generation(json!({"input": 10, "output": 1})))),
                entry("e2", None),
            ],
        );
        assert_eq!(
            last.as_deref(),
            Some("e2"),
            "a custom entry still moves the watermark"
        );
        cache.remember("s", true, last, &usage);

        // Next read, still settled: start from e2 with what was counted.
        let hit = cache.settled("s", true).expect("settled sessions are kept");
        assert_eq!(hit.last_entry_id, "e2");
        let (usage, last) = fold_entries(
            hit.usage,
            Some(hit.last_entry_id),
            vec![entry(
                "e3",
                Some(generation(json!({"input": 5, "output": 2}))),
            )],
        );
        let totals = usage.clone().finish_totals(1);
        assert_eq!((totals.turns, totals.input_tokens), (2, Some(15)));
        assert_eq!(last.as_deref(), Some("e3"));

        // Nothing new: the watermark stays.
        let (_, last) = fold_entries(usage.clone(), last, vec![]);
        assert_eq!(last.as_deref(), Some("e3"));

        // Running again: its newest messages can still change, so it is
        // neither served from the cache nor kept in it.
        assert!(cache.settled("s", false).is_none());
        cache.remember("s", false, last, &usage);
        assert!(cache.settled("s", true).is_none(), "forgotten once it ran");
    }

    #[test]
    fn subtree_and_omit_traces_are_opt_in() {
        let req: SessionMetricsRequestV1 =
            serde_json::from_value(json!({"session_id": "s_child"})).unwrap();
        assert!(
            !req.subtree,
            "the tree is walked from its root unless asked"
        );
        assert!(!req.omit_traces, "traces are reported unless asked");
        let req: SessionMetricsRequestV1 = serde_json::from_value(
            json!({"session_id": "s_child", "subtree": true, "omit_traces": true}),
        )
        .unwrap();
        assert!(req.subtree && req.omit_traces);
    }

    #[test]
    fn the_request_takes_either_session_id_spelling() {
        for payload in [
            json!({"root_session_id": "s_root"}),
            json!({"session_id": "s_root"}),
        ] {
            let req: SessionMetricsRequestV1 = serde_json::from_value(payload.clone()).unwrap();
            assert_eq!(req.root_session_id, "s_root", "{payload}");
            assert!(!req.omit_context, "context is reported unless asked");
        }
        let req: SessionMetricsRequestV1 =
            serde_json::from_value(json!({"root_session_id": "s_root", "omit_context": true}))
                .unwrap();
        assert!(req.omit_context);
    }

    #[test]
    fn omitted_context_is_neither_reported_nor_read_back() {
        assert_eq!(session_context(Some(7), true), (None, false));
        assert_eq!(session_context(None::<u8>, true), (None, false));
        assert_eq!(session_context(Some(7), false), (Some(7), false));
        assert_eq!(
            session_context(None::<u8>, false),
            (None, true),
            "a missing snapshot is read back from state when context is reported"
        );
    }

    #[test]
    fn unreported_usage_stays_a_present_null_never_a_measured_zero() {
        let mut usage = UsageAccumulator::default();
        usage.observe(&message(json!({
            "role": "assistant",
            "content": [],
            "stop_reason": "end",
            "usage": {"input": 7},
            "model": "model",
            "provider": "provider",
            "timestamp": 1
        })));
        let wire =
            serde_json::to_value(usage.finish_session(&session("s_root", None, 0), None)).unwrap();
        assert_eq!(wire["input_tokens"], json!(7));
        for unreported in ["output_tokens", "reasoning_tokens", "cost_usd"] {
            assert!(
                wire.get(unreported).is_some_and(serde_json::Value::is_null),
                "{unreported} must be present and null, got {:?}",
                wire.get(unreported)
            );
        }
    }

    #[test]
    fn incomplete_metrics_can_preserve_observed_values() {
        let mut usage = UsageAccumulator::default();
        usage.observe(&message(json!({
            "role": "assistant",
            "content": [{
                "type": "function_call",
                "id": "call-1",
                "function_id": "state::get",
                "arguments": {}
            }],
            "stop_reason": "function_call",
            "usage": {"input": 10, "output": 2},
            "model": "model",
            "provider": "provider",
            "timestamp": 1
        })));
        let response = SessionMetricsResponseV1 {
            root_session_id: "root".into(),
            complete: false,
            totals: usage.finish_totals(1),
            by_session: Vec::new(),
            traces: None,
        };
        assert!(!response.complete);
        assert!(response.by_session.is_empty());
        assert_eq!(response.totals.function_calls, 1);
        assert_eq!(response.totals.input_tokens, Some(10));
        assert_eq!(response.traces, None);
    }

    #[test]
    fn metrics_complete_only_for_present_terminal_turns() {
        assert!(!terminal_status(None));
        assert!(!terminal_status(Some(TurnStatus::Running)));
        assert!(!terminal_status(Some(TurnStatus::AwaitingFunctions)));
        assert!(terminal_status(Some(TurnStatus::Completed)));
        assert!(terminal_status(Some(TurnStatus::Cancelled)));
        assert!(terminal_status(Some(TurnStatus::Failed)));
        assert!(!terminal_snapshot(Some(TurnStatus::Completed), true));
        assert!(terminal_snapshot(Some(TurnStatus::Completed), false));
    }

    #[test]
    fn trace_summary_covers_only_the_session_tree_and_deduplicates_traces() {
        let sessions = [
            session("root", None, 0),
            session("child", Some("root"), 1),
            session("without-spans", Some("root"), 1),
        ];
        let summary = summarize_trace_groups(
            &sessions,
            vec![
                TraceGroup {
                    value: "root".into(),
                    trace_ids: vec!["trace-a".into(), "trace-shared".into()],
                    span_count: 5,
                    first_seen_ms: 100,
                    last_seen_ms: 300,
                    duration_ms: 200,
                    error_count: 1,
                },
                TraceGroup {
                    value: "child".into(),
                    trace_ids: vec!["trace-shared".into(), "trace-b".into()],
                    span_count: 7,
                    first_seen_ms: 250,
                    last_seen_ms: 600,
                    duration_ms: 350,
                    error_count: 2,
                },
                TraceGroup {
                    value: "unrelated".into(),
                    trace_ids: vec!["trace-other".into()],
                    span_count: 99,
                    first_seen_ms: 1,
                    last_seen_ms: 999,
                    duration_ms: 998,
                    error_count: 9,
                },
            ],
        );

        assert_eq!(summary.trace_count, 3);
        assert_eq!(summary.span_count, 12);
        assert_eq!(summary.error_span_count, 3);
        assert_eq!(summary.duration_ms, 500);
        assert_eq!(summary.by_session.len(), 3);
        assert_eq!(summary.by_session[0].trace_count, 2);
        assert_eq!(summary.by_session[1].span_count, 7);
        assert_eq!(
            summary.by_session[2],
            SessionTraceUsageV1 {
                session_id: "without-spans".into(),
                parent_session_id: Some("root".into()),
                depth: 1,
                trace_count: 0,
                span_count: 0,
                error_span_count: 0,
                duration_ms: 0,
            }
        );
    }
}
