use iii_auth::config::{AuthConfig, StoreBackend};
use iii_auth::store::InMemoryAuthStore;
use iii_auth::{register_client, token_endpoint, validate_session};
use serde_json::json;

fn cfg() -> AuthConfig {
    let admin_env = format!(
        "III_AUTH_INTEGRATION_ADMIN_{}",
        uuid::Uuid::new_v4().simple()
    );
    std::env::set_var(&admin_env, "admin-secret");
    AuthConfig {
        issuer: "https://auth.test".to_string(),
        audience: "iii-test".to_string(),
        store: StoreBackend::Memory,
        supported_scopes: vec![
            "mcp:tools".to_string(),
            "function:demo::read".to_string(),
            "trigger:http".to_string(),
        ],
        default_scopes: vec!["mcp:tools".to_string()],
        registration_admin_token_env: admin_env,
        ..AuthConfig::default()
    }
}

#[tokio::test]
async fn dcr_token_validate_flow() -> anyhow::Result<()> {
    let store = InMemoryAuthStore::new();
    let cfg = cfg();
    let registration = register_client(
        &store,
        &cfg,
        json!({
            "headers": { "authorization": "Bearer admin-secret" },
            "client_name": "integration",
            "scope": "function:demo::read trigger:http"
        }),
    )
    .await?;
    let token = token_endpoint(
        &store,
        &cfg,
        json!({
            "grant_type": "client_credentials",
            "client_id": registration["client_id"],
            "client_secret": registration["client_secret"],
            "scope": "function:demo::read trigger:http"
        }),
    )
    .await?;
    let decision = validate_session(
        &store,
        &cfg,
        json!({ "headers": { "Authorization": format!("Bearer {}", token["access_token"].as_str().unwrap()) } }),
    )
    .await?;
    assert_eq!(decision.allowed_functions, vec!["demo::read"]);
    assert_eq!(
        decision.allowed_trigger_types,
        Some(vec!["http".to_string()])
    );
    Ok(())
}
