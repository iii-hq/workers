use std::{collections::HashSet, path::Path, str::FromStr};

use cron::Schedule;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ScanModeV1, SecurityScanError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepositoryScheduleV1 {
    /// Six-field (seconds through weekday) or seven-field (plus year) UTC cron expression.
    pub expression: String,
    /// Local Git ref resolved to a commit when the schedule fires.
    pub target_ref: String,
    pub mode: ScanModeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepositoryGitHubConfigV1 {
    /// GitHub repository in the exact form `owner/name`.
    pub full_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepositoryConfigV1 {
    pub id: String,
    pub path: String,
    /// Omit when this local checkout has no operator-verified GitHub mapping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<RepositoryGitHubConfigV1>,
    /// Omit to disable scheduled scans for this repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<RepositoryScheduleV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisConfigV1 {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub max_turns: u32,
    pub max_output_tokens: u64,
    pub max_total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchiveConfigV1 {
    /// Worker-facing `storage` bucket that stores JSON run records.
    pub bucket: String,
    /// Object key prefix. Defaults to `runs/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub repositories: Vec<RepositoryConfigV1>,
    pub analysis: AnalysisConfigV1,
    /// Optional `storage` bucket for durable JSON copies of run records.
    /// History in `state` remains authoritative for the Console list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveConfigV1>,
}

impl WorkerConfig {
    pub fn validate(&self) -> Result<(), SecurityScanError> {
        let mut ids = HashSet::new();
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
            if let Some(github) = &repository.github {
                if !is_valid_github_full_name(&github.full_name) {
                    return Err(invalid(format!(
                        "repository {} github.full_name must be exactly owner/name using letters, digits, '.', '_' or '-'",
                        repository.id
                    )));
                }
            }
            if let Some(schedule) = &repository.schedule {
                validate_schedule(&repository.id, schedule)?;
            }
        }
        if !self.repositories.is_empty() && self.analysis.model.trim().is_empty() {
            return Err(invalid("analysis.model cannot be empty"));
        }
        if self
            .analysis
            .provider
            .as_ref()
            .is_some_and(|provider| provider.trim().is_empty())
        {
            return Err(invalid("analysis.provider cannot be empty when set"));
        }
        if !(1..=10).contains(&self.analysis.max_turns) {
            return Err(invalid("analysis.max_turns must be between 1 and 10"));
        }
        if self.analysis.max_output_tokens == 0 {
            return Err(invalid("analysis.max_output_tokens must be positive"));
        }
        if self.analysis.max_total_tokens < self.analysis.max_output_tokens {
            return Err(invalid(
                "analysis.max_total_tokens must be at least max_output_tokens",
            ));
        }
        if self
            .analysis
            .max_cost_usd
            .is_some_and(|cost| !cost.is_finite() || cost <= 0.0)
        {
            return Err(invalid(
                "analysis.max_cost_usd must be finite and positive when set",
            ));
        }
        if let Some(archive) = &self.archive {
            if archive.bucket.trim().is_empty() {
                return Err(invalid("archive.bucket cannot be empty"));
            }
            if archive
                .prefix
                .as_ref()
                .is_some_and(|prefix| prefix.contains("..") || prefix.contains('\\'))
            {
                return Err(invalid("archive.prefix cannot contain '..' or backslashes"));
            }
        }
        Ok(())
    }

    pub(crate) fn repository(&self, id: &str) -> Option<&RepositoryConfigV1> {
        self.repositories
            .iter()
            .find(|repository| repository.id == id)
    }
}

pub(crate) fn is_valid_github_full_name(full_name: &str) -> bool {
    let mut parts = full_name.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    parts.next().is_none() && is_valid_github_name(owner) && is_valid_github_name(name)
}

pub(crate) fn is_valid_github_name(value: &str) -> bool {
    value
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !value.ends_with('.')
        && !value.contains("..")
}

fn validate_schedule(
    repository_id: &str,
    schedule: &RepositoryScheduleV1,
) -> Result<(), SecurityScanError> {
    let expression = schedule.expression.trim();
    let field_count = expression.split_whitespace().count();
    if expression != schedule.expression || !matches!(field_count, 6 | 7) {
        return Err(invalid(format!(
            "repository {repository_id} schedule.expression must be a trimmed six- or seven-field UTC cron expression"
        )));
    }
    Schedule::from_str(expression).map_err(|error| {
        invalid(format!(
            "repository {repository_id} schedule.expression is invalid: {error}"
        ))
    })?;
    if !is_safe_target_ref(&schedule.target_ref) {
        return Err(invalid(format!(
            "repository {repository_id} schedule.target_ref is not a valid Git ref"
        )));
    }
    Ok(())
}

fn is_safe_target_ref(target_ref: &str) -> bool {
    if target_ref.is_empty()
        || target_ref.len() > 1_024
        || target_ref == "@"
        || target_ref.trim() != target_ref
        || target_ref.starts_with('-')
        || target_ref.starts_with('/')
        || target_ref.ends_with('/')
        || target_ref.ends_with('.')
        || target_ref.contains("//")
        || target_ref.contains("..")
        || target_ref.contains("@{")
        || target_ref.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(character, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
    {
        return false;
    }

    target_ref
        .split('/')
        .all(|component| !component.starts_with('.') && !component.ends_with(".lock"))
}

fn invalid(message: impl Into<String>) -> SecurityScanError {
    SecurityScanError::InvalidRequest(message.into())
}
