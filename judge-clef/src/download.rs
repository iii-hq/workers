//! Fetch a pinned Clef checkpoint from the Hugging Face Hub into hf-hub's cache
//! (`$HF_HOME`, default `~/.cache/huggingface`). No token: the repos are public.
use anyhow::{anyhow, Result};
use hf_hub::api::sync::ApiBuilder;
use hf_hub::{Repo, RepoType};
use std::path::{Path, PathBuf};

/// A model the `model` setting accepts: a pinned GGUF of arch `clef`
/// (backbone and joint schema head) and the pinned repository of its
/// tokenizer.
pub struct Model {
    pub name: &'static str,
    pub gguf_repo: &'static str,
    pub gguf_file: &'static str,
    pub gguf_revision: &'static str,
    pub tokenizer_repo: &'static str,
    pub tokenizer_revision: &'static str,
    pub description: &'static str,
}

/// ggml-org's llama.cpp conversion of Cloudflare/clef-flash, with Cloudflare's
/// tokenizer. Against the reference (bf16 backbone and head) on 28 questions,
/// Q4_K_M has a mean |Δp| of 0.007 (max 0.065 on the CPU, 0.080 on Vulkan)
/// and keeps the top option on 26, the misses being near-ties
/// (`tests/clef.rs`); Q8_0 is about 3x closer but 9.7 GB.
pub const MODELS: [Model; 1] = [Model {
    name: "clef-flash",
    gguf_repo: "ggml-org/Clef-Flash-GGUF",
    gguf_file: "Clef-Flash-Q4_K_M.gguf",
    gguf_revision: "4a7a08c09bc63baf043b62b5ba89dd67a0357d95",
    tokenizer_repo: "Cloudflare/clef-flash",
    tokenizer_revision: "17f0b0ad64efb65d273590632833508766b2aae6",
    description: "Clef-Flash: Qwen3.5-9B backbone and joint schema head, Q4_K_M (6.5 GB), Cloudflare's joint decision model",
}];

pub fn model(name: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.name == name)
}

#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub model: String,
    /// The GGUF repository's revision (or `local:<dir>`).
    pub revision: String,
    pub gguf: PathBuf,
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
    // The small file first: a bad tokenizer pin fails before the GGUF download.
    let tokenizer = repo(m.tokenizer_repo, m.tokenizer_revision).get("tokenizer.json")?;
    Ok(Checkpoint {
        model: m.name.into(),
        revision: m.gguf_revision.into(),
        gguf: repo(m.gguf_repo, m.gguf_revision).get(m.gguf_file)?,
        tokenizer,
    })
}

/// A checkpoint already on disk (air-gapped installs, tests), read as model
/// `model`: `model.gguf` and `tokenizer.json`.
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
        gguf: file("model.gguf")?,
        tokenizer: file("tokenizer.json")?,
    })
}
