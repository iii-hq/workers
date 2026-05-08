use anyhow::Context;
use std::path::Path;

/// Format the rules + artifact for sending as the user message in the AI check.
///
/// Layout:
/// ```text
/// ## Project rules
///
/// {rules trimmed}
///
/// ## Artifact under review
///
/// Path: `{artifact_path}`
///
///    1: <line 1>
///    2: <line 2>
///    ...
/// ```
pub fn build_user_prompt(rules: &str, artifact_path: &str, artifact_text: &str) -> String {
    format!(
        "## Project rules\n\n{rules}\n\n## Artifact under review\n\nPath: `{path}`\n\n{numbered}\n",
        rules = rules.trim(),
        path = artifact_path,
        numbered = line_numbered(artifact_text),
    )
}

/// Parse the model's response. `Ok(())` if the model returned exactly `PASS`
/// (whitespace tolerated). `Err(body)` for any other content; the body is
/// surfaced verbatim to the caller for display.
pub fn parse_response(text: &str) -> Result<(), String> {
    let trimmed = text.trim();
    if trimmed == "PASS" {
        Ok(())
    } else {
        Err(trimmed.to_string())
    }
}

/// One Anthropic Messages API call against one rendered artifact. Returns
/// `Ok(Ok(()))` on PASS, `Ok(Err(violation_block))` on FAIL, `Err` on transport
/// or API failure.
pub fn check_artifact(
    artifact: &Path,
    rules: &str,
    system_prompt: &str,
    model: &str,
    api_key_env_var: &str,
    max_tokens: u32,
) -> anyhow::Result<Result<(), String>> {
    let api_key = std::env::var(api_key_env_var)
        .with_context(|| format!("env var {api_key_env_var} not set"))?;

    let artifact_text = std::fs::read_to_string(artifact)
        .with_context(|| format!("reading {}", artifact.display()))?;
    let path_str = artifact.display().to_string();
    let user_prompt = build_user_prompt(rules, &path_str, &artifact_text);

    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": system_prompt,
        "messages": [{ "role": "user", "content": user_prompt }],
    });

    let resp = reqwest::blocking::Client::new()
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .with_context(|| "POST https://api.anthropic.com/v1/messages")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().unwrap_or_default();
        anyhow::bail!("Anthropic API returned {status}: {body_text}");
    }

    let json: serde_json::Value = resp
        .json()
        .with_context(|| "parsing Anthropic API response JSON")?;
    let text = json
        .pointer("/content/0/text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("unexpected response shape: {json}"))?;

    Ok(parse_response(text))
}

fn line_numbered(content: &str) -> String {
    content
        .lines()
        .enumerate()
        .map(|(i, line)| format!("{:>4}: {}", i + 1, line))
        .collect::<Vec<_>>()
        .join("\n")
}
