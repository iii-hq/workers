//! Fetch a pinned Clef checkpoint from the Hugging Face Hub into hf-hub's cache
//! (`$HF_HOME`, default `~/.cache/huggingface`). No token: the repos are public.
use anyhow::{anyhow, Result};
use hf_hub::api::sync::ApiBuilder;
use hf_hub::{Repo, RepoType};
use std::path::{Path, PathBuf};

/// A model the `model` setting accepts: a pinned backbone GGUF and the pinned
/// repository of its joint schema head and tokenizer.
pub struct Model {
    pub name: &'static str,
    pub gguf_repo: &'static str,
    pub gguf_file: &'static str,
    pub gguf_revision: &'static str,
    pub head_repo: &'static str,
    pub head_revision: &'static str,
    pub description: &'static str,
}

/// Cloudflare/clef-flash's head and tokenizer, with bartowski's llama.cpp
/// conversion of its backbone (arch `qwen35`, keeping the untied
/// `output.weight` the head reads). Against the reference (bf16 backbone and
/// head) on 28 questions, Q4_K_M has a mean |Δp| of 0.008 (max 0.09 on the
/// CPU, 0.15 on Vulkan) and keeps every top option but near-ties
/// (`tests/clef.rs`); Q8_0 is about 3x closer but 9.5 GB.
pub const MODELS: [Model; 1] = [Model {
    name: "clef-flash",
    gguf_repo: "bartowski/Cloudflare_clef-flash-GGUF",
    gguf_file: "Cloudflare_clef-flash-Q4_K_M.gguf",
    gguf_revision: "d7f376ea88c05e7bb1014dd5351a93df9dd8029e",
    head_repo: "Cloudflare/clef-flash",
    head_revision: "17f0b0ad64efb65d273590632833508766b2aae6",
    description: "Clef-Flash: Qwen3.5-9B backbone Q4_K_M (5.8 GB) and joint schema head (244 MB), Cloudflare's joint decision model",
}];

pub fn model(name: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.name == name)
}

#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub model: String,
    /// The head repository's revision (or `local:<dir>`).
    pub revision: String,
    pub gguf: PathBuf,
    pub head: PathBuf,
    pub head_config: PathBuf,
    pub tokenizer: PathBuf,
}

fn known(name: &str) -> Result<&'static Model> {
    model(name).ok_or_else(|| anyhow!("unknown clef model {name:?}"))
}

pub fn fetch(name: &str) -> Result<Checkpoint> {
    let m = known(name)?;
    let api = ApiBuilder::new().with_progress(true).build()?;
    let repo = |id: &str, revision: &str| {
        api.repo(Repo::with_revision(
            id.into(),
            RepoType::Model,
            revision.into(),
        ))
    };
    // The small files first: a bad head pin fails before the GGUF download.
    let head = repo(m.head_repo, m.head_revision);
    let (weights, head_config, tokenizer) = (
        head.get("joint_head.safetensors")?,
        head.get("joint_head_config.json")?,
        head.get("tokenizer.json")?,
    );
    Ok(Checkpoint {
        model: m.name.into(),
        revision: m.head_revision.into(),
        gguf: repo(m.gguf_repo, m.gguf_revision).get(m.gguf_file)?,
        head: weights,
        head_config,
        tokenizer,
    })
}

/// A checkpoint already on disk (air-gapped installs, tests), read as model
/// `model`: `backbone.gguf`, `joint_head.safetensors`,
/// `joint_head_config.json` and `tokenizer.json`.
pub fn local(model: &str, dir: &Path) -> Result<Checkpoint> {
    known(model)?;
    let file = |name: &str| {
        let path = dir.join(name);
        path.is_file()
            .then_some(path)
            .ok_or_else(|| anyhow!("checkpoint directory {} lacks {name}", dir.display()))
    };
    Ok(Checkpoint {
        model: model.into(),
        revision: format!("local:{}", dir.display()),
        gguf: file("backbone.gguf")?,
        head: file("joint_head.safetensors")?,
        head_config: file("joint_head_config.json")?,
        tokenizer: file("tokenizer.json")?,
    })
}
