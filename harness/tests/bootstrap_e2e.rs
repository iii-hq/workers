//! T2 — fresh-install bootstrap end-to-end.
//!
//! Documents the manual verification flow for the iii-directory migration:
//!
//! 1. Wipe `./data/skills`.
//! 2. Spin up `iii` engine + `iii-directory` worker + `harness`.
//! 3. Wait briefly for [`harness::skills::bootstrap_run`] to drain its
//!    JoinSet.
//! 4. `directory::skills::fetch-skill iii://shell/index` returns non-empty content.
//! 5. Triggering `directory::skills::download worker=shell` fires a
//!    `ui::skills::changed::<browser_id>` push to any subscribed browser.
//!
//! Marked `#[ignore]` because the integration prerequisites
//! (`iii`, `iii-directory`, the seven bundled workers, and a network
//! connection to the registry) are heavier than the standard `cargo test`
//! environment provides. Run explicitly with:
//!
//! ```bash
//! cd harness && cargo test --test bootstrap_e2e -- --ignored --nocapture
//! ```
//!
//! See `docs/superpowers/specs/2026-05-12-iii-directory-migration-design.md`
//! §Verification for the live-engine smoke flow.

#![allow(clippy::doc_markdown)]

mod common;

use std::time::Duration;

use common::Harness;
use iii_sdk::TriggerRequest;
use serde_json::{json, Value};

#[tokio::test]
#[ignore = "requires iii engine + iii-directory + bundled workers + network"]
async fn fresh_install_bootstraps_shell_skill() {
    // Step 1 — wipe data/skills under the harness CWD. The Harness helper
    // boots the iii engine with its default config; the skills folder it
    // writes to is determined by iii-directory's `skills_folder` config.
    let _ = std::fs::remove_dir_all("./data/skills");

    let Some(harness) = Harness::boot().await else {
        eprintln!("skipping: prerequisites missing");
        return;
    };

    // Step 2 — run the bootstrap. In production this is awaited by
    // register_with_iii_with_engine_url; here we call it directly so the
    // test is self-contained.
    harness::skills::bootstrap_run(&harness.iii)
        .await
        .expect("bootstrap_run should not error (best-effort)");

    // Step 3 — give iii-directory a moment to settle (filesystem writes,
    // index regeneration). 2s is comfortably above the local steady state.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Step 4 — directory::skills::fetch-skill iii://shell/index must return non-empty body.
    let resp: Value = harness
        .iii
        .trigger(TriggerRequest {
            function_id: "directory::skills::fetch-skill".into(),
            payload: json!({ "uri": "iii://shell/index" }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .expect("directory::skills::fetch-skill should succeed");

    let body = resp
        .get("contents")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        !body.trim().is_empty(),
        "iii://shell/index returned empty body: {resp:?}"
    );
}
