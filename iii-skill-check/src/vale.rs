use std::path::Path;

/// Shell out to the local `vale` binary against a list of rendered artifacts using the
/// repo's `.vale.ini`. Each Vale alert maps to one Violation.
pub fn run(_artifacts: &[&Path], _vale_config: &Path) -> anyhow::Result<Vec<crate::structure::Violation>> {
    anyhow::bail!("vale::run not yet implemented")
}
