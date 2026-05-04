//! Integration coverage for oauth-openai-codex.
//!
//! These tests exercise the public API as a black box: the authorize URL
//! builder via the `oauth_openai_codex::testing` re-exports, and the wiring
//! path via an in-memory [`oauth_openai_codex::testing::CredentialBus`] that
//! records the function ids and trigger configs the worker registers.

use std::sync::{Arc, Mutex};

use auth_credentials::Credential;
use iii_sdk::{IIIError, RegisterFunctionMessage, RegisterTriggerInput};

struct InMemoryBus {
    set_tokens: Mutex<Vec<(String, Credential)>>,
    functions: Mutex<Vec<RegisterFunctionMessage>>,
    triggers: Mutex<Vec<RegisterTriggerInput>>,
}

impl InMemoryBus {
    fn new() -> Self {
        Self {
            set_tokens: Mutex::new(Vec::new()),
            functions: Mutex::new(Vec::new()),
            triggers: Mutex::new(Vec::new()),
        }
    }

    fn recorded_set_tokens(&self) -> Vec<(String, Credential)> {
        self.set_tokens.lock().unwrap().clone()
    }

    fn recorded_functions(&self) -> Vec<RegisterFunctionMessage> {
        self.functions.lock().unwrap().clone()
    }

    fn recorded_triggers(&self) -> Vec<RegisterTriggerInput> {
        self.triggers.lock().unwrap().clone()
    }
}

impl oauth_openai_codex::testing::CredentialBus for InMemoryBus {
    fn set_token<'a>(
        &'a self,
        provider: &'a str,
        cred: &'a Credential,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), IIIError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.set_tokens
                .lock()
                .unwrap()
                .push((provider.to_string(), cred.clone()));
            Ok(())
        })
    }

    fn record_function(&self, msg: &RegisterFunctionMessage) {
        self.functions.lock().unwrap().push(msg.clone());
    }

    fn record_trigger(&self, input: &RegisterTriggerInput) {
        self.triggers.lock().unwrap().push(input.clone());
    }
}

#[test]
fn library_exports_register_entry_point() {
    let _ = &oauth_openai_codex::register_with_iii;
}

#[test]
fn authorize_url_includes_required_pkce_params() {
    let url = oauth_openai_codex::testing::build_authorize_url(
        "TEST_CHALLENGE",
        "TEST_STATE",
        "http://127.0.0.1:53701/callback",
    );
    assert!(url.contains("code_challenge=TEST_CHALLENGE"));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains("client_id="));
    assert!(url.contains("redirect_uri="));
    assert!(url.contains("state=TEST_STATE"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains("offline_access"));
}

#[tokio::test]
async fn register_with_iii_records_expected_function_ids() {
    let iii = iii_sdk::III::new("ws://127.0.0.1:1");
    let bus = Arc::new(InMemoryBus::new());
    let bus_dyn: Arc<dyn oauth_openai_codex::testing::CredentialBus> = bus.clone();

    let refs = oauth_openai_codex::testing::register_with_iii_with_bus(&iii, bus_dyn)
        .await
        .expect("registration succeeds");

    let ids: Vec<String> = bus
        .recorded_functions()
        .iter()
        .map(|m| m.id.clone())
        .collect();
    assert!(ids.contains(&"oauth::openai-codex::login".to_string()));
    assert!(ids.contains(&"oauth::openai-codex::refresh".to_string()));
    assert!(ids.contains(&"oauth::openai-codex::status".to_string()));
    assert_eq!(ids.len(), 3);

    assert!(bus.recorded_triggers().is_empty());
    // Handlers return the credential to the caller; no bus.set_token call
    // happens during register today.
    assert!(bus.recorded_set_tokens().is_empty());

    refs.unregister_all();
}
