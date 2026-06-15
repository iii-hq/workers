//! @engine steps: drive the production surface registered in-process
//! over a live engine. Soft-skip when no engine is reachable.

use cucumber::gherkin::Step;
use cucumber::{given, when};

use crate::common::world::ContextWorld;
use crate::steps::common_steps::docstring_payload;

#[given("an engine connection")]
async fn engine_connection(world: &mut ContextWorld) {
    let _ = world.skip_without_engine();
}

#[when(regex = r#"^I call "([^"]+)" on the engine with:$"#)]
async fn call_engine_with(world: &mut ContextWorld, function: String, step: &Step) {
    if world.skip_without_engine() {
        return;
    }
    let payload = docstring_payload(world, step);
    world.call_engine(&function, payload).await;
}
