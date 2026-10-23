//! Fetch a laya checkpoint from the Hugging Face Hub into hf-hub's cache
//! (`$HF_HOME`, default `~/.cache/huggingface`). No token: the repo is public.
use anyhow::{anyhow, ensure, Result};
use hf_hub::api::sync::{ApiBuilder, ApiRepo};
use hf_hub::{Cache, Repo, RepoType};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub const LAYA_REPO: &str = "convaiinnovations/laya";

/// The checkpoints the `model` setting accepts.
pub const MODELS: [&str; 3] = ["laya", "laya-multilingual", "laya-typed-decisions"];

#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub model: String,
    /// The commit whose snapshot the files were read from, as laya reports it;
    /// `local:<dir>` for a checkpoint on disk.
    pub revision: String,
    pub agent_config: PathBuf,
    pub tokenizer: PathBuf,
    /// The encoder and the decision head as one GGUF (f16) for llama.cpp.
    pub gguf: PathBuf,
}

/// `laya` (English, ModernBERT-large), `laya-multilingual` (mmBERT-base) or
/// `laya-typed-decisions` (ModernBERT-large fine-tuned for laya's four
/// workflows, 1024-token window), each with the tokenizer in its own
/// `tokenizer/` directory, as laya loads them.
pub fn fetch(model: &str, revision: Option<&str>, gguf: Option<&Path>) -> Result<Checkpoint> {
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
    let repo = Repo::with_revision(
        LAYA_REPO.into(),
        RepoType::Model,
        revision.unwrap_or("main").into(),
    );
    // `from_env` on both: `ApiBuilder::new` ignores HF_HOME (and HF_ENDPOINT),
    // so the files and the GGUF would land in different caches.
    let cache = Cache::from_env();
    let api = ApiBuilder::from_env()
        .with_progress(true)
        .build()?
        .repo(repo.clone());
    // The weights and the encoder config only feed the conversion.
    let needed = [
        "rl_agent_config.json",
        "tokenizer/tokenizer.json",
        "model.safetensors",
        "encoder/config.json",
    ];
    let needed = &needed[..if gguf.is_some() { 2 } else { 4 }];
    let needed: Vec<String> = needed.iter().map(|f| format!("{subdir}{f}")).collect();
    if let Err(error) = follow(&api, &cache, &repo, &needed) {
        tracing::warn!(
            %error,
            revision = repo.revision(),
            "could not check the laya revision on the Hugging Face Hub; using the cached snapshot"
        );
    }
    let get = |file: &str| api.get(&format!("{subdir}{file}"));
    let agent_config = get("rl_agent_config.json")?;
    let tokenizer = get("tokenizer/tokenizer.json")?;
    // Every file resolves through the same ref, so one snapshot names them all.
    let revision = snapshot(&agent_config, &needed[0])
        .ok_or_else(|| anyhow!("{} is not in a hub snapshot", agent_config.display()))?
        .to_owned();
    let gguf = match gguf {
        Some(path) if path.is_file() => path.to_path_buf(),
        Some(path) => return Err(anyhow!("GGUF {} does not exist", path.display())),
        None => {
            // Keyed by the snapshot the inputs came from: new weights, configs
            // or tokenizer convert again, a model-card-only commit does not.
            let dir = cache
                .path()
                .parent()
                .map_or_else(std::env::temp_dir, PathBuf::from)
                .join("judge-laya");
            converted(
                &get("model.safetensors")?,
                &get("encoder/config.json")?,
                &agent_config,
                &tokenizer,
                &dir.join(format!("{model}-{revision}.gguf")),
            )?
        }
    };
    Ok(Checkpoint {
        model: model.into(),
        revision,
        agent_config,
        tokenizer,
        gguf,
    })
}

/// Move `refs/<revision>` to the commit the revision names on the Hub now, when
/// one of `files` differs there from the cached snapshot. hf-hub's `get` never
/// asks the Hub about a file it already holds in the snapshot that ref names,
/// so without this a null `revision` would stay on the first `main` fetched.
/// A commit that leaves `files` alone (most only edit the model card) keeps
/// the cached snapshot, and with it the GGUF named after it.
fn follow(api: &ApiRepo, cache: &Cache, repo: &Repo, files: &[String]) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct Head {
        sha: String,
        siblings: Vec<Sibling>,
    }
    #[derive(serde::Deserialize)]
    struct Sibling {
        rfilename: String,
        #[serde(rename = "blobId")]
        blob_id: String,
        lfs: Option<Lfs>,
    }
    #[derive(serde::Deserialize)]
    struct Lfs {
        sha256: String,
    }
    let hex = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit());
    let head: Head = api
        .info_request()
        .query("blobs", "true")
        .call()?
        .into_json()?;
    // The commit and the blob ids become cache paths.
    ensure!(hex(&head.sha), "the Hub named commit {:?}", head.sha);
    // hf-hub names a blob after the file's LFS sha256, else its git blob id.
    let blob = |file: &String| {
        head.siblings
            .iter()
            .find(|sibling| &sibling.rfilename == file)
            .map(|sibling| {
                sibling
                    .lfs
                    .as_ref()
                    .map_or(&sibling.blob_id, |lfs| &lfs.sha256)
            })
            .filter(|id| hex(id))
    };
    let cached = cache.repo(repo.clone());
    let current = files.iter().all(|file| {
        let path = cached.get(file).and_then(|path| path.canonicalize().ok());
        path.as_deref().and_then(Path::file_name) == blob(file).map(OsStr::new)
    });
    if current {
        return Ok(());
    }
    // Link the blobs already cached into the new snapshot, as huggingface_hub
    // does; `get` would download them again.
    #[cfg(unix)]
    {
        let dir = cache.path().join(repo.folder_name());
        for file in files {
            let link = dir.join("snapshots").join(&head.sha).join(file);
            let Some(id) = blob(file).filter(|id| dir.join("blobs").join(id).is_file()) else {
                continue;
            };
            if !link.exists() {
                std::fs::create_dir_all(link.parent().expect("a snapshot file has a parent"))?;
                // Relative, like hf-hub's own links: up past the file's
                // directories and `snapshots/<sha>`.
                let up = "../".repeat(file.matches('/').count() + 2);
                std::os::unix::fs::symlink(format!("{up}blobs/{id}"), &link)?;
            }
        }
    }
    cached.create_ref(&head.sha)?;
    Ok(())
}

/// The commit of the hub snapshot `path` holds `file` in
/// (`…/snapshots/<sha>/<file>`), as laya's `snapshot_revision` reads it.
fn snapshot<'a>(path: &'a Path, file: &str) -> Option<&'a str> {
    let dir = path.ancestors().nth(Path::new(file).components().count())?;
    let sha = dir.file_name()?.to_str()?;
    (dir.parent()?.file_name()? == "snapshots").then_some(sha)
}

/// The GGUF at `out`, converted from the checkpoint when it is not there yet
/// (seconds; about 840 MB for `laya`).
fn converted(
    weights: &Path,
    encoder_config: &Path,
    agent_config: &Path,
    tokenizer: &Path,
    out: &Path,
) -> Result<PathBuf> {
    if !out.is_file() {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let started = std::time::Instant::now();
        tracing::info!(out = %out.display(), "converting the laya checkpoint to GGUF");
        crate::gguf::convert(weights, encoder_config, agent_config, tokenizer, out)?;
        tracing::info!(
            seconds = started.elapsed().as_secs_f32(),
            "laya checkpoint converted"
        );
    }
    Ok(out.to_path_buf())
}

/// A checkpoint already on disk (air-gapped installs, tests): `model.safetensors`,
/// `encoder/config.json`, `rl_agent_config.json`, `tokenizer.json`, and
/// optionally `model.gguf`; without it the checkpoint is converted once into
/// the temporary directory, keyed by the weights' path, size and mtime.
pub fn local(model: &str, dir: &Path) -> Result<Checkpoint> {
    let file = |name: &str| {
        let path = dir.join(name);
        path.is_file()
            .then_some(path)
            .ok_or_else(|| anyhow!("checkpoint directory {} lacks {name}", dir.display()))
    };
    let agent_config = file("rl_agent_config.json")?;
    let tokenizer = file("tokenizer.json")?;
    let gguf = match file("model.gguf") {
        Ok(path) => path,
        Err(_) => {
            use std::hash::{Hash, Hasher};
            let weights = file("model.safetensors")?;
            let meta = std::fs::metadata(&weights)?;
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            (weights.canonicalize()?, meta.len(), meta.modified().ok()).hash(&mut hash);
            let out = std::env::temp_dir()
                .join("judge-laya")
                .join(format!("{model}-{:016x}.gguf", hash.finish()));
            converted(
                &weights,
                &file("encoder/config.json")?,
                &agent_config,
                &tokenizer,
                &out,
            )?
        }
    };
    Ok(Checkpoint {
        model: model.into(),
        revision: format!("local:{}", dir.display()),
        agent_config,
        tokenizer,
        gguf,
    })
}

#[cfg(test)]
mod tests {
    use super::snapshot;
    use std::path::Path;

    #[test]
    fn snapshot_names_the_commit_a_cached_file_came_from() {
        let repo = Path::new("/srv/snapshots/hub/models--convaiinnovations--laya");
        for file in [
            "rl_agent_config.json",
            "multilingual/tokenizer/tokenizer.json",
        ] {
            let path = repo.join("snapshots/7b928d82").join(file);
            assert_eq!(snapshot(&path, file), Some("7b928d82"), "{file}");
        }
        // Not in a snapshot, though `snapshots` is further up the path.
        let blob = repo.join("blobs/891102d3");
        assert_eq!(snapshot(&blob, "model.safetensors"), None);
        let local = Path::new("checkpoint/rl_agent_config.json");
        assert_eq!(snapshot(local, "rl_agent_config.json"), None);
    }
}
