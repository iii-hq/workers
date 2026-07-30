use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::context::E2eContext;

use super::{
    common, CleanupFuture, CriterionSpec, EvaluationFuture, ExecutionPolicy, ObjectiveEvaluation,
    ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "research_pipeline";

const ARTICLE_URL: &str = "https://en.wikipedia.org/wiki/Cache_replacement_policies";
const ARTICLE_KEY: &str = "article";
const SUMMARY_KEY: &str = "summary";
const FACTS_KEY: &str = "facts";
const MIN_ARTICLE_CHARS: usize = 5_000;
const MAX_ARTICLE_CHARS: usize = 6_500;

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let names = Names::new(run_id);
    ScenarioSpec {
        id: ID,
        prompt: prompt(&names),
        execution: ExecutionPolicy {
            max_turns: 32,
            max_output_tokens: Some(16_384),
            max_total_tokens: 1_000_000,
            stuck_timeout_seconds: 300,
        },
        denied_functions: &[],
        threshold: 90,
        criteria: vec![
            CriterionSpec {
                id: "source_capture",
                weight: 25,
                description: "All wakes are armed before the Wikipedia article is fetched and saved.",
            },
            CriterionSpec {
                id: "parallel_analysis",
                weight: 30,
                description: "The article wake causes two analysts to be spawned directly and in parallel.",
            },
            CriterionSpec {
                id: "barrier_fan_in",
                weight: 25,
                description: "The analysts persist valid outputs and the named barrier retires after both arrive.",
            },
            CriterionSpec {
                id: "research_brief",
                weight: 20,
                description: "The coordinator returns a merged brief in its barrier-woken turn and leaves no binding armed.",
            },
        ],
        judge_reference: None,
        evaluate,
        cleanup: Some(cleanup),
    }
}

fn prompt(names: &Names) -> String {
    format!(
        r#"Build a self-driving web-research pipeline in the isolated state scope `{scope}`.

The control plane is parent-owned: a reaction may wake this coordinator, but it never starts an
agent. Every analyst must be spawned directly from a live coordinator turn.

Register every reaction before fetching anything:

1. Arm a one-shot state wake for key `{article_key}`, with top-level label `{article_label}`.
2. Arm one one-shot state wake over the whole scope, with top-level label `{complete_label}`.
   Gate it with the shipped named-set barrier using id `{barrier_id}`, expecting exactly
   `{summary_key}` and `{facts_key}`, and carry each event's new value.
3. Arm a one-shot timer deadline five minutes out, with any non-empty top-level label. Every
   registration is wake-only: omit any function target.

Then fetch `{url}` as markdown. Save exactly one state value at `{scope}` / `{article_key}`:

`{{ "url": "{url}", "title": "Cache replacement policies", "content": "<markdown prefix>" }}`

`content` must be the first roughly 6,000 characters of the fetched markdown, between 5,000 and
6,500 characters. Do not summarize it. After the write, end the turn; do not poll or spawn yet.

When the `{article_label}` wake arrives, use its `new_value` payload directly. In one response,
directly spawn these two leaf sessions so they run in parallel:

- A SUMMARIZER leaf: receive the complete article object inline, then write `{scope}` /
  `{summary_key}` as
  `{{ "role": "summarizer", "title": "<title>", "bullets": ["...", "...", "...", "...", "..."] }}`.
  The bullets must contain exactly five crisp, non-empty points.
- A FACT-EXTRACTOR leaf: receive the same complete article object inline, then write `{scope}` /
  `{facts_key}` as
  `{{ "role": "fact-extractor", "facts": ["...", "...", "...", "...", "..."] }}`.
  Include at least five concrete facts, algorithms, or figures from the article.

Narrow each leaf to function discovery plus the single state-write capability. They need no
state read because the article payload is inline, and they must not spawn, register reactions,
fetch, or coordinate. End the coordinator turn immediately after both direct spawns.

When the `{complete_label}` barrier wake arrives, remove the timer deadline and become the
RESEARCH REPORTER in this same session; do not spawn a reporter. Return a brief headed with the
analyst title, followed by `Summary` and `Concrete facts` sections. Reuse all five summary
bullets and every extracted fact verbatim so the merge is directly auditable. Do not answer
before the barrier wake, and leave no binding armed."#,
        scope = names.scope,
        article_key = ARTICLE_KEY,
        summary_key = SUMMARY_KEY,
        facts_key = FACTS_KEY,
        article_label = names.article_label,
        complete_label = names.complete_label,
        barrier_id = names.barrier_id,
        url = ARTICLE_URL,
    )
}

fn evaluate<'a>(
    context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    run_id: &'a str,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let names = Names::new(run_id);
        let article = get_state(context, &names.scope, ARTICLE_KEY).await?;
        let summary = get_state(context, &names.scope, SUMMARY_KEY).await?;
        let facts = get_state(context, &names.scope, FACTS_KEY).await?;
        let calls = common::function_calls(&observation.transcript);

        let article_watch = calls.iter().position(|call| is_article_watch(call, &names));
        let completion_watch = calls
            .iter()
            .position(|call| is_completion_watch(call, &names));
        let deadline_watch = calls.iter().position(is_deadline_watch);
        let fetch = calls
            .iter()
            .position(|call| is_article_fetch(&call.function_id, &call.arguments));
        let source_pipelines: Vec<_> = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| is_source_pipeline(call, &names))
            .collect();
        let article_writes: Vec<_> = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| {
                call.function_id == "state::set"
                    && call.arguments.get("scope").and_then(Value::as_str)
                        == Some(names.scope.as_str())
                    && call.arguments.get("key").and_then(Value::as_str) == Some(ARTICLE_KEY)
            })
            .collect();
        let article_write = article_writes.first().map(|(position, _)| *position);
        let source_pipeline = source_pipelines.first().map(|(position, _)| *position);
        let source_action = fetch.or(source_pipeline);
        let registrations = calls
            .iter()
            .filter(|call| call.function_id == "engine::register_trigger")
            .count();
        let armed_before_fetch = source_action.is_some_and(|source| {
            registrations == 3
                && [article_watch, completion_watch, deadline_watch]
                    .iter()
                    .all(|position| position.is_some_and(|position| position < source))
        });
        let source_order = fetch
            .zip(article_write)
            .is_some_and(|(fetch, write)| fetch < write)
            || (source_pipelines.len() == 1 && fetch.is_none() && article_writes.is_empty());
        let article_valid = valid_article(&article);
        let exact_article_write = (article_writes.len() == 1
            && article_writes[0].1.arguments.get("value") == Some(&article))
            || (source_pipelines.len() == 1 && article_valid);

        let spawns: Vec<_> = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.function_id == "harness::spawn")
            .collect();
        let analyst_sessions: BTreeSet<_> = observation
            .metrics
            .by_session
            .iter()
            .filter(|session| {
                session.depth == 1
                    && session.parent_session_id.as_deref() == Some(names.root_session.as_str())
            })
            .map(|session| session.session_id.clone())
            .collect();
        let spawned_after_article = article_write
            .or(source_pipeline)
            .is_some_and(|write| spawns.iter().all(|(position, _)| *position > write));
        let parallel_calls = max_parallel_spawns(&observation.transcript) == 2;

        let sessions_direct =
            observation.metrics.totals.sessions == 3 && analyst_sessions.len() == 2;

        let summary_valid = valid_summary(&summary);
        let facts_valid = valid_facts(&facts);
        let mut analyst_writes = true;
        let mut analyst_discipline = true;
        let mut analyst_activity = Vec::new();
        let mut written_keys = BTreeSet::new();
        for session_id in &analyst_sessions {
            let child_transcript = context.transcript(session_id).await?;
            analyst_activity.extend(activity_window(&child_transcript));
            let child_calls = common::function_calls(&child_transcript);
            let writes: Vec<_> = child_calls
                .iter()
                .filter(|call| call.function_id == "state::set")
                .collect();
            if writes.len() != 1 {
                analyst_writes = false;
            } else {
                let arguments = &writes[0].arguments;
                let expected = match arguments.get("key").and_then(Value::as_str) {
                    Some(SUMMARY_KEY) => Some((SUMMARY_KEY, &summary)),
                    Some(FACTS_KEY) => Some((FACTS_KEY, &facts)),
                    _ => None,
                };
                if let Some((key, value)) = expected {
                    written_keys.insert(key);
                    analyst_writes &=
                        arguments == &json!({ "scope": names.scope, "key": key, "value": value });
                } else {
                    analyst_writes = false;
                }
            }
            analyst_discipline &= child_calls.iter().all(|call| {
                call.function_id == "state::set"
                    || call.function_id.starts_with("engine::functions::")
            });
        }
        analyst_writes &= written_keys == BTreeSet::from([SUMMARY_KEY, FACTS_KEY]);
        let overlapping_sessions = activity_windows_overlap(&analyst_activity);
        let parallel_spawns = parallel_calls || overlapping_sessions;

        let records = common::trigger_fired_records(&observation.transcript);
        let article_records: Vec<_> = records
            .iter()
            .filter(|record| {
                record.get("label").and_then(Value::as_str) == Some(names.article_label.as_str())
            })
            .collect();
        let completion_records: Vec<_> = records
            .iter()
            .filter(|record| {
                record.get("label").and_then(Value::as_str) == Some(names.complete_label.as_str())
            })
            .collect();
        let article_woke = article_records.len() == 1
            && article_records[0].get("retired").and_then(Value::as_bool) == Some(true);
        let barrier_woke = completion_records.len() == 3
            && completion_records
                .iter()
                .filter(|record| record.get("retired").and_then(Value::as_bool) == Some(false))
                .count()
                == 2
            && completion_records
                .iter()
                .filter(|record| record.get("retired").and_then(Value::as_bool) == Some(true))
                .count()
                == 1;

        let active_bindings = common::active_binding_count(context, &names.root_session).await?;
        let report_merged = response_merges(&observation.response, &summary, &facts);
        let no_errors = observation.metrics.totals.function_call_errors == 0;

        let source_captured =
            armed_before_fetch && source_order && article_valid && exact_article_write;
        let direct_parallel_analysis =
            spawns.len() == 2 && spawned_after_article && parallel_spawns && sessions_direct;
        let fan_in_complete = summary_valid
            && facts_valid
            && analyst_writes
            && analyst_discipline
            && article_woke
            && barrier_woke;
        let report_complete = report_merged && active_bindings == 0 && no_errors;

        Ok(ObjectiveEvaluation {
            hard_gates: vec![
                common::gate(
                    "source_captured_after_watches",
                    source_captured,
                    format!(
                        "armed_before_fetch={armed_before_fetch}, source_order={source_order}, \
                         article_valid={article_valid}, exact_write={exact_article_write}"
                    ),
                ),
                common::gate(
                    "analysts_spawned_directly_in_parallel",
                    direct_parallel_analysis,
                    format!(
                        "spawns={}, parallel_calls={parallel_calls}, \
                         overlapping_sessions={overlapping_sessions}, \
                         direct_sessions={sessions_direct}",
                        spawns.len()
                    ),
                ),
                common::gate(
                    "analyst_outputs_joined_by_barrier",
                    fan_in_complete,
                    format!(
                        "summary_valid={summary_valid}, facts_valid={facts_valid}, \
                         analyst_writes={analyst_writes}, analyst_discipline={analyst_discipline}, \
                         article_woke={article_woke}, barrier_woke={barrier_woke}"
                    ),
                ),
                common::gate(
                    "brief_returned_and_bindings_clean",
                    report_complete,
                    format!(
                        "report_merged={report_merged}, active_bindings={active_bindings}, \
                         function_errors={}",
                        observation.metrics.totals.function_call_errors
                    ),
                ),
            ],
            awards: vec![
                common::award(
                    "source_capture",
                    if source_captured { 25 } else { 0 },
                    "awarded when all wakes precede one valid markdown fetch and article write",
                ),
                common::award(
                    "parallel_analysis",
                    if direct_parallel_analysis { 30 } else { 0 },
                    "awarded for two direct, parallel analyst sessions",
                ),
                common::award(
                    "barrier_fan_in",
                    if fan_in_complete { 25 } else { 0 },
                    "awarded when both durable analyst outputs retire the named barrier",
                ),
                common::award(
                    "research_brief",
                    if report_complete { 20 } else { 0 },
                    "awarded for a merged root-session brief and complete binding cleanup",
                ),
            ],
        })
    })
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let names = Names::new(run_id);
        let listed = context
            .trigger_value(
                "harness::triggers::list",
                json!({ "session_id": names.root_session }),
            )
            .await?;
        for subscription_id in listed
            .get("subscriptions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|subscription| subscription.get("subscription_id").and_then(Value::as_str))
        {
            let _: Value = context
                .trigger(
                    "harness::triggers::unregister",
                    json!({
                        "session_id": names.root_session,
                        "subscription_id": subscription_id,
                    }),
                )
                .await?;
        }
        for key in [ARTICLE_KEY, SUMMARY_KEY, FACTS_KEY] {
            let _: Value = context
                .trigger("state::delete", json!({ "scope": names.scope, "key": key }))
                .await?;
        }
        let _: Value = context
            .trigger(
                "state::delete",
                json!({ "scope": "state_barrier", "key": names.barrier_id }),
            )
            .await?;
        Ok(())
    })
}

async fn get_state(context: &E2eContext, scope: &str, key: &str) -> anyhow::Result<Value> {
    Ok(common::state_value(
        context
            .trigger_value("state::get", json!({ "scope": scope, "key": key }))
            .await?,
    ))
}

fn is_article_watch(call: &common::ObservedFunctionCall, names: &Names) -> bool {
    is_state_wake(call, names, Some(ARTICLE_KEY), &names.article_label)
}

fn is_completion_watch(call: &common::ObservedFunctionCall, names: &Names) -> bool {
    is_state_wake(call, names, None, &names.complete_label)
        && has_named_barrier(&call.arguments, names)
}

fn is_state_wake(
    call: &common::ObservedFunctionCall,
    names: &Names,
    key: Option<&str>,
    label: &str,
) -> bool {
    call.function_id == "engine::register_trigger"
        && call.arguments.get("trigger_type").and_then(Value::as_str) == Some("state")
        && call
            .arguments
            .pointer("/config/scope")
            .and_then(Value::as_str)
            == Some(names.scope.as_str())
        && call
            .arguments
            .pointer("/config/key")
            .and_then(Value::as_str)
            == key
        && call.arguments.get("label").and_then(Value::as_str) == Some(label)
        && common::requested_once(&call.arguments)
        && common::is_wake_registration(&call.arguments)
}

fn is_deadline_watch(call: &common::ObservedFunctionCall) -> bool {
    call.function_id == "engine::register_trigger"
        && call.arguments.get("trigger_type").and_then(Value::as_str) == Some("timer")
        && call
            .arguments
            .get("label")
            .and_then(Value::as_str)
            .is_some_and(|label| !label.trim().is_empty())
        && call
            .arguments
            .pointer("/config/in_ms")
            .and_then(Value::as_u64)
            .is_some_and(|in_ms| (240_000..=360_000).contains(&in_ms))
        && common::requested_once(&call.arguments)
        && common::is_wake_registration(&call.arguments)
}

fn is_source_pipeline(call: &common::ObservedFunctionCall, names: &Names) -> bool {
    if call.function_id != "fp::pipe" {
        return false;
    }
    let Some(steps) = call.arguments.get("through").and_then(Value::as_array) else {
        return false;
    };
    let fetch = steps.iter().position(|step| {
        step.get("function")
            .and_then(Value::as_str)
            .zip(step.get("payload"))
            .is_some_and(|(function_id, payload)| is_article_fetch(function_id, payload))
    });
    let take = steps.iter().position(|step| {
        step.get("function").and_then(Value::as_str) == Some("fp::take")
            && step
                .pointer("/payload/n")
                .and_then(Value::as_u64)
                .is_some_and(|length| {
                    (MIN_ARTICLE_CHARS as u64..=MAX_ARTICLE_CHARS as u64).contains(&length)
                })
    });
    let write = steps.iter().position(|step| {
        step.get("function").and_then(Value::as_str) == Some("state::set")
            && step.get("into").and_then(Value::as_str) == Some("/value/content")
            && step.pointer("/payload/scope").and_then(Value::as_str) == Some(names.scope.as_str())
            && step.pointer("/payload/key").and_then(Value::as_str) == Some(ARTICLE_KEY)
            && step.pointer("/payload/value/url").and_then(Value::as_str) == Some(ARTICLE_URL)
            && step.pointer("/payload/value/title").and_then(Value::as_str)
                == Some("Cache replacement policies")
    });
    fetch
        .zip(take)
        .zip(write)
        .is_some_and(|((fetch, take), write)| fetch < take && take < write)
}

fn is_article_fetch(function_id: &str, arguments: &Value) -> bool {
    matches!(function_id, "web::fetch" | "scrapling::fetch")
        && arguments.get("url").and_then(Value::as_str) == Some(ARTICLE_URL)
        && arguments.get("format").and_then(Value::as_str) == Some("markdown")
}

fn has_named_barrier(arguments: &Value, names: &Names) -> bool {
    let expected = BTreeSet::from([SUMMARY_KEY.to_string(), FACTS_KEY.to_string()]);
    arguments
        .get("conditions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|condition| {
            condition.get("function_id").and_then(Value::as_str) == Some("state::barrier")
                && condition.pointer("/config/id").and_then(Value::as_str)
                    == Some(names.barrier_id.as_str())
                && condition.pointer("/config/carry").and_then(Value::as_str) == Some("/new_value")
                && condition
                    .pointer("/config/expect")
                    .and_then(Value::as_array)
                    .map(|keys| {
                        keys.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect::<BTreeSet<_>>()
                    })
                    == Some(expected.clone())
        })
}

fn valid_article(article: &Value) -> bool {
    article.get("url").and_then(Value::as_str) == Some(ARTICLE_URL)
        && article
            .get("title")
            .and_then(Value::as_str)
            .is_some_and(|title| title.eq_ignore_ascii_case("Cache replacement policies"))
        && article
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| {
                let length = content.chars().count();
                let content = content.to_ascii_lowercase();
                (MIN_ARTICLE_CHARS..=MAX_ARTICLE_CHARS).contains(&length)
                    && content.contains("cache")
                    && (content.contains("least recently used") || content.contains("lru"))
            })
}

fn valid_summary(summary: &Value) -> bool {
    summary.get("role").and_then(Value::as_str) == Some("summarizer")
        && summary
            .get("title")
            .and_then(Value::as_str)
            .is_some_and(|title| !title.trim().is_empty())
        && string_list(summary, "bullets")
            .is_some_and(|bullets| bullets.len() == 5 && all_substantive(&bullets))
}

fn valid_facts(facts: &Value) -> bool {
    facts.get("role").and_then(Value::as_str) == Some("fact-extractor")
        && string_list(facts, "facts")
            .is_some_and(|facts| facts.len() >= 5 && all_substantive(&facts))
}

fn response_merges(response: &str, summary: &Value, facts: &Value) -> bool {
    let Some(title) = summary.get("title").and_then(Value::as_str) else {
        return false;
    };
    let Some(bullets) = string_list(summary, "bullets") else {
        return false;
    };
    let Some(facts) = string_list(facts, "facts") else {
        return false;
    };
    let normalized = response.to_ascii_lowercase();
    response.contains(title)
        && normalized.contains("summary")
        && normalized.contains("concrete facts")
        && bullets
            .iter()
            .chain(facts.iter())
            .all(|item| response.contains(item))
}

fn string_list<'a>(value: &'a Value, key: &str) -> Option<Vec<&'a str>> {
    value
        .get(key)?
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect()
}

fn all_substantive(values: &[&str]) -> bool {
    values
        .iter()
        .all(|value| value.trim().chars().count() >= 12)
}

fn activity_window(transcript: &Value) -> Option<(i64, i64)> {
    let mut timestamps = transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("message"))
        .filter(|message| {
            matches!(
                message.get("role").and_then(Value::as_str),
                Some("assistant" | "function_result")
            )
        })
        .filter_map(|message| message.get("timestamp").and_then(Value::as_i64));
    let first = timestamps.next()?;
    Some(
        timestamps.fold((first, first), |(started_at, finished_at), timestamp| {
            (started_at.min(timestamp), finished_at.max(timestamp))
        }),
    )
}

fn activity_windows_overlap(windows: &[(i64, i64)]) -> bool {
    windows.len() == 2 && windows[0].0 < windows[1].1 && windows[1].0 < windows[0].1
}

fn max_parallel_spawns(transcript: &Value) -> usize {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("message"))
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .map(|message| {
            message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|block| {
                    normalized_block_call(block)
                        .is_some_and(|(function, _)| function == "harness::spawn")
                })
                .count()
        })
        .max()
        .unwrap_or(0)
}

fn normalized_block_call(block: &Value) -> Option<(&str, &Value)> {
    if block.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    let function = block.get("function_id")?.as_str()?;
    let arguments = block.get("arguments")?;
    if function == "agent_trigger" {
        return Some((
            arguments.get("function")?.as_str()?,
            arguments.get("payload")?,
        ));
    }
    Some((function, arguments))
}

struct Names {
    scope: String,
    root_session: String,
    article_label: String,
    complete_label: String,
    barrier_id: String,
}

impl Names {
    fn new(run_id: &str) -> Self {
        let scope = format!("e2e:research:{run_id}");
        Self {
            root_session: format!("e2e_{run_id}"),
            article_label: format!("article-ready:{run_id}"),
            complete_label: format!("analysts-complete:{run_id}"),
            barrier_id: format!("research:{run_id}:analysts"),
            scope,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_is_objective_and_valid() {
        let scenario = scenario("run");
        scenario.validate().unwrap();
        assert!(!scenario.needs_judge());
        assert!(!scenario.prompt.contains("harness::react"));
    }

    #[test]
    fn accepts_model_chosen_timer_label_and_child_ids() {
        let deadline = common::ObservedFunctionCall {
            function_id: "engine::register_trigger".to_string(),
            arguments: json!({
                "trigger_type": "timer",
                "config": { "in_ms": 300_000 },
                "label": "deadline:run",
                "once": true
            }),
        };
        let transcript = json!({
            "messages": [
                {
                    "message": {
                        "role": "assistant",
                        "content": [{
                            "type": "function_call",
                            "function_id": "agent_trigger",
                            "arguments": {
                                "function": "harness::spawn",
                                "payload": { "task": "summarize" }
                            }
                        }]
                    }
                },
                {
                    "message": {
                        "role": "assistant",
                        "content": [{
                            "type": "function_call",
                            "function_id": "agent_trigger",
                            "arguments": {
                                "function": "harness::spawn",
                                "payload": {
                                    "session_id": "model-chosen-facts",
                                    "task": "extract facts"
                                }
                            }
                        }]
                    }
                }
            ]
        });
        let first_child = json!({
            "messages": [
                { "message": { "role": "assistant", "timestamp": 10 } },
                { "message": { "role": "function_result", "timestamp": 30 } }
            ]
        });
        let second_child = json!({
            "messages": [
                { "message": { "role": "assistant", "timestamp": 20 } },
                { "message": { "role": "function_result", "timestamp": 40 } }
            ]
        });
        let activity = [
            activity_window(&first_child).unwrap(),
            activity_window(&second_child).unwrap(),
        ];

        assert!(is_deadline_watch(&deadline));
        assert_eq!(max_parallel_spawns(&transcript), 1);
        assert!(activity_windows_overlap(&activity));
    }

    #[test]
    fn validates_structured_analyst_outputs_and_verbatim_merge() {
        let summary = json!({
            "role": "summarizer",
            "title": "Cache replacement policies",
            "bullets": [
                "First substantive summary point.",
                "Second substantive summary point.",
                "Third substantive summary point.",
                "Fourth substantive summary point.",
                "Fifth substantive summary point."
            ]
        });
        let facts = json!({
            "role": "fact-extractor",
            "facts": [
                "LRU evicts the least recently used item.",
                "FIFO evicts the oldest inserted item.",
                "Belady's algorithm is optimal with future knowledge.",
                "LFU uses access frequency.",
                "Random replacement chooses an item randomly."
            ]
        });
        let response = format!(
            "# {}\n\n## Summary\n{}\n\n## Concrete facts\n{}",
            summary["title"].as_str().unwrap(),
            string_list(&summary, "bullets").unwrap().join("\n"),
            string_list(&facts, "facts").unwrap().join("\n")
        );

        assert!(valid_summary(&summary));
        assert!(valid_facts(&facts));
        assert!(response_merges(&response, &summary, &facts));
    }

    #[test]
    fn recognizes_atomic_fetch_trim_and_save_pipeline() {
        let names = Names::new("run");
        for fetch in ["web::fetch", "scrapling::fetch"] {
            let call = common::ObservedFunctionCall {
                function_id: "fp::pipe".to_string(),
                arguments: json!({
                    "through": [
                        {
                            "function": fetch,
                            "payload": { "url": ARTICLE_URL, "format": "markdown" }
                        },
                        { "function": "fp::get", "payload": { "path": "/body" } },
                        { "function": "fp::take", "payload": { "n": 6000 } },
                        {
                            "function": "state::set",
                            "into": "/value/content",
                            "payload": {
                                "scope": names.scope,
                                "key": ARTICLE_KEY,
                                "value": {
                                    "url": ARTICLE_URL,
                                    "title": "Cache replacement policies"
                                }
                            }
                        }
                    ]
                }),
            };

            assert!(is_source_pipeline(&call, &names), "{fetch}");
        }
    }
}
