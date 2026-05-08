use std::path::Path;

#[derive(Debug, Clone)]
pub struct Violation {
    pub file: String,
    pub line: Option<usize>,
    pub message: String,
}

/// Run Layer 1 deterministic structure checks against a worker dir.
pub fn check(_dir: &Path) -> anyhow::Result<Vec<Violation>> {
    anyhow::bail!("structure::check not yet implemented")
}
