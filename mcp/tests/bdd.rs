//! BDD entry point. Runs every `.feature` file under `tests/features/`.
//!
//! Filter by tag to isolate slices:
//!
//! ```text
//! cargo test --test bdd                          # everything
//! cargo test --test bdd -- --tags @pure          # no engine required
//! cargo test --test bdd -- --tags @initialize    # one feature group
//! cargo test --test bdd -- --tags "@engine and not @errors"
//! ```
//!
//! Implementation note: a single binary is required because cucumber
//! needs full control of the runner (`harness = false`). The `@engine`
//! scenarios assume a single iii engine is reachable at
//! `III_ENGINE_WS_URL` (default `ws://127.0.0.1:49134`); when it is
//! unavailable they short-circuit into a soft skip without failing.

mod common;
mod steps;

use cucumber::World;

use crate::common::world::McpWorld;

#[tokio::main]
async fn main() {
    // Bring up the shared engine connection + the in-process mcp::handler
    // registration exactly once. On a host without an engine this returns
    // `None` and every `@engine` scenario short-circuits into a soft skip.
    let _ = common::engine::get_or_init().await;

    McpWorld::cucumber()
        // Scenarios share a single engine connection + a single
        // registration of `mcp::handler` and the HTTP trigger; running
        // them concurrently means two scenarios trampling each other's
        // SSE streams.
        .max_concurrent_scenarios(1)
        .before(|_feature, _rule, _scenario, world| {
            Box::pin(async move {
                if let Some(iii) = common::engine::get_or_init().await {
                    world.iii = Some(iii);
                }
                world.stash.clear();
            })
        })
        .run_and_exit("tests/features")
        .await;
}
