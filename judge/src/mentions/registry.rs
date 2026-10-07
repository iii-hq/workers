//! The installed mention providers: functions whose registration metadata
//! carries a `mention` descriptor (`crates/mention-contract`). Listed with
//! `include_internal: true` (mention functions are internal) and cached
//! briefly — every agent step consults it.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use mention_contract::MentionProvider;
use serde_json::{json, Value};
use tokio::sync::Mutex;

const FUNCTIONS_LIST_ID: &str = "engine::functions::list";
const LIST_TIMEOUT_MS: u64 = 3_000;
const CACHE_TTL: Duration = Duration::from_secs(15);

/// A provider and the get function that declared it.
#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    pub get_function_id: String,
    pub descriptor: MentionProvider,
}

/// Providers out of an `engine::functions::list` answer, sorted by name.
/// A name claimed by two functions goes to the lexicographically first
/// function id — the console resolves the same way.
pub fn parse_providers(response: &Value) -> Vec<Provider> {
    let mut by_name: BTreeMap<String, Provider> = BTreeMap::new();
    let Some(rows) = response.get("functions").and_then(Value::as_array) else {
        return Vec::new();
    };
    for row in rows {
        let Some(function_id) = row.get("function_id").and_then(Value::as_str) else {
            continue;
        };
        let Some(descriptor) = row.get("metadata").and_then(MentionProvider::from_metadata) else {
            continue;
        };
        let candidate = Provider {
            get_function_id: function_id.to_string(),
            descriptor,
        };
        match by_name.get(&candidate.descriptor.name) {
            Some(existing) if existing.get_function_id <= candidate.get_function_id => {}
            _ => {
                by_name.insert(candidate.descriptor.name.clone(), candidate);
            }
        }
    }
    by_name.into_values().collect()
}

#[derive(Default)]
struct Cached {
    at: Option<Instant>,
    providers: Vec<Provider>,
}

/// The provider list, fetched at most every `CACHE_TTL`. A failed fetch
/// keeps serving the last good list.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    cached: Arc<Mutex<Cached>>,
}

impl ProviderRegistry {
    pub async fn providers(&self, iii: &IIIClient) -> Vec<Provider> {
        let mut cached = self.cached.lock().await;
        if cached.at.is_some_and(|at| at.elapsed() < CACHE_TTL) {
            return cached.providers.clone();
        }
        match iii
            .trigger(TriggerRequest {
                function_id: FUNCTIONS_LIST_ID.into(),
                payload: json!({ "include_internal": true }),
                action: None,
                timeout_ms: Some(LIST_TIMEOUT_MS),
            })
            .await
        {
            Ok(response) => {
                cached.providers = parse_providers(&response);
                cached.at = Some(Instant::now());
            }
            Err(error) => {
                tracing::debug!(%error, "mention providers could not be listed; keeping the last list");
            }
        }
        cached.providers.clone()
    }
}
