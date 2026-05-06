//! BDD entry point. Runs every `.feature` file under `tests/features/`.
//!
//! Filter by tag to isolate slices:
//!
//! ```text
//! cargo test --test bdd                       # everything
//! cargo test --test bdd -- --tags @pure       # no engine required
//! cargo test --test bdd -- --tags @tools      # one feature group
//! cargo test --test bdd -- --tags "@engine and not @prompts"
//! ```
//!
//! ## Engine availability
//!
//! By default a missing engine is treated as a soft skip: `world.iii`
//! is `None` and the `@engine`-tagged steps short-circuit. That keeps
//! `cargo test --test bdd -- --tags @pure` working on dev machines
//! without an iii engine running.
//!
//! In CI you almost always want the opposite: a missing engine is a
//! configuration bug, not a green test run. Set
//! `III_ENGINE_REQUIRED=1` in the CI environment and the harness will
//! panic at startup instead of silently skipping.

mod common;
mod steps;

use cucumber::World;

use crate::common::world::IiiMcpWorld;

#[tokio::main]
async fn main() {
    // Bring up the shared engine connection + in-process registrations
    // exactly once.
    let engine = common::engine::get_or_init().await;

    if engine.is_none() && require_engine() {
        panic!(
            "III_ENGINE_REQUIRED is set but the iii engine at {} is unreachable. \
             Start an iii engine (or unset III_ENGINE_REQUIRED to soft-skip).",
            common::engine::ws_url()
        );
    }

    IiiMcpWorld::cucumber()
        // Scenarios share a single engine connection + a single set of
        // registered functions; running them concurrently means two
        // scenarios can race on the JSON-RPC envelope assertions.
        .max_concurrent_scenarios(1)
        .before(|_feature, _rule, _scenario, world| {
            Box::pin(async move {
                if let Some(iii) = common::engine::get_or_init().await {
                    world.iii = Some(iii.clone());
                    if let Some(shared) = common::workers::shared() {
                        world.cfg = shared.cfg.clone();
                    }
                }
                world.stash.clear();
            })
        })
        .run_and_exit("tests/features")
        .await;
}

fn require_engine() -> bool {
    matches!(
        std::env::var("III_ENGINE_REQUIRED").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes")
    )
}
