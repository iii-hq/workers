//! Cucumber BDD entry point. All `.feature` files under
//! `tests/features/**` are discovered at runtime.

mod common;
mod steps;

use cucumber::World as _;

use crate::common::world::SessionWorld;

#[tokio::main]
async fn main() {
    // Bring up the shared engine connection + in-process registration
    // exactly once. On a host without an engine this returns None and
    // every `@engine` scenario short-circuits into a soft skip.
    let _ = common::engine::get_or_init().await;

    SessionWorld::cucumber()
        .max_concurrent_scenarios(1)
        .before(|_feature, _rule, _scenario, world| {
            Box::pin(async move {
                world.reset_pure();
                world.engine = common::engine::get_or_init().await;
            })
        })
        // Dispose the scenario's engine observers and bridge relays, so
        // later scenarios' events stop reaching them.
        .after(|_feature, _rule, _scenario, _finished, world| {
            Box::pin(async move {
                if let Some(world) = world {
                    world.retire_engine_observers().await;
                }
            })
        })
        .run_and_exit("tests/features")
        .await;
}
