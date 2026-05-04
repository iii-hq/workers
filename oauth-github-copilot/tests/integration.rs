//! Integration coverage for oauth-github-copilot.
//!
//! Asserts the extracted `copilot_headers()` carry the GitHub-Copilot CLI
//! wire format and that the worker's wiring registers the right oauth
//! function ids via an in-memory bus recorder.

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

impl oauth_github_copilot::testing::CredentialBus for InMemoryBus {
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
    let _ = &oauth_github_copilot::register_with_iii;
}

#[test]
fn copilot_headers_carry_editor_identity() {
    let headers = oauth_github_copilot::testing::copilot_headers();
    assert!(headers.contains_key("editor-version"));
    assert!(headers.contains_key("editor-plugin-version"));
    assert!(headers.contains_key("copilot-integration-id"));
    assert_eq!(
        headers.get(reqwest::header::ACCEPT).unwrap(),
        "application/json"
    );
}

#[tokio::test]
async fn register_with_iii_records_expected_function_ids() {
    let iii = iii_sdk::III::new("ws://127.0.0.1:1");
    let bus = Arc::new(InMemoryBus::new());
    let bus_dyn: Arc<dyn oauth_github_copilot::testing::CredentialBus> = bus.clone();

    let refs = oauth_github_copilot::testing::register_with_iii_with_bus(&iii, bus_dyn)
        .await
        .expect("registration succeeds");

    let ids: Vec<String> = bus
        .recorded_functions()
        .iter()
        .map(|m| m.id.clone())
        .collect();
    assert!(ids.contains(&"oauth::github-copilot::login".to_string()));
    assert!(ids.contains(&"oauth::github-copilot::refresh".to_string()));
    assert!(ids.contains(&"oauth::github-copilot::status".to_string()));
    assert_eq!(ids.len(), 3);

    assert!(bus.recorded_triggers().is_empty());
    // Handlers return the credential to the caller; no bus.set_token call
    // happens during register today.
    assert!(bus.recorded_set_tokens().is_empty());

    refs.unregister_all();
}
