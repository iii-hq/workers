use iii_sdk::{register_worker, InitOptions, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[allow(dead_code)]
mod config;

#[derive(Deserialize, JsonSchema)]
struct AnalyzeArgs {
    text: String,
}

#[derive(Serialize, JsonSchema)]
struct AnalyzeResult {
    words: usize,
    chars: usize,
    lines: usize,
}

#[derive(Deserialize, JsonSchema)]
struct DiffArgs {
    before: String,
    after: String,
}

#[derive(Serialize, JsonSchema)]
struct DiffResult {
    added: Vec<String>,
    removed: Vec<String>,
}

#[derive(Default, Deserialize, JsonSchema)]
struct SummarizeArgs {}

#[derive(Serialize, JsonSchema)]
struct SummaryResult {
    analyses: usize,
    avg_words: f64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    iii.register_function(
        RegisterFunction::new("textstats::analyze", |args: AnalyzeArgs| {
            let words = args.text.split_whitespace().count();
            let chars = args.text.chars().count();
            let lines = args.text.lines().count();
            Ok::<_, String>(AnalyzeResult {
                words,
                chars,
                lines,
            })
        })
        .description("Count words, characters, and lines in a string."),
    );

    iii.register_function(
        RegisterFunction::new("textstats::diff", |args: DiffArgs| {
            let before: Vec<&str> = args.before.split_whitespace().collect();
            let after: Vec<&str> = args.after.split_whitespace().collect();
            Ok::<_, String>(DiffResult {
                added: after
                    .iter()
                    .filter(|w| !before.contains(w))
                    .map(|s| (*s).to_string())
                    .collect(),
                removed: before
                    .iter()
                    .filter(|w| !after.contains(w))
                    .map(|s| (*s).to_string())
                    .collect(),
            })
        })
        .description("Compute word-level diff between two strings."),
    );

    iii.register_function(
        RegisterFunction::new("textstats::summarize", |_args: SummarizeArgs| {
            Ok::<_, String>(SummaryResult {
                analyses: 0,
                avg_words: 0.0,
            })
        })
        .description("Return rollup stats over the recent analysis window."),
    );

    tokio::signal::ctrl_c().await?;
    Ok(())
}
