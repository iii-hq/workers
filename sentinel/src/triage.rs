//! Triage: which groups deserve a person's attention.
//!
//! Most of what an engine reports as an error is not a defect: a function
//! called while its worker was restarting, a caller told that its id does not
//! exist, a token that does not match on this machine. Those groups are real
//! and their counters keep running, but listing them beside a crash buries
//! the crash. So each group is labelled once, `triage.delay_ms` after it is
//! first seen:
//!
//! 1. **Rule.** A "function not found" whose function is registered by now
//!    was an outage, not a wrong call: a start-up race when its occurrences
//!    all fell inside that window, the environment when they did not.
//! 2. **Judge.** Everything else goes to `judge::evaluate` as one Choice
//!    question, batched. The judge sees the group as the store keeps it —
//!    redacted at capture — plus what the registry says about a missing
//!    function.
//!
//! A label never moves a group: resolving and ignoring stay human. A judge
//! that is not deployed or fails leaves the groups untriaged, and a later
//! sweep asks again.

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};

use crate::registry::{EngineRegistry, Registry};
use crate::store::{Db, Store, UntriagedGroup};
use crate::{
    GroupStatusV1, GroupTriageV1, SentinelError, TriageKindV1, TriageSourceV1, WorkerConfig,
};

pub const JUDGE_FUNCTION_ID: &str = "judge::evaluate";
/// Groups per sweep, and so per judge call.
pub const BATCH: usize = 128;
/// The judge's own budget for one batch; 60 groups took four seconds.
const JUDGE_TIMEOUT_MS: u64 = 60_000;
/// The hub adds 5 s of bus slack on top of the judge's budget.
pub const JUDGE_WAIT_MS: u64 = JUDGE_TIMEOUT_MS + 5_000;
/// After a failed judge call, leave the judge alone this long.
const PAUSE_MS: i64 = 300_000;
/// Longest message sample the judge sees.
const MAX_MESSAGE_CHARS: usize = 1_000;

/// A caller error or an environment problem this frequent, for this long,
/// is a program repeating a failing call, not someone's one-off mistake.
/// Measured on a live store: with these, 377 open groups became 38 that
/// held 97% of the occurrences.
pub const PERSISTENT_OCCURRENCES: u64 = 20;
pub const PERSISTENT_SPAN_MS: i64 = 3_600_000;

/// Whether a group belongs in the default list. Untriaged groups do: a
/// missing label must never hide anything. [`relevant_sql`] is the same
/// rule for the list query.
pub fn is_relevant(
    status: GroupStatusV1,
    kind: Option<TriageKindV1>,
    occurrence_count: u64,
    first_seen_ms: i64,
    last_seen_ms: i64,
) -> bool {
    let persistent = occurrence_count >= PERSISTENT_OCCURRENCES
        && last_seen_ms - first_seen_ms >= PERSISTENT_SPAN_MS;
    status == GroupStatusV1::Regressed
        || match kind {
            None | Some(TriageKindV1::Defect) => true,
            Some(TriageKindV1::CallerError | TriageKindV1::Environment) => persistent,
            Some(TriageKindV1::Transient | TriageKindV1::TestTraffic) => false,
        }
}

/// [`is_relevant`] over `sentinel_groups` columns. The kind is matched as
/// the prefix of the stored triage, which this worker serializes itself
/// with `kind` first: plain `LIKE` works on every database the `database`
/// worker speaks, and needs no column a migration would have to add.
pub fn relevant_sql() -> String {
    let defect = kind_sql(TriageKindV1::Defect);
    let caller = kind_sql(TriageKindV1::CallerError);
    let environment = kind_sql(TriageKindV1::Environment);
    format!(
        "(status = 'regressed' OR triage IS NULL OR {defect} \
         OR (({caller} OR {environment}) \
         AND occurrence_count >= {PERSISTENT_OCCURRENCES} \
         AND last_seen_ms - first_seen_ms >= {PERSISTENT_SPAN_MS}))"
    )
}

fn kind_sql(kind: TriageKindV1) -> String {
    format!("triage LIKE '{{\"kind\":\"{}\"%'", kind.as_str())
}

/// `judge::evaluate`.
#[async_trait]
pub trait Judge: Send + Sync {
    /// One call: the judge's typed envelope, `status` included.
    async fn evaluate(&self, request: Value) -> Result<Value, SentinelError>;
}

pub struct Triage<D: Db, E: EngineRegistry> {
    store: Arc<Store<D>>,
    registry: Arc<Registry<E>>,
    judge: Arc<dyn Judge>,
    paused_until_ms: AtomicI64,
    /// Untriaged groups the sweeps have already passed over. A group the
    /// judge cannot label right now (paused, not deployed, or it left the
    /// group out) stays untriaged; without this, 128 of them at the head
    /// would keep every newer group, even one the rule settles, waiting.
    offset: AtomicUsize,
}

impl<D: Db, E: EngineRegistry> Triage<D, E> {
    pub fn new(store: Arc<Store<D>>, registry: Arc<Registry<E>>, judge: Arc<dyn Judge>) -> Self {
        Self {
            store,
            registry,
            judge,
            paused_until_ms: AtomicI64::new(0),
            offset: AtomicUsize::new(0),
        }
    }

    /// Label the groups that are due. Returns how many were labelled.
    pub async fn sweep(&self, config: &WorkerConfig, now_ms: i64) -> Result<usize, SentinelError> {
        if !config.enabled || !config.triage.enabled {
            return Ok(0);
        }
        let window_ms = config.triage.delay_ms as i64;
        let offset = self.offset.load(Ordering::Relaxed);
        let due = self
            .store
            .untriaged_groups(now_ms - window_ms, BATCH, offset)
            .await?;
        let fetched = due.len();
        let labelled = self.label(due, window_ms, now_ms).await?;
        // Labelled groups leave the untriaged set; the rest of this page now
        // sits right after `offset`. A short page was the end: start over.
        let next = if fetched < BATCH {
            0
        } else {
            offset + fetched.saturating_sub(labelled)
        };
        self.offset.store(next, Ordering::Relaxed);
        Ok(labelled)
    }

    async fn label(
        &self,
        due: Vec<UntriagedGroup>,
        window_ms: i64,
        now_ms: i64,
    ) -> Result<usize, SentinelError> {
        let mut labelled = 0;
        let mut asking = Vec::new();
        for group in due {
            let mut missing = None;
            if let Some((function, namespace)) = missing_function(&group) {
                if self
                    .registry
                    .is_registered(namespace.as_deref(), &function)
                    .await
                {
                    // The function exists, so the caller was right: it was
                    // unavailable. Briefly is a restart; for longer, the
                    // worker was down or in another namespace.
                    let kind = if group.last_seen_ms - group.first_seen_ms <= window_ms {
                        TriageKindV1::Transient
                    } else {
                        TriageKindV1::Environment
                    };
                    let triage = GroupTriageV1 {
                        kind,
                        confidence: 1.0,
                        source: TriageSourceV1::Rule,
                        model: None,
                        at_ms: now_ms,
                    };
                    self.store.set_triage(&group.id, &triage).await?;
                    labelled += 1;
                    continue;
                }
                missing = Some(json!({ "id": function, "registered_now": false }));
            }
            asking.push(evaluation(&group, missing, now_ms));
        }

        if asking.is_empty()
            || now_ms < self.paused_until_ms.load(Ordering::Relaxed)
            || !self.registry.is_registered(None, JUDGE_FUNCTION_ID).await
        {
            return Ok(labelled);
        }
        let asked = asking.len();
        let request = json!({ "timeout_ms": JUDGE_TIMEOUT_MS, "evaluations": asking });
        let reply = match self.judge.evaluate(request).await {
            Ok(reply) if reply["status"] == "ok" => reply,
            failure => {
                let reason = match failure {
                    Ok(reply) => reply["code"].as_str().unwrap_or("error").to_string(),
                    Err(error) => error.to_string(),
                };
                tracing::warn!(%reason, groups = asked, "the judge did not triage; asking again later");
                self.paused_until_ms
                    .store(now_ms + PAUSE_MS, Ordering::Relaxed);
                return Ok(labelled);
            }
        };

        let model = reply["model"].as_str().map(str::to_string);
        let Some(results) = reply["results"].as_object() else {
            return Ok(labelled);
        };
        for (group_id, result) in results {
            let answer = &result["answers"]["kind"];
            let Some(kind) = answer["choice"]
                .as_str()
                .and_then(|choice| serde_json::from_value::<TriageKindV1>(json!(choice)).ok())
            else {
                continue;
            };
            let triage = GroupTriageV1 {
                kind,
                confidence: answer["confidence"].as_f64().unwrap_or(0.0),
                source: TriageSourceV1::Judge,
                model: model.clone(),
                at_ms: now_ms,
            };
            self.store.set_triage(group_id, &triage).await?;
            labelled += 1;
        }
        Ok(labelled)
    }
}

/// The function a "not found" failure names, and the namespace it was looked
/// up in when the message says. The group's own namespace is no help here:
/// an unregistered function has no owner, so the group falls back to the
/// engine's.
fn missing_function(group: &UntriagedGroup) -> Option<(String, Option<String>)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"Function (\S+) not found(?: in namespace ([^\s.]+))?")
            .expect("not-found pattern compiles")
    });
    if let Some(captures) = re.captures(&group.message_sample) {
        return Some((
            captures[1].to_string(),
            captures
                .get(2)
                .map(|namespace| namespace.as_str().to_string()),
        ));
    }
    // The engine's own span: the failing function is the missing one.
    (group.message_sample.trim() == "Function not found")
        .then(|| group.function_id.clone())
        .flatten()
        .map(|function| (function, None))
}

fn evaluation(group: &UntriagedGroup, missing: Option<Value>, now_ms: i64) -> Value {
    let mut state = json!({
        "worker": group.service_name,
        "function": group.function_id,
        "exception": group.exception_type,
        "message": truncate_chars(&group.message_sample, MAX_MESSAGE_CHARS),
        "source": group.source,
        "occurrences": group.occurrence_count,
        "sessions_affected": group.sessions_affected,
        "active_minutes": (group.last_seen_ms - group.first_seen_ms).max(0) / 60_000,
        "minutes_since_last": (now_ms - group.last_seen_ms).max(0) / 60_000,
    });
    if let Some(missing) = missing {
        state["missing_function"] = missing;
    }
    json!({ "id": group.id, "state": state, "questions": { "kind": kind_question() } })
}

/// One Choice question. A yes/no "should someone fix this?" was tried on the
/// live groups and answered between 0.15 and 0.51 for all of them; the
/// choice separated them.
fn kind_question() -> Value {
    json!({
        "type": "choice",
        "instructions": "An error monitor grouped this failure on a backend made of workers \
            that call each other's functions by id through an engine. `worker` reported it; \
            `iii` is the engine itself, reporting a call it could not route. Most of what \
            reaches the monitor is not a bug: only call it a defect when the reporting \
            worker's own code is at fault.",
        "criteria": {
            "defect": "The reporting worker's own code failed on input it should handle: a \
                crash or panic, an internal error, an unhandled case, corrupted or malformed \
                output.",
            "caller_error": "The request was rejected because the caller sent something \
                invalid, and the message says what: wrong argument types, a missing field, an \
                unknown id, session or path, an invalid state transition, a pattern that matched \
                nothing.",
            "transient": "Brief unavailability that went away on its own: a dependency \
                starting or restarting, a dropped connection, a timeout during a burst, or a \
                function that belonged to a browser tab, console session or temporary process \
                (its id carries a uuid or hash) that has since closed.",
            "environment": "This deployment's setup: credentials, tokens, missing files, \
                ports, mismatched worker versions, or a function whose worker is not installed \
                or not running here.",
            "test_traffic": "Produced on purpose by tests, probes or deliberately fake ids."
        }
    })
}

fn truncate_chars(value: &str, max: usize) -> String {
    match value.char_indices().nth(max) {
        Some((offset, _)) => format!("{}…", &value[..offset]),
        None => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(function_id: Option<&str>, message: &str) -> UntriagedGroup {
        UntriagedGroup {
            id: "grp_1".into(),
            namespace: "default".into(),
            service_name: "iii".into(),
            function_id: function_id.map(str::to_string),
            exception_type: None,
            message_sample: message.into(),
            source: "trace".into(),
            occurrence_count: 1,
            sessions_affected: 0,
            first_seen_ms: 0,
            last_seen_ms: 0,
        }
    }

    #[test]
    fn the_missing_function_comes_from_the_message_or_the_engine_span() {
        assert_eq!(
            missing_function(&group(
                Some("harness::status"),
                "harness/state: state::get harness_turn/s_1: remote error (function_not_found): \
                 Function state::get not found in namespace my-project."
            )),
            Some(("state::get".into(), Some("my-project".into())))
        );
        assert_eq!(
            missing_function(&group(Some("state::claim-namespace"), "Function not found")),
            Some(("state::claim-namespace".into(), None))
        );
        assert_eq!(
            missing_function(&group(Some("browser::screenshot"), "unknown session s_1")),
            None
        );
    }

    #[test]
    fn a_long_message_is_cut_on_a_character_boundary() {
        let cut = truncate_chars(&"ç".repeat(MAX_MESSAGE_CHARS + 5), MAX_MESSAGE_CHARS);
        assert_eq!(cut.chars().count(), MAX_MESSAGE_CHARS + 1);
        assert!(cut.ends_with('…'));
    }
}
