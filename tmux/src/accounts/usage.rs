//! Claude Code's usage endpoint (agent-accounts D10–D12):
//! `GET /api/oauth/usage` with the account's OAuth token, as Claude Code
//! 2.1.322 asks it (`Authorization: Bearer …`,
//! `anthropic-beta: oauth-2025-04-20`). The answer has a window per key
//! (`five_hour`, `seven_day`, `seven_day_opus`, `seven_day_sonnet`, …), each
//! `{utilization, resets_at}` with `utilization` from 0 to 100, plus
//! `extra_usage` and a `limits` list whose `weekly_scoped` entries name a
//! model. Undocumented: a key that is null or missing is a window the plan
//! doesn't have.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::credentials::Secret;
use super::time::json_time_ms;
use crate::model::{ClaudeUsage, ExtraUsage, Limit, ModelLimit};

pub const USAGE_PATH: &str = "/api/oauth/usage";
pub const OAUTH_BETA: &str = "oauth-2025-04-20";

/// How often each account is read at most (D12).
pub const READ_EVERY_MS: u64 = 5 * 60_000;
/// How often `refresh: true` may read an account.
pub const FORCE_EVERY_MS: u64 = 60_000;
/// How long a 429 stops the readings of an account.
pub const RATE_LIMIT_BACKOFF_MS: u64 = 15 * 60_000;
/// How long one request may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

fn limit(value: &Value) -> Option<Limit> {
    let object = value.as_object()?;
    let used = object
        .get("utilization")
        .or_else(|| object.get("percent"))
        .and_then(Value::as_f64)?;
    Some(Limit {
        used_percent: used.max(0.0),
        resets_at_ms: object.get("resets_at").and_then(json_time_ms),
        reset_since_read: false,
    })
}

/// One answer of the endpoint (or Claude Code's cached copy of one).
pub fn parse_usage(value: &Value, as_of_ms: u64) -> ClaudeUsage {
    let window = |key: &str| value.get(key).and_then(limit);
    let mut usage = ClaudeUsage {
        five_hour: window("five_hour"),
        week: window("seven_day"),
        week_opus: window("seven_day_opus"),
        week_sonnet: window("seven_day_sonnet"),
        week_models: Vec::new(),
        extra: value.get("extra_usage").and_then(|extra| {
            let enabled = extra.get("is_enabled")?.as_bool()?;
            Some(ExtraUsage {
                enabled,
                used_percent: extra.get("utilization").and_then(Value::as_f64),
            })
        }),
        as_of_ms,
        error: None,
    };
    for entry in value
        .get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "weekly_all" if usage.week.is_none() => usage.week = limit(entry),
            "weekly_scoped" => {
                let Some(name) = entry
                    .pointer("/scope/model/display_name")
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                let Some(found) = limit(entry) else {
                    continue;
                };
                usage.week_models.push(ModelLimit {
                    name: name.to_string(),
                    limit: found,
                });
            }
            _ => {}
        }
    }
    for (name, slot) in [
        ("opus", &mut usage.week_opus),
        ("sonnet", &mut usage.week_sonnet),
    ] {
        match slot {
            Some(found) => {
                if !usage
                    .week_models
                    .iter()
                    .any(|m| m.name.to_lowercase().contains(name))
                {
                    let mut label = name.to_string();
                    label[..1].make_ascii_uppercase();
                    usage.week_models.push(ModelLimit {
                        name: label,
                        limit: found.clone(),
                    });
                }
            }
            None => {
                *slot = usage
                    .week_models
                    .iter()
                    .find(|m| m.name.to_lowercase().contains(name))
                    .map(|m| m.limit.clone());
            }
        }
    }
    usage
}

/// A window whose reset passed after the reading is empty now (D11).
pub fn settle(usage: &mut ClaudeUsage, now_ms: u64) {
    let settle_one = |limit: &mut Limit| {
        if limit.resets_at_ms.is_some_and(|at| at <= now_ms) {
            limit.used_percent = 0.0;
            limit.reset_since_read = true;
            limit.resets_at_ms = None;
        }
    };
    for limit in [
        &mut usage.five_hour,
        &mut usage.week,
        &mut usage.week_opus,
        &mut usage.week_sonnet,
    ]
    .into_iter()
    .flatten()
    {
        settle_one(limit);
    }
    for model in &mut usage.week_models {
        settle_one(&mut model.limit);
    }
}

/// Why a reading failed; the words the feed's `error` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    RateLimited,
    Unauthorized,
    Network,
    Unexpected(u16),
    Unreadable,
    NotAllowed,
}

impl FetchError {
    pub fn word(&self) -> String {
        match self {
            Self::RateLimited => "rate_limited".into(),
            Self::Unauthorized => "unauthorized".into(),
            Self::Network => "network".into(),
            Self::Unexpected(status) => format!("http_{status}"),
            Self::Unreadable => "unreadable".into(),
            Self::NotAllowed => "not_allowed".into(),
        }
    }
}

/// The HTTP client for the endpoint.
#[derive(Clone)]
pub struct UsageClient {
    http: reqwest::Client,
    origin: String,
}

impl UsageClient {
    /// `None` when the client can't be built as it must be (no proxy, no
    /// redirect, a timeout): then nothing is ever asked, rather than asked
    /// through a client that could carry the token elsewhere.
    pub fn new(origin: &str) -> Option<Self> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            // No proxy from the environment ever sees the token.
            .no_proxy()
            .user_agent(concat!("iii-tmux/", env!("CARGO_PKG_VERSION")))
            .build()
            .inspect_err(
                |error| tracing::warn!(%error, "usage client not built; usage is not read"),
            )
            .ok()?;
        Some(Self {
            http,
            origin: origin.to_string(),
        })
    }

    /// The origin it asks.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// One reading. The token goes to the endpoint and nowhere else.
    pub async fn fetch(&self, token: &Secret) -> Result<Value, FetchError> {
        let url = super::paths::allowed_origin(&self.origin)
            .and_then(|origin| origin.join(USAGE_PATH).ok())
            .ok_or(FetchError::NotAllowed)?;
        let response = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {}", token.expose()))
            .header("anthropic-beta", OAUTH_BETA)
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|_| FetchError::Network)?;
        match response.status().as_u16() {
            200..=299 => response
                .json::<Value>()
                .await
                .map_err(|_| FetchError::Unreadable),
            401 | 403 => Err(FetchError::Unauthorized),
            429 => Err(FetchError::RateLimited),
            status => Err(FetchError::Unexpected(status)),
        }
    }
}

/// What the worker knows of one account's usage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    /// The last good reading.
    #[serde(default)]
    pub usage: Option<ClaudeUsage>,
    /// Why the latest attempt failed.
    #[serde(default)]
    pub error: Option<String>,
    /// When the latest attempt was made.
    #[serde(default)]
    pub attempted_at_ms: u64,
    /// No attempt before this time (after a 429).
    #[serde(default)]
    pub backoff_until_ms: u64,
    /// The endpoint refused the token (401): it counts as expired until it changes.
    #[serde(default)]
    pub rejected: bool,
}

impl Reading {
    /// Whether an attempt may be made now: `force` (an explicit refresh) after
    /// a minute, otherwise after five, never during a back-off.
    pub fn due(&self, now_ms: u64, force: bool) -> bool {
        if now_ms < self.backoff_until_ms {
            return false;
        }
        if self.attempted_at_ms == 0 {
            return true;
        }
        // A failure that is no refusal (the network, the endpoint) is tried
        // again sooner.
        let retry = matches!(self.error.as_deref(), Some(e) if e != "unauthorized");
        let every = if force || retry {
            FORCE_EVERY_MS
        } else {
            READ_EVERY_MS
        };
        now_ms.saturating_sub(self.attempted_at_ms) >= every
    }

    /// The reading's age, `None` without one.
    pub fn age_ms(&self, now_ms: u64) -> Option<u64> {
        self.usage
            .as_ref()
            .map(|usage| now_ms.saturating_sub(usage.as_of_ms))
    }

    /// Record an attempt's outcome.
    pub fn record(&mut self, outcome: Result<ClaudeUsage, FetchError>, now_ms: u64) {
        self.attempted_at_ms = now_ms;
        match outcome {
            Ok(usage) => {
                self.usage = Some(usage);
                self.error = None;
                self.rejected = false;
            }
            Err(error) => {
                if error == FetchError::RateLimited {
                    self.backoff_until_ms = now_ms + RATE_LIMIT_BACKOFF_MS;
                }
                if error == FetchError::Unauthorized {
                    self.rejected = true;
                }
                self.error = Some(error.word());
            }
        }
    }

    /// Take Claude Code's own cached reading when it is newer than ours.
    pub fn adopt(&mut self, cached: ClaudeUsage) {
        let newer = self
            .usage
            .as_ref()
            .is_none_or(|ours| cached.as_of_ms > ours.as_of_ms);
        if newer {
            self.usage = Some(cached);
            self.error = None;
        }
    }
}

/// The cache file: the readings by account id, no token in it.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CacheFile {
    #[serde(default)]
    pub claude: HashMap<String, Reading>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a real answer (Claude Code 2.1.322's cached copy on a
    /// Max plan, 2026-11-11), numbers changed, unknown keys trimmed.
    pub const REAL_SHAPE: &str = r#"{
      "five_hour": {"utilization": 34, "resets_at": "2026-11-11T21:10:00.000000+00:00", "limit_dollars": null},
      "seven_day": {"utilization": 62, "resets_at": "2026-11-14T17:00:00.129271+00:00", "limit_dollars": null,
                    "used_dollars": null, "remaining_dollars": null, "locked_reason": null},
      "seven_day_oauth_apps": null, "seven_day_opus": {"utilization": 88.0, "resets_at": "2026-11-14T17:00:00+00:00"},
      "seven_day_sonnet": null, "tangelo": null,
      "extra_usage": {"is_enabled": false, "monthly_limit": null, "used_credits": null, "utilization": null},
      "limits": [
        {"kind": "weekly_all", "group": "weekly", "percent": 62, "severity": "normal",
         "resets_at": "2026-11-14T17:00:00.129271+00:00", "scope": null, "is_active": true},
        {"kind": "weekly_scoped", "group": "weekly", "percent": 41, "severity": "normal",
         "resets_at": "2026-11-14T17:00:00.129442+00:00",
         "scope": {"model": {"id": null, "display_name": "Fable"}, "surface": null}, "is_active": false}
      ],
      "spend": {"percent": 0, "enabled": false}
    }"#;

    #[test]
    fn a_real_answer_parses() {
        let value: Value = serde_json::from_str(REAL_SHAPE).unwrap();
        let usage = parse_usage(&value, 7);
        assert_eq!(usage.five_hour.as_ref().unwrap().used_percent, 34.0);
        let week = usage.week.as_ref().unwrap();
        assert_eq!(week.used_percent, 62.0);
        assert_eq!(week.resets_at_ms, Some(1_794_675_600_129));
        assert_eq!(usage.week_opus.as_ref().unwrap().used_percent, 88.0);
        assert!(usage.week_sonnet.is_none());
        assert_eq!(
            usage
                .week_models
                .iter()
                .map(|m| (m.name.as_str(), m.limit.used_percent))
                .collect::<Vec<_>>(),
            vec![("Fable", 41.0), ("Opus", 88.0)]
        );
        assert_eq!(
            usage.extra,
            Some(ExtraUsage {
                enabled: false,
                used_percent: None
            })
        );
        assert_eq!(usage.as_of_ms, 7);
    }

    #[test]
    fn the_limits_list_fills_what_the_keys_leave_out() {
        let value = serde_json::json!({
            "five_hour": null, "seven_day": null,
            "limits": [
                {"kind": "weekly_all", "percent": 12, "resets_at": "2026-11-14T17:00:00Z"},
                {"kind": "weekly_scoped", "percent": 30, "resets_at": "2026-11-14T17:00:00Z",
                 "scope": {"model": {"display_name": "Sonnet"}}}
            ]
        });
        let usage = parse_usage(&value, 0);
        assert!(usage.five_hour.is_none());
        assert_eq!(usage.week.unwrap().used_percent, 12.0);
        assert_eq!(usage.week_sonnet.unwrap().used_percent, 30.0);
    }

    #[test]
    fn a_window_past_its_reset_reads_empty() {
        let value: Value = serde_json::from_str(REAL_SHAPE).unwrap();
        let mut usage = parse_usage(&value, 0);
        settle(&mut usage, 1_794_675_600_129);
        let week = usage.week.as_ref().unwrap();
        assert_eq!((week.used_percent, week.reset_since_read), (0.0, true));
        assert!(usage.five_hour.as_ref().unwrap().reset_since_read);
        assert!(usage.week_models.iter().all(|m| m.limit.reset_since_read));
    }

    #[test]
    fn readings_respect_their_cadence_and_backoff() {
        let mut reading = Reading::default();
        assert!(reading.due(10, false));
        reading.record(Err(FetchError::RateLimited), 1_000);
        assert_eq!(reading.error.as_deref(), Some("rate_limited"));
        assert!(!reading.due(1_000 + RATE_LIMIT_BACKOFF_MS - 1, true));
        assert!(reading.due(1_000 + RATE_LIMIT_BACKOFF_MS, true));
        reading.backoff_until_ms = 0;
        let value: Value = serde_json::from_str(REAL_SHAPE).unwrap();
        reading.record(Ok(parse_usage(&value, 2_000)), 2_000);
        assert!(reading.error.is_none());
        assert!(!reading.due(2_000 + FORCE_EVERY_MS - 1, true));
        assert!(reading.due(2_000 + FORCE_EVERY_MS, true));
        assert!(!reading.due(2_000 + FORCE_EVERY_MS, false));
        assert!(reading.due(2_000 + READ_EVERY_MS, false));
        reading.record(Err(FetchError::Unauthorized), 3_000);
        assert!(reading.rejected);
        assert!(reading.usage.is_some(), "the last good reading stays");
        let mut older = parse_usage(&value, 1_500);
        older.week = None;
        reading.adopt(older);
        assert!(
            reading.usage.as_ref().unwrap().week.is_some(),
            "older loses"
        );
        reading.adopt(parse_usage(&serde_json::json!({}), 9_000));
        assert_eq!(reading.usage.as_ref().unwrap().as_of_ms, 9_000);
    }

    #[tokio::test]
    async fn the_client_refuses_other_hosts() {
        let client = UsageClient::new("http://10.1.2.3:9").unwrap();
        assert_eq!(
            client.fetch(&Secret::new("t".into())).await,
            Err(FetchError::NotAllowed)
        );
    }
}
