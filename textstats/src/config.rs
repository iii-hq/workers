// Runtime config struct, deserialized from `config.yaml`. Not consumed by
// iii-skill-check; the renderer inlines `config.yaml` verbatim under
// `## Configuration` rather than introspecting the schema here.
use serde::Deserialize;

/// Runtime config for the textstats worker.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Number of recent analyses retained for textstats::summarize rollups.
    #[serde(default = "default_recent_window")]
    pub recent_window: usize,

    /// ISO 639-1 language codes the analyzer uses as tokenization hints.
    #[serde(default)]
    pub language_hints: Vec<String>,

    /// Where the worker persists rolling analysis state on disk.
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
}

fn default_recent_window() -> usize {
    100
}

fn default_data_dir() -> String {
    "~/.iii/data/textstats".into()
}
