use std::path::Path;

/// Format the rules + artifact for sending as the user message in the AI check.
pub fn build_user_prompt(_rules: &str, _artifact_path: &str, _artifact_text: &str) -> String {
    unimplemented!("ai::build_user_prompt not yet implemented")
}

/// Parse the model's response. `Ok(())` if the model returned PASS, `Err(body)`
/// for any other content.
pub fn parse_response(_text: &str) -> Result<(), String> {
    unimplemented!("ai::parse_response not yet implemented")
}

/// One Anthropic Messages API call against one rendered artifact. Returns
/// `Ok(Ok(()))` on PASS, `Ok(Err(violation_block))` on FAIL, `Err` on transport
/// or API failure.
pub fn check_artifact(
    _artifact: &Path,
    _rules: &str,
    _system_prompt: &str,
    _model: &str,
    _api_key_env_var: &str,
    _max_tokens: u32,
) -> anyhow::Result<Result<(), String>> {
    anyhow::bail!("ai::check_artifact not yet implemented")
}
