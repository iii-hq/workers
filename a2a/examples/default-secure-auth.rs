// Engine control-plane denylist for iii-a2a deploys.
//
// This is NOT a literal v0.3 hard-floor reproduction. The v0.3
// `ALWAYS_HIDDEN_PREFIXES` covered `engine::*`, `state::*`, `stream::*`,
// `iii.*`, `iii::*`, `mcp::*`, `a2a::*` as PREFIX MATCHES — `AuthResult.
// forbidden_functions` only takes literal function IDs, so a precise
// reproduction would have to enumerate every state::/stream::/iii:: ID
// the running engine version exposes.
//
// What this example DOES block (high-impact, control-plane / SDK plumbing
// IDs the engine exposes today): engine::workers::register,
// engine::{logs,traces}::clear, engine::channels::create,
// engine::baggage::*, engine::log::*, iii::durable::publish,
// iii::otel_passthrough.
//
// What it does NOT block (covered structurally elsewhere):
// - `mcp::*` and `a2a::*` — blocked by the protocol-loop guard inside
//   iii-mcp / iii-a2a (`is_protocol_loop`); never reach the engine.
// - `state::*` and `stream::*` raw KV / channel plumbing — these are
//   engine-version-coupled. Add the specific IDs your engine exposes
//   to `forbidden_functions` below, or use `expose_functions` allowlist
//   in iii-worker-manager (wildcards work there) to invert the policy.
//
// Recommended: pair this with an `expose_functions` ALLOWLIST in
// iii-worker-manager config so unknown IDs default to denied, then use
// this `forbidden_functions` list as a belt-and-suspenders second layer.

use iii_sdk::{AuthInput, AuthResult, InitOptions, RegisterFunction, register_worker};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let engine_url =
        std::env::var("III_ENGINE_URL").unwrap_or_else(|_| "ws://localhost:49134".to_string());

    let iii = register_worker(&engine_url, InitOptions::default());

    iii.register_function(
        RegisterFunction::new_async("myproject::auth", |input: AuthInput| async move {
            // The MCP / A2A worker forwards `--rbac-tag <TAG>` as this
            // header on its WebSocket upgrade. Read it here to scope
            // per-deploy policy if you need it.
            let rbac_tag = input.headers.get("x-iii-rbac-tag").cloned();
            tracing::info!(
                rbac_tag = ?rbac_tag,
                ip = %input.ip_address,
                "myproject::auth invoked"
            );

            Ok::<_, String>(AuthResult {
                allowed_functions: vec![],
                forbidden_functions: forbidden_functions(),
                allowed_trigger_types: None,
                allow_trigger_type_registration: false,
                allow_function_registration: true,
                function_registration_prefix: None,
                context: json!({ "rbac_tag": rbac_tag }),
            })
        })
        .description("Default-secure auth function: denies engine control-plane IDs"),
    );

    tracing::info!("myproject::auth registered. Wire it into iii-worker-manager:");
    tracing::info!("  workers:");
    tracing::info!("    - name: iii-worker-manager");
    tracing::info!("      config:");
    tracing::info!("        rbac:");
    tracing::info!("          auth_function_id: myproject::auth");
    tokio::signal::ctrl_c().await?;

    Ok(())
}

// 14 concrete control-plane / SDK-plumbing IDs to deny by default. Pair
// with an `expose_functions` allowlist in iii-worker-manager for full
// belt-and-suspenders coverage; this list is the explicit deny floor.
fn forbidden_functions() -> Vec<String> {
    vec![
        "engine::workers::register".to_string(),
        "engine::logs::clear".to_string(),
        "engine::traces::clear".to_string(),
        "engine::channels::create".to_string(),
        "engine::baggage::set".to_string(),
        "engine::baggage::get".to_string(),
        "engine::baggage::clear".to_string(),
        "engine::log::debug".to_string(),
        "engine::log::info".to_string(),
        "engine::log::warn".to_string(),
        "engine::log::error".to_string(),
        "engine::log::trace".to_string(),
        "iii::durable::publish".to_string(),
        "iii::otel_passthrough".to_string(),
    ]
}
