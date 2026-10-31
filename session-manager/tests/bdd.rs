//! Cucumber BDD entry point. All `.feature` files under
//! `tests/features/**` are discovered at runtime.

mod common;
mod steps;

use cucumber::World as _;

use crate::common::world::SessionWorld;

#[tokio::main]
async fn main() {
    // Bring up the shared engine connection + in-process registration
    // exactly once. On a host without an engine, or on an engine that
    // already runs session-manager, this returns None and every `@engine`
    // scenario short-circuits into a soft skip.
    let _ = common::engine::get_or_init().await;

    SessionWorld::cucumber()
        .max_concurrent_scenarios(1)
        .before(|feature, rule, scenario, world| {
            // Tags are stored without the `@`.
            let engine_scenario = feature
                .tags
                .iter()
                .chain(rule.iter().flat_map(|r| &r.tags))
                .chain(&scenario.tags)
                .any(|tag| tag == "engine");
            Box::pin(async move {
                world.reset_pure();
                world.engine = common::engine::for_scenario(engine_scenario).await;
            })
        })
        .run_and_exit("tests/features")
        .await;
}
