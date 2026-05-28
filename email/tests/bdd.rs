//! BDD entry point. Runs every `.feature` file under `tests/features/`.
//!
//! Filter by tag to isolate slices:
//!
//! ```text
//! cargo test --test bdd                                 # everything
//! cargo test --test bdd -- --tags @pure                 # no engine, no network
//! cargo test --test bdd -- --tags @smtp                 # requires mock or live SMTP
//! cargo test --test bdd -- --tags @imap                 # requires live IMAP with IDLE
//! cargo test --test bdd -- --tags "@pure and not @engine"
//! ```
//!
//! `@pure` scenarios are subprocess-only — they shell out to the compiled
//! `email` binary and assert on `--manifest` / `--help`. They run in CI
//! with zero external dependencies.
//!
//! `@engine` / `@smtp` / `@imap` scenarios short-circuit into a soft skip
//! when their dependencies aren't reachable, so the suite stays green on
//! contributor laptops without a full mail-server stack.

mod common;
mod steps;

use cucumber::World;

use crate::common::world::EmailWorld;

#[tokio::main]
async fn main() {
    let _ = common::engine::get_or_init().await;

    EmailWorld::cucumber()
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
