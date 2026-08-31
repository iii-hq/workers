#[derive(Debug)]
enum CasOutcome {
    Swapped,
    Current(Value),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunIndexRecordV1 {
    schema_version: String,
    summary: PublicRunSummaryV1,
    has_materialized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    harness_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ActionSessionIndexRecordV1 {
    action_id: String,
}

impl From<&RunRecordV1> for RunIndexRecordV1 {
    fn from(run: &RunRecordV1) -> Self {
        Self {
            schema_version: "1".into(),
            summary: PublicRunSummaryV1::from(run),
            has_materialized: run.materialized.is_some(),
            harness_session_id: run
                .harness
                .as_ref()
                .map(|harness| harness.session_id.clone()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct StorageGetWire {
    body_base64: String,
}

#[derive(Debug, Deserialize)]
struct WorktreeListWire {
    #[serde(default)]
    worktrees: Vec<WorktreeWire>,
}

#[derive(Debug, Deserialize)]
struct WorktreeWire {
    worktree_id: String,
    repo_path: String,
    path: String,
    base_sha: String,
    lifecycle: String,
}

#[derive(Debug, Deserialize)]
struct WorktreeCreateWire {
    worktree_id: String,
    path: String,
    base_sha: String,
}

#[derive(Debug, Deserialize)]
struct HarnessSendWire {
    session_id: String,
    turn_id: String,
    accepted: bool,
}

#[derive(Debug, Deserialize)]
struct HarnessStatusWire {
    #[serde(default)]
    turn_id: Option<String>,
    status: String,
    #[serde(default)]
    expects_wake: bool,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    result_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct GithubApiRequestWire {
    path: String,
    method: &'static str,
    fields: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paginate: Option<bool>,
    timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
struct GithubApiResponseWire {
    value: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GithubCompletenessWire {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GithubAvailabilityWire {
    Available,
    AuthenticationRequired,
    PermissionDenied,
    FeatureDisabled,
    RepositoryUnavailable,
    TemporarilyUnavailable,
    ClientUnavailable,
    MalformedResponse,
}

#[derive(Debug, Deserialize)]
struct DependabotAlertsResponseWire {
    repository: String,
    completeness: GithubCompletenessWire,
    availability: GithubAvailabilityWire,
    collected_count: usize,
    alerts: Vec<DependabotAlertWire>,
}

#[derive(Debug, Deserialize)]
struct DependabotAlertWire {
    number: u64,
    state: String,
    severity: String,
    package_name: String,
    ecosystem: String,
    manifest_path: String,
    ghsa_id: String,
    cve_id: Option<String>,
    advisory_summary: String,
    vulnerable_version_range: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct CodeScanningAlertsResponseWire {
    repository: String,
    completeness: GithubCompletenessWire,
    availability: GithubAvailabilityWire,
    collected_count: usize,
    alerts: Vec<CodeScanningAlertWire>,
    latest_analysis: LatestCodeScanningAnalysisWire,
}

#[derive(Debug, Deserialize)]
struct CodeScanningAlertWire {
    number: u64,
    state: String,
    rule_id: String,
    rule_name: Option<String>,
    rule_description: String,
    security_severity: Option<String>,
    severity: String,
    tool_name: String,
    commit_sha: Option<String>,
    path: Option<String>,
    start_line: Option<u64>,
    end_line: Option<u64>,
    created_at: String,
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LatestCodeScanningAnalysisWire {
    availability: GithubAvailabilityWire,
    tool_name: Option<String>,
    commit_sha: Option<String>,
    created_at: Option<String>,
    error: Option<String>,
    warning: Option<String>,
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

fn dependabot_api_request(repository: &str) -> Result<GithubApiRequestWire, SecurityScanError> {
    alerts_api_request(repository, "dependabot/alerts")
}

fn code_scanning_alerts_api_request(
    repository: &str,
) -> Result<GithubApiRequestWire, SecurityScanError> {
    alerts_api_request(repository, "code-scanning/alerts")
}

fn code_scanning_analysis_api_request(
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

fn dependabot_api_response(
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

fn code_scanning_api_response(
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

fn normalize_dependabot_response(
    github_full_name: &str,
    collected_at: i64,
    response: DependabotAlertsResponseWire,
) -> Result<ReconciliationSourceCollectionV1, SecurityScanError> {
    validate_github_response(
        github_full_name,
        &response.repository,
        response.collected_count,
        response.alerts.len(),
    )?;
    let status = source_status(response.completeness, response.availability);
    let available = response.availability == GithubAvailabilityWire::Available;
    let records = if available {
        response
            .alerts
            .into_iter()
            .map(|alert| normalize_dependabot_alert(github_full_name, alert))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let record_count = available.then(|| count_u32(records.len()));
    let health = ReconciliationSourceHealthV1 {
        status: match status {
            ReconciliationSourceStatusV1::Complete => ReconciliationHealthStatusV1::Healthy,
            ReconciliationSourceStatusV1::Partial => ReconciliationHealthStatusV1::Warning,
            _ => ReconciliationHealthStatusV1::Unknown,
        },
        tool: None,
        commit_sha: None,
        observed_at: None,
    };
    Ok(ReconciliationSourceCollectionV1 {
        summary: ReconciliationSourceSummaryV1 {
            source: ReconciliationSourceV1::Dependabot,
            status,
            scope: ReconciliationScopeV1::RepositoryDefaultBranch,
            collected_at: Some(collected_at),
            record_count,
            health,
        },
        records,
    })
}

fn normalize_code_scanning_response(
    github_full_name: &str,
    target_sha: &str,
    collected_at: i64,
    response: CodeScanningAlertsResponseWire,
) -> Result<ReconciliationSourceCollectionV1, SecurityScanError> {
    validate_github_response(
        github_full_name,
        &response.repository,
        response.collected_count,
        response.alerts.len(),
    )?;
    let primary_available = response.availability == GithubAvailabilityWire::Available;
    let mut records = if primary_available {
        response
            .alerts
            .into_iter()
            .map(|alert| normalize_code_scanning_alert(github_full_name, target_sha, alert))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let mut status = source_status(response.completeness, response.availability);
    let mut record_count = primary_available.then(|| count_u32(records.len()));
    let latest_available =
        response.latest_analysis.availability == GithubAvailabilityWire::Available;
    if primary_available && !latest_available {
        if records.is_empty() {
            status = unavailable_status(response.latest_analysis.availability);
            record_count = None;
        } else {
            status = ReconciliationSourceStatusV1::Partial;
        }
    }
    if !primary_available {
        records.clear();
    }
    let health = code_scanning_health(&response.latest_analysis);
    Ok(ReconciliationSourceCollectionV1 {
        summary: ReconciliationSourceSummaryV1 {
            source: ReconciliationSourceV1::CodeScanning,
            status,
            scope: ReconciliationScopeV1::RepositorySnapshot,
            collected_at: Some(collected_at),
            record_count,
            health,
        },
        records,
    })
}

fn normalize_dependabot_alert(
    github_full_name: &str,
    alert: DependabotAlertWire,
) -> Result<ReconciliationAlertV1, SecurityScanError> {
    validate_open_state(&alert.state)?;
    let package_name = sanitize_public_text(&alert.package_name, 256);
    let ecosystem = sanitize_public_text(&alert.ecosystem, 64);
    let vulnerable_range = sanitize_public_text(&alert.vulnerable_version_range, 512);
    let mut structured_ids = Vec::new();
    if let Some(identifier) = structured_identifier(&alert.ghsa_id) {
        structured_ids.push(identifier);
    }
    if let Some(identifier) = alert.cve_id.as_deref().and_then(structured_identifier) {
        if !structured_ids.contains(&identifier) {
            structured_ids.push(identifier);
        }
    }
    Ok(ReconciliationAlertV1 {
        source: ReconciliationSourceV1::Dependabot,
        number: alert.number,
        severity: normalize_severity(&alert.severity),
        lifecycle: ReconciliationLifecycleV1::Open,
        scope: ReconciliationScopeV1::RepositoryDefaultBranch,
        title: sanitize_public_text(&alert.advisory_summary, 512),
        description: format!(
            "Affected package {package_name} ({ecosystem}); vulnerable range {vulnerable_range}."
        ),
        public_url: github_alert_url(
            github_full_name,
            ReconciliationSourceV1::Dependabot,
            alert.number,
        )?,
        structured_ids,
        path: safe_repository_path(&alert.manifest_path),
        start_line: None,
        end_line: None,
        observed_at: nonempty_text(&alert.updated_at, 64),
    })
}

fn normalize_code_scanning_alert(
    github_full_name: &str,
    target_sha: &str,
    alert: CodeScanningAlertWire,
) -> Result<ReconciliationAlertV1, SecurityScanError> {
    validate_open_state(&alert.state)?;
    let scope = if alert
        .commit_sha
        .as_deref()
        .is_some_and(|sha| sha.eq_ignore_ascii_case(target_sha))
    {
        ReconciliationScopeV1::ExactCommit
    } else {
        ReconciliationScopeV1::RepositorySnapshot
    };
    let rule_id = structured_identifier(&alert.rule_id);
    let title = alert
        .rule_name
        .as_deref()
        .map(|value| sanitize_public_text(value, 256))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| sanitize_public_text(&alert.rule_description, 512));
    let mut description = sanitize_public_text(&alert.rule_description, 512);
    if description.is_empty() {
        description = "Code-scanning alert".into();
    }
    let observed_at = alert
        .updated_at
        .as_deref()
        .and_then(|value| nonempty_text(value, 64))
        .or_else(|| nonempty_text(&alert.created_at, 64));
    let severity = alert
        .security_severity
        .as_deref()
        .unwrap_or(&alert.severity);
    let _tool_name = sanitize_public_text(&alert.tool_name, 256);
    Ok(ReconciliationAlertV1 {
        source: ReconciliationSourceV1::CodeScanning,
        number: alert.number,
        severity: normalize_severity(severity),
        lifecycle: ReconciliationLifecycleV1::Open,
        scope,
        title,
        description,
        public_url: github_alert_url(
            github_full_name,
            ReconciliationSourceV1::CodeScanning,
            alert.number,
        )?,
        structured_ids: rule_id.into_iter().collect(),
        path: alert.path.as_deref().and_then(safe_repository_path),
        start_line: alert.start_line,
        end_line: alert.end_line,
        observed_at,
    })
}

fn source_status(
    completeness: GithubCompletenessWire,
    availability: GithubAvailabilityWire,
) -> ReconciliationSourceStatusV1 {
    if availability != GithubAvailabilityWire::Available {
        return unavailable_status(availability);
    }
    match completeness {
        GithubCompletenessWire::Complete => ReconciliationSourceStatusV1::Complete,
        GithubCompletenessWire::Partial => ReconciliationSourceStatusV1::Partial,
    }
}

fn unavailable_status(availability: GithubAvailabilityWire) -> ReconciliationSourceStatusV1 {
    match availability {
        GithubAvailabilityWire::Available => ReconciliationSourceStatusV1::Complete,
        GithubAvailabilityWire::AuthenticationRequired => {
            ReconciliationSourceStatusV1::AuthenticationRequired
        }
        GithubAvailabilityWire::PermissionDenied => ReconciliationSourceStatusV1::PermissionDenied,
        GithubAvailabilityWire::FeatureDisabled => ReconciliationSourceStatusV1::Disabled,
        GithubAvailabilityWire::RepositoryUnavailable
        | GithubAvailabilityWire::TemporarilyUnavailable
        | GithubAvailabilityWire::ClientUnavailable
        | GithubAvailabilityWire::MalformedResponse => ReconciliationSourceStatusV1::Unavailable,
    }
}

fn code_scanning_health(latest: &LatestCodeScanningAnalysisWire) -> ReconciliationSourceHealthV1 {
    let tool = latest
        .tool_name
        .as_deref()
        .and_then(|value| nonempty_text(value, 256));
    let commit_sha = latest.commit_sha.as_deref().and_then(validated_sha);
    let observed_at = latest
        .created_at
        .as_deref()
        .and_then(|value| nonempty_text(value, 64));
    let status = if latest.availability != GithubAvailabilityWire::Available {
        ReconciliationHealthStatusV1::Unknown
    } else if latest.error.is_some() {
        ReconciliationHealthStatusV1::Error
    } else if latest.warning.is_some() {
        ReconciliationHealthStatusV1::Warning
    } else if tool.is_some() || commit_sha.is_some() || observed_at.is_some() {
        ReconciliationHealthStatusV1::Healthy
    } else {
        ReconciliationHealthStatusV1::Unknown
    };
    ReconciliationSourceHealthV1 {
        status,
        tool,
        commit_sha,
        observed_at,
    }
}

fn validate_github_response(
    expected_repository: &str,
    actual_repository: &str,
    collected_count: usize,
    alert_count: usize,
) -> Result<(), SecurityScanError> {
    if !crate::config::is_valid_github_full_name(expected_repository)
        || actual_repository != expected_repository
    {
        return Err(SecurityScanError::Dependency(
            "GitHub security response repository did not match the configured mapping".into(),
        ));
    }
    if collected_count != alert_count {
        return Err(SecurityScanError::Dependency(
            "GitHub security response count did not match its alert records".into(),
        ));
    }
    Ok(())
}

fn validate_open_state(state: &str) -> Result<(), SecurityScanError> {
    if state.eq_ignore_ascii_case("open") {
        Ok(())
    } else {
        Err(SecurityScanError::Dependency(
            "GitHub security response contained a non-open alert".into(),
        ))
    }
}

fn github_alert_url(
    github_full_name: &str,
    source: ReconciliationSourceV1,
    number: u64,
) -> Result<String, SecurityScanError> {
    if !crate::config::is_valid_github_full_name(github_full_name) {
        return Err(SecurityScanError::Dependency(
            "configured GitHub repository is not a valid owner/name".into(),
        ));
    }
    let kind = match source {
        ReconciliationSourceV1::Dependabot => "dependabot",
        ReconciliationSourceV1::CodeScanning => "code-scanning",
    };
    Ok(format!(
        "https://github.com/{github_full_name}/security/{kind}/{number}"
    ))
}

fn normalize_severity(value: &str) -> SeverityV1 {
    match value.trim().to_ascii_lowercase().as_str() {
        "critical" => SeverityV1::Critical,
        "high" | "error" => SeverityV1::High,
        "medium" | "moderate" | "warning" => SeverityV1::Medium,
        "low" => SeverityV1::Low,
        _ => SeverityV1::Info,
    }
}

fn safe_repository_path(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.starts_with('/')
        || value.contains('\\')
        || value.split('/').any(|part| part.is_empty() || part == "..")
        || value.chars().any(char::is_control)
    {
        return None;
    }
    nonempty_text(value, 1_024)
}

fn structured_identifier(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 256
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        })
    {
        return None;
    }
    Some(value.to_string())
}

fn validated_sha(value: &str) -> Option<String> {
    (value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn nonempty_text(value: &str, max_chars: usize) -> Option<String> {
    let value = sanitize_public_text(value, max_chars);
    (!value.is_empty()).then_some(value)
}

fn sanitize_public_text(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    let mut pending_space = false;
    for character in value.chars() {
        if output.chars().count() == max_chars {
            break;
        }
        if character.is_control() || character.is_whitespace() {
            pending_space = !output.is_empty();
            continue;
        }
        if pending_space {
            output.push(' ');
            pending_space = false;
        }
        output.push(character);
    }
    output.trim().to_string()
}

fn count_u32(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn materialized_from_existing(
    worktree: WorktreeWire,
    repository: &RepositoryConfigV1,
    target_sha: &str,
) -> Result<MaterializedTargetV1, SecurityScanError> {
    if worktree.repo_path != repository.path {
        return Err(SecurityScanError::Dependency(format!(
            "recovered worktree {} belongs to an unexpected repository",
            worktree.worktree_id
        )));
    }
    materialized(
        worktree.worktree_id,
        worktree.path,
        worktree.base_sha,
        target_sha,
    )
}

fn materialized_from_created(
    worktree: WorktreeCreateWire,
    target_sha: &str,
) -> Result<MaterializedTargetV1, SecurityScanError> {
    materialized(
        worktree.worktree_id,
        worktree.path,
        worktree.base_sha,
        target_sha,
    )
}

fn materialized(
    worktree_id: String,
    path: String,
    base_sha: String,
    target_sha: &str,
) -> Result<MaterializedTargetV1, SecurityScanError> {
    if !base_sha.eq_ignore_ascii_case(target_sha) {
        return Err(SecurityScanError::Dependency(format!(
            "worktree resolved {} instead of requested {}",
            base_sha, target_sha
        )));
    }
    Ok(MaterializedTargetV1 {
        worktree_id,
        path,
        base_sha,
    })
}

fn harness_request(plan: &AnalysisPlan) -> Value {
    let session_metadata = match &plan.run_id {
        Some(run_id) => json!({
            "security_scan": true,
            "security_scan_run_id": run_id,
        }),
        None => json!({ "security_scan": true }),
    };
    json!({
        "session_id": plan.session_id,
        "message": plan.message,
        "model": plan.model,
        "provider": plan.provider,
        "idempotency_key": plan.idempotency_key,
        "session": {
            "title": "Security review",
            "metadata": session_metadata,
        },
        "options": {
            "system_prompt": plan.system_prompt,
            "system_prompt_strategy": "override",
            "mode": "agent",
            "max_turns": plan.max_turns,
            "max_output_tokens": plan.max_output_tokens,
            "max_total_tokens": plan.max_total_tokens,
            "max_cost_usd": plan.max_cost_usd,
            "output": {
                "type": "json",
                "schema": plan.output_schema,
            },
            "functions": {
                "allow": plan.allowed_functions,
                "deny": plan.denied_functions,
                "expose": "agent_trigger",
            },
            "metadata": if plan.filesystem_root.is_empty() {
                json!({})
            } else {
                json!({ "fs_scope": { "root": plan.filesystem_root } })
            },
        },
    })
}

fn completion_event(
    status: HarnessStatusWire,
    harness: &crate::HarnessRunV1,
) -> Result<Option<crate::TurnCompletedEventV1>, SecurityScanError> {
    if status.turn_id.as_deref() != Some(harness.turn_id.as_str()) {
        return Ok(None);
    }
    if status.expects_wake || matches!(status.status.as_str(), "running" | "awaiting_functions") {
        return Ok(None);
    }
    if !matches!(status.status.as_str(), "completed" | "cancelled" | "failed") {
        return Err(SecurityScanError::Dependency(format!(
            "harness::status returned unknown status {}",
            status.status
        )));
    }
    Ok(Some(crate::TurnCompletedEventV1 {
        session_id: harness.session_id.clone(),
        turn_id: harness.turn_id.clone(),
        status: status.status,
        terminal: true,
        result: status.result,
        result_error: status.result_error,
        reason: None,
    }))
}

fn queue_definition() -> Value {
    json!({
        "queue": RUN_QUEUE,
        "config": {
            "type": "fifo",
            "message_group_field": "repository",
            "concurrency": 4,
            "max_retries": 3,
            "backoff_ms": 1_000,
            "poll_interval_ms": 100,
            "redeliver_on_engine_restart": true,
        },
    })
}

fn action_queue_definition() -> Value {
    json!({
        "queue": ACTION_QUEUE,
        "config": {
            "type": "fifo",
            "message_group_field": "action_id",
            "concurrency": 4,
            "max_retries": 3,
            "backoff_ms": 1_000,
            "poll_interval_ms": 100,
            "redeliver_on_engine_restart": true,
        },
    })
}

fn run_update_payload(run: &RunRecordV1) -> Value {
    json!({
        "stream_name": RUN_STREAM_NAME,
        "group_id": RUN_STREAM_GROUP,
        "type": RUN_UPDATED_EVENT_TYPE,
        "data": {
            "run_id": run.run_id,
            "repository": run.repository,
            "status": run.status,
            "attempt": run.attempt,
            "updated_at": run.updated_at,
            "completed_at": run.completed_at,
        },
    })
}

fn reconciliation_update_payload(run_id: &str) -> Value {
    json!({
        "stream_name": RUN_STREAM_NAME,
        "group_id": RUN_STREAM_GROUP,
        "type": RECONCILIATION_UPDATED_EVENT_TYPE,
        "data": { "run_id": run_id },
    })
}

fn action_update_payload(action: &crate::SecurityActionRecordV1) -> Value {
    json!({
        "stream_name": RUN_STREAM_NAME,
        "group_id": RUN_STREAM_GROUP,
        "type": "security-scan:action-updated",
        "data": {
            "action_id": action.action_id,
            "run_id": action.run_id,
            "status": action.status,
            "updated_at": action.updated_at,
        },
    })
}

fn snapshot_is_newer(
    existing: &ReconciliationSnapshotV1,
    candidate: &ReconciliationSnapshotV1,
) -> bool {
    let latest = |snapshot: &ReconciliationSnapshotV1| {
        snapshot
            .sources
            .iter()
            .filter_map(|source| source.collected_at)
            .max()
    };
    match (latest(existing), latest(candidate)) {
        (Some(existing), Some(candidate)) => existing > candidate,
        (Some(_), None) => true,
        _ => false,
    }
}

fn serialize<T: serde::Serialize>(value: &T, label: &str) -> Result<Value, SecurityScanError> {
    serde_json::to_value(value).map_err(|error| {
        SecurityScanError::Dependency(format!("could not serialize {label}: {error}"))
    })
}

fn mark_backfill_complete<T>(pending: &AtomicBool, result: &Result<T, SecurityScanError>) {
    if result.is_ok() {
        pending.store(false, Ordering::Release);
    }
}

fn parse_optional_run(
    value: Value,
    run_id: &str,
) -> Result<Option<RunRecordV1>, SecurityScanError> {
    if value.is_null() {
        return Ok(None);
    }
    parse_run(value, run_id).map(Some)
}

fn parse_optional_action(
    value: Value,
    action_id: &str,
) -> Result<Option<crate::SecurityActionRecordV1>, SecurityScanError> {
    if value.is_null() {
        return Ok(None);
    }
    parse_action(value, action_id).map(Some)
}

fn parse_action(
    value: Value,
    action_id: &str,
) -> Result<crate::SecurityActionRecordV1, SecurityScanError> {
    serde_json::from_value(value).map_err(|error| {
        SecurityScanError::Dependency(format!(
            "could not parse private action record {action_id}: {error}"
        ))
    })
}

fn function_info_matches(value: &Value, function_id: &str) -> bool {
    if value.is_null() {
        return false;
    }
    value
        .get("id")
        .or_else(|| value.get("function_id"))
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        == Some(function_id)
        || value.to_string().contains(function_id)
}

fn parse_run(value: Value, run_id: &str) -> Result<RunRecordV1, SecurityScanError> {
    serde_json::from_value(value).map_err(|error| {
        SecurityScanError::Dependency(format!(
            "could not parse private state record {run_id}: {error}"
        ))
    })
}

fn parse_state_list<T>(value: &Value, label: &str) -> Result<Vec<T>, SecurityScanError>
where
    T: DeserializeOwned,
{
    let candidates: Vec<&Value> = match value {
        Value::Array(values) => values.iter().collect(),
        Value::Object(map) => {
            if let Some(Value::Array(values)) = map.get("values").or_else(|| map.get("items")) {
                values.iter().collect()
            } else {
                map.values().collect()
            }
        }
        Value::Null => Vec::new(),
        _ => {
            return Err(SecurityScanError::Dependency(
                "private state list returned an unsupported shape".into(),
            ))
        }
    };
    let mut records = Vec::new();
    for value in candidates {
        if value.is_null() {
            continue;
        }
        records.push(serde_json::from_value(value.clone()).map_err(|error| {
            SecurityScanError::Dependency(format!(
                "could not parse {label} state list record: {error}"
            ))
        })?);
    }
    Ok(records)
}

fn is_queueable(status: RunStatusV1) -> bool {
    matches!(
        status,
        RunStatusV1::Queued
            | RunStatusV1::Materializing
            | RunStatusV1::Materialized
            | RunStatusV1::Dispatching
    )
}

fn is_terminal(status: RunStatusV1) -> bool {
    matches!(
        status,
        RunStatusV1::Completed | RunStatusV1::Failed | RunStatusV1::Cancelled
    )
}

fn needs_full_reconciliation(record: &RunIndexRecordV1) -> bool {
    record.summary.status == RunStatusV1::Analyzing
        || (is_terminal(record.summary.status) && record.has_materialized)
}

fn dependency_parse(dependency: &str, error: serde_json::Error) -> SecurityScanError {
    SecurityScanError::Dependency(format!("could not parse {dependency} response: {error}"))
}

fn accessor_is_missing(error: &SecurityScanError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("function_not_found") || message.contains("not found")
}

fn is_object_not_found(error: &SecurityScanError) -> bool {
    let message = error.to_string();
    message.contains("OBJECT_NOT_FOUND")
        || message.to_ascii_lowercase().contains("object not found")
}

fn worktree_is_missing(error: &SecurityScanError) -> bool {
    error.to_string().contains("W200")
}

