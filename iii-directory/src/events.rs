//! The change events this worker hosts.
//!
//! Each is a private function whose handler returns the event it is called
//! with. The worker calls it when the change happens, and its observers
//! receive the event:
//!
//! - `directory::skills::on-change`: after every successful
//!   `directory::skills::download` that wrote at least one skill markdown
//!   file, a `directory::skills::update`, `create`, or `delete`, and a kit
//!   apply that wrote skills (`kit`) or downloaded the bundles of the
//!   workers it added or updated (`download`). External edits under the
//!   read-only `agents_skills_folder` call it too, through the fs watcher
//!   (doorbell only: the worker never writes there).
//! - `directory::system-prompts::on-change`: after every successful
//!   `directory::skills::download` that wrote at least one system prompt, a
//!   `directory::system-prompts::update`, or a
//!   `directory::system-prompts::create` / `delete`.
//! - `directory::agents::on-change`: after every successful download
//!   (a kit apply's worker bundles included), kit apply that wrote profiles,
//!   or direct create, update, delete, or external edit of an agent profile.
//! - `directory::kits::on-change`: when a kit plan is created or discarded,
//!   while an apply runs (step progress) and when it lands, and when an
//!   update check changes the set of available kit updates.
//!
//! None takes args: an observer selects changes with filters on the event
//! (`input.op`, `input.name`, `input.plan_id`). The `mcp` worker, the
//! harness and the directory's own console page observe them:
//!
//! ```text
//! iii.observe(Source::function("directory::skills::on-change").deliver(Deliver::Output),
//!             "mcp::__on_skills_changed")
//! ```
//!
//! The coupling stays one-way: an observer knows the directory worker hosts
//! `directory::skills::on-change`; the directory worker never has to know
//! who observes it.

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SKILLS_ON_CHANGE: &str = "directory::skills::on-change";
pub const SYSTEM_PROMPTS_ON_CHANGE: &str = "directory::system-prompts::on-change";
pub const AGENTS_ON_CHANGE: &str = "directory::agents::on-change";
pub const KITS_ON_CHANGE: &str = "directory::kits::on-change";

/// A change to the skills, system prompts or agent profiles this worker
/// manages: what `directory::skills::on-change`,
/// `directory::system-prompts::on-change` and `directory::agents::on-change`
/// observers receive.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangeEvent {
    /// `create`, `update`, `delete` (one file), `download` (a bundle),
    /// `kit` (a kit apply) or `external` (an edit on disk outside this
    /// worker).
    pub op: String,
    /// `create`, `update`, `delete`: the edited skill (its id), system
    /// prompt or agent profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `create`, `update`, `delete` of a skill: its id, the same value as
    /// `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The namespace written: skill edits, and a `download` by
    /// `directory::skills::download`. A kit apply downloads the bundles of
    /// the workers it added or updated, one namespace each, and its
    /// `download` carries none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// `download`: `repo` or `registry` (`registry` for a kit apply's
    /// worker bundles).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `kit`: the kit applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit: Option<String>,
    /// `kit`: the applied plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
}

/// A change to kit state: what `directory::kits::on-change` observers
/// receive.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KitsChangeEvent {
    /// `plan` and `discard` (a plan created or discarded), `progress` (an
    /// apply's steps so far), `apply` (an apply landed) or `updates` (the set
    /// of available kit updates changed).
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// `plan`, `apply`: `install`, `update` or `remove`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// `progress`: the apply's steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<Value>>,
    /// `progress`: the error that stopped the apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One of this worker's change functions, by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeSource(&'static str);

impl ChangeSource {
    pub fn id(self) -> &'static str {
        self.0
    }
}

/// The four change functions, passed into the write modules (`download`,
/// `update`, `agents`, `kits`) so they call the matching
/// `directory::*::on-change` after a successful write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeSources {
    pub skills: ChangeSource,
    pub system_prompts: ChangeSource,
    pub agents: ChangeSource,
    pub kits: ChangeSource,
}

impl ChangeSources {
    pub const fn new() -> Self {
        Self {
            skills: ChangeSource(SKILLS_ON_CHANGE),
            system_prompts: ChangeSource(SYSTEM_PROMPTS_ON_CHANGE),
            agents: ChangeSource(AGENTS_ON_CHANGE),
            kits: ChangeSource(KITS_ON_CHANGE),
        }
    }
}

impl Default for ChangeSources {
    fn default() -> Self {
        Self::new()
    }
}

/// Call `source` with `payload` as a fire-and-forget `CallAction::Void`, so
/// the mutation that produced the change is not held by its observers. The
/// call is skipped when nothing observes the function (one atomic load), and
/// a failure is logged and swallowed: an observer must not break the write
/// path.
pub async fn emit(iii: &IIIClient, source: ChangeSource, payload: Value) {
    if !iii.is_observed(source.id()) {
        return;
    }
    let request = CallRequest {
        function_id: source.id().to_string(),
        payload,
        action: Some(CallAction::Void),
        timeout_ms: None,
    };
    if let Err(e) = iii.call(request).await {
        tracing::warn!(
            function_id = source.id(),
            error = %e,
            "on-change call failed"
        );
    }
}

/// Register the four change functions. Each is private (only this worker
/// calls it), takes no args, and returns the event it is called with.
pub fn register_all(iii: &IIIClient) -> ChangeSources {
    let sources = ChangeSources::new();

    iii.register_function(
        SKILLS_ON_CHANGE,
        RegisterFunction::new(|event: ChangeEvent| Ok::<_, Error>(event))
            .description(
                "Called after a directory::skills::download that wrote at least one skill \
                 markdown file, or a directory::skills::update, create, or delete. Also called \
                 with { op: \"external\" } when a watched skills root changes on disk outside \
                 this worker, including the read-only agents skills root when that root exists \
                 at startup. Observe it; filter on input.op, input.name (the skill id) or \
                 input.namespace.",
            )
            .private(),
    );

    iii.register_function(
        SYSTEM_PROMPTS_ON_CHANGE,
        RegisterFunction::new(|event: ChangeEvent| Ok::<_, Error>(event))
            .description(
                "Called after a directory::skills::download that wrote at least one system \
                 prompt, or a directory::system-prompts::update, create, or delete. Also called \
                 with { op: \"external\" } when a watched system-prompts root changes on disk \
                 outside this worker. Observe it; filter on input.op or input.name.",
            )
            .private(),
    );

    iii.register_function(
        AGENTS_ON_CHANGE,
        RegisterFunction::new(|event: ChangeEvent| Ok::<_, Error>(event))
            .description(
                "Called after a directory::skills::download that wrote at least one agent \
                 profile, or a directory::agents::update, create, or delete. Also called with \
                 { op: \"external\" } when a watched agents file changes on disk outside this \
                 worker. Observe it; filter on input.op or input.name.",
            )
            .private(),
    );

    iii.register_function(
        KITS_ON_CHANGE,
        RegisterFunction::new(|event: KitsChangeEvent| Ok::<_, Error>(event))
            .description(
                "Called when kit state changes: a plan is created or discarded \
                 ({ op: \"plan\" | \"discard\", kit, plan_id }), an apply makes progress or \
                 finishes ({ op: \"progress\", plan_id, steps } / { op: \"apply\", kit, plan_id }), \
                 or an update check finds a different set of available updates \
                 ({ op: \"updates\" }). Observe it; filter on input.op or input.plan_id.",
            )
            .private(),
    );

    tracing::info!(
        functions = ?[
            SKILLS_ON_CHANGE,
            SYSTEM_PROMPTS_ON_CHANGE,
            AGENTS_ON_CHANGE,
            KITS_ON_CHANGE
        ],
        "registered change functions"
    );
    sources
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sources_name_their_functions() {
        let sources = ChangeSources::new();
        assert_eq!(sources.skills.id(), SKILLS_ON_CHANGE);
        assert_eq!(sources.system_prompts.id(), SYSTEM_PROMPTS_ON_CHANGE);
        assert_eq!(sources.agents.id(), AGENTS_ON_CHANGE);
        assert_eq!(sources.kits.id(), KITS_ON_CHANGE);
    }

    #[test]
    fn change_events_round_trip_every_emitted_shape() {
        for payload in [
            json!({ "op": "create", "name": "reviewer" }),
            json!({ "op": "update", "namespace": "iii", "id": "orders/create" }),
            json!({ "op": "download", "namespace": "iii", "source": "registry" }),
            // A kit apply's worker bundles download: no single namespace.
            json!({ "op": "download", "source": "registry" }),
            json!({ "op": "kit", "kit": "deepwork", "plan_id": "p-1" }),
            json!({ "op": "external" }),
        ] {
            let event: ChangeEvent = serde_json::from_value(payload.clone()).unwrap();
            assert_eq!(serde_json::to_value(event).unwrap(), payload);
        }
    }

    #[test]
    fn kits_change_events_round_trip_every_emitted_shape() {
        for payload in [
            json!({ "op": "plan", "kit": "deepwork", "plan_id": "p-1", "kind": "install" }),
            json!({ "op": "discard", "plan_id": "p-1", "kit": "deepwork" }),
            json!({ "op": "progress", "plan_id": "p-1", "kit": "deepwork", "steps": [{ "id": "s" }] }),
            json!({ "op": "apply", "kit": "deepwork", "plan_id": "p-1", "kind": "update" }),
            json!({ "op": "updates" }),
        ] {
            let event: KitsChangeEvent = serde_json::from_value(payload.clone()).unwrap();
            assert_eq!(serde_json::to_value(event).unwrap(), payload);
        }
    }
}
