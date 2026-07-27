use std::time::{Duration, Instant};

use harness::functions::send::{MessageInput, SendOptions, SendRequest, SendResponse, SessionInit};
use harness::types::turn::FunctionPolicy;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::context::ScenarioContext;
use crate::error::{EvalError, FailureClass, Phase};
use crate::limits::{limit_failures, E2eLimitsV1};
use crate::report::{E2eScenarioReportV1, ScenarioObservationV1};

use super::ScenarioSpec;

pub async fn run(
    context: &ScenarioContext,
    run_id: &str,
    limits: &E2eLimitsV1,
    spec: ScenarioSpec,
) -> E2eScenarioReportV1 {
    let started = Instant::now();
    let session_id = format!("e2e_{}", Uuid::new_v4().simple());
    let mut report = E2eScenarioReportV1::new(spec.id, session_id.clone(), spec.prompt.clone());
    let binding = match context.bind_completion(&session_id).await {
        Ok(binding) => binding,
        Err(error) => {
            report.push_failure(error);
            report.finish(elapsed_ms(started));
            return report;
        }
    };

    let execution = async {
        let response: SendResponse = context
            .trigger_phase(
                Phase::Send,
                "harness::send",
                send_request(context, run_id, &session_id, limits, &spec),
            )
            .await?;
        validate_send_response(&response, &session_id)?;
        report.turn_id = Some(response.turn_id.clone());
        context
            .await_completion(&session_id, &response.turn_id)
            .await?;

        let metrics = context.metrics(&session_id).await?;
        validate_metrics(&metrics)?;
        let transcript = context.transcript(&session_id).await?;
        report.observation = Some(ScenarioObservationV1 {
            metrics,
            transcript,
        });
        let evaluation = (spec.evaluate)(
            context,
            report.observation.as_ref().unwrap(),
            &spec.evaluation_context,
        )
        .await?;
        report.evaluation = Some(evaluation);
        Ok(())
    };

    if let Err(error) = bounded(limits.execution.scenario_timeout(), spec.id, execution).await {
        report.push_failure(error);
        if let Err(stop_error) = context
            .stop_session(&session_id, report.turn_id.as_deref())
            .await
        {
            report.push_failure(stop_error);
        }
    }
    let limit_failures = report
        .observation
        .as_ref()
        .map(|observation| limit_failures(*limits, &observation.metrics))
        .unwrap_or_default();
    for failure in limit_failures {
        report.push_failure(failure);
    }
    binding.unregister();
    report.finish(elapsed_ms(started));
    report
}

fn send_request(
    context: &ScenarioContext,
    run_id: &str,
    session_id: &str,
    limits: &E2eLimitsV1,
    spec: &ScenarioSpec,
) -> SendRequest {
    let subject = context.subject();
    SendRequest {
        session_id: Some(session_id.to_string()),
        message: MessageInput::Text(spec.prompt.clone()),
        model: Some(subject.model.clone()),
        provider: Some(subject.provider.clone()),
        idempotency_key: Some(format!("e2e:{run_id}:{}:send", spec.id)),
        session: Some(SessionInit {
            title: Some(format!("Harness E2E: {}", spec.id)),
            metadata: Some(json!({
                "e2e_run_id": run_id,
                "e2e_scenario": spec.id,
            })),
        }),
        options: Some(SendOptions {
            system_prompt: Some(subject.system_prompt.clone()),
            system_prompt_strategy: subject.system_prompt_strategy,
            max_turns: Some(limits.execution.max_turns),
            max_output_tokens: Some(limits.execution.max_output_tokens_per_call),
            max_total_tokens: limits.execution.max_total_tokens,
            max_cost_usd: limits.execution.max_cost_usd,
            thinking_level: subject.thinking_level,
            provider_options: subject.provider_options.clone(),
            functions: Some(FunctionPolicy {
                allow: vec!["*".into()],
                ..FunctionPolicy::default()
            }),
            ..SendOptions::default()
        }),
    }
}

fn validate_send_response(
    response: &SendResponse,
    expected_session_id: &str,
) -> Result<(), EvalError> {
    if response.session_id != expected_session_id {
        return Err(EvalError::new(
            FailureClass::SubjectError,
            Phase::Send,
            None,
            Some("harness::send".into()),
            format!(
                "harness::send returned session {} instead of {expected_session_id}",
                response.session_id
            ),
        ));
    }
    if !response.accepted {
        return Err(EvalError::new(
            FailureClass::SubjectError,
            Phase::Send,
            None,
            Some("harness::send".into()),
            format!("harness::send did not accept the turn: {response:?}"),
        ));
    }
    Ok(())
}

fn validate_metrics(
    metrics: &harness::functions::metrics::SessionMetricsResponseV1,
) -> Result<(), EvalError> {
    if !metrics.complete {
        return Err(EvalError::evidence(
            "harness::metrics",
            "session metrics are incomplete",
        ));
    }
    for (name, value) in [
        ("input_tokens", metrics.totals.input_tokens),
        ("output_tokens", metrics.totals.output_tokens),
    ] {
        if value == Some(0) {
            return Err(EvalError::evidence(
                "harness::metrics",
                format!("{name} was reported as zero"),
            ));
        }
    }
    Ok(())
}

pub fn assistant_texts(transcript: &Value) -> Vec<String> {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("message"))
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .map(message_text)
        .collect()
}

pub fn state_value(response: Value) -> Value {
    match response {
        Value::Object(mut object)
            if object.get("ok").and_then(Value::as_bool) == Some(true)
                && object.contains_key("value") =>
        {
            object.remove("value").unwrap_or(Value::Null)
        }
        response => response,
    }
}

async fn bounded<F>(timeout: Duration, scenario_id: &str, future: F) -> Result<(), EvalError>
where
    F: std::future::Future<Output = Result<(), EvalError>>,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(result) => result,
        Err(_) => Err(EvalError::timeout(
            Phase::Await,
            format!("scenario {scenario_id} exceeded {}s", timeout.as_secs()),
        )),
    }
}

fn message_text(message: &Value) -> String {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| {
            (block.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| block.get("text").and_then(Value::as_str))
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("")
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}
