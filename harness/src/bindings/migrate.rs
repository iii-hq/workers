//! Binding records written before 0.26 name what they watch as a trigger
//! type and its config (`dedup_key.trigger_type` / `dedup_key.config`), and
//! carry a `trigger_id`. The boot pass rewrites each one into an observer
//! binding: the type and config become a [`SourceSpec`] through the table in
//! spec §11 and the workers contract (§1.1 fixed names, `W::X` → `W::on-X`),
//! the config becomes the source's args when the function takes args and
//! filters on `input.<field>` otherwise (§1.4, the §2 session note, §3). The
//! stored config is the agent's raw request, so a `timer` with `{ in_ms }`
//! counts from the record's `created_at`. A record that does not translate is
//! retired, and its owner is told why and what to observe instead.
//!
//! The rewrite compares the stored JSON itself, not a re-serialized
//! [`Binding`]: every later compare-and-set compares the current shape.

use serde_json::{json, Map, Value};

use super::{Binding, SourceSpec};
use crate::deps::Deps;
use crate::state;

/// The functions whose observers pass args (workers contract §1.3). Every
/// other source takes filters.
const ARGS_FUNCTIONS: [&str; 20] = [
    "cron::on-tick",
    "http::on-request",
    "queue::on-message",
    "database::on-row-changed",
    "harness::on-timer",
    "console::ui::asset",
    "shell::on-changed",
    "shell::on-git-changed",
    "shell::on-job-finished",
    "github::pr::on-event",
    "storage::on-object-created",
    "storage::on-object-deleted",
    "kanban::on-change",
    "kanban::on-comment",
    "stories::on-changed",
    "computer::on-frame-changed",
    "browser::on-frame-event",
    "browser::on-crawl-item",
    "ios-simulator::on-frame-event",
    "demo::on-counter-changed",
];

/// Agent feeds take `{ session_id }` args (workers contract §1.3).
fn is_agent_feed(function_id: &str) -> bool {
    function_id.ends_with("::on-agent-event") || function_id.ends_with("::on-raw-event")
}

fn takes_args(function_id: &str) -> bool {
    ARGS_FUNCTIONS.contains(&function_id) || is_agent_feed(function_id)
}

/// Config keys that only ever steered the old registration, never the event.
const DROPPED_KEYS: [&str; 2] = ["metadata", "condition_function_id"];

/// `config` without the keys that steered the old registration.
fn args_of(config: &Value) -> Option<Value> {
    let mut map = config.as_object()?.clone();
    for key in DROPPED_KEYS {
        map.remove(key);
    }
    (!map.is_empty()).then_some(Value::Object(map))
}

/// An `http` binding's config named its route `api_path` and `http_method`
/// (`GET` when unset); `http::on-request` takes them as `path` and `method`.
fn http_route_args(config: &Value) -> Option<Value> {
    let mut args = args_of(config)?;
    let map = args.as_object_mut()?;
    if let Some(path) = map.remove("api_path") {
        map.insert("path".to_string(), path);
    }
    let method = map.remove("http_method").unwrap_or_else(|| json!("GET"));
    map.entry("method").or_insert(method);
    Some(args)
}

/// One `$eq` filter per scalar config field, on `input.<field>`.
fn eq_filters(config: &Value, fields: &[(&str, &str)]) -> Option<Value> {
    let config = config.as_object()?;
    let mut filters = Map::new();
    for (field, path) in fields {
        if let Some(value) = config.get(*field).filter(|v| !v.is_null()) {
            filters.insert(path.to_string(), json!({ "$eq": value }));
        }
    }
    (!filters.is_empty()).then_some(Value::Object(filters))
}

/// `$eq` for one value, `$in` for any other count.
fn one_of(values: Vec<Value>) -> Value {
    match <[Value; 1]>::try_from(values) {
        Ok([value]) => json!({ "$eq": value }),
        Err(values) => json!({ "$in": values }),
    }
}

/// Merge `extra` filters into `filters`.
fn with_filters(filters: Option<Value>, extra: Map<String, Value>) -> Option<Value> {
    let mut merged = match filters {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    merged.extend(extra);
    (!merged.is_empty()).then_some(Value::Object(merged))
}

/// Session-scoped event types: their events carry `session_metadata`, and
/// their configs selected sessions by a `metadata` subset match.
fn is_session_scoped(trigger_type: &str) -> bool {
    trigger_type.starts_with("session::") || trigger_type.starts_with("approval::")
}

/// The filters a session-scoped config's `metadata` and `roles` became: one
/// `$eq` on `input.session_metadata.<key>` per metadata key (the old subset
/// match compared each key's whole value), and `input.message.role` `$in`
/// the roles.
fn session_filters(config: &Value) -> Map<String, Value> {
    let mut filters = Map::new();
    if let Some(metadata) = config.get("metadata").and_then(Value::as_object) {
        for (key, value) in metadata {
            filters.insert(
                format!("input.session_metadata.{key}"),
                json!({ "$eq": value }),
            );
        }
    }
    if let Some(roles) = config.get("roles").and_then(Value::as_array) {
        filters.insert("input.message.role".into(), json!({ "$in": roles }));
    }
    filters
}

/// The `configuration` config's filters: `input.id` for `configuration_id`,
/// and `input.event_type` for `event_types` (omitted: every event type).
fn configuration_filters(config: &Value) -> Option<Value> {
    let mut extra = Map::new();
    if let Some(types) = config.get("event_types").and_then(Value::as_array) {
        extra.insert("input.event_type".into(), one_of(types.clone()));
    }
    with_filters(
        eq_filters(config, &[("configuration_id", "input.id")]),
        extra,
    )
}

/// The `worker` config's `operations` as `input.event` values of
/// `engine::on-workers-changed`: `add` is a connect, `remove` a disconnect,
/// and no operation either.
fn worker_events(config: &Value) -> Value {
    let mut events: Vec<Value> = Vec::new();
    for operation in config
        .get("operations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let event = match operation {
            "add" => "worker_connected",
            "remove" => "worker_disconnected",
            _ => continue,
        };
        if !events.iter().any(|known| known == event) {
            events.push(json!(event));
        }
    }
    if events.is_empty() {
        events = vec![json!("worker_connected"), json!("worker_disconnected")];
    }
    json!({ "input.event": { "$in": events } })
}

/// A stored `timer` config's deadline as an absolute `at`: `{ at }` as
/// stored, `{ in_ms }` counted from the record's creation.
fn timer_at(config: &Value, created_at: i64) -> Option<i64> {
    if let Some(at) = config.get("at").and_then(Value::as_i64) {
        return Some(at);
    }
    let in_ms = config.get("in_ms").and_then(Value::as_i64)?;
    Some(created_at.saturating_add(in_ms))
}

/// The `W::on-<event>` function a worker's own type `T` became.
fn on_function(trigger_type: &str) -> Option<String> {
    if let Some((worker, event)) = trigger_type.rsplit_once("::") {
        if worker.is_empty() || event.is_empty() {
            return None;
        }
        if event.starts_with("on-") {
            return Some(trigger_type.to_string());
        }
        return Some(format!("{worker}::on-{event}"));
    }
    let (worker, event) = trigger_type.split_once(':')?;
    (!worker.is_empty() && !event.is_empty()).then(|| format!("{worker}::on-{event}"))
}

/// What a stored `(trigger_type, config)` of a record created at
/// `created_at` observes now. `None` when the type has no observable function
/// an agent may observe.
pub fn translate(trigger_type: &str, config: &Value, created_at: i64) -> Option<SourceSpec> {
    let source = |function_id: &str, args: Option<Value>, filters: Option<Value>| SourceSpec {
        function_id: function_id.to_string(),
        namespace: None,
        args,
        filters,
    };
    let spec = match trigger_type {
        "timer" => source(
            crate::timer::ON_TIMER_FN,
            Some(json!({ "at": timer_at(config, created_at)? })),
            None,
        ),
        "http" => source("http::on-request", http_route_args(config), None),
        "cron" => source(
            "cron::on-tick",
            Some(json!({ "expression": config.get("expression")?.as_str()? })),
            None,
        ),
        "state" => source(
            "state::on-changed",
            None,
            eq_filters(config, &[("scope", "input.scope"), ("key", "input.key")]),
        ),
        "database::row-changed" => source("database::on-row-changed", args_of(config), None),
        "durable:subscriber" => source("queue::on-message", args_of(config), None),
        "subscribe" => source(
            "publish",
            None,
            eq_filters(config, &[("topic", "input.topic")]),
        ),
        "stream" | "stream:join" | "stream:leave" => {
            let function_id = match trigger_type {
                "stream" => "stream::on-changed",
                "stream:join" => "stream::on-join",
                _ => "stream::on-leave",
            };
            SourceSpec {
                namespace: Some("default".to_string()),
                ..source(
                    function_id,
                    None,
                    eq_filters(
                        config,
                        &[
                            ("stream_name", "input.stream_name"),
                            ("group_id", "input.group_id"),
                        ],
                    ),
                )
            }
        }
        "compose-operation" => {
            let mut filters = eq_filters(config, &[("operation_id", "input.operation_id")])
                .unwrap_or_else(|| json!({}));
            if config.get("terminal_only").and_then(Value::as_bool) == Some(true) {
                filters["input.terminal"] = json!({ "$eq": true });
            }
            SourceSpec {
                namespace: crate::functions::observe::compose_namespace(),
                ..source(
                    crate::functions::observe::COMPOSE_PROGRESS_FN,
                    None,
                    (filters != json!({})).then_some(filters),
                )
            }
        }
        "log" => {
            let errors_only = config.get("level").and_then(Value::as_str) == Some("error");
            SourceSpec {
                namespace: Some("default".to_string()),
                ..source(
                    "observability::on-log",
                    None,
                    errors_only.then(|| json!({ "input.severity_number": { "$gte": 17 } })),
                )
            }
        }
        "trace" => SourceSpec {
            namespace: Some("default".to_string()),
            ..source("observability::on-span", None, None)
        },
        "configuration" => source(
            "configuration::on-updated",
            None,
            configuration_filters(config),
        ),
        "engine::functions-available" => source("engine::on-functions-changed", None, None),
        "engine::workers-available" => source("engine::on-workers-changed", None, None),
        "worker" => source(
            "engine::on-workers-changed",
            None,
            Some(worker_events(config)),
        ),
        "console:assets" => source("console::ui::on-assets-changed", None, None),
        // An observer of `console::ui::asset` is its asset's content source
        // (contract rows 118-120): the ADE calls its target with `{ path }`
        // and stores the `content` of the answer. A binding's target is the
        // delivery hop, whose answer carries no content, so the record is
        // retired.
        "console:script" | "console:style" | "console:module" => return None,
        other if other.starts_with("harness::") => return None,
        other => {
            let function_id = on_function(other)?;
            if takes_args(&function_id) {
                source(&function_id, args_of(config), None)
            } else {
                let fields: Vec<(String, String)> = config
                    .as_object()
                    .map(|map| {
                        map.iter()
                            .filter(|(key, value)| {
                                !DROPPED_KEYS.contains(&key.as_str())
                                    && !value.is_object()
                                    && !value.is_array()
                            })
                            .map(|(key, _)| (key.clone(), format!("input.{key}")))
                            .collect()
                    })
                    .unwrap_or_default();
                let fields: Vec<(&str, &str)> = fields
                    .iter()
                    .map(|(key, path)| (key.as_str(), path.as_str()))
                    .collect();
                let mut filters = eq_filters(config, &fields);
                if is_session_scoped(other) {
                    filters = with_filters(filters, session_filters(config));
                }
                source(&function_id, None, filters)
            }
        }
    };
    Some(spec)
}

/// What the boot pass does with one stored record.
#[derive(Debug)]
pub enum Upgrade {
    /// Already an observer binding.
    Current(Binding),
    /// Translated: write `next` over the stored value.
    Rewrite { next: Binding },
    /// Nothing observable replaces what it watched: retire it. `why` is the
    /// owner notice's reason and what to observe instead.
    Retire {
        binding: Binding,
        watched: String,
        why: &'static str,
    },
    /// Not a binding record at all; left alone.
    Unreadable,
}

/// Decide what to do with one stored record.
pub fn upgrade(raw: &Value) -> Upgrade {
    let Ok(mut binding) = serde_json::from_value::<Binding>(raw.clone()) else {
        return Upgrade::Unreadable;
    };
    if binding.source.is_some() {
        return match serde_json::to_value(&binding) {
            Ok(encoded) if encoded == *raw => Upgrade::Current(binding),
            _ => Upgrade::Rewrite { next: binding },
        };
    }
    let stored = binding.dedup_key.clone().unwrap_or(Value::Null);
    let trigger_type = stored
        .get("trigger_type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let config = stored.get("config").cloned().unwrap_or(Value::Null);
    let Some(source) = trigger_type
        .as_deref()
        .and_then(|trigger_type| translate(trigger_type, &config, binding.created_at))
    else {
        let why = retirement_reason(trigger_type.as_deref());
        let watched = match trigger_type {
            Some(trigger_type) => format!("`{trigger_type}` {config}"),
            None => "an unrecorded source".to_string(),
        };
        return Upgrade::Retire {
            binding,
            watched,
            why,
        };
    };
    // The observer is created by the replay that follows.
    binding.observer_id = None;
    binding.dedup_key = Some(rewritten_key(&stored, &source));
    binding.source = Some(source);
    Upgrade::Rewrite { next: binding }
}

/// Why a stored type does not translate, and what its owner observes instead.
fn retirement_reason(trigger_type: Option<&str>) -> &'static str {
    match trigger_type {
        Some("timer") => {
            "its stored deadline has neither `at` nor `in_ms`. To be told at a deadline, \
             observe harness::on-timer with args { in_ms } (or { at }, epoch ms) through \
             harness::observe."
        }
        Some("console:script" | "console:style" | "console:module") => {
            "an observer of console::ui::asset serves its asset's content to the console, \
             and a binding's target answers with no content. To be told when console UI \
             assets change, observe console::ui::on-assets-changed through harness::observe."
        }
        Some(other) if other.starts_with("harness::") => {
            "an agent observes no harness function except harness::on-timer. Watch the work \
             itself (the state, stream or function it writes) with harness::observe."
        }
        _ => {
            "nothing observable replaces what it watched. Find the function that emits the \
             event with engine::functions::list and observe it with harness::observe."
        }
    }
}

/// The canonical request in its current shape: the source fields in place of
/// the type and config, and `action` lifted out of the old `metadata`.
fn rewritten_key(stored: &Value, source: &SourceSpec) -> Value {
    json!({
        "function_id": source.function_id,
        "namespace": source.namespace,
        "args": source.args,
        "filters": source.filters,
        "label": stored.get("label"),
        "action": stored.get("metadata").and_then(|m| m.get("action")),
        "lifecycle": stored.get("lifecycle"),
        "target": stored.get("target"),
        "conditions": stored.get("conditions"),
    })
}

/// Counts from one migration pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Migrated {
    pub rewritten: usize,
    pub retired: usize,
}

/// Rewrite every stored record into its current shape, retiring the ones
/// that do not translate. Runs before anything compares a record.
pub async fn run(deps: &Deps) -> Migrated {
    let timeout_ms = deps.cfg().await.session_timeout_ms;
    let raws = match state::list_values::<Value>(&deps.iii, state::BINDING_SCOPE, timeout_ms).await
    {
        Ok(raws) => raws,
        Err(error) => {
            tracing::warn!(%error, "binding migration skipped: store unreadable");
            return Migrated::default();
        }
    };
    let mut migrated = Migrated::default();
    for raw in raws {
        match upgrade(&raw) {
            Upgrade::Current(_) | Upgrade::Unreadable => {}
            Upgrade::Rewrite { next } => {
                let Ok(encoded) = serde_json::to_value(&next) else {
                    continue;
                };
                match state::cas_value(
                    &deps.iii,
                    state::BINDING_SCOPE,
                    &next.id,
                    Some(raw),
                    encoded,
                    timeout_ms,
                )
                .await
                {
                    Ok(None) => migrated.rewritten += 1,
                    Ok(Some(_)) => {
                        tracing::warn!(binding = %next.id, "binding moved during migration")
                    }
                    Err(error) => {
                        tracing::warn!(binding = %next.id, %error, "binding migration failed")
                    }
                }
            }
            Upgrade::Retire {
                binding,
                watched,
                why,
            } => {
                match state::cas_value(
                    &deps.iii,
                    state::BINDING_SCOPE,
                    &binding.id,
                    Some(raw),
                    Value::Null,
                    timeout_ms,
                )
                .await
                {
                    Ok(None) => {
                        migrated.retired += 1;
                        notify_retired(deps, &binding, &watched, why).await;
                    }
                    Ok(Some(_)) => {
                        tracing::warn!(binding = %binding.id, "binding moved during retirement")
                    }
                    Err(error) => {
                        tracing::warn!(binding = %binding.id, %error, "binding retirement failed")
                    }
                }
            }
        }
    }
    migrated
}

/// The owner's notice for a retired binding.
fn retirement_notice(binding: &Binding, watched: &str, why: &str) -> String {
    format!(
        "[notification] binding {} (was watching {watched}) was RETIRED: {why}",
        binding.id
    )
}

/// Tell the owner its binding is gone, why, and what to observe instead.
async fn notify_retired(deps: &Deps, binding: &Binding, watched: &str, why: &str) {
    let notice = retirement_notice(binding, watched, why);
    if let Err(error) = crate::functions::send::inject(
        deps,
        &binding.owner.session_id,
        crate::types::message::AgentMessage::user_text(notice),
        Some(&format!("e_untranslated_{}", binding.id)),
        Some(&json!({ "notification": true, "binding": binding.id })),
    )
    .await
    {
        tracing::warn!(binding = %binding.id, %error, "binding retirement notice failed");
    }
    crate::subscriptions::fired::emit(
        &deps.session().await,
        &binding.owner.session_id,
        &format!("e_obsinvalid_{}", binding.id),
        crate::subscriptions::fired::retirement_record(
            binding,
            crate::subscriptions::fired::FireOutcome::Invalidated,
            crate::subscriptions::fired::RetirementReason::Invalidated,
            Some(why),
            crate::subscriptions::fired::now_ms(),
        ),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record created at this instant.
    const CREATED_AT: i64 = 1_700_000_000_000;

    fn tr(trigger_type: &str, config: &Value) -> Option<SourceSpec> {
        translate(trigger_type, config, CREATED_AT)
    }

    fn spec(function_id: &str, args: Option<Value>, filters: Option<Value>) -> SourceSpec {
        SourceSpec {
            function_id: function_id.into(),
            namespace: None,
            args,
            filters,
        }
    }

    #[test]
    fn the_spec_table_translates_each_stored_type() {
        assert_eq!(
            tr("timer", &json!({ "at": 1_700_000_000_000_i64 })),
            Some(spec(
                "harness::on-timer",
                Some(json!({ "at": 1_700_000_000_000_i64 })),
                None
            ))
        );
        assert_eq!(
            tr("cron", &json!({ "expression": "0 * * * * *" })),
            Some(spec(
                "cron::on-tick",
                Some(json!({ "expression": "0 * * * * *" })),
                None
            ))
        );
        assert_eq!(
            tr("state", &json!({ "scope": "run", "key": "go" })),
            Some(spec(
                "state::on-changed",
                None,
                Some(json!({
                    "input.scope": { "$eq": "run" },
                    "input.key": { "$eq": "go" },
                }))
            ))
        );
        assert_eq!(
            tr(
                "database::row-changed",
                &json!({ "db": "primary", "table": "orders", "metadata": { "x": 1 } })
            ),
            Some(spec(
                "database::on-row-changed",
                Some(json!({ "db": "primary", "table": "orders" })),
                None
            ))
        );
        assert_eq!(
            tr("engine::workers-available", &json!({})),
            Some(spec("engine::on-workers-changed", None, None))
        );
    }

    #[test]
    fn a_worker_type_becomes_its_on_function() {
        assert_eq!(
            tr("browser::navigated", &json!({ "session_id": "b1" })),
            Some(spec(
                "browser::on-navigated",
                None,
                Some(json!({ "input.session_id": { "$eq": "b1" } }))
            ))
        );
        assert_eq!(
            tr("kanban:change", &json!({ "ticket_id": "T-1" })),
            Some(spec(
                "kanban::on-change",
                Some(json!({ "ticket_id": "T-1" })),
                None
            ))
        );
        assert_eq!(
            tr("directory::skills::on-change", &json!({})),
            Some(spec("directory::skills::on-change", None, None))
        );
        assert_eq!(
            tr("claude::agent-event", &json!({ "session_id": "s" })),
            Some(spec(
                "claude::on-agent-event",
                Some(json!({ "session_id": "s" })),
                None
            ))
        );
    }

    #[test]
    fn harness_types_and_bare_names_do_not_translate() {
        assert_eq!(tr("harness::turn-completed", &json!({})), None);
        assert_eq!(tr("webhook", &json!({})), None);
        assert_eq!(tr("timer", &json!({})), None, "a timer without a deadline");
    }

    /// The stored config is the agent's raw request: `in_ms` counts from the
    /// record's creation.
    #[test]
    fn a_relative_timer_counts_from_the_records_creation() {
        assert_eq!(
            tr("timer", &json!({ "in_ms": 600_000 })),
            Some(spec(
                "harness::on-timer",
                Some(json!({ "at": CREATED_AT + 600_000 })),
                None
            ))
        );
    }

    #[test]
    fn the_fixed_names_translate() {
        assert_eq!(
            tr(
                "http",
                &json!({ "method": "POST", "path": "/hook", "metadata": { "x": 1 } })
            ),
            Some(spec(
                "http::on-request",
                Some(json!({ "method": "POST", "path": "/hook" })),
                None
            ))
        );
        // As 0.25 stored it: `api_path` and `http_method`, which defaulted to GET.
        assert_eq!(
            tr(
                "http",
                &json!({ "api_path": "hook", "middleware_function_ids": ["mw::auth"] })
            ),
            Some(spec(
                "http::on-request",
                Some(
                    json!({ "method": "GET", "path": "hook", "middleware_function_ids": ["mw::auth"] })
                ),
                None
            ))
        );
        assert_eq!(
            tr(
                "http",
                &json!({ "api_path": "/hook", "http_method": "POST" })
            ),
            Some(spec(
                "http::on-request",
                Some(json!({ "method": "POST", "path": "/hook" })),
                None
            ))
        );
        assert_eq!(
            tr("console:assets", &json!({})),
            Some(spec("console::ui::on-assets-changed", None, None))
        );
        for asset in ["console:script", "console:style", "console:module"] {
            assert_eq!(
                tr(
                    asset,
                    &json!({ "path": "demo/page.js", "metadata": { "x": 1 } })
                ),
                None,
                "{asset}"
            );
            assert_eq!(tr(asset, &json!({})), None, "{asset} without a path");
        }
    }

    #[test]
    fn a_configuration_binding_keeps_its_event_types() {
        assert_eq!(
            tr(
                "configuration",
                &json!({ "configuration_id": "harness", "event_types": ["configuration:updated"] })
            ),
            Some(spec(
                "configuration::on-updated",
                None,
                Some(json!({
                    "input.id": { "$eq": "harness" },
                    "input.event_type": { "$eq": "configuration:updated" },
                }))
            ))
        );
        assert_eq!(
            tr(
                "configuration",
                &json!({ "event_types": ["configuration:registered", "configuration:deleted"] })
            ),
            Some(spec(
                "configuration::on-updated",
                None,
                Some(json!({
                    "input.event_type": {
                        "$in": ["configuration:registered", "configuration:deleted"]
                    },
                }))
            ))
        );
        assert_eq!(
            tr("configuration", &json!({ "configuration_id": "harness" })),
            Some(spec(
                "configuration::on-updated",
                None,
                Some(json!({ "input.id": { "$eq": "harness" } }))
            )),
            "no event_types: every event type"
        );
    }

    #[test]
    fn session_metadata_and_roles_become_filters() {
        assert_eq!(
            tr(
                "session::message-added",
                &json!({
                    "session_id": "s1",
                    "roles": ["assistant", "user"],
                    "metadata": { "tenant": "acme", "tags": ["a"] },
                })
            ),
            Some(spec(
                "session::on-message-added",
                None,
                Some(json!({
                    "input.session_id": { "$eq": "s1" },
                    "input.message.role": { "$in": ["assistant", "user"] },
                    "input.session_metadata.tenant": { "$eq": "acme" },
                    "input.session_metadata.tags": { "$eq": ["a"] },
                }))
            ))
        );
        assert_eq!(
            tr(
                "approval::pending-created",
                &json!({ "metadata": { "tenant": "acme" } })
            ),
            Some(spec(
                "approval::on-pending-created",
                None,
                Some(json!({ "input.session_metadata.tenant": { "$eq": "acme" } }))
            ))
        );
        // Another worker's `metadata` steered only the old registration.
        assert_eq!(
            tr(
                "browser::navigated",
                &json!({ "session_id": "b1", "metadata": { "x": 1 } })
            ),
            Some(spec(
                "browser::on-navigated",
                None,
                Some(json!({ "input.session_id": { "$eq": "b1" } }))
            ))
        );
    }

    #[test]
    fn a_worker_binding_observes_worker_presence() {
        assert_eq!(
            tr(
                "worker",
                &json!({ "operations": ["add"], "stages": ["ready"] })
            ),
            Some(spec(
                "engine::on-workers-changed",
                None,
                Some(json!({ "input.event": { "$in": ["worker_connected"] } }))
            ))
        );
        let both = Some(spec(
            "engine::on-workers-changed",
            None,
            Some(json!({
                "input.event": { "$in": ["worker_connected", "worker_disconnected"] }
            })),
        ));
        assert_eq!(
            tr("worker", &json!({ "operations": ["add", "remove"] })),
            both
        );
        assert_eq!(tr("worker", &json!({})), both, "no operations: both");
    }

    fn stored(dedup_key: Value) -> Value {
        json!({
            "id": "sub_1",
            "trigger_id": "sdk:sub_1",
            "owner": { "session_id": "s1" },
            "target": { "function_id": "harness::send" },
            "lifecycle": { "once": true },
            "causation": { "depth": 0 },
            "dedup_key": dedup_key,
            "fires": 0,
            "created_at": 7
        })
    }

    #[test]
    fn a_stored_record_is_rewritten_with_its_source() {
        let raw = stored(json!({
            "trigger_type": "state",
            "config": { "scope": "run", "key": "go" },
            "label": "go time",
            "metadata": { "action": "the go key was written" },
        }));
        let Upgrade::Rewrite { next } = upgrade(&raw) else {
            panic!("a translatable record is rewritten");
        };
        assert!(
            next.observer_id.is_none(),
            "the replay creates the observer"
        );
        assert_eq!(
            next.source.as_ref().unwrap().function_id,
            "state::on-changed"
        );
        assert_eq!(next.event_action(), Some("the go key was written"));
        assert_eq!(next.dedup_key.as_ref().unwrap()["label"], json!("go time"));
        let encoded = serde_json::to_value(&next).unwrap();
        assert!(encoded.get("trigger_id").is_none());
        assert!(encoded.get("causation").is_none());
        assert!(matches!(upgrade(&encoded), Upgrade::Current(_)));
    }

    #[test]
    fn a_stored_relative_timer_is_rewritten_to_its_absolute_deadline() {
        let raw = stored(json!({
            "trigger_type": "timer",
            "config": { "in_ms": 5_000 },
            "lifecycle": { "once": true },
        }));
        let Upgrade::Rewrite { next } = upgrade(&raw) else {
            panic!("an in_ms timer translates");
        };
        let source = next.source.as_ref().unwrap();
        assert_eq!(source.function_id, "harness::on-timer");
        assert_eq!(source.args, Some(json!({ "at": 7 + 5_000 })));
        assert_eq!(
            next.dedup_key.as_ref().unwrap()["args"],
            json!({ "at": 5_007 })
        );
    }

    /// A stored console asset binding (here a wake) becomes no observer: the
    /// ADE would call its target for the asset's content on every boot,
    /// firing the binding each time. The owner is told what to observe for
    /// asset changes.
    #[test]
    fn a_stored_console_asset_binding_is_retired_with_an_asset_notice() {
        for (asset, path) in [
            ("console:script", "demo/page.js"),
            ("console:style", "demo/page.css"),
            ("console:module", "demo/chart.js"),
        ] {
            let raw = stored(json!({
                "trigger_type": asset,
                "config": { "path": path },
            }));
            let Upgrade::Retire {
                binding,
                watched,
                why,
            } = upgrade(&raw)
            else {
                panic!("{asset}: a console asset binding is retired");
            };
            assert!(watched.contains(asset), "{watched}");
            assert!(watched.contains(path), "{watched}");
            let notice = retirement_notice(&binding, &watched, why);
            assert!(notice.contains("RETIRED"), "{notice}");
            assert!(notice.contains("console::ui::asset"), "{notice}");
            assert!(
                notice.contains("console::ui::on-assets-changed"),
                "{notice}"
            );
            assert!(!notice.contains("engine::functions::list"), "{notice}");
        }
    }

    #[test]
    fn a_timer_without_a_deadline_is_retired_with_a_timer_notice() {
        let raw = stored(json!({ "trigger_type": "timer", "config": {} }));
        let Upgrade::Retire {
            binding,
            watched,
            why,
        } = upgrade(&raw)
        else {
            panic!("a timer without at or in_ms has no deadline");
        };
        let notice = retirement_notice(&binding, &watched, why);
        assert!(notice.contains("neither `at` nor `in_ms`"), "{notice}");
        assert!(notice.contains("harness::on-timer"), "{notice}");
        assert!(!notice.contains("engine::functions::list"), "{notice}");
    }

    #[test]
    fn an_untranslatable_record_is_retired() {
        let raw = stored(json!({ "trigger_type": "harness::turn-completed", "config": {} }));
        let Upgrade::Retire { watched, why, .. } = upgrade(&raw) else {
            panic!("a harness type has nothing to observe");
        };
        assert!(watched.contains("harness::turn-completed"));
        assert!(why.contains("harness::on-timer"));
        let Upgrade::Retire { why, .. } =
            upgrade(&stored(json!({ "trigger_type": "webhook", "config": {} })))
        else {
            panic!("a bare unknown type has nothing to observe");
        };
        assert!(why.contains("engine::functions::list"));
        let Upgrade::Retire { why, .. } = upgrade(&stored(
            json!({ "trigger_type": "console:style", "config": {} }),
        )) else {
            panic!("a console asset without a path is retired too");
        };
        assert!(why.contains("console::ui::on-assets-changed"), "{why}");
        let keyless = stored(Value::Null);
        assert!(matches!(upgrade(&keyless), Upgrade::Retire { .. }));
        assert!(matches!(
            upgrade(&json!({ "nope": 1 })),
            Upgrade::Unreadable
        ));
    }
}
