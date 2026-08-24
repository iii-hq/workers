use std::collections::BTreeMap;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use super::*;

#[derive(Debug, Serialize)]
pub(super) struct GithubApiRequestWire {
    path: String,
    method: &'static str,
    fields: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paginate: Option<bool>,
    timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct GithubApiResponseWire {
    value: Value,
}

#[derive(Debug, Deserialize)]
struct RawDependabotAlertWire {
    number: u64,
    state: String,
    dependency: RawDependencyWire,
    security_advisory: RawSecurityAdvisoryWire,
    security_vulnerability: RawSecurityVulnerabilityWire,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct RawDependencyWire {
    package: RawPackageWire,
    manifest_path: String,
}

#[derive(Debug, Deserialize)]
struct RawPackageWire {
    ecosystem: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawSecurityAdvisoryWire {
    ghsa_id: String,
    cve_id: Option<String>,
    summary: String,
    severity: String,
}

#[derive(Debug, Deserialize)]
struct RawSecurityVulnerabilityWire {
    vulnerable_version_range: String,
}

#[derive(Debug, Deserialize)]
struct RawCodeScanningAlertWire {
    number: u64,
    state: String,
    rule: RawCodeScanningRuleWire,
    tool: RawCodeScanningToolWire,
    most_recent_instance: Option<RawCodeScanningInstanceWire>,
    created_at: String,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawCodeScanningRuleWire {
    id: String,
    name: Option<String>,
    description: String,
    security_severity_level: Option<String>,
    severity: String,
}

#[derive(Debug, Deserialize)]
struct RawCodeScanningToolWire {
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawCodeScanningInstanceWire {
    commit_sha: Option<String>,
    location: Option<RawCodeScanningLocationWire>,
}

#[derive(Debug, Deserialize)]
struct RawCodeScanningLocationWire {
    path: String,
    start_line: Option<u64>,
    end_line: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawCodeScanningAnalysisWire {
    tool: RawCodeScanningToolWire,
    commit_sha: Option<String>,
    created_at: Option<String>,
    error: Option<String>,
    warning: Option<String>,
}

pub(super) fn dependabot_api_request(
    repository: &str,
) -> Result<GithubApiRequestWire, SecurityScanError> {
    alerts_api_request(repository, "dependabot/alerts")
}

pub(super) fn code_scanning_alerts_api_request(
    repository: &str,
) -> Result<GithubApiRequestWire, SecurityScanError> {
    alerts_api_request(repository, "code-scanning/alerts")
}

pub(super) fn code_scanning_analysis_api_request(
    repository: &str,
) -> Result<GithubApiRequestWire, SecurityScanError> {
    validate_repository(repository)?;
    Ok(GithubApiRequestWire {
        path: format!("repos/{repository}/code-scanning/analyses"),
        method: "GET",
        fields: BTreeMap::from([("per_page".into(), "1".into())]),
        paginate: None,
        timeout_ms: RPC_TIMEOUT_MS,
    })
}

fn alerts_api_request(
    repository: &str,
    resource: &str,
) -> Result<GithubApiRequestWire, SecurityScanError> {
    validate_repository(repository)?;
    Ok(GithubApiRequestWire {
        path: format!("repos/{repository}/{resource}"),
        method: "GET",
        fields: BTreeMap::from([
            ("per_page".into(), "100".into()),
            ("state".into(), "open".into()),
        ]),
        paginate: Some(true),
        timeout_ms: RPC_TIMEOUT_MS,
    })
}

fn validate_repository(repository: &str) -> Result<(), SecurityScanError> {
    if crate::config::is_valid_github_full_name(repository) {
        Ok(())
    } else {
        Err(SecurityScanError::Dependency(
            "configured GitHub repository is not a valid owner/name".into(),
        ))
    }
}

pub(super) fn dependabot_api_response(
    repository: &str,
    response: Result<GithubApiResponseWire, SecurityScanError>,
) -> DependabotAlertsResponseWire {
    let alerts = match response {
        Ok(response) => match parse_api_pages::<RawDependabotAlertWire>(response.value) {
            Ok(alerts) => alerts,
            Err(_) => {
                return unavailable_dependabot_response(
                    repository,
                    GithubAvailabilityWire::MalformedResponse,
                )
            }
        },
        Err(error) => {
            return unavailable_dependabot_response(repository, classify_github_api_error(&error))
        }
    };
    let (completeness, alerts) = bounded_alerts(
        alerts
            .into_iter()
            .map(|alert| DependabotAlertWire {
                number: alert.number,
                state: alert.state,
                severity: alert.security_advisory.severity,
                package_name: alert.dependency.package.name,
                ecosystem: alert.dependency.package.ecosystem,
                manifest_path: alert.dependency.manifest_path,
                ghsa_id: alert.security_advisory.ghsa_id,
                cve_id: alert.security_advisory.cve_id,
                advisory_summary: alert.security_advisory.summary,
                vulnerable_version_range: alert.security_vulnerability.vulnerable_version_range,
                updated_at: alert.updated_at,
            })
            .collect(),
    );
    DependabotAlertsResponseWire {
        repository: repository.into(),
        completeness,
        availability: GithubAvailabilityWire::Available,
        collected_count: alerts.len(),
        alerts,
    }
}

pub(super) fn code_scanning_api_response(
    repository: &str,
    alerts_response: Result<GithubApiResponseWire, SecurityScanError>,
    analysis_response: Result<GithubApiResponseWire, SecurityScanError>,
) -> CodeScanningAlertsResponseWire {
    let (completeness, availability, alerts) = match alerts_response {
        Ok(response) => match parse_api_pages::<RawCodeScanningAlertWire>(response.value) {
            Ok(alerts) => {
                let (completeness, alerts) = bounded_alerts(
                    alerts
                        .into_iter()
                        .map(|alert| {
                            let (commit_sha, path, start_line, end_line) =
                                match alert.most_recent_instance {
                                    Some(instance) => match instance.location {
                                        Some(location) => (
                                            instance.commit_sha,
                                            Some(location.path),
                                            location.start_line,
                                            location.end_line,
                                        ),
                                        None => (instance.commit_sha, None, None, None),
                                    },
                                    None => (None, None, None, None),
                                };
                            CodeScanningAlertWire {
                                number: alert.number,
                                state: alert.state,
                                rule_id: alert.rule.id,
                                rule_name: alert.rule.name,
                                rule_description: alert.rule.description,
                                security_severity: alert.rule.security_severity_level,
                                severity: alert.rule.severity,
                                tool_name: alert.tool.name,
                                commit_sha,
                                path,
                                start_line,
                                end_line,
                                created_at: alert.created_at,
                                updated_at: alert.updated_at,
                            }
                        })
                        .collect(),
                );
                (completeness, GithubAvailabilityWire::Available, alerts)
            }
            Err(_) => (
                GithubCompletenessWire::Partial,
                GithubAvailabilityWire::MalformedResponse,
                Vec::new(),
            ),
        },
        Err(error) => (
            GithubCompletenessWire::Partial,
            classify_github_api_error(&error),
            Vec::new(),
        ),
    };
    CodeScanningAlertsResponseWire {
        repository: repository.into(),
        completeness,
        availability,
        collected_count: alerts.len(),
        alerts,
        latest_analysis: latest_analysis(analysis_response),
    }
}

fn latest_analysis(
    response: Result<GithubApiResponseWire, SecurityScanError>,
) -> LatestCodeScanningAnalysisWire {
    let unavailable = |availability| LatestCodeScanningAnalysisWire {
        availability,
        tool_name: None,
        commit_sha: None,
        created_at: None,
        error: None,
        warning: None,
    };
    let analyses = match response {
        Ok(response) => match parse_api_pages::<RawCodeScanningAnalysisWire>(response.value) {
            Ok(analyses) => analyses,
            Err(_) => return unavailable(GithubAvailabilityWire::MalformedResponse),
        },
        Err(error) => return unavailable(classify_github_api_error(&error)),
    };
    let Some(analysis) = analyses.into_iter().next() else {
        return unavailable(GithubAvailabilityWire::Available);
    };
    LatestCodeScanningAnalysisWire {
        availability: GithubAvailabilityWire::Available,
        tool_name: Some(analysis.tool.name),
        commit_sha: analysis.commit_sha,
        created_at: analysis.created_at,
        error: analysis.error,
        warning: analysis.warning,
    }
}

fn unavailable_dependabot_response(
    repository: &str,
    availability: GithubAvailabilityWire,
) -> DependabotAlertsResponseWire {
    DependabotAlertsResponseWire {
        repository: repository.into(),
        completeness: GithubCompletenessWire::Partial,
        availability,
        collected_count: 0,
        alerts: Vec::new(),
    }
}

fn bounded_alerts<T>(mut alerts: Vec<T>) -> (GithubCompletenessWire, Vec<T>) {
    let completeness = if alerts.len() > GITHUB_ALERT_LIMIT {
        alerts.truncate(GITHUB_ALERT_LIMIT);
        GithubCompletenessWire::Partial
    } else {
        GithubCompletenessWire::Complete
    };
    (completeness, alerts)
}

fn parse_api_pages<T>(value: Value) -> Result<Vec<T>, SecurityScanError>
where
    T: DeserializeOwned,
{
    match value {
        Value::Array(values) => parse_api_page(Value::Array(values)),
        Value::String(output) => {
            let mut records = Vec::new();
            let mut page_count = 0;
            for page in serde_json::Deserializer::from_str(&output).into_iter::<Value>() {
                let page = page.map_err(|_| malformed_api_response())?;
                records.extend(parse_api_page(page)?);
                page_count += 1;
            }
            if page_count == 0 {
                return Err(malformed_api_response());
            }
            Ok(records)
        }
        _ => Err(malformed_api_response()),
    }
}

fn parse_api_page<T>(page: Value) -> Result<Vec<T>, SecurityScanError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(page).map_err(|_| malformed_api_response())
}

fn malformed_api_response() -> SecurityScanError {
    SecurityScanError::Dependency("github::api returned malformed alert data".into())
}

fn classify_github_api_error(error: &SecurityScanError) -> GithubAvailabilityWire {
    let message = error.to_string().to_ascii_lowercase();
    if contains_any(
        &message,
        &[
            "not enabled",
            "must be enabled",
            "dependabot alerts are disabled",
            "code scanning is disabled",
            "advanced security is disabled",
        ],
    ) {
        GithubAvailabilityWire::FeatureDisabled
    } else if contains_any(
        &message,
        &[
            "http 401",
            "bad credentials",
            "authentication required",
            "gh auth login",
            "not logged into",
        ],
    ) {
        GithubAvailabilityWire::AuthenticationRequired
    } else if contains_any(
        &message,
        &[
            "http 403",
            "forbidden",
            "resource not accessible",
            "insufficient permission",
        ],
    ) {
        GithubAvailabilityWire::PermissionDenied
    } else if contains_any(&message, &["http 404", "not found"]) {
        GithubAvailabilityWire::RepositoryUnavailable
    } else {
        GithubAvailabilityWire::TemporarilyUnavailable
    }
}

fn contains_any(message: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| message.contains(needle))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn dependabot_alert(number: u64) -> Value {
        json!({
            "number": number,
            "state": "open",
            "dependency": {
                "package": { "ecosystem": "cargo", "name": "demo" },
                "manifest_path": "Cargo.lock"
            },
            "security_advisory": {
                "ghsa_id": "GHSA-demo",
                "cve_id": "CVE-2026-1",
                "summary": "short summary",
                "severity": "high"
            },
            "security_vulnerability": {
                "vulnerable_version_range": "< 2.0.0"
            },
            "updated_at": "2026-01-02T00:00:00Z"
        })
    }

    #[test]
    fn api_requests_use_existing_read_only_escape_hatch_contract() {
        let request = serde_json::to_value(dependabot_api_request("iii-hq/iii").unwrap()).unwrap();
        assert_eq!(request["path"], "repos/iii-hq/iii/dependabot/alerts");
        assert_eq!(request["method"], "GET");
        assert_eq!(request["fields"]["state"], "open");
        assert_eq!(request["fields"]["per_page"], "100");
        assert_eq!(request["paginate"], true);
        assert_eq!(request["timeout_ms"], RPC_TIMEOUT_MS);
    }

    #[test]
    fn paginated_api_output_is_flattened_and_bounded() {
        let pages = format!(
            "{}\n{}",
            json!([dependabot_alert(1)]),
            json!([dependabot_alert(2)])
        );
        let response = dependabot_api_response(
            "iii-hq/iii",
            Ok(GithubApiResponseWire {
                value: Value::String(pages),
            }),
        );
        assert_eq!(response.completeness, GithubCompletenessWire::Complete);
        assert_eq!(response.collected_count, 2);

        let response = dependabot_api_response(
            "iii-hq/iii",
            Ok(GithubApiResponseWire {
                value: Value::Array(
                    (0..=GITHUB_ALERT_LIMIT as u64)
                        .map(dependabot_alert)
                        .collect(),
                ),
            }),
        );
        assert_eq!(response.completeness, GithubCompletenessWire::Partial);
        assert_eq!(response.collected_count, GITHUB_ALERT_LIMIT);
    }

    #[test]
    fn api_failures_are_classified_without_retaining_diagnostics() {
        let response = dependabot_api_response(
            "iii-hq/iii",
            Err(SecurityScanError::Dependency(
                "github::api failed: HTTP 401 bad credentials token-secret".into(),
            )),
        );
        assert_eq!(
            response.availability,
            GithubAvailabilityWire::AuthenticationRequired
        );
        assert!(response.alerts.is_empty());
    }
}
