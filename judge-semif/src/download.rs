//! Fetch a pinned GGUF from the Hugging Face Hub into hf-hub's cache
//! (`$HF_HOME`, default `~/.cache/huggingface`). No token: the repos are public.
use anyhow::{anyhow, Result};
use hf_hub::api::sync::ApiBuilder;
use hf_hub::{Repo, RepoType};
use std::path::{Path, PathBuf};

/// A model the `model` setting accepts: SemIf's pinned GGUF.
pub struct Model {
    pub name: &'static str,
    pub repo: &'static str,
    pub file: &'static str,
    pub revision: &'static str,
    pub description: &'static str,
}

/// SemIf's `manifests/models.json` (`browser_high_memory_artifact`).
pub const MODELS: [Model; 1] = [Model {
    name: "qwen3.5-4b",
    repo: "bartowski/Qwen_Qwen3.5-4B-GGUF",
    file: "Qwen_Qwen3.5-4B-Q4_K_M.gguf",
    revision: "4168f45a16a1290d65a4ec0fa312ae917a4c15d6",
    description: "Qwen3.5-4B Q4_K_M (3.0 GB), SemIf's direct option-logit baseline",
}];

pub fn model(name: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.name == name)
}

#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub model: String,
    pub revision: String,
    pub gguf: PathBuf,
}

pub fn fetch(name: &str) -> Result<Checkpoint> {
    let m = model(name).ok_or_else(|| anyhow!("unknown SemIf model {name:?}"))?;
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
    })
}

/// A GGUF already on disk (air-gapped installs, tests).
pub fn local(name: &str, gguf: &Path) -> Result<Checkpoint> {
    if !gguf.is_file() {
        return Err(anyhow!("GGUF {} does not exist", gguf.display()));
    }
    Ok(Checkpoint {
        model: name.into(),
        revision: format!("local:{}", gguf.display()),
        gguf: gguf.into(),
    })
}
