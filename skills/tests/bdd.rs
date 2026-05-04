//! BDD entry point. Runs every `.feature` file under `tests/features/`.
//!
//! Filter by tag to isolate slices:
//!
//! ```text
//! cargo test --test bdd                               # everything
//! cargo test --test bdd -- --tags @pure               # no engine required
//! cargo test --test bdd -- --tags @skills_register    # one feature group
//! cargo test --test bdd -- --tags "@engine and not @notifications"
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
        // Scenarios share a single engine connection + a single set of
        // registered functions; running them concurrently means two
        // scenarios trampling each other's state writes.
        .max_concurrent_scenarios(1)
        .before(|_feature, _rule, _scenario, world| {
            Box::pin(async move {
                if let Some(iii) = common::engine::get_or_init().await {
                    world.iii = Some(iii.clone());
                    if let Some(shared) = common::workers::shared() {
                        world.cfg = shared.cfg.clone();
                    }
                    // Start every scenario with both registries empty
                    // so per-test assertions aren't polluted by earlier
                    // writes.
                    common::workers::reset_state(&iii).await;
                }
                world.stash.clear();
            })
        })
        .run_and_exit("tests/features")
        .await;
}
