use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::clients::FunctionDescriptor;
use crate::policy::CompiledPolicy;
use crate::types::turn::{SkillAck, SkillContext};

pub(crate) const SKILLS_GET_ID: &str = "directory::skills::get";
const SKILLS_LIST_ID: &str = "directory::skills::list";
const SKILLS_CHANGE_FN_ID: &str = "harness::on-skills-change";
const SKILLS_CHANGE_TRIGGER: &str = "directory::skills::on-change";
const SAFETY_RELOAD_SECS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Skill {
    pub id: String,
    description: String,
}

#[derive(Debug)]
pub struct SkillsSnapshot {
    pub(crate) skills: Vec<Skill>,
    pub(crate) generation: u64,
    fingerprint: Option<String>,
}

impl SkillsSnapshot {
    pub fn available(&self) -> bool {
        self.fingerprint.is_some()
    }
}

struct SkillsState {
    snapshot: RwLock<Arc<SkillsSnapshot>>,
    reload: Mutex<()>,
}

#[derive(Clone)]
pub struct SkillsCell(Arc<SkillsState>);

impl SkillsCell {
    pub async fn read(&self) -> RwLockReadGuard<'_, Arc<SkillsSnapshot>> {
        self.0.snapshot.read().await
    }

    async fn write(&self) -> RwLockWriteGuard<'_, Arc<SkillsSnapshot>> {
        self.0.snapshot.write().await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedView {
    pub body: String,
    pub fingerprint: String,
    pub generation: u64,
    pub unknown: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EffectiveView {
    Unavailable,
    Removed { generation: u64 },
    Available(RenderedView),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SyncPlan {
    None,
    Acknowledge(SkillAck),
    FreezeBaseline { body: String, ack: SkillAck },
    Append { message: String, ack: SkillAck },
}

fn view_ack(view: &EffectiveView) -> Option<SkillAck> {
    match view {
        EffectiveView::Unavailable => None,
        EffectiveView::Removed { generation } => Some(SkillAck {
            generation: *generation,
            fingerprint: None,
        }),
        EffectiveView::Available(view) => Some(SkillAck {
            generation: view.generation,
            fingerprint: Some(view.fingerprint.clone()),
        }),
    }
}

pub(crate) fn plan_sync(
    previous: Option<&SkillAck>,
    started: bool,
    view: &EffectiveView,
) -> SyncPlan {
    let Some(next) = view_ack(view) else {
        return SyncPlan::None;
    };
    if previous.is_some_and(|previous| previous == &next) {
        return SyncPlan::None;
    }
    if previous.is_some_and(|previous| previous.fingerprint == next.fingerprint) {
        return SyncPlan::Acknowledge(next);
    }
    match (previous, started, view) {
        (None, false, EffectiveView::Available(view)) => SyncPlan::FreezeBaseline {
            body: view.body.clone(),
            ack: next,
        },
        (None, _, EffectiveView::Removed { .. }) => SyncPlan::Acknowledge(next),
        (_, _, EffectiveView::Available(view)) => SyncPlan::Append {
            message: format!(
                "The available skills have changed. This list supersedes the previous\navailable skills list.\n{}",
                view.body
            ),
            ack: next,
        },
        (_, _, EffectiveView::Removed { .. }) => SyncPlan::Append {
            message: "Skill guidance is no longer available. Do not use any previously listed skill."
                .to_string(),
            ack: next,
        },
        (_, _, EffectiveView::Unavailable) => SyncPlan::None,
    }
}

fn canonical_filter(filter: Option<&[String]>) -> Option<Vec<String>> {
    let mut filter = filter?.to_vec();
    if filter.is_empty() {
        return None;
    }
    filter.sort();
    filter.dedup();
    Some(filter)
}

pub(crate) fn new_context(requested: Option<&[String]>, view: &EffectiveView) -> SkillContext {
    SkillContext {
        filter: canonical_filter(requested),
        baseline: match view {
            EffectiveView::Available(view) => Some(view.body.clone()),
            EffectiveView::Unavailable | EffectiveView::Removed { .. } => None,
        },
    }
}

pub(crate) fn next_context(previous: &SkillContext, requested: Option<&[String]>) -> SkillContext {
    match requested {
        None => previous.clone(),
        Some(requested) => SkillContext {
            filter: canonical_filter(Some(requested)),
            baseline: previous.baseline.clone(),
        },
    }
}

#[derive(Deserialize)]
struct ListOutput {
    skills: Vec<ListRow>,
}

#[derive(Deserialize)]
struct ListRow {
    id: String,
    title: String,
    description: String,
    #[serde(default)]
    disable_model_invocation: bool,
}

pub fn new_cell() -> SkillsCell {
    SkillsCell(Arc::new(SkillsState {
        snapshot: RwLock::new(Arc::new(SkillsSnapshot {
            skills: Vec::new(),
            generation: 0,
            fingerprint: None,
        })),
        reload: Mutex::new(()),
    }))
}

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sentence(value: &str) -> String {
    if value.ends_with(['.', '!', '?']) {
        value.to_string()
    } else {
        format!("{value}.")
    }
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn digest(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

pub(crate) fn fingerprint(value: &str) -> String {
    digest(value)
}

pub(crate) fn attribution<'a>(
    context: Option<&'a SkillContext>,
    legacy: Option<&'a str>,
) -> Option<&'a str> {
    match context {
        Some(context) => context.baseline.as_deref().filter(|body| !body.is_empty()),
        None => legacy.filter(|body| !body.is_empty()),
    }
}

pub(crate) fn refresh_filter(local: &mut Option<SkillContext>, durable: Option<&SkillContext>) {
    if let (Some(local), Some(durable)) = (local.as_mut(), durable) {
        local.filter = durable.filter.clone();
    }
}

fn parse_observation(value: &Value) -> Result<Vec<Skill>, String> {
    let output: ListOutput = serde_json::from_value(value.clone())
        .map_err(|error| format!("malformed directory::skills::list response: {error}"))?;
    let mut seen = HashSet::new();
    let mut skills = Vec::new();
    for row in output.skills {
        if row.id.trim().is_empty() {
            return Err("directory::skills::list returned a blank skill id".to_string());
        }
        if !seen.insert(row.id.clone()) {
            return Err(format!(
                "directory::skills::list returned duplicate skill id {:?}",
                row.id
            ));
        }
        if row.disable_model_invocation {
            continue;
        }
        let title = normalize(&row.title);
        let description = normalize(&row.description);
        skills.push(Skill {
            description: if !description.is_empty() {
                description
            } else if !title.is_empty() {
                title
            } else {
                row.id.clone()
            },
            id: row.id,
        });
    }
    skills.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(skills)
}

fn render(skills: &[Skill], filter: Option<&[String]>) -> RenderedView {
    let selected: Option<HashSet<&str>> = filter
        .filter(|ids| !ids.is_empty())
        .map(|ids| ids.iter().map(String::as_str).collect());
    let mut body = String::from(
        "<available_skills>\nCall `directory::skills::get` with an exact id before following a skill.",
    );
    for skill in skills.iter().filter(|skill| {
        selected
            .as_ref()
            .is_none_or(|ids| ids.contains(skill.id.as_str()))
    }) {
        body.push_str("\n- **");
        body.push_str(&escape(&skill.id));
        body.push_str("** — ");
        body.push_str(&sentence(&escape(&skill.description)));
    }
    body.push_str("\n</available_skills>");

    let known: HashSet<&str> = skills.iter().map(|skill| skill.id.as_str()).collect();
    let unknown = filter
        .filter(|ids| !ids.is_empty())
        .into_iter()
        .flatten()
        .filter(|id| !known.contains(id.as_str()))
        .cloned()
        .collect();
    RenderedView {
        fingerprint: digest(&body),
        body,
        generation: 0,
        unknown,
    }
}

#[cfg(test)]
fn snapshot(skills: Vec<Skill>, generation: u64) -> SkillsSnapshot {
    let fingerprint = Some(render(&skills, None).fingerprint);
    SkillsSnapshot {
        skills,
        generation,
        fingerprint,
    }
}

pub(crate) fn effective_view(
    snapshot: &SkillsSnapshot,
    filter: Option<&[String]>,
    policy: &CompiledPolicy,
    functions: &[FunctionDescriptor],
) -> EffectiveView {
    if !policy.allows(SKILLS_GET_ID)
        || !functions
            .iter()
            .any(|function| function.function_id == SKILLS_GET_ID)
    {
        return EffectiveView::Removed {
            generation: snapshot.generation,
        };
    }
    if !snapshot.available() {
        return EffectiveView::Unavailable;
    }
    let mut rendered = render(&snapshot.skills, filter);
    rendered.generation = snapshot.generation;
    for id in &rendered.unknown {
        tracing::warn!(skill_id = %id, "requested skill id is not currently available; omitting it from the model index");
    }
    let has_rows = rendered.body.lines().any(|line| line.starts_with("- **"));
    if has_rows {
        EffectiveView::Available(rendered)
    } else {
        EffectiveView::Removed {
            generation: snapshot.generation,
        }
    }
}

async fn admit(cell: &SkillsCell, value: Value) -> Result<usize, String> {
    let skills = parse_observation(&value)?;
    let fingerprint = render(&skills, None).fingerprint;
    let count = skills.len();
    let mut guard = cell.write().await;
    if guard.fingerprint.as_deref() == Some(&fingerprint) {
        return Ok(count);
    }
    *guard = Arc::new(SkillsSnapshot {
        skills,
        generation: guard.generation + 1,
        fingerprint: Some(fingerprint),
    });
    Ok(count)
}

#[derive(Clone)]
struct DirectoryClient {
    iii: Arc<IIIClient>,
    timeout_ms: u64,
}

impl DirectoryClient {
    async fn skills_list(&self) -> Result<Value, String> {
        self.iii
            .trigger(TriggerRequest {
                function_id: SKILLS_LIST_ID.to_string(),
                payload: json!({ "include_description": true }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .map_err(|error| format!("{SKILLS_LIST_ID}: {error}"))
    }
}

async fn reload(iii: &Arc<IIIClient>, cell: &SkillsCell, timeout_ms: u64) -> Option<usize> {
    let client = DirectoryClient {
        iii: iii.clone(),
        timeout_ms,
    };
    reload_with(cell, || client.skills_list()).await
}

async fn reload_with<F, Fut>(cell: &SkillsCell, read: F) -> Option<usize>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let _reload = cell.0.reload.lock().await;
    let result = match read().await {
        Ok(value) => admit(cell, value).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(count) => Some(count),
        Err(error) => {
            tracing::warn!(error = %error, "skill catalog reload failed; preserving the last admitted snapshot");
            None
        }
    }
}

pub async fn seed(iii: &Arc<IIIClient>, cell: &SkillsCell, timeout_ms: u64) {
    if let Some(count) = reload(iii, cell, timeout_ms).await {
        tracing::info!(count, "seeded skill catalog cache");
    }
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub(crate) struct OnSkillsChangeEvent {
    #[serde(default)]
    op: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub(crate) struct OnSkillsChangeResponse {
    ok: bool,
}

pub fn register_trigger(iii: &Arc<IIIClient>, cell: SkillsCell, timeout_ms: u64) {
    let reload_iii = iii.clone();
    let reload_cell = cell.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(SAFETY_RELOAD_SECS));
        ticker.tick().await;
        loop {
            ticker.tick().await;
            reload(&reload_iii, &reload_cell, timeout_ms).await;
        }
    });

    let handler_iii = iii.clone();
    iii.register_function(
        SKILLS_CHANGE_FN_ID,
        RegisterFunction::new_async(move |event: OnSkillsChangeEvent| {
            let iii = handler_iii.clone();
            let cell = cell.clone();
            async move {
                let _ = event.op;
                reload(&iii, &cell, timeout_ms).await;
                Ok::<_, Error>(OnSkillsChangeResponse { ok: true })
            }
        })
        .description("Internal: refresh the cached model-invocable skill catalog.")
        .metadata(json!({ "internal": true })),
    );

    if let Err(error) = iii.register_trigger(RegisterTriggerInput::new(
        SKILLS_CHANGE_TRIGGER.to_string(),
        SKILLS_CHANGE_FN_ID.to_string(),
        json!({}),
    )) {
        tracing::warn!(error = %error, "binding directory::skills::on-change failed; relying on safety reloads");
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::clients::FunctionDescriptor;
    use crate::policy::CompiledPolicy;
    use crate::types::turn::FunctionPolicy;
    use crate::types::turn::{SkillAck, SkillContext};

    fn policy(allow: &[&str]) -> CompiledPolicy {
        CompiledPolicy::from(Some(&FunctionPolicy {
            allow: allow.iter().map(|value| (*value).to_string()).collect(),
            deny: Vec::new(),
            expose: Default::default(),
        }))
    }

    fn functions(ids: &[&str]) -> Vec<FunctionDescriptor> {
        ids.iter()
            .map(|id| FunctionDescriptor {
                function_id: (*id).to_string(),
                description: None,
                parameters: None,
            })
            .collect()
    }

    fn catalog(id: &str, description: &str) -> Value {
        json!({"skills": [{
            "id": id,
            "title": id,
            "description": description,
            "disable_model_invocation": false
        }]})
    }

    #[tokio::test]
    async fn concurrent_reloads_observe_and_admit_in_request_order() {
        let cell = new_cell();
        let (first_started_tx, first_started_rx) = tokio::sync::oneshot::channel();
        let (release_first_tx, release_first_rx) = tokio::sync::oneshot::channel();
        let first_cell = cell.clone();
        let first = tokio::spawn(async move {
            reload_with(&first_cell, || async move {
                first_started_tx.send(()).unwrap();
                release_first_rx.await.unwrap();
                Ok(catalog("older", "Older observation"))
            })
            .await
        });
        first_started_rx.await.unwrap();

        let (second_ready_tx, second_ready_rx) = tokio::sync::oneshot::channel();
        let (second_started_tx, mut second_started_rx) = tokio::sync::oneshot::channel();
        let second_cell = cell.clone();
        let second = tokio::spawn(async move {
            second_ready_tx.send(()).unwrap();
            reload_with(&second_cell, || async move {
                second_started_tx.send(()).unwrap();
                Ok(catalog("newer", "Newer observation"))
            })
            .await
        });
        second_ready_rx.await.unwrap();
        let blocked = tokio::time::timeout(Duration::from_millis(50), &mut second_started_rx)
            .await
            .is_err();
        assert!(
            blocked,
            "the second remote read must wait for the first admission"
        );

        release_first_tx.send(()).unwrap();
        assert_eq!(first.await.unwrap(), Some(1));
        second_started_rx.await.unwrap();
        assert_eq!(second.await.unwrap(), Some(1));

        let snapshot = cell.read().await.clone();
        assert_eq!(snapshot.generation, 2);
        assert_eq!(snapshot.skills[0].id, "newer");
    }

    #[tokio::test]
    async fn failed_reload_preserves_the_last_good_snapshot() {
        let cell = new_cell();
        assert_eq!(
            reload_with(&cell, || async { Ok(catalog("good", "Last good")) }).await,
            Some(1)
        );
        let admitted = cell.read().await.clone();

        assert_eq!(
            reload_with(&cell, || async { Err("read failed".to_string()) }).await,
            None
        );
        assert_eq!(
            reload_with(&cell, || async {
                Ok(json!({"skills": [{"id": "broken"}]}))
            })
            .await,
            None
        );

        let preserved = cell.read().await.clone();
        assert!(Arc::ptr_eq(&admitted, &preserved));
        assert_eq!(preserved.skills[0].id, "good");
    }

    #[test]
    fn observation_is_strict_and_render_is_canonical() {
        let rows = parse_observation(&json!({
            "skills": [
                {
                    "id": "z<&>", "title": "Zed title", "description": "  Does   zed work  ",
                    "disable_model_invocation": false, "bytes": 999, "modified_at": "later"
                },
                {
                    "id": "alpha", "title": "  Alpha   title ", "description": " \n\t ",
                    "disable_model_invocation": false, "bytes": 1, "modified_at": "earlier"
                },
                {
                    "id": "hidden", "title": "Hidden", "description": "Never advertise",
                    "disable_model_invocation": true, "bytes": 1, "modified_at": "now"
                }
            ]
        }))
        .expect("valid observation");

        let rendered = render(&rows, None);
        assert_eq!(
            rendered.body,
            "<available_skills>\nCall `directory::skills::get` with an exact id before following a skill.\n- **alpha** — Alpha title.\n- **z&lt;&amp;&gt;** — Does zed work.\n</available_skills>"
        );
        assert_eq!(
            rendered.fingerprint,
            "sha256:9e5df1c3393287d207e3099686a953f39dbe41e037e3e8b2cf07963ad08cab56"
        );

        assert!(parse_observation(&json!({"skills": [{
            "id": " ", "title": "x", "description": "x", "disable_model_invocation": false
        }]}))
        .is_err());
        assert!(parse_observation(&json!({"skills": [
            {"id": "same", "title": "x", "description": "x", "disable_model_invocation": false},
            {"id": "same", "title": "y", "description": "y", "disable_model_invocation": false}
        ]}))
        .is_err());
        assert!(parse_observation(&json!({"skills": [{
            "id": "x", "title": "x", "disable_model_invocation": false
        }]}))
        .is_err());
    }

    #[test]
    fn missing_disable_model_invocation_is_enabled_but_a_malformed_value_rejects_the_observation() {
        let rows = parse_observation(&json!({"skills": [{
            "id": "legacy", "title": "Legacy", "description": "Older directory row"
        }]}))
        .expect("rolling-upgrade row without the field remains valid");
        assert_eq!(
            rows,
            vec![Skill {
                id: "legacy".into(),
                description: "Older directory row".into(),
            }]
        );

        assert!(parse_observation(&json!({"skills": [{
            "id": "bad", "title": "Bad", "description": "Bad row",
            "disable_model_invocation": "false"
        }]}))
        .is_err());
    }

    #[tokio::test]
    async fn generation_changes_only_when_the_canonical_index_changes() {
        let cell = new_cell();
        assert!(!cell.read().await.available());

        let first = json!({"skills": [
            {
                "id": "one", "title": "One", "description": "First skill",
                "disable_model_invocation": false, "bytes": 1, "modified_at": "a"
            },
            {
                "id": "two", "title": "Two", "description": "Second skill",
                "disable_model_invocation": false, "bytes": 2, "modified_at": "b"
            }
        ]});
        let first_body = render(&parse_observation(&first).unwrap(), None).body;
        admit(&cell, first).await.unwrap();
        assert_eq!(cell.read().await.generation, 1);

        let noisy_reversed = json!({"skills": [
            {
                "id": "two", "title": "Two", "description": " Second  skill ",
                "disable_model_invocation": false, "bytes": 999, "modified_at": "later"
            },
            {
                "id": "one", "title": "One", "description": " First  skill ",
                "disable_model_invocation": false, "bytes": 998, "modified_at": "earlier"
            }
        ]});
        assert_eq!(
            render(&parse_observation(&noisy_reversed).unwrap(), None).body,
            first_body
        );
        admit(&cell, noisy_reversed).await.unwrap();
        assert_eq!(cell.read().await.generation, 1);

        assert!(admit(&cell, json!({"skills": [{"id": "broken"}]}))
            .await
            .is_err());
        let snapshot = cell.read().await.clone();
        assert_eq!(snapshot.generation, 1);
        assert_eq!(snapshot.skills[0].id, "one");
    }

    #[test]
    fn exact_filter_and_function_policy_define_the_effective_view() {
        let rows = parse_observation(&json!({"skills": [
            {"id": "one", "title": "One", "description": "First", "disable_model_invocation": false},
            {"id": "two", "title": "Two", "description": "Second", "disable_model_invocation": false}
        ]}))
        .unwrap();
        let snapshot = snapshot(rows, 4);
        let registry = functions(&[SKILLS_GET_ID]);

        let view = effective_view(
            &snapshot,
            Some(&["two".to_string(), "unknown".to_string()]),
            &policy(&[SKILLS_GET_ID]),
            &registry,
        );
        let EffectiveView::Available(view) = view else {
            panic!("expected available view");
        };
        assert!(view.body.contains("**two**"));
        assert!(!view.body.contains("**one**"));
        assert_eq!(view.unknown, vec!["unknown"]);

        assert!(matches!(
            effective_view(&snapshot, None, &policy(&[]), &registry),
            EffectiveView::Removed { generation: 4 }
        ));
        assert!(matches!(
            effective_view(&snapshot, None, &policy(&[SKILLS_GET_ID]), &functions(&[])),
            EffectiveView::Removed { generation: 4 }
        ));
    }

    #[test]
    fn sync_plan_preserves_unavailable_and_emits_exact_durable_corrections() {
        assert_eq!(
            plan_sync(None, false, &EffectiveView::Unavailable),
            SyncPlan::None
        );

        let available = EffectiveView::Available(RenderedView {
            body: "<available_skills>\n- **one** — First.\n</available_skills>".into(),
            fingerprint: "sha256:one".into(),
            generation: 2,
            unknown: Vec::new(),
        });
        assert!(matches!(
            plan_sync(None, false, &available),
            SyncPlan::FreezeBaseline { .. }
        ));

        let SyncPlan::Append { message, ack } = plan_sync(None, true, &available) else {
            panic!("late first availability must be explicit");
        };
        assert_eq!(
            message,
            "The available skills have changed. This list supersedes the previous\navailable skills list.\n<available_skills>\n- **one** — First.\n</available_skills>"
        );
        assert_eq!(ack.generation, 2);

        let admitted = SkillAck {
            generation: 2,
            fingerprint: Some("sha256:one".into()),
        };
        let moved_without_view_change = EffectiveView::Available(RenderedView {
            generation: 3,
            ..match available.clone() {
                EffectiveView::Available(view) => view,
                _ => unreachable!(),
            }
        });
        assert_eq!(
            plan_sync(Some(&admitted), true, &moved_without_view_change),
            SyncPlan::Acknowledge(SkillAck {
                generation: 3,
                fingerprint: Some("sha256:one".into())
            })
        );

        let SyncPlan::Append { message, ack } = plan_sync(
            Some(&admitted),
            true,
            &EffectiveView::Removed { generation: 3 },
        ) else {
            panic!("removal must append");
        };
        assert_eq!(
            message,
            "Skill guidance is no longer available. Do not use any previously listed skill."
        );
        assert_eq!(ack.fingerprint, None);

        let removed = SkillAck {
            generation: 3,
            fingerprint: None,
        };
        assert!(matches!(
            plan_sync(Some(&removed), true, &available),
            SyncPlan::Append { .. }
        ));
    }

    #[test]
    fn a_send_time_baseline_change_before_generation_is_still_a_transcript_correction() {
        let baseline = SkillAck {
            generation: 1,
            fingerprint: Some("sha256:baseline".into()),
        };
        let current = EffectiveView::Available(RenderedView {
            body: "<available_skills>current</available_skills>".into(),
            fingerprint: "sha256:current".into(),
            generation: 2,
            unknown: Vec::new(),
        });

        let SyncPlan::Append { message, ack } = plan_sync(Some(&baseline), false, &current) else {
            panic!("a frozen send-time baseline must not be rewritten")
        };
        assert!(message.ends_with("<available_skills>current</available_skills>"));
        assert_eq!(ack.generation, 2);
        assert_eq!(ack.fingerprint.as_deref(), Some("sha256:current"));
    }

    #[test]
    fn context_defaults_to_all_inherits_only_when_omitted_and_never_rewrites_baseline() {
        let view = EffectiveView::Available(RenderedView {
            body: "baseline all".into(),
            fingerprint: "sha256:all".into(),
            generation: 1,
            unknown: Vec::new(),
        });
        assert_eq!(
            new_context(None, &view),
            SkillContext {
                filter: None,
                baseline: Some("baseline all".into())
            }
        );
        assert_eq!(
            new_context(Some(&["two".into(), "one".into(), "two".into()]), &view).filter,
            Some(vec!["one".to_string(), "two".to_string()])
        );

        let previous = SkillContext {
            filter: Some(vec!["one".into()]),
            baseline: Some("frozen first view".into()),
        };
        assert_eq!(next_context(&previous, None), previous);
        assert_eq!(
            next_context(&previous, Some(&[])),
            SkillContext {
                filter: None,
                baseline: Some("frozen first view".into())
            }
        );
        assert_eq!(
            next_context(&previous, Some(&["two".into()])),
            SkillContext {
                filter: Some(vec!["two".into()]),
                baseline: Some("frozen first view".into())
            }
        );
    }

    #[test]
    fn frozen_baseline_is_attributed_separately_from_legacy_bodies() {
        let context = SkillContext {
            filter: None,
            baseline: Some("names only".into()),
        };
        assert_eq!(
            attribution(Some(&context), Some("legacy body")),
            Some("names only")
        );
        assert_eq!(attribution(None, Some("legacy body")), Some("legacy body"));
        assert_eq!(attribution(Some(&SkillContext::default()), None), None);
    }

    #[test]
    fn durable_filter_refresh_preserves_the_in_memory_frozen_baseline() {
        let mut local = Some(SkillContext {
            filter: Some(vec!["old".into()]),
            baseline: Some("generation baseline".into()),
        });
        let durable = SkillContext {
            filter: Some(vec!["new".into()]),
            baseline: Some("must not replace baseline".into()),
        };

        refresh_filter(&mut local, Some(&durable));

        assert_eq!(
            local,
            Some(SkillContext {
                filter: Some(vec!["new".into()]),
                baseline: Some("generation baseline".into())
            })
        );
    }
}
