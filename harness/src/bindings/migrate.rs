//! Binding records written before 0.26 name what they watch as a trigger
//! type and its config (`dedup_key.trigger_type` / `dedup_key.config`), and
//! carry a `trigger_id`. The boot pass rewrites each one into an observer
//! binding: the type and config become a [`SourceSpec`] through the table in
//! spec §11 and the workers contract (`W::X` → `W::on-X`), the config becomes
//! the source's args when the function takes args and `$eq` filters on
//! `input.<field>` otherwise. A record that does not translate is retired,
//! and its owner is told what to observe instead.
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

/// What a stored `(trigger_type, config)` observes now. `None` when the type
/// has no observable function an agent may observe.
pub fn translate(trigger_type: &str, config: &Value) -> Option<SourceSpec> {
    let source = |function_id: &str, args: Option<Value>, filters: Option<Value>| SourceSpec {
        function_id: function_id.to_string(),
        namespace: None,
        args,
        filters,
    };
    let spec = match trigger_type {
        "timer" => source(
            crate::timer::ON_TIMER_FN,
            Some(json!({ "at": config.get("at")?.as_i64()? })),
            None,
        ),
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
            eq_filters(config, &[("configuration_id", "input.id")]),
        ),
        "engine::functions-available" => source("engine::on-functions-changed", None, None),
        "engine::workers-available" => source("engine::on-workers-changed", None, None),
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
                source(&function_id, None, eq_filters(config, &fields))
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
    /// Nothing observable replaces what it watched: retire it.
    Retire { binding: Binding, watched: String },
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
        .and_then(|trigger_type| translate(trigger_type, &config))
    else {
        let watched = match trigger_type {
            Some(trigger_type) => format!("`{trigger_type}` {config}"),
            None => "an unrecorded source".to_string(),
        };
        return Upgrade::Retire { binding, watched };
    };
    // The observer is created by the replay that follows.
    binding.observer_id = None;
    binding.dedup_key = Some(rewritten_key(&stored, &source));
    binding.source = Some(source);
    Upgrade::Rewrite { next: binding }
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
            Upgrade::Retire { binding, watched } => {
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
                        notify_retired(deps, &binding, &watched).await;
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

/// Tell the owner its binding is gone and what to observe instead.
async fn notify_retired(deps: &Deps, binding: &Binding, watched: &str) {
    let notice = format!(
        "[notification] binding {} (was watching {watched}) was RETIRED: nothing observable \
         replaces what it watched. Find the function that emits the event with \
         engine::functions::list and observe it with harness::observe.",
        binding.id
    );
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
            Some("nothing observable replaces what this binding watched"),
            crate::subscriptions::fired::now_ms(),
        ),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

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
            translate("timer", &json!({ "at": 1_700_000_000_000_i64 })),
            Some(spec(
                "harness::on-timer",
                Some(json!({ "at": 1_700_000_000_000_i64 })),
                None
            ))
        );
        assert_eq!(
            translate("cron", &json!({ "expression": "0 * * * * *" })),
            Some(spec(
                "cron::on-tick",
                Some(json!({ "expression": "0 * * * * *" })),
                None
            ))
        );
        assert_eq!(
            translate("state", &json!({ "scope": "run", "key": "go" })),
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
            translate(
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
            translate("engine::workers-available", &json!({})),
            Some(spec("engine::on-workers-changed", None, None))
        );
    }

    #[test]
    fn a_worker_type_becomes_its_on_function() {
        assert_eq!(
            translate("browser::navigated", &json!({ "session_id": "b1" })),
            Some(spec(
                "browser::on-navigated",
                None,
                Some(json!({ "input.session_id": { "$eq": "b1" } }))
            ))
        );
        assert_eq!(
            translate("kanban:change", &json!({ "ticket_id": "T-1" })),
            Some(spec(
                "kanban::on-change",
                Some(json!({ "ticket_id": "T-1" })),
                None
            ))
        );
        assert_eq!(
            translate("directory::skills::on-change", &json!({})),
            Some(spec("directory::skills::on-change", None, None))
        );
        assert_eq!(
            translate("claude::agent-event", &json!({ "session_id": "s" })),
            Some(spec(
                "claude::on-agent-event",
                Some(json!({ "session_id": "s" })),
                None
            ))
        );
    }

    #[test]
    fn harness_types_and_bare_names_do_not_translate() {
        assert_eq!(translate("harness::turn-completed", &json!({})), None);
        assert_eq!(translate("webhook", &json!({})), None);
        assert_eq!(translate("timer", &json!({ "in_ms": 5 })), None);
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
    fn an_untranslatable_record_is_retired() {
        let raw = stored(json!({ "trigger_type": "harness::turn-completed", "config": {} }));
        let Upgrade::Retire { watched, .. } = upgrade(&raw) else {
            panic!("a harness type has nothing to observe");
        };
        assert!(watched.contains("harness::turn-completed"));
        let keyless = stored(Value::Null);
        assert!(matches!(upgrade(&keyless), Upgrade::Retire { .. }));
        assert!(matches!(
            upgrade(&json!({ "nope": 1 })),
            Upgrade::Unreadable
        ));
    }
}
