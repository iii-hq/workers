mod common;
mod steps;

use cucumber::World;

use crate::common::world::CodeWorld;

#[tokio::main]
async fn main() {
    CodeWorld::cucumber()
        .max_concurrent_scenarios(1)
        .run_and_exit("tests/features")
        .await;
}
