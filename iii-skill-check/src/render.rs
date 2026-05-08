use std::collections::BTreeMap;
use std::path::Path;

/// Output of rendering one worker.
///
/// `leaves` is keyed by leaf name (e.g. `"analyze"`), values are the full
/// rendered body for `skills/<leaf>.md`. BTreeMap so iteration order is stable.
pub struct RenderOutput {
    pub readme: String,
    pub skill: String,
    pub leaves: BTreeMap<String, String>,
}

/// Render a worker dir into its three artifact strings (no IO).
///
/// Reads:
///   - `<dir>/iii.worker.yaml`
///   - `<dir>/config.yaml`
///   - `<dir>/src/**/*.rs`
///   - `<dir>/docs/intro.md`, `<dir>/docs/quickstart.md`, `<dir>/docs/migration.md` (optional)
///   - `<dir>/docs/leaves/*.md`
pub fn render_worker(_dir: &Path) -> anyhow::Result<RenderOutput> {
    anyhow::bail!("render::render_worker not yet implemented")
}
