use std::path::Path;

/// One Claude API call against one rendered artifact. Returns `Ok(Ok(()))` on PASS,
/// `Ok(Err(violation_block))` on FAIL, `Err` on transport / API failure.
pub fn check_artifact(
    _artifact: &Path,
    _rules: &str,
    _prompt: &str,
) -> anyhow::Result<Result<(), String>> {
    anyhow::bail!("ai::check_artifact not yet implemented")
}
