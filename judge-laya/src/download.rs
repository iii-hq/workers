//! Fetch a laya checkpoint from the Hugging Face Hub into hf-hub's cache
//! (`$HF_HOME`, default `~/.cache/huggingface`). No token: both repos are public.
use anyhow::{anyhow, Result};
use hf_hub::api::sync::ApiBuilder;
use hf_hub::{Repo, RepoType};
use std::path::PathBuf;

pub const LAYA_REPO: &str = "convaiinnovations/laya";
const ENGLISH_TOKENIZER_REPO: &str = "answerdotai/ModernBERT-large";

/// The checkpoints the `model` setting accepts.
pub const MODELS: [&str; 2] = ["laya", "laya-multilingual"];

#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub model: String,
    pub revision: String,
    pub weights: PathBuf,
    pub encoder_config: PathBuf,
    pub agent_config: PathBuf,
    pub tokenizer: PathBuf,
}

/// `laya` (English, ModernBERT-large) or `laya-multilingual` (mmBERT-base). The
/// English checkpoint ships no tokenizer; it uses ModernBERT-large's.
pub fn fetch(model: &str, revision: Option<&str>) -> Result<Checkpoint> {
    let subdir = match model {
        "laya" => "",
        "laya-multilingual" => "multilingual/",
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
    Ok(Checkpoint {
        model: model.into(),
        revision: resolved,
        weights: get("model.safetensors")?,
        encoder_config: get("encoder/config.json")?,
        agent_config: get("rl_agent_config.json")?,
        tokenizer,
    })
}

/// A checkpoint already on disk (air-gapped installs, tests): `model.safetensors`,
/// `encoder/config.json`, `rl_agent_config.json`, `tokenizer.json`.
pub fn local(model: &str, dir: &std::path::Path) -> Result<Checkpoint> {
    let file = |name: &str| {
        let path = dir.join(name);
        path.is_file()
            .then_some(path)
            .ok_or_else(|| anyhow!("checkpoint directory {} lacks {name}", dir.display()))
    };
    Ok(Checkpoint {
        model: model.into(),
        revision: format!("local:{}", dir.display()),
        weights: file("model.safetensors")?,
        encoder_config: file("encoder/config.json")?,
        agent_config: file("rl_agent_config.json")?,
        tokenizer: file("tokenizer.json")?,
    })
}
