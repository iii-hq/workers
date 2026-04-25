//! Unit tests for `build_agent_card`.
//!
//! These tests assert the static-shape pieces of the A2A v0.3 agent card:
//! the `/a2a` suffix on the advertised JSON-RPC interface, the configurable
//! documentation URL, and the configurable identity (name / description /
//! provider). The tests intentionally point `III` at a non-listening port so
//! `list_functions()` errors out and `skills` falls back to an empty vec —
//! that's the documented behaviour, and it lets us cover the static fields
//! without spinning up the engine.

use iii_a2a::handler::{AgentIdentity, ExposureConfig, build_agent_card};
use iii_sdk::III;

fn unreachable_iii() -> III {
    // Port 1 is reserved/unbound on every sane host; the SDK's reconnect
    // logic will keep retrying in the background, but `list_functions()`
    // returns Err immediately because no connection is established. That
    // matches the `Err(_) => vec![]` branch in `build_agent_card`.
    III::new("ws://127.0.0.1:1")
}

#[tokio::test]
async fn default_identity_advertises_a2a_suffix_and_docs_url() {
    let iii = unreachable_iii();
    let cfg = ExposureConfig::new(false, None);
    let identity = AgentIdentity::default();

    let card = build_agent_card(&iii, &cfg, "http://localhost:3111", &identity).await;

    assert_eq!(card.supported_interfaces.len(), 1);
    assert_eq!(
        card.supported_interfaces[0].url, "http://localhost:3111/a2a",
        "supported_interfaces[].url must point at the JSON-RPC mount, not the bare base URL"
    );
    assert_eq!(card.supported_interfaces[0].protocol_binding, "JSONRPC");
    assert_eq!(card.supported_interfaces[0].protocol_version, "0.3");

    assert_eq!(
        card.documentation_url.as_deref(),
        Some("https://github.com/iii-hq/workers/tree/main/a2a"),
        "default docs_url must point at the workers repo a2a folder"
    );

    assert_eq!(card.name, "iii-engine");
    let provider = card
        .provider
        .expect("default identity always has a provider");
    assert_eq!(provider.organization, "iii");
    assert_eq!(provider.url, "https://github.com/iii-hq/iii");
}

#[tokio::test]
async fn trailing_slash_in_base_url_is_normalised() {
    let iii = unreachable_iii();
    let cfg = ExposureConfig::new(false, None);
    let identity = AgentIdentity::default();

    let card = build_agent_card(&iii, &cfg, "http://localhost:3111/", &identity).await;

    assert_eq!(
        card.supported_interfaces[0].url, "http://localhost:3111/a2a",
        "trailing slash on base_url must not produce a doubled `//a2a`"
    );
}

#[tokio::test]
async fn custom_identity_flows_through() {
    let iii = unreachable_iii();
    let cfg = ExposureConfig::new(false, None);
    let identity = AgentIdentity {
        name: "acme-orchestrator".to_string(),
        description: "Acme order pipeline agent".to_string(),
        provider_org: "Acme Corp".to_string(),
        provider_url: "https://acme.example/agents".to_string(),
        docs_url: "https://docs.acme.example/agents/orchestrator".to_string(),
    };

    let card = build_agent_card(&iii, &cfg, "https://agent.acme.example", &identity).await;

    assert_eq!(card.name, "acme-orchestrator");
    assert_eq!(card.description, "Acme order pipeline agent");
    assert_eq!(
        card.documentation_url.as_deref(),
        Some("https://docs.acme.example/agents/orchestrator")
    );
    let provider = card
        .provider
        .expect("custom identity always has a provider");
    assert_eq!(provider.organization, "Acme Corp");
    assert_eq!(provider.url, "https://acme.example/agents");

    assert_eq!(
        card.supported_interfaces[0].url, "https://agent.acme.example/a2a",
        "custom base_url must still get the /a2a suffix"
    );

    // No engine connection, so `list_functions()` errors and skills is empty —
    // documents the Err branch in build_agent_card.
    assert!(card.skills.is_empty());
}
