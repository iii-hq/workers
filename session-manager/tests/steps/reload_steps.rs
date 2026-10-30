//! @engine adapter hot-reload steps: a second fs-mode runtime on the
//! engine whose adapter a scenario changes. Every step soft-skips when no
//! engine is reachable.

use cucumber::gherkin::Step;
use cucumber::{given, when};
use serde_json::Value;

use crate::common::reload::ReloadableInstance;
use crate::common::world::SessionWorld;

#[given(
    regex = r#"^a reloadable session-manager over a directory holding a session "([^"]+)" with metadata:$"#
)]
async fn reloadable_with_session(world: &mut SessionWorld, alias: String, step: &Step) {
    let Some(iii) = world.engine.clone() else {
        return;
    };
    let raw = world.substitute(step.docstring.as_deref().unwrap_or("{}"));
    let metadata: Value = serde_json::from_str(raw.trim())
        .unwrap_or_else(|e| panic!("metadata docstring must be valid JSON: {e}"));

    let instance = ReloadableInstance::start(&iii);
    let session_id = format!("reload-{}", uuid::Uuid::new_v4().simple());
    instance.store_session(&session_id, metadata).await;
    world.aliases.insert(alias, session_id);
    world.reloadable = Some(instance);
}

#[when("the reloadable session-manager's adapter changes to an empty directory")]
async fn reloadable_adapter_changes(world: &mut SessionWorld) {
    if world.engine.is_none() {
        return;
    }
    world
        .reloadable
        .as_mut()
        .expect("no reloadable session-manager in this scenario")
        .change_to_empty_dir()
        .await;
}
