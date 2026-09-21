//! The operator-facing configuration, registered with the `configuration`
//! worker under the id `sentinel` and hot-reloaded in place.
//!
//! Every field has a shipped default, so a partial document is valid and an
//! unknown key is a typo rather than a silently ignored setting. What the
//! operator actually has to choose is small: which repositories map to which
//! workers, and which model an investigation opens with.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::str::FromStr;

use cron::Schedule;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::SentinelError;

/// Sources of error events. Both are engine triggers; turning one off
/// unbinds it rather than filtering downstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct SourcesConfigV1 {
    pub trace: TraceSourceConfigV1,
    pub log: LogSourceConfigV1,
}

impl Default for SourcesConfigV1 {
    fn default() -> Self {
        Self {
            trace: TraceSourceConfigV1 { enabled: true },
            log: LogSourceConfigV1 {
                enabled: true,
                join_window_ms: 2_000,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct TraceSourceConfigV1 {
    pub enabled: bool,
}

impl Default for TraceSourceConfigV1 {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct LogSourceConfigV1 {
    pub enabled: bool,
    /// How long an ERROR log waits for the error span of its own trace before
    /// it is promoted to a group of its own. Logs are emitted inside the
    /// handler and the span closes after, so the log almost always arrives
    /// first: this window is the wait, not a race.
    pub join_window_ms: u64,
}

impl Default for LogSourceConfigV1 {
    fn default() -> Self {
        Self {
            enabled: true,
            join_window_ms: 2_000,
        }
    }
}

/// The ingest circuit breaker. Retries cover a hiccup; these cover the
/// database being down, and stop the queue from amplifying it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct IngestConfigV1 {
    pub breaker_failures: u32,
    pub breaker_cooldown_ms: u64,
}

impl Default for IngestConfigV1 {
    fn default() -> Self {
        Self {
            breaker_failures: 5,
            breaker_cooldown_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct FingerprintConfigV1 {
    /// Markers after which a number is part of the error's identity and is
    /// never masked: `HTTP 429` and `HTTP 500` are different failures and
    /// must not collapse into one group.
    pub identity_numbers: Vec<String>,
}

impl Default for FingerprintConfigV1 {
    fn default() -> Self {
        Self {
            identity_numbers: ["HTTP", "status", "code", "errno", "exit", "signal", "port"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }
}

/// Extra value patterns to redact on capture. The built-in set (bearer
/// tokens, JWTs, prefixed API keys, PEM blocks, URL credentials, sensitive
/// `key=value` pairs, e-mail addresses) always runs; these are the shapes a
/// particular deployment also considers secret.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct RedactionConfigV1 {
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct EvidenceConfigV1 {
    /// Upper bound on one frozen bundle. Spans furthest from the origin are
    /// dropped first when a trace exceeds it.
    pub max_bytes: u64,
    /// How long after an error span the trace is re-read once, to catch the
    /// ancestors that were still open at capture time. One pass, not polling.
    pub settle_delay_ms: u64,
    /// Upper bound on the evidence digest inlined into the investigation
    /// message. The rest stays in `sentinel::evidence::get`.
    pub message_max_bytes: u64,
}

impl Default for EvidenceConfigV1 {
    fn default() -> Self {
        Self {
            max_bytes: 1_048_576,
            settle_delay_ms: 5_000,
            message_max_bytes: 65_536,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct RetentionConfigV1 {
    /// Bundles kept per group, on top of the first occurrence and one per
    /// distinct worker version — the older ones keep their row and lose their
    /// evidence.
    pub evidence_per_group: u32,
    pub occurrences_per_group: u32,
    pub buckets_days: u32,
    /// Resolved groups with no occurrence for this long are archived: the row
    /// stays, the evidence goes.
    pub resolved_ttl_days: u32,
    /// Six- or seven-field UTC cron expression for the daily prune.
    pub cron: String,
}

impl Default for RetentionConfigV1 {
    fn default() -> Self {
        Self {
            evidence_per_group: 5,
            occurrences_per_group: 1_000,
            buckets_days: 30,
            resolved_ttl_days: 90,
            cron: "0 0 3 * * *".into(),
        }
    }
}

/// The model an investigation opens with. There are no turn, token or cost
/// ceilings: the session runs beside the group's page, the user watches it,
/// and Stop is one click away.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct InvestigationConfigV1 {
    /// Catalog id such as `anthropic::claude-sonnet-5`. Empty means every
    /// investigation must name its own.
    pub model: String,
    /// Explicit provider, when `model` is not a catalog id that carries one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// Where a worker's source lives on this machine. A worker with no repository
/// is still grouped, still investigated — the agent just works from the
/// evidence alone and says so.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct RepositoryConfigV1 {
    pub id: String,
    /// Absolute path to a local checkout.
    pub path: String,
    /// Workers whose code lives in this checkout. A worker belongs to at most
    /// one repository.
    pub workers: Vec<String>,
}

/// Optional durable JSON copies in `storage`. The database stays the source
/// of truth; an empty bucket disables the copy entirely.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct ArchiveConfigV1 {
    pub bucket: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct WorkerConfig {
    pub enabled: bool,
    pub sources: SourcesConfigV1,
    pub ingest: IngestConfigV1,
    pub fingerprint: FingerprintConfigV1,
    /// Service names never ingested, on top of this worker's own.
    pub ignore_services: Vec<String>,
    /// Attribute keys kept in evidence beyond the built-in allowlist.
    pub attribute_allowlist: Vec<String>,
    pub redaction: RedactionConfigV1,
    pub evidence: EvidenceConfigV1,
    pub retention: RetentionConfigV1,
    pub investigation: InvestigationConfigV1,
    pub repositories: Vec<RepositoryConfigV1>,
    /// Span `service.name` to registered worker name, for SDKs that report a
    /// binary name instead of the name the worker registered with.
    pub service_aliases: BTreeMap<String, String>,
    /// `database` worker connection name.
    pub database: String,
    /// How long the worker/function registry snapshot is trusted.
    pub workers_ttl_ms: u64,
    pub archive: ArchiveConfigV1,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sources: SourcesConfigV1::default(),
            ingest: IngestConfigV1::default(),
            fingerprint: FingerprintConfigV1::default(),
            ignore_services: Vec::new(),
            attribute_allowlist: Vec::new(),
            redaction: RedactionConfigV1::default(),
            evidence: EvidenceConfigV1::default(),
            retention: RetentionConfigV1::default(),
            investigation: InvestigationConfigV1::default(),
            repositories: Vec::new(),
            service_aliases: BTreeMap::new(),
            database: "primary".into(),
            workers_ttl_ms: 60_000,
            archive: ArchiveConfigV1::default(),
        }
    }
}

impl WorkerConfig {
    /// Reject a configuration the worker cannot honour. A missing repository
    /// directory is deliberately *not* an error: it disables code access for
    /// those workers and shows as `exists: false` in `sentinel::status`,
    /// because a checkout that is moved or not yet cloned must not stop the
    /// monitor from grouping errors.
    pub fn validate(&self) -> Result<(), SentinelError> {
        self.validate_repositories()?;
        self.validate_sources()?;
        self.validate_evidence()?;
        self.validate_retention()?;

        for (index, pattern) in self.redaction.patterns.iter().enumerate() {
            regex::Regex::new(pattern).map_err(|error| {
                invalid(format!(
                    "redaction.patterns[{index}] is not a valid regular expression: {error}"
                ))
            })?;
        }

        for marker in &self.fingerprint.identity_numbers {
            if marker.trim().is_empty() || marker.split_whitespace().count() != 1 {
                return Err(invalid(
                    "fingerprint.identity_numbers entries must be single non-empty tokens",
                ));
            }
        }

        if self
            .investigation
            .provider
            .as_ref()
            .is_some_and(|provider| provider.trim().is_empty())
        {
            return Err(invalid("investigation.provider cannot be empty when set"));
        }

        for (span_service, worker) in &self.service_aliases {
            if span_service.trim().is_empty() || worker.trim().is_empty() {
                return Err(invalid("service_aliases keys and values cannot be empty"));
            }
        }

        if self.database.trim().is_empty() {
            return Err(invalid("database cannot be empty"));
        }
        if self.workers_ttl_ms < 1_000 {
            return Err(invalid("workers_ttl_ms must be at least 1000"));
        }
        if self.archive.bucket.trim().is_empty() && self.archive.prefix.is_some() {
            return Err(invalid("archive.prefix needs an archive.bucket"));
        }
        if self
            .archive
            .prefix
            .as_ref()
            .is_some_and(|prefix| prefix.contains("..") || prefix.contains('\\'))
        {
            return Err(invalid("archive.prefix cannot contain '..' or backslashes"));
        }
        Ok(())
    }

    fn validate_repositories(&self) -> Result<(), SentinelError> {
        let mut ids = HashSet::new();
        let mut workers = HashSet::new();
        for repository in &self.repositories {
            if repository.id.trim().is_empty() {
                return Err(invalid("repository id cannot be empty"));
            }
            if !ids.insert(repository.id.as_str()) {
                return Err(invalid(format!(
                    "repository id {} is configured more than once",
                    repository.id
                )));
            }
            if !Path::new(&repository.path).is_absolute() {
                return Err(invalid(format!(
                    "repository {} path must be absolute",
                    repository.id
                )));
            }
            for worker in &repository.workers {
                if worker.trim().is_empty() {
                    return Err(invalid(format!(
                        "repository {} lists an empty worker name",
                        repository.id
                    )));
                }
                if !workers.insert(worker.as_str()) {
                    return Err(invalid(format!(
                        "worker {worker} is mapped to more than one repository"
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_sources(&self) -> Result<(), SentinelError> {
        if !(100..=60_000).contains(&self.sources.log.join_window_ms) {
            return Err(invalid(
                "sources.log.join_window_ms must be between 100 and 60000",
            ));
        }
        if self.ingest.breaker_failures == 0 {
            return Err(invalid("ingest.breaker_failures must be positive"));
        }
        if self.ingest.breaker_cooldown_ms < 1_000 {
            return Err(invalid("ingest.breaker_cooldown_ms must be at least 1000"));
        }
        Ok(())
    }

    fn validate_evidence(&self) -> Result<(), SentinelError> {
        if self.evidence.max_bytes < 65_536 {
            return Err(invalid("evidence.max_bytes must be at least 65536"));
        }
        if self.evidence.message_max_bytes < 4_096 {
            return Err(invalid("evidence.message_max_bytes must be at least 4096"));
        }
        if self.evidence.message_max_bytes > self.evidence.max_bytes {
            return Err(invalid(
                "evidence.message_max_bytes cannot exceed evidence.max_bytes",
            ));
        }
        if self.evidence.settle_delay_ms > 60_000 {
            return Err(invalid("evidence.settle_delay_ms cannot exceed 60000"));
        }
        Ok(())
    }

    fn validate_retention(&self) -> Result<(), SentinelError> {
        if self.retention.evidence_per_group == 0 {
            return Err(invalid("retention.evidence_per_group must be positive"));
        }
        if self.retention.occurrences_per_group == 0 {
            return Err(invalid("retention.occurrences_per_group must be positive"));
        }
        if self.retention.buckets_days == 0 {
            return Err(invalid("retention.buckets_days must be positive"));
        }
        if self.retention.resolved_ttl_days == 0 {
            return Err(invalid("retention.resolved_ttl_days must be positive"));
        }
        let expression = self.retention.cron.trim();
        if expression != self.retention.cron
            || !matches!(expression.split_whitespace().count(), 6 | 7)
        {
            return Err(invalid(
                "retention.cron must be a trimmed six- or seven-field UTC cron expression",
            ));
        }
        Schedule::from_str(expression)
            .map_err(|error| invalid(format!("retention.cron is invalid: {error}")))?;
        Ok(())
    }

    /// The repository holding a worker's source, when one is mapped.
    pub fn repository_for_worker(&self, worker: &str) -> Option<&RepositoryConfigV1> {
        self.repositories
            .iter()
            .find(|repository| repository.workers.iter().any(|name| name == worker))
    }

    /// The registered worker name a span's `service.name` stands for.
    pub fn resolve_service_alias<'a>(&'a self, service_name: &'a str) -> &'a str {
        self.service_aliases
            .get(service_name)
            .map(String::as_str)
            .unwrap_or(service_name)
    }
}

fn invalid(message: impl Into<String>) -> SentinelError {
    SentinelError::InvalidRequest(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapped(id: &str, path: &str, workers: &[&str]) -> RepositoryConfigV1 {
        RepositoryConfigV1 {
            id: id.into(),
            path: path.into(),
            workers: workers.iter().map(|worker| worker.to_string()).collect(),
        }
    }

    #[test]
    fn shipped_defaults_are_idle_and_valid() {
        let config = WorkerConfig::default();
        assert!(config.repositories.is_empty());
        assert!(config.investigation.model.is_empty());
        assert!(config.archive.bucket.is_empty());
        assert_eq!(config.database, "primary");
        config.validate().expect("shipped defaults validate");
    }

    #[test]
    fn a_partial_document_fills_in_from_the_shipped_defaults() {
        let config: WorkerConfig = serde_json::from_value(serde_json::json!({
            "investigation": { "model": "anthropic::claude-sonnet-5" }
        }))
        .expect("a partial document is valid");
        assert_eq!(config.investigation.model, "anthropic::claude-sonnet-5");
        assert_eq!(config.sources.log.join_window_ms, 2_000);
        assert_eq!(config.retention.evidence_per_group, 5);
    }

    #[test]
    fn an_unknown_key_is_rejected_rather_than_silently_ignored() {
        let error = serde_json::from_value::<WorkerConfig>(serde_json::json!({
            "investigaton": { "model": "x" }
        }))
        .expect_err("a typo must not parse");
        assert!(error.to_string().contains("investigaton"));
    }

    #[test]
    fn a_relative_repository_path_is_rejected() {
        let config = WorkerConfig {
            repositories: vec![mapped("workers", "workspaces/workers", &["harness"])],
            ..WorkerConfig::default()
        };
        let error = config.validate().expect_err("relative path");
        assert!(error.to_string().contains("must be absolute"));
    }

    #[test]
    fn a_missing_directory_is_allowed_so_grouping_survives_a_moved_checkout() {
        let config = WorkerConfig {
            repositories: vec![mapped("workers", "/nonexistent/workers", &["harness"])],
            ..WorkerConfig::default()
        };
        config.validate().expect("a missing checkout is not fatal");
    }

    #[test]
    fn a_worker_cannot_belong_to_two_repositories() {
        let config = WorkerConfig {
            repositories: vec![
                mapped("workers", "/tmp/workers", &["harness"]),
                mapped("fork", "/tmp/fork", &["harness"]),
            ],
            ..WorkerConfig::default()
        };
        let error = config.validate().expect_err("duplicate worker mapping");
        assert!(error.to_string().contains("more than one repository"));
    }

    #[test]
    fn an_invalid_redaction_pattern_is_rejected() {
        let config = WorkerConfig {
            redaction: RedactionConfigV1 {
                patterns: vec!["([unclosed".into()],
            },
            ..WorkerConfig::default()
        };
        let error = config.validate().expect_err("invalid regex");
        assert!(error.to_string().contains("redaction.patterns[0]"));
    }

    #[test]
    fn an_invalid_prune_schedule_is_rejected() {
        let config = WorkerConfig {
            retention: RetentionConfigV1 {
                cron: "every tuesday".into(),
                ..RetentionConfigV1::default()
            },
            ..WorkerConfig::default()
        };
        let error = config.validate().expect_err("invalid cron");
        assert!(error.to_string().contains("retention.cron"));
    }

    #[test]
    fn the_message_digest_cannot_outgrow_the_bundle_it_comes_from() {
        let config = WorkerConfig {
            evidence: EvidenceConfigV1 {
                max_bytes: 65_536,
                message_max_bytes: 131_072,
                ..EvidenceConfigV1::default()
            },
            ..WorkerConfig::default()
        };
        let error = config.validate().expect_err("digest larger than bundle");
        assert!(error.to_string().contains("message_max_bytes"));
    }

    #[test]
    fn repository_and_alias_lookups_answer_what_the_ingest_asks() {
        let mut config = WorkerConfig {
            repositories: vec![mapped("workers", "/tmp/workers", &["harness", "ade"])],
            ..WorkerConfig::default()
        };
        config
            .service_aliases
            .insert("sentinel-bin".into(), "sentinel".into());

        assert_eq!(
            config.repository_for_worker("ade").map(|r| r.id.as_str()),
            Some("workers")
        );
        assert!(config.repository_for_worker("queue").is_none());
        assert_eq!(config.resolve_service_alias("sentinel-bin"), "sentinel");
        assert_eq!(config.resolve_service_alias("queue"), "queue");
    }
}
