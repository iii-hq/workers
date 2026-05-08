use std::collections::BTreeMap;
use std::path::Path;

/// Output of rendering one worker.
///
/// `leaves` is keyed by leaf name (e.g. `"analyze"`), value = the full rendered
/// body for `skills/<leaf>.md`. BTreeMap so iteration order is stable.
pub struct RenderOutput {
    pub readme: String,
    pub skill: String,
    pub leaves: BTreeMap<String, String>,
}

/// Render a worker dir into its three artifact strings (no IO writes).
///
/// Reads:
///   - `<dir>/iii.worker.yaml` for `name`
///   - `<dir>/config.yaml` (inlined verbatim into ## Configuration)
///   - `<dir>/docs/intro.md` (used in README and skill.md)
///   - `<dir>/docs/quickstart.md`
///   - `<dir>/docs/migration.md` (optional)
///   - `<dir>/docs/leaves/*.md` (one per `skills/<leaf>.md` output)
///
/// Does NOT parse `<dir>/src/` — function ids, signatures, descriptions, and
/// custom trigger payloads are auto-generated elsewhere.
pub fn render_worker(_dir: &Path) -> anyhow::Result<RenderOutput> {
    anyhow::bail!("render::render_worker not yet implemented")
}
