//! BDD entry point. Runs every `.feature` file under `tests/features/`.
//!
//! Filter by tag to isolate slices:
//!
//! ```text
//! cargo test --test bdd                          # everything
//! cargo test --test bdd -- --tags @pure          # no engine required
//! cargo test --test bdd -- --tags @download      # download scenarios
//! cargo test --test bdd -- --tags "@engine and not @download"
//! ```

mod common;
mod steps;

use cucumber::World;

use crate::common::world::IiiSkillsWorld;

#[tokio::main]
async fn main() {
    // Bring up the shared engine connection + in-process registrations
    // exactly once. On a host without an engine this returns None and
    // every `@engine` scenario short-circuits into a soft skip.
    let _ = common::engine::get_or_init().await;

    IiiSkillsWorld::cucumber()
        // Scenarios share a single engine connection + one skills_folder
        // tempdir; running them concurrently means two scenarios
        // trampling each other's fixture writes.
        .max_concurrent_scenarios(1)
        .before(|_feature, _rule, _scenario, world| {
            Box::pin(async move {
                if let Some(iii) = common::engine::get_or_init().await {
                    world.iii = Some(iii.clone());
                    if let Some(shared) = common::workers::shared() {
                        world.cfg = shared.cfg.clone();
                        world.skills_folder = Some(shared.skills_folder.clone());
                        world.registry_url = Some(shared.mock_server.uri());
                        common::workers::reset_fs(&shared.skills_folder);
                        common::workers::reset_mocks(&shared.mock_server).await;
                    }
                }
                world.stash.clear();
            })
        })
        .run_and_exit("tests/features")
        .await;
}
