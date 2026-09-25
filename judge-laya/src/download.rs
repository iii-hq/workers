//! Fetch a laya checkpoint from the Hugging Face Hub into hf-hub's cache
//! (`$HF_HOME`, default `~/.cache/huggingface`). No token: both repos are public.
use anyhow::{anyhow, Result};
use hf_hub::api::sync::ApiBuilder;
use hf_hub::{Repo, RepoType};
use std::path::{Path, PathBuf};

pub const LAYA_REPO: &str = "convaiinnovations/laya";
const ENGLISH_TOKENIZER_REPO: &str = "answerdotai/ModernBERT-large";

/// The checkpoints the `model` setting accepts.
pub const MODELS: [&str; 3] = ["laya", "laya-multilingual", "laya-typed-decisions"];

#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub model: String,
    pub revision: String,
    pub weights: PathBuf,
    pub encoder_config: PathBuf,
    pub agent_config: PathBuf,
    pub tokenizer: PathBuf,
    /// The encoder as a GGUF (f16) for llama.cpp; the head stays in `weights`.
    pub encoder_gguf: PathBuf,
}

/// `laya` (English, ModernBERT-large), `laya-multilingual` (mmBERT-base) or
/// `laya-typed-decisions` (ModernBERT-large fine-tuned for laya's four
/// workflows, 1024-token window). Only the English checkpoint ships no
/// tokenizer; it uses ModernBERT-large's.
pub fn fetch(
    model: &str,
    revision: Option<&str>,
    encoder_gguf: Option<&std::path::Path>,
) -> Result<Checkpoint> {
    let subdir = match model {
        "laya" => "",
        "laya-multilingual" => "multilingual/",
        "laya-typed-decisions" => "typed-decisions/",
        other => {
            return Err(anyhow!(
                "unknown laya model {other:?}; expected one of {MODELS:?}"
            ))
        }
    };
    let api = ApiBuilder::new().with_progress(true).build()?;
    let repo = api.repo(Repo::with_revision(
        LAYA_REPO.into(),
        RepoType::Model,
        revision.unwrap_or("main").into(),
    ));
    let resolved = repo
        .info()
        .map(|info| info.sha)
        .unwrap_or_else(|_| revision.unwrap_or("main").into());
    let get = |file: &str| repo.get(&format!("{subdir}{file}"));
    let tokenizer = if subdir.is_empty() {
        api.model(ENGLISH_TOKENIZER_REPO.into())
            .get("tokenizer.json")?
    } else {
        get("tokenizer/tokenizer.json")?
    };
    let weights = get("model.safetensors")?;
    let encoder_config = get("encoder/config.json")?;
    let encoder_gguf = match encoder_gguf {
        Some(path) if path.is_file() => path.to_path_buf(),
        Some(path) => return Err(anyhow!("encoder GGUF {} does not exist", path.display())),
        None => {
            // Keyed by the resolved revision: a new checkpoint converts again.
            let dir = hf_hub::Cache::from_env()
                .path()
                .parent()
                .map_or_else(std::env::temp_dir, PathBuf::from)
                .join("judge-laya");
            converted(
                &weights,
                &encoder_config,
                &tokenizer,
                &dir.join(format!("{model}-{resolved}-encoder.gguf")),
            )?
        }
    };
    Ok(Checkpoint {
        model: model.into(),
        revision: resolved,
        weights,
        encoder_config,
        agent_config: get("rl_agent_config.json")?,
        tokenizer,
        encoder_gguf,
    })
}

/// The encoder GGUF at `out`, converted from the checkpoint when it is not
/// there yet (seconds; about 800 MB for `laya`).
fn converted(
    weights: &Path,
    encoder_config: &Path,
    tokenizer: &Path,
    out: &Path,
) -> Result<PathBuf> {
    if !out.is_file() {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let started = std::time::Instant::now();
        tracing::info!(out = %out.display(), "converting the laya encoder to GGUF");
        crate::gguf::convert(weights, encoder_config, tokenizer, out)?;
        tracing::info!(
            seconds = started.elapsed().as_secs_f32(),
            "laya encoder converted"
        );
    }
    Ok(out.to_path_buf())
}

/// A checkpoint already on disk (air-gapped installs, tests): `model.safetensors`,
/// `encoder/config.json`, `rl_agent_config.json`, `tokenizer.json`, and
/// optionally `encoder.gguf`; without it the encoder is converted once into
/// the temporary directory, keyed by the weights' path, size and mtime.
pub fn local(model: &str, dir: &Path) -> Result<Checkpoint> {
    let file = |name: &str| {
        let path = dir.join(name);
        path.is_file()
            .then_some(path)
            .ok_or_else(|| anyhow!("checkpoint directory {} lacks {name}", dir.display()))
    };
    let weights = file("model.safetensors")?;
    let encoder_config = file("encoder/config.json")?;
    let tokenizer = file("tokenizer.json")?;
    let encoder_gguf = match file("encoder.gguf") {
        Ok(path) => path,
        Err(_) => {
            use std::hash::{Hash, Hasher};
            let meta = std::fs::metadata(&weights)?;
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            (weights.canonicalize()?, meta.len(), meta.modified().ok()).hash(&mut hash);
            let out = std::env::temp_dir()
                .join("judge-laya")
                .join(format!("{model}-{:016x}-encoder.gguf", hash.finish()));
            converted(&weights, &encoder_config, &tokenizer, &out)?
        }
    };
    Ok(Checkpoint {
        model: model.into(),
        revision: format!("local:{}", dir.display()),
        weights,
        encoder_config,
        agent_config: file("rl_agent_config.json")?,
        tokenizer,
        encoder_gguf,
    })
}
