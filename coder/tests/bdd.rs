//! BDD entry point. Runs every `.feature` file under `tests/features/`.
//!
//! Filter by tag to isolate slices:
//!
//! ```text
//! cargo test --test bdd                              # everything
//! cargo test --test bdd -- --tags @search            # one tag
//! cargo test --test bdd -- --tags @security
//! cargo test --test bdd -- --tags "@engine and not @unix"
//! ```
//!
//! All `@engine`-tagged scenarios soft-skip when no iii engine is
//! reachable, so CI hosts without `iii` on PATH still pass.

mod common;
mod steps;

use cucumber::World;

use crate::common::world::CoderWorld;

#[tokio::main]
async fn main() {
    // Bring up the shared engine connection + in-process registrations
    // exactly once. On a host without an engine this returns None and
    // every `@engine` scenario short-circuits into a soft skip.
    let _ = common::engine::get_or_init().await;

    CoderWorld::cucumber()
        // Scenarios share a single engine connection + one base_path
        // tempdir; running them concurrently means two scenarios
        // trampling each other's fixture writes.
        .max_concurrent_scenarios(1)
        .before(|_feature, _rule, _scenario, world| {
            Box::pin(async move {
                if let Some(iii) = common::engine::get_or_init().await {
                    world.iii = Some(iii.clone());
                    if let Some(shared) = common::workers::shared() {
                        world.cfg = shared.cfg.clone();
                        world.base_path = Some(shared.base_path.clone());
                        common::workers::reset_fs(&shared.base_path);
                    }
                }
                world.stash.clear();
            })
        })
        .run_and_exit("tests/features")
        .await;
}
