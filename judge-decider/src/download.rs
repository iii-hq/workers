//! Fetch a pinned GGUF from the Hugging Face Hub into hf-hub's cache
//! (`$HF_HOME`, default `~/.cache/huggingface`). No token: the repos are public.
use anyhow::{anyhow, Result};
use hf_hub::api::sync::ApiBuilder;
use hf_hub::{Repo, RepoType};
use std::path::{Path, PathBuf};

/// A model the `model` setting accepts: a pinned GGUF and its softmax
/// temperature (`decider_config.json`).
pub struct Model {
    pub name: &'static str,
    pub repo: &'static str,
    pub file: &'static str,
    pub revision: &'static str,
    pub description: &'static str,
    pub temperature: f64,
}

/// Mapika/decider-4b at tag v2 (49564ddc, JevBench v1.4.2 rank 1), quantized
/// by llama.cpp; the repository's main is v2.1, better at sampled play but
/// worse on hard decisions, and has no published GGUF.
pub const MODELS: [Model; 1] = [Model {
    name: "decider-4b-v2",
    repo: "mindchain/decider-4b-v2-GGUF",
    file: "decider-4b.v2-Q4_K_M.gguf",
    revision: "2796fac5cdad018ef6d9a4a004735ff819f424a2",
    description: "decider-4b v2 Q4_K_M (2.7 GB), a Qwen3.5-4B-Base fine-tune trained for option-logit decisions",
    temperature: 1.935,
}];

pub fn model(name: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.name == name)
}

#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub model: String,
    pub revision: String,
    pub gguf: PathBuf,
    pub temperature: f64,
}

fn known(name: &str) -> Result<&'static Model> {
    model(name).ok_or_else(|| anyhow!("unknown decider model {name:?}"))
}

pub fn fetch(name: &str) -> Result<Checkpoint> {
    let m = known(name)?;
    let api = ApiBuilder::new().with_progress(true).build()?;
    let repo = api.repo(Repo::with_revision(
        m.repo.into(),
        RepoType::Model,
        m.revision.into(),
    ));
    Ok(Checkpoint {
        model: m.name.into(),
        revision: m.revision.into(),
        gguf: repo.get(m.file)?,
        temperature: m.temperature,
    })
}

/// A GGUF already on disk (air-gapped installs, tests), read as model `name`.
pub fn local(name: &str, gguf: &Path) -> Result<Checkpoint> {
    let m = known(name)?;
    if !gguf.is_file() {
        return Err(anyhow!("GGUF {} does not exist", gguf.display()));
    }
    Ok(Checkpoint {
        model: name.into(),
        revision: format!("local:{}", gguf.display()),
        gguf: gguf.into(),
        temperature: m.temperature,
    })
}
