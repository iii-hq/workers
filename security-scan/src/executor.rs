use std::{path::Component, sync::Arc};

use async_trait::async_trait;

use crate::{
    build_analysis_plan, ids, AnalysisPlan, AssessmentStatusV1, EnqueueRequest, ExecuteResponseV1,
    HarnessRunV1, MaterializedTargetV1, RepositoryConfigV1, RunErrorV1, RunRecordV1, RunStatusV1,
    SecurityAreaAssessmentV1, SecurityReportV1, SecurityRuntime, SecurityScanError,
    TurnCompletedEventV1, TurnCompletedResponseV1, WorkerConfig,
};

const MAX_STEP_FAILURES: u32 = 3;
const SECRET_REDACTION: &str = "<redacted>";
const PRIVATE_KEY_MARKERS: [(&str, &str); 3] = [
    ("-----BEGIN PRIVATE KEY-----", "-----END PRIVATE KEY-----"),
    (
        "-----BEGIN RSA PRIVATE KEY-----",
        "-----END RSA PRIVATE KEY-----",
    ),
    (
        "-----BEGIN OPENSSH PRIVATE KEY-----",
        "-----END OPENSSH PRIVATE KEY-----",
    ),
];
const TOKEN_PREFIXES: [(&str, usize); 12] = [
    ("github_pat_", 20),
    ("ghp_", 20),
    ("gho_", 20),
    ("ghs_", 20),
    ("ghu_", 20),
    ("ghr_", 20),
    ("glpat-", 20),
    ("xoxb-", 20),
    ("sk_live_", 16),
    ("npm_", 20),
    ("AKIA", 16),
    ("ASIA", 16),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisHandle {
    pub session_id: String,
    pub turn_id: String,
}

#[async_trait]
pub trait ExecutionRuntime: SecurityRuntime {
    async fn get_run_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<RunRecordV1>, SecurityScanError>;

    async fn materialize_target(
        &self,
        repository: &RepositoryConfigV1,
        run: &RunRecordV1,
    ) -> Result<MaterializedTargetV1, SecurityScanError>;

    async fn cleanup_target(
        &self,
        _target: &MaterializedTargetV1,
    ) -> Result<(), SecurityScanError> {
        Ok(())
    }

    async fn start_analysis(&self, plan: AnalysisPlan)
        -> Result<AnalysisHandle, SecurityScanError>;

    async fn completed_analysis(
        &self,
        _run: &RunRecordV1,
    ) -> Result<Option<TurnCompletedEventV1>, SecurityScanError> {
        Ok(None)
    }
}

pub struct SecurityScanExecutor<R> {
    runtime: Arc<R>,
    config: WorkerConfig,
}

impl<R> SecurityScanExecutor<R>
where
    R: ExecutionRuntime,
{
    pub fn new(runtime: Arc<R>, config: WorkerConfig) -> Self {
        Self { runtime, config }
    }

    pub async fn execute(
        &self,
        request: EnqueueRequest,
    ) -> Result<ExecuteResponseV1, SecurityScanError> {
        match self.execute_inner(&request).await {
            Ok(response) => Ok(response),
            Err(error) => match self.record_step_failure(&request, &error).await? {
                Some(response) => Ok(response),
                None => Err(error),
            },
        }
    }

    async fn execute_inner(
        &self,
        request: &EnqueueRequest,
    ) -> Result<ExecuteResponseV1, SecurityScanError> {
        let Some(run) = self.runtime.get_run(&request.run_id).await? else {
            return Err(SecurityScanError::InvalidRequest(format!(
                "unknown run {}",
                request.run_id
            )));
        };
        if run.repository != request.repository
            || run.attempt != request.attempt
            || request.step > run.step
        {
            return Ok(response(&run, true));
        }
        match run.status {
            RunStatusV1::Queued | RunStatusV1::Materializing => self.materialize(run).await,
            RunStatusV1::Materialized | RunStatusV1::Dispatching => self.start_analysis(run).await,
            RunStatusV1::Analyzing => {
                let woke = self.reconcile_analysis(&run).await?;
                if woke {
                    let current =
                        self.runtime
                            .get_run(&request.run_id)
                            .await?
                            .ok_or_else(|| {
                                SecurityScanError::Dependency(format!(
                                    "run {} disappeared during reconciliation",
                                    request.run_id
                                ))
                            })?;
                    Ok(response(&current, false))
                } else {
                    Ok(response(&run, true))
                }
            }
            RunStatusV1::Completed
            | RunStatusV1::Failed
            | RunStatusV1::Cancelling
            | RunStatusV1::Cancelled => Ok(response(&run, true)),
        }
    }

    async fn record_step_failure(
        &self,
        request: &EnqueueRequest,
        error: &SecurityScanError,
    ) -> Result<Option<ExecuteResponseV1>, SecurityScanError> {
        let Some(run) = self.runtime.get_run(&request.run_id).await? else {
            return Ok(None);
        };
        if run.repository != request.repository
            || run.attempt != request.attempt
            || request.step > run.step
            || !matches!(
                run.status,
                RunStatusV1::Queued
                    | RunStatusV1::Materializing
                    | RunStatusV1::Materialized
                    | RunStatusV1::Dispatching
            )
        {
            return Ok(Some(response(&run, true)));
        }

        let mut failed = run.clone();
        failed.step_failures = failed.step_failures.saturating_add(1);
        failed.updated_at = ids::now_ms();
        let terminal = matches!(error, SecurityScanError::InvalidRequest(_))
            || failed.step_failures >= MAX_STEP_FAILURES;
        if terminal {
            failed.status = RunStatusV1::Failed;
            failed.completed_at = Some(failed.updated_at);
        }
        let stage = if run.step == 0 {
            "target materialization"
        } else {
            "analysis dispatch"
        };
        failed.error = Some(RunErrorV1 {
            code: if terminal {
                "step_failed".into()
            } else {
                "step_retrying".into()
            },
            message: format!("{stage} failed; dependency details are available in worker logs"),
            retryable: !matches!(error, SecurityScanError::InvalidRequest(_)),
        });
        if !self.runtime.replace_run(&run, failed.clone()).await? {
            return Ok(None);
        }
        if terminal {
            if let Err(cleanup_error) = self.cleanup_terminal(&failed).await {
                tracing::warn!(
                    run_id = %failed.run_id,
                    error = %cleanup_error,
                    "failed run checkout cleanup failed"
                );
            }
        }
        Ok(terminal.then(|| response(&failed, false)))
    }

    pub async fn on_turn_completed(
        &self,
        event: TurnCompletedEventV1,
    ) -> Result<TurnCompletedResponseV1, SecurityScanError> {
        if !event.terminal {
            return Ok(TurnCompletedResponseV1 {
                woke: false,
                status: None,
            });
        }
        let Some(run) = self.runtime.get_run_by_session(&event.session_id).await? else {
            return Ok(TurnCompletedResponseV1 {
                woke: false,
                status: None,
            });
        };
        if run.status != RunStatusV1::Analyzing
            || run
                .harness
                .as_ref()
                .is_none_or(|harness| harness.turn_id != event.turn_id)
        {
            return Ok(TurnCompletedResponseV1 {
                woke: false,
                status: Some(run.status),
            });
        }

        // A trigger event is only a wake-up signal. Read the terminal result
        // back from Harness so a forged or duplicated callback cannot inject
        // a report into the durable run record.
        let Some(authoritative) = self.runtime.completed_analysis(&run).await? else {
            return Ok(TurnCompletedResponseV1 {
                woke: false,
                status: Some(run.status),
            });
        };
        self.finish_analysis(run, authoritative).await
    }

    async fn finish_analysis(
        &self,
        run: RunRecordV1,
        event: TurnCompletedEventV1,
    ) -> Result<TurnCompletedResponseV1, SecurityScanError> {
        if !event.terminal
            || run.harness.as_ref().is_none_or(|harness| {
                harness.session_id != event.session_id || harness.turn_id != event.turn_id
            })
        {
            return Ok(TurnCompletedResponseV1 {
                woke: false,
                status: Some(run.status),
            });
        }

        let now = ids::now_ms();
        let mut finished = run.clone();
        finished.completed_at = Some(now);
        finished.updated_at = now;
        if event.status == "completed" {
            match event
                .result
                .ok_or_else(|| "Harness completed without a result".to_string())
                .and_then(|value| {
                    serde_json::from_value::<SecurityReportV1>(value)
                        .map_err(|error| format!("invalid security report: {error}"))
                })
                .and_then(|report| validate_report(report, &run))
            {
                Ok(report) => {
                    finished.status = RunStatusV1::Completed;
                    finished.report = Some(report);
                    finished.error = None;
                }
                Err(message) => {
                    finished.status = RunStatusV1::Failed;
                    finished.error = Some(RunErrorV1 {
                        code: "invalid_report".into(),
                        message,
                        retryable: true,
                    });
                }
            }
        } else if event.status == "cancelled" {
            finished.status = RunStatusV1::Cancelled;
            finished.error = None;
        } else {
            finished.status = RunStatusV1::Failed;
            finished.error = Some(RunErrorV1 {
                code: "analysis_failed".into(),
                message: sanitize_failure_message(
                    &run,
                    event.result_error.or(event.reason).unwrap_or_else(|| {
                        format!("Harness turn ended with status {}", event.status)
                    }),
                ),
                retryable: true,
            });
        }

        if !self.runtime.replace_run(&run, finished.clone()).await? {
            return Ok(TurnCompletedResponseV1 {
                woke: false,
                status: Some(run.status),
            });
        }
        if let Err(error) = self.cleanup_terminal(&finished).await {
            tracing::warn!(run_id = %finished.run_id, %error, "terminal checkout cleanup failed");
        }
        Ok(TurnCompletedResponseV1 {
            woke: true,
            status: Some(finished.status),
        })
    }

    pub async fn reconcile_analysis(&self, run: &RunRecordV1) -> Result<bool, SecurityScanError> {
        if run.status != RunStatusV1::Analyzing {
            return Ok(false);
        }
        let Some(event) = self.runtime.completed_analysis(run).await? else {
            return Ok(false);
        };
        Ok(self.finish_analysis(run.clone(), event).await?.woke)
    }

    pub async fn cleanup_terminal(&self, run: &RunRecordV1) -> Result<bool, SecurityScanError> {
        if !matches!(
            run.status,
            RunStatusV1::Completed | RunStatusV1::Failed | RunStatusV1::Cancelled
        ) {
            return Ok(false);
        }
        let Some(target) = run.materialized.as_ref() else {
            return Ok(false);
        };
        self.runtime.cleanup_target(target).await?;
        let mut cleaned = run.clone();
        cleaned.materialized = None;
        cleaned.updated_at = ids::now_ms();
        self.runtime.replace_run(run, cleaned).await
    }

    async fn materialize(
        &self,
        mut run: RunRecordV1,
    ) -> Result<ExecuteResponseV1, SecurityScanError> {
        if run.status == RunStatusV1::Queued {
            let mut claimed = run.clone();
            claimed.status = RunStatusV1::Materializing;
            claimed.updated_at = ids::now_ms();
            if !self.runtime.replace_run(&run, claimed.clone()).await? {
                return Ok(response(&run, true));
            }
            run = claimed;
        } else if run.status != RunStatusV1::Materializing {
            return Ok(response(&run, true));
        }

        let repository = self.config.repository(&run.repository).ok_or_else(|| {
            SecurityScanError::InvalidRequest(format!(
                "repository {} is no longer configured",
                run.repository
            ))
        })?;
        let target = self.runtime.materialize_target(repository, &run).await?;
        if !target.base_sha.eq_ignore_ascii_case(&run.target_sha) {
            return Err(SecurityScanError::Dependency(format!(
                "materialized commit {} does not match requested {}",
                target.base_sha, run.target_sha
            )));
        }

        let mut materialized = run.clone();
        materialized.status = RunStatusV1::Materialized;
        materialized.step = 1;
        materialized.step_failures = 0;
        materialized.materialized = Some(target);
        materialized.updated_at = ids::now_ms();
        if !self.runtime.replace_run(&run, materialized.clone()).await? {
            return Ok(response(&run, true));
        }
        self.runtime
            .enqueue_execute(EnqueueRequest::new(
                materialized.run_id.clone(),
                materialized.repository.clone(),
                materialized.attempt,
                materialized.step,
            ))
            .await?;
        Ok(response(&materialized, false))
    }

    async fn start_analysis(
        &self,
        mut run: RunRecordV1,
    ) -> Result<ExecuteResponseV1, SecurityScanError> {
        if run.status == RunStatusV1::Materialized {
            let mut claimed = run.clone();
            claimed.status = RunStatusV1::Dispatching;
            claimed.updated_at = ids::now_ms();
            if !self.runtime.replace_run(&run, claimed.clone()).await? {
                return Ok(response(&run, true));
            }
            run = claimed;
        } else if run.status != RunStatusV1::Dispatching {
            return Ok(response(&run, true));
        }
        let target = run.materialized.as_ref().ok_or_else(|| {
            SecurityScanError::Dependency(format!(
                "run {} is materialized without a target checkpoint",
                run.run_id
            ))
        })?;
        let plan = build_analysis_plan(&run, &target.path, &self.config.analysis);
        let handle = self.runtime.start_analysis(plan).await?;
        let mut analyzing = run.clone();
        analyzing.status = RunStatusV1::Analyzing;
        analyzing.step = 2;
        analyzing.step_failures = 0;
        analyzing.harness = Some(HarnessRunV1 {
            session_id: handle.session_id,
            turn_id: handle.turn_id,
        });
        analyzing.updated_at = ids::now_ms();
        if !self.runtime.replace_run(&run, analyzing.clone()).await? {
            return Ok(response(&run, true));
        }
        self.reconcile_analysis(&analyzing).await?;
        Ok(response(&analyzing, false))
    }
}

fn response(run: &RunRecordV1, skipped: bool) -> ExecuteResponseV1 {
    ExecuteResponseV1 {
        skipped,
        status: run.status,
        step: run.step,
    }
}

fn sanitize_failure_message(run: &RunRecordV1, message: String) -> String {
    let mut sanitized = message;
    if let Some(root) = run
        .materialized
        .as_ref()
        .map(|target| target.path.as_str())
        .filter(|root| !root.is_empty())
    {
        sanitized = sanitized.replace(root, "<checkout>");
    }
    sanitized = redact_secret_material(&sanitized);
    if sanitized.chars().count() > 2_000 {
        sanitized = sanitized.chars().take(2_000).collect();
        sanitized.push('…');
    }
    sanitized
}

fn validate_report(
    mut report: SecurityReportV1,
    run: &RunRecordV1,
) -> Result<SecurityReportV1, String> {
    const MAX_PUBLIC_REPORT_CHARS: usize = 1_000_000;

    validate_text("summary", &report.summary, 8_000, true)?;
    let mut public_chars = report.summary.chars().count();
    if report.findings.len() > 200 {
        return Err("invalid security report: more than 200 findings".into());
    }
    let internal_root = run
        .materialized
        .as_ref()
        .map(|target| target.path.as_str())
        .filter(|path| !path.is_empty());
    reject_internal_root("summary", &report.summary, internal_root)?;
    for (area, assessment) in [
        ("vulnerabilities", &report.assessments.vulnerabilities),
        ("dependencies", &report.assessments.dependencies),
        ("secrets", &report.assessments.secrets),
        ("supply_chain", &report.assessments.supply_chain),
    ] {
        public_chars =
            public_chars.saturating_add(validate_assessment(area, assessment, internal_root)?);
    }
    for (index, finding) in report.findings.iter_mut().enumerate() {
        let prefix = format!("finding {index}");
        validate_text(&format!("{prefix} rule_id"), &finding.rule_id, 256, true)?;
        validate_text(&format!("{prefix} title"), &finding.title, 512, true)?;
        validate_text(
            &format!("{prefix} description"),
            &finding.description,
            16_000,
            true,
        )?;
        validate_text(
            &format!("{prefix} evidence"),
            &finding.evidence,
            16_000,
            true,
        )?;
        validate_text(
            &format!("{prefix} remediation"),
            &finding.remediation,
            16_000,
            true,
        )?;
        for text in [
            finding.rule_id.as_str(),
            finding.title.as_str(),
            finding.description.as_str(),
            finding.evidence.as_str(),
            finding.remediation.as_str(),
        ] {
            public_chars = public_chars.saturating_add(text.chars().count());
        }
        for (field, text) in [
            ("rule_id", finding.rule_id.as_str()),
            ("title", finding.title.as_str()),
            ("description", finding.description.as_str()),
            ("evidence", finding.evidence.as_str()),
            ("remediation", finding.remediation.as_str()),
        ] {
            reject_internal_root(&format!("{prefix} {field}"), text, internal_root)?;
        }
        if let Some(location) = &finding.location {
            validate_location(&prefix, location)?;
            public_chars = public_chars.saturating_add(location.path.chars().count());
        }
        if run.mode == crate::ScanModeV1::Scan {
            finding.suggested_patch = None;
        } else if let Some(patch) = &finding.suggested_patch {
            validate_text(&format!("{prefix} suggested_patch"), patch, 64_000, false)?;
            reject_internal_root(&format!("{prefix} suggested_patch"), patch, internal_root)?;
            public_chars = public_chars.saturating_add(patch.chars().count());
        }
        if public_chars > MAX_PUBLIC_REPORT_CHARS {
            return Err(format!(
                "invalid security report: public content exceeds {MAX_PUBLIC_REPORT_CHARS} characters"
            ));
        }
    }
    Ok(report)
}

fn validate_assessment(
    area: &str,
    assessment: &SecurityAreaAssessmentV1,
    internal_root: Option<&str>,
) -> Result<usize, String> {
    let label = format!("assessment {area} reason");
    match assessment.status {
        AssessmentStatusV1::Unknown => {
            return Err(format!(
                "invalid security report: assessment {area} must be assessed or not_assessed"
            ));
        }
        AssessmentStatusV1::NotAssessed
            if assessment
                .reason
                .as_deref()
                .is_none_or(|reason| reason.trim().is_empty()) =>
        {
            return Err(format!(
                "invalid security report: assessment {area} requires a reason when not_assessed"
            ));
        }
        AssessmentStatusV1::Assessed | AssessmentStatusV1::NotAssessed => {}
    }
    let Some(reason) = assessment.reason.as_deref() else {
        return Ok(0);
    };
    validate_text(&label, reason, 2_000, false)?;
    reject_internal_root(&label, reason, internal_root)?;
    Ok(reason.chars().count())
}

fn reject_internal_root(
    label: &str,
    value: &str,
    internal_root: Option<&str>,
) -> Result<(), String> {
    if internal_root.is_some_and(|root| value.contains(root)) {
        return Err(format!(
            "invalid security report: {label} exposes the internal checkout root"
        ));
    }
    Ok(())
}

fn reject_secret_material(label: &str, value: &str) -> Result<(), String> {
    if !secret_material_spans(value).is_empty() {
        return Err(format!(
            "invalid security report: {label} contains credential-like secret material"
        ));
    }
    Ok(())
}

fn redact_secret_material(value: &str) -> String {
    let spans = secret_material_spans(value);
    if spans.is_empty() {
        return value.to_string();
    }

    let mut redacted = String::with_capacity(value.len());
    let mut cursor = 0;
    for (start, end) in spans {
        redacted.push_str(&value[cursor..start]);
        redacted.push_str(SECRET_REDACTION);
        cursor = end;
    }
    redacted.push_str(&value[cursor..]);
    redacted
}

fn secret_material_spans(value: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    collect_private_key_spans(value, &mut spans);
    collect_known_token_spans(value, &mut spans);
    collect_credential_url_spans(value, &mut spans);
    collect_credential_assignment_spans(value, &mut spans);
    merge_spans(spans, value.len())
}

fn collect_private_key_spans(value: &str, spans: &mut Vec<(usize, usize)>) {
    for (begin, end) in PRIVATE_KEY_MARKERS {
        let mut cursor = 0;
        while let Some(relative_start) = value[cursor..].find(begin) {
            let start = cursor + relative_start;
            let body_start = start + begin.len();
            let block_end = value[body_start..]
                .find(end)
                .map_or(value.len(), |relative_end| {
                    body_start + relative_end + end.len()
                });
            spans.push((start, block_end));
            if block_end == value.len() {
                break;
            }
            cursor = block_end;
        }
    }
}

fn collect_known_token_spans(value: &str, spans: &mut Vec<(usize, usize)>) {
    let bytes = value.as_bytes();
    for (prefix, minimum_tail) in TOKEN_PREFIXES {
        for (start, _) in value.match_indices(prefix) {
            let tail_start = start + prefix.len();
            let mut end = tail_start;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || matches!(bytes[end], b'_' | b'-'))
            {
                end += 1;
            }
            if end.saturating_sub(tail_start) >= minimum_tail {
                spans.push((start, end));
            }
        }
    }
}

fn collect_credential_url_spans(value: &str, spans: &mut Vec<(usize, usize)>) {
    let bytes = value.as_bytes();
    for (separator, _) in value.match_indices("://") {
        let mut scheme_start = separator;
        while scheme_start > 0 && is_url_scheme_byte(bytes[scheme_start - 1]) {
            scheme_start -= 1;
        }
        if scheme_start == separator || !bytes[scheme_start].is_ascii_alphabetic() {
            continue;
        }

        let authority_start = separator + 3;
        let mut authority_end = authority_start;
        while authority_end < bytes.len()
            && !bytes[authority_end].is_ascii_whitespace()
            && !matches!(bytes[authority_end], b'/' | b'?' | b'#' | b'"' | b'\'')
        {
            authority_end += 1;
        }
        let Some(at_offset) = bytes[authority_start..authority_end]
            .iter()
            .rposition(|byte| *byte == b'@')
        else {
            continue;
        };
        let userinfo_end = authority_start + at_offset;
        if userinfo_end == authority_start {
            continue;
        }
        let userinfo = &value[authority_start..userinfo_end];
        if userinfo.contains(':') || contains_percent_encoded_colon(userinfo) {
            spans.push((authority_start, userinfo_end));
        }
    }
}

fn collect_credential_assignment_spans(value: &str, spans: &mut Vec<(usize, usize)>) {
    let bytes = value.as_bytes();
    for (separator, byte) in bytes.iter().copied().enumerate() {
        if !matches!(byte, b'=' | b':') || is_comparison_operator(bytes, separator) {
            continue;
        }

        let mut key_end = separator;
        while key_end > 0 && bytes[key_end - 1].is_ascii_whitespace() {
            key_end -= 1;
        }
        if key_end > 0 && matches!(bytes[key_end - 1], b'"' | b'\'') {
            key_end -= 1;
        }
        let mut key_start = key_end;
        while key_start > 0 && is_assignment_key_byte(bytes[key_start - 1]) {
            key_start -= 1;
        }
        if key_start == key_end || !is_credential_key(&value[key_start..key_end]) {
            continue;
        }
        if byte == b':' && !is_colon_assignment_context(value, key_start) {
            continue;
        }

        let mut value_start = separator + 1;
        while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
            value_start += 1;
        }
        if value_start == bytes.len() {
            continue;
        }
        let quote = matches!(bytes[value_start], b'"' | b'\'').then_some(bytes[value_start]);
        if quote.is_some() {
            value_start += 1;
        }
        let redact_to_line_end = byte == b':'
            || value[key_start..key_end]
                .to_ascii_lowercase()
                .ends_with("authorization");
        let value_end = assignment_value_end(bytes, value_start, quote, redact_to_line_end);
        if value_start == value_end || is_safe_secret_placeholder(&value[value_start..value_end]) {
            continue;
        }
        spans.push((value_start, value_end));
    }
}

fn is_url_scheme_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
}

fn contains_percent_encoded_colon(value: &str) -> bool {
    value.as_bytes().windows(3).any(|window| {
        window[0] == b'%' && window[1] == b'3' && window[2].eq_ignore_ascii_case(&b'a')
    })
}

fn is_assignment_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

fn is_comparison_operator(bytes: &[u8], separator: usize) -> bool {
    if bytes[separator] != b'=' {
        return false;
    }
    bytes
        .get(separator + 1)
        .is_some_and(|byte| matches!(byte, b'=' | b'>'))
        || separator
            .checked_sub(1)
            .and_then(|index| bytes.get(index))
            .is_some_and(|byte| matches!(byte, b'=' | b'!' | b'<' | b'>'))
}

fn is_credential_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .map(|character| match character {
            '-' | '.' => '_',
            _ => character.to_ascii_lowercase(),
        })
        .collect();
    const EXACT_KEYS: [&str; 13] = [
        "password",
        "passwd",
        "pwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
        "client_secret",
        "credential",
        "credentials",
        "authorization",
    ];
    const KEY_SUFFIXES: [&str; 13] = [
        "_password",
        "_passwd",
        "_pwd",
        "_secret",
        "_token",
        "_api_key",
        "_apikey",
        "_access_key",
        "_private_key",
        "_client_secret",
        "_credential",
        "_credentials",
        "_authorization",
    ];
    const COMPACT_SUFFIXES: [&str; 6] = [
        "password",
        "passwd",
        "secret",
        "token",
        "apikey",
        "credentials",
    ];

    EXACT_KEYS.contains(&normalized.as_str())
        || KEY_SUFFIXES
            .iter()
            .any(|suffix| normalized.ends_with(suffix))
        || COMPACT_SUFFIXES
            .iter()
            .any(|suffix| normalized.len() > suffix.len() && normalized.ends_with(suffix))
}

fn is_colon_assignment_context(value: &str, key_start: usize) -> bool {
    let segment_start = value[..key_start]
        .rfind(['\n', '\r', '{', '[', ',', ';'])
        .map_or(0, |index| index + 1);
    value[segment_start..key_start]
        .bytes()
        .all(|byte| byte.is_ascii_whitespace() || matches!(byte, b'"' | b'\'' | b'`' | b'-' | b'*'))
}

fn assignment_value_end(
    bytes: &[u8],
    start: usize,
    quote: Option<u8>,
    redact_to_line_end: bool,
) -> usize {
    let mut end = start;
    let mut escaped = false;
    while end < bytes.len() {
        let byte = bytes[end];
        if let Some(quote) = quote {
            if byte == quote && !escaped {
                break;
            }
            escaped = byte == b'\\' && !escaped;
            if byte != b'\\' {
                escaped = false;
            }
        } else {
            let terminates_value = if redact_to_line_end {
                matches!(byte, b'\n' | b'\r' | b',' | b';' | b'}' | b']')
            } else {
                byte.is_ascii_whitespace() || matches!(byte, b',' | b';' | b'"' | b'\'')
            };
            if terminates_value {
                break;
            }
        }
        end += 1;
    }
    end
}

fn is_safe_secret_placeholder(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "<redacted>"
            | "[redacted]"
            | "redacted"
            | "<masked>"
            | "[masked]"
            | "masked"
            | "<hidden>"
            | "[hidden]"
            | "hidden"
            | "<omitted>"
            | "[omitted]"
            | "omitted"
            | "***"
            | "none"
            | "null"
            | "undefined"
            | "unset"
    ) || is_environment_reference(&normalized)
}

fn is_environment_reference(value: &str) -> bool {
    let name = value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .or_else(|| value.strip_prefix('$'));
    name.is_some_and(|name| {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    })
}

fn merge_spans(mut spans: Vec<(usize, usize)>, value_len: usize) -> Vec<(usize, usize)> {
    spans.retain(|(start, end)| start < end && *end <= value_len);
    spans.sort_unstable_by_key(|(start, end)| (*start, *end));
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        if let Some((_, previous_end)) = merged.last_mut() {
            if start <= *previous_end {
                *previous_end = (*previous_end).max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

fn validate_text(label: &str, value: &str, max_chars: usize, required: bool) -> Result<(), String> {
    if required && value.trim().is_empty() {
        return Err(format!("invalid security report: {label} is empty"));
    }
    if value.chars().count() > max_chars {
        return Err(format!(
            "invalid security report: {label} exceeds {max_chars} characters"
        ));
    }
    if value.contains('\0') {
        return Err(format!("invalid security report: {label} contains NUL"));
    }
    reject_secret_material(label, value)?;
    Ok(())
}

fn validate_location(prefix: &str, location: &crate::FindingLocationV1) -> Result<(), String> {
    validate_text(
        &format!("{prefix} location.path"),
        &location.path,
        4_096,
        true,
    )?;
    let path = std::path::Path::new(&location.path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "invalid security report: {prefix} location must be a repository-relative path"
        ));
    }
    if location.line_start == Some(0) || location.line_end == Some(0) {
        return Err(format!(
            "invalid security report: {prefix} location lines are one-based"
        ));
    }
    if let (Some(start), Some(end)) = (location.line_start, location.line_end) {
        if end < start {
            return Err(format!(
                "invalid security report: {prefix} location line_end precedes line_start"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod report_tests {
    use super::*;
    use crate::{
        FindingLocationV1, ScanModeV1, SecurityAssessmentsV1, SecurityFindingV1, SeverityV1,
    };

    fn run(mode: ScanModeV1) -> RunRecordV1 {
        RunRecordV1 {
            schema_version: "1".into(),
            run_id: "sec_x".into(),
            repository: "repo".into(),
            target_sha: "a".repeat(40),
            mode,
            operation_nonce: "private_nonce".into(),
            status: RunStatusV1::Analyzing,
            attempt: 1,
            step: 2,
            step_failures: 0,
            materialized: Some(MaterializedTargetV1 {
                worktree_id: "wt_x".into(),
                path: "/private/internal/wt_x".into(),
                base_sha: "a".repeat(40),
            }),
            harness: None,
            report: None,
            error: None,
            created_at: 1,
            updated_at: 1,
            completed_at: None,
        }
    }

    fn report(path: &str) -> SecurityReportV1 {
        SecurityReportV1 {
            summary: "one finding".into(),
            assessments: assessed_areas(),
            findings: vec![SecurityFindingV1 {
                rule_id: "SEC-1".into(),
                severity: SeverityV1::High,
                title: "Unsafe input".into(),
                description: "Untrusted input reaches a command".into(),
                evidence: "The call is not escaped".into(),
                location: Some(FindingLocationV1 {
                    path: path.into(),
                    line_start: Some(10),
                    line_end: Some(10),
                }),
                remediation: "Use an argv API".into(),
                suggested_patch: Some("diff --git a/src/x.rs b/src/x.rs".into()),
            }],
        }
    }

    fn assessed_areas() -> SecurityAssessmentsV1 {
        let assessed = SecurityAreaAssessmentV1 {
            status: AssessmentStatusV1::Assessed,
            reason: None,
        };
        SecurityAssessmentsV1 {
            vulnerabilities: assessed.clone(),
            dependencies: assessed.clone(),
            secrets: assessed.clone(),
            supply_chain: assessed,
        }
    }

    #[test]
    fn report_rejects_internal_or_parent_paths() {
        assert!(validate_report(
            report("/private/internal/wt_x/src/x.rs"),
            &run(ScanModeV1::Suggest)
        )
        .is_err());
        assert!(validate_report(report("../outside"), &run(ScanModeV1::Suggest)).is_err());
    }

    #[test]
    fn report_rejects_internal_roots_in_every_public_text_surface() {
        let mut summary = report("src/x.rs");
        summary.summary = "reviewed /private/internal/wt_x".into();
        assert!(validate_report(summary, &run(ScanModeV1::Suggest)).is_err());

        let mut title = report("src/x.rs");
        title.findings[0].title = "leak /private/internal/wt_x".into();
        assert!(validate_report(title, &run(ScanModeV1::Suggest)).is_err());
    }

    #[test]
    fn scan_mode_strips_suggested_patches() {
        let report = validate_report(report("src/x.rs"), &run(ScanModeV1::Scan)).unwrap();
        assert!(report.findings[0].suggested_patch.is_none());
    }

    #[test]
    fn report_requires_explicit_coverage_for_every_area() {
        let mut missing = report("src/x.rs");
        missing.assessments.dependencies = SecurityAreaAssessmentV1::default();
        let error = validate_report(missing, &run(ScanModeV1::Scan)).unwrap_err();
        assert!(error.contains("assessment dependencies must be assessed or not_assessed"));

        let mut unexplained = report("src/x.rs");
        unexplained.assessments.secrets.status = AssessmentStatusV1::NotAssessed;
        let error = validate_report(unexplained, &run(ScanModeV1::Scan)).unwrap_err();
        assert!(error.contains("assessment secrets requires a reason"));

        let mut explained = report("src/x.rs");
        explained.assessments.secrets = SecurityAreaAssessmentV1 {
            status: AssessmentStatusV1::NotAssessed,
            reason: Some("No supported credential manifest was present.".into()),
        };
        assert!(validate_report(explained, &run(ScanModeV1::Scan)).is_ok());
    }

    #[test]
    fn legacy_reports_deserialize_with_unknown_coverage() {
        let legacy: SecurityReportV1 = serde_json::from_value(serde_json::json!({
            "summary": "legacy report",
            "findings": []
        }))
        .unwrap();
        assert_eq!(
            legacy.assessments.vulnerabilities.status,
            AssessmentStatusV1::Unknown
        );
    }

    #[test]
    fn failure_messages_redact_the_internal_checkout_root() {
        let message = sanitize_failure_message(
            &run(ScanModeV1::Scan),
            "could not read /private/internal/wt_x/src/main.rs".into(),
        );
        assert_eq!(message, "could not read <checkout>/src/main.rs");
    }

    #[test]
    fn failure_messages_redact_credentials_before_persistence() {
        let known_token = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";
        let user_token = "ghu_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";
        let refresh_token = "ghr_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";
        let message = sanitize_failure_message(
            &run(ScanModeV1::Scan),
            format!(
                "checkout /private/internal/wt_x failed: \
                 DATABASE_URL=postgres://url-user-canary:url-password-canary@db.internal/app; \
                 API_TOKEN=assignment-canary\npassword: correct horse battery staple\n\
                 Authorization: Bearer auth-canary\nknown={known_token}\nuser={user_token}\nrefresh={refresh_token}"
            ),
        );

        for secret in [
            "url-user-canary",
            "url-password-canary",
            "assignment-canary",
            "correct",
            "horse",
            "battery",
            "staple",
            "auth-canary",
            known_token,
            user_token,
            refresh_token,
        ] {
            assert!(!message.contains(secret));
        }
        assert!(message.contains("checkout <checkout> failed"));
        assert!(message.contains("postgres://<redacted>@db.internal/app"));
        assert!(message.contains("API_TOKEN=<redacted>"));
        assert!(message.contains("password: <redacted>"));
        assert!(message.contains("Authorization: <redacted>"));
    }

    #[test]
    fn report_rejects_secret_values_without_echoing_them() {
        for canary in [
            "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890",
            "ghu_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890",
            "ghr_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890",
        ] {
            let mut leaked = report("src/x.rs");
            leaked.findings[0].evidence = format!("hard-coded credential: {canary}");

            let error = validate_report(leaked, &run(ScanModeV1::Suggest)).unwrap_err();
            assert!(error.contains("credential-like secret material"));
            assert!(!error.contains(canary));
        }

        let canary = "ghu_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";
        let mut path_leak = report("src/x.rs");
        path_leak.findings[0].location.as_mut().unwrap().path = canary.into();
        let error = validate_report(path_leak, &run(ScanModeV1::Suggest)).unwrap_err();
        assert!(error.contains("credential-like secret material"));
        assert!(!error.contains(canary));

        for (leak, secret) in [
            (
                "DATABASE_URL=postgres://report-user-canary:report-password-canary@db.internal/app",
                "report-password-canary",
            ),
            (
                "API_TOKEN=assignment-report-canary",
                "assignment-report-canary",
            ),
            ("password: \"yaml-report-canary\"", "yaml-report-canary"),
        ] {
            let mut leaked = report("src/x.rs");
            leaked.findings[0].evidence = leak.into();

            let error = validate_report(leaked, &run(ScanModeV1::Suggest)).unwrap_err();
            assert!(error.contains("credential-like secret material"));
            assert!(!error.contains(secret));
        }
    }

    #[test]
    fn credential_detection_preserves_non_secret_references() {
        let safe = "docs https://github.com/iii-hq/iii ssh://git@github.com/iii-hq/iii \
                    MODE=scan API_TOKEN=${API_TOKEN} password=<redacted>\npassword: <redacted>";
        assert_eq!(redact_secret_material(safe), safe);

        let mut safe_report = report("src/x.rs");
        safe_report.findings[0].evidence = safe.into();
        assert!(validate_report(safe_report, &run(ScanModeV1::Suggest)).is_ok());
    }

    #[test]
    fn report_rejects_oversized_combined_public_content() {
        let template = report("src/x.rs").findings.remove(0);
        let large_text = "x".repeat(6_000);
        let mut oversized = SecurityReportV1 {
            summary: "large report".into(),
            assessments: assessed_areas(),
            findings: Vec::new(),
        };
        for index in 0..60 {
            let mut finding = template.clone();
            finding.rule_id = format!("SEC-{index}");
            finding.description = large_text.clone();
            finding.evidence = large_text.clone();
            finding.remediation = large_text.clone();
            oversized.findings.push(finding);
        }

        let error = validate_report(oversized, &run(ScanModeV1::Suggest)).unwrap_err();
        assert!(error.contains("public content exceeds 1000000 characters"));
    }
}
