use github::webhooks::setup::{
    blockers, evaluate, function_missing, CheckState, SetupFix, TunnelProbe,
    CLOUDFLARED_INSTALL_URL, DEFAULT_LISTENER_PORT,
};
use github::webhooks::Failure;
use serde_json::{json, Value};

fn tunnel_with(cloudflared: Value) -> Result<Value, TunnelProbe> {
    Ok(json!({"status": "stopped", "leases": [], "prerequisites": {"cloudflared": cloudflared}}))
}
fn listener_on() -> Result<Option<Value>, String> {
    Ok(Some(
        json!({"port": 3111, "webhook_listener": {"host": "127.0.0.1", "port": 3112}}),
    ))
}
fn states(checks: &[github::webhooks::setup::SetupCheck]) -> Vec<CheckState> {
    checks.iter().map(|c| c.state).collect()
}

#[test]
fn everything_present_is_ready() {
    let checks = evaluate(
        &tunnel_with(
            json!({"found": true, "path": "/opt/homebrew/bin/cloudflared", "version": "cloudflared version 2026.9.1"}),
        ),
        &listener_on(),
    );
    assert_eq!(states(&checks), [CheckState::Ok; 3]);
    assert!(checks[1].detail.contains("2026.9.1"));
    assert!(checks[2].detail.contains("127.0.0.1:3112"));
    assert!(blockers(&checks).is_none());
}

#[test]
fn missing_quick_tunnel_offers_install_and_blocks_cloudflared() {
    let checks = evaluate(&Err(TunnelProbe::NotInstalled), &listener_on());
    assert_eq!(
        states(&checks),
        [CheckState::Missing, CheckState::Blocked, CheckState::Ok]
    );
    assert!(matches!(
        &checks[0].fix,
        Some(SetupFix::InstallWorker { worker, .. }) if worker == "quick-tunnel"
    ));
    let message = blockers(&checks).unwrap();
    assert!(message.contains("quick-tunnel worker"));
    assert!(message.contains("github::setup::webhooks-status"));
}

#[test]
fn missing_cloudflared_points_at_cloudflare_and_never_installs() {
    let checks = evaluate(
        &tunnel_with(
            json!({"found": false, "error": "cloudflared not found (cloudflared)", "install_url": "https://example.test/install"}),
        ),
        &listener_on(),
    );
    assert_eq!(checks[1].state, CheckState::Missing);
    assert_eq!(
        checks[1].fix,
        Some(SetupFix::OpenUrl {
            url: "https://example.test/install".into()
        })
    );
    // Falls back to Cloudflare's page when quick-tunnel omits it.
    let checks = evaluate(&tunnel_with(json!({"found": false})), &listener_on());
    assert_eq!(
        checks[1].fix,
        Some(SetupFix::OpenUrl {
            url: CLOUDFLARED_INSTALL_URL.into()
        })
    );
}

#[test]
fn listener_off_offers_enable_and_old_workers_are_unknown_not_blocking() {
    let checks = evaluate(
        &tunnel_with(json!({"found": true})),
        &Ok(Some(json!({"webhook_listener": null}))),
    );
    assert_eq!(checks[2].state, CheckState::Missing);
    assert_eq!(
        checks[2].fix,
        Some(SetupFix::EnableHttpListener {
            port: DEFAULT_LISTENER_PORT
        })
    );
    // An old quick-tunnel (no prerequisites) and an old http (no
    // configuration-id) cannot be verified: they suggest an update but never
    // block a watch.
    let checks = evaluate(&Ok(json!({"status": "ready"})), &Ok(None));
    assert_eq!(
        states(&checks),
        [CheckState::Ok, CheckState::Unknown, CheckState::Unknown]
    );
    assert!(
        matches!(&checks[1].fix, Some(SetupFix::UpdateWorker { worker, .. }) if worker == "quick-tunnel")
    );
    assert!(
        matches!(&checks[2].fix, Some(SetupFix::UpdateWorker { worker, .. }) if worker == "http")
    );
    assert!(blockers(&checks).is_none());
    // Transient errors are unknown too, and never refuse a watch.
    let checks = evaluate(
        &Err(TunnelProbe::NotResponding("timeout".into())),
        &Err("timeout".into()),
    );
    assert_eq!(states(&checks), [CheckState::Unknown; 3]);
    assert!(blockers(&checks).is_none());
}

#[test]
fn a_restarting_quick_tunnel_is_unknown_and_never_blocks() {
    let checks = evaluate(
        &Err(TunnelProbe::NotResponding(
            "declared in compose but not answering".into(),
        )),
        &listener_on(),
    );
    assert_eq!(checks[0].state, CheckState::Unknown);
    assert_eq!(checks[1].state, CheckState::Unknown);
    assert!(
        blockers(&checks).is_none(),
        "a restart must not refuse a watch"
    );
}

#[test]
fn only_the_structured_function_not_found_code_means_missing() {
    let remote = |code: &str, message: &str| {
        Failure::Engine(iii_sdk::Error::Remote {
            code: code.into(),
            message: message.into(),
            stacktrace: None,
        })
    };
    assert!(function_missing(&remote(
        "function_not_found",
        "Function quick-tunnel::status not found in namespace default."
    )));
    // Text that merely mentions it is not the engine's answer.
    assert!(!function_missing(&remote(
        "invocation_failed",
        "function_not_found in payload"
    )));
    assert!(!function_missing(&Failure::Invalid(
        "remote error (function_not_found): not found in namespace".into()
    )));
}
