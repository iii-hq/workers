//! Cucumber BDD entry point. All `.feature` files under
//! `tests/features/**` are discovered at runtime.

mod common;
mod steps;

use cucumber::World as _;

use crate::common::world::ContextWorld;

fn main() {
    // Set before any thread starts. The private HOME keeps the SDK client's
    // `iii ps` marker out of the real ~/.iii/run.
    let scratch = tempfile::tempdir().expect("tempdir");
    std::env::set_var("HOME", scratch.path());
    // Declared after `scratch`, so the engine is reaped before its dir goes.
    let _engine = if std::env::var_os("III_ENGINE_WS_URL").is_none() {
        common::engine::spawn_private(scratch.path()).map(|(engine, url)| {
            std::env::set_var("III_ENGINE_WS_URL", url);
            engine
        })
    } else {
        None
    };
    run();
}

#[tokio::main]
async fn run() {
    // Bring up the shared engine connection + in-process registration
    // exactly once. On a host without an engine this returns None and
    // every `@engine` scenario short-circuits into a soft skip.
    let _ = common::engine::get_or_init().await;

    ContextWorld::cucumber()
        .max_concurrent_scenarios(1)
        .before(|_feature, _rule, _scenario, world| {
            Box::pin(async move {
                world.reset_pure();
                world.engine = common::engine::get_or_init().await;
            })
        })
        .run_and_exit("tests/features")
        .await;
}
