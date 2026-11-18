//! Fetch a laya checkpoint from the Hugging Face Hub into hf-hub's cache
//! (`$HF_HOME`, default `~/.cache/huggingface`). No token: the repo is public.
use anyhow::{Result, anyhow, ensure};
use hf_hub::api::sync::{ApiBuilder, ApiRepo};
use hf_hub::{Cache, Repo, RepoType};
use std::path::{Path, PathBuf};

pub const LAYA_REPO: &str = "convaiinnovations/laya";

/// The reviewed commit of [`LAYA_REPO`] (laya's `PINNED_REVISIONS`); every
/// checkpoint is read from it.
pub const REVISION: &str = "55cf4c4ebb4ebe31b2550e8bdf3bd21b99753851";

/// sha256 of each file a load reads at [`REVISION`]. The weights' come from
/// laya's `verify/checkpoints.json`; the others were hashed from the pinned
/// snapshot after checking each against its git blob id on the Hub.
const DIGESTS: [(&str, &str); 12] = [
    (
        "model.safetensors",
        "891102d372688fc2a094dac56a384bc537b87c63f21f9f3dac0be2b7cbc8d86c",
    ),
    (
        "rl_agent_config.json",
        "ae287b56bbcf5f8c4f4541ae9dfd00c914c4c48b940b8398c3058af37ba92bbd",
    ),
    (
        "encoder/config.json",
        "bf3ab80598fdccf414855a2ce80f22859e4492d06ca8a62ddd1cfb63972f8979",
    ),
    (
        "tokenizer/tokenizer.json",
        "6c8aaa9a542084f2457eab775d4eeb51f92a70c0fd9de28d5edb0ddec3c08d30",
    ),
    (
        "multilingual/model.safetensors",
        "9d628fd971b700382ac6f65920a86f149777b2e748e0c955fb3b19695aa8f204",
    ),
    (
        "multilingual/rl_agent_config.json",
        "25061739243b617ad88d1219ba6f8a9c86c5881ca28df024fa2d9b3b2fcc30c6",
    ),
    (
        "multilingual/encoder/config.json",
        "83f6916d13ef0f556ac461f28308dc2bffa7ebeadee8ec9e2db5812020ea5bb4",
    ),
    (
        "multilingual/tokenizer/tokenizer.json",
        "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f",
    ),
    (
        "typed-decisions/model.safetensors",
        "4fa56de72383a9d3efa9cfa78955733c81b9fc8067a587ca4beb82c78107a24e",
    ),
    (
        "typed-decisions/rl_agent_config.json",
        "ebf0cd524d92342a6be5e48e9fca3d7c2babfb5a56ccd79d2171ef5d8c7f7be8",
    ),
    (
        "typed-decisions/encoder/config.json",
        "5268d24ad3b77c8151de5dcb0762ba4391619aad9ab0bda33e36fb083cfeae6d",
    ),
    (
        "typed-decisions/tokenizer/tokenizer.json",
        "6c8aaa9a542084f2457eab775d4eeb51f92a70c0fd9de28d5edb0ddec3c08d30",
    ),
];

/// The checkpoints the `model` setting accepts.
pub const MODELS: [&str; 3] = ["laya", "laya-multilingual", "laya-typed-decisions"];

#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub model: String,
    /// [`REVISION`], as laya reports it; `local:<dir>` for a checkpoint on disk.
    pub revision: String,
    pub agent_config: PathBuf,
    pub tokenizer: PathBuf,
    /// The encoder and the decision head as one GGUF (f16) for llama.cpp.
    pub gguf: PathBuf,
}

/// `laya` (English, ModernBERT-large), `laya-multilingual` (mmBERT-base) or
/// `laya-typed-decisions` (ModernBERT-large fine-tuned for laya's four
/// workflows, 1024-token window), each with the tokenizer in its own
/// `tokenizer/` directory, as laya loads them. Every file read is checked
/// against its sha256 at [`REVISION`] first; a GGUF converted from verified
/// files is trusted as derived state.
pub fn fetch(model: &str, gguf: Option<&Path>) -> Result<Checkpoint> {
    let subdir = match model {
        "laya" => "",
        "laya-multilingual" => "multilingual/",
        "laya-typed-decisions" => "typed-decisions/",
        other => {
            return Err(anyhow!(
                "unknown laya model {other:?}; expected one of {MODELS:?}"
            ));
        }
    };
    let repo = Repo::with_revision(LAYA_REPO.into(), RepoType::Model, REVISION.into());
    // `from_env` on both: `ApiBuilder::new` ignores HF_HOME (and HF_ENDPOINT),
    // so the files and the GGUF would land in different caches.
    let cache = Cache::from_env();
    let api = ApiBuilder::from_env()
        .with_progress(true)
        .build()?
        .repo(repo.clone());
    let out = cache
        .path()
        .parent()
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("judge-laya")
        .join(format!("{model}-{REVISION}.gguf"));
    let convert = gguf.is_none() && !out.is_file();
    // The weights and the encoder config only feed the conversion.
    let needed = [
        "rl_agent_config.json",
        "tokenizer/tokenizer.json",
        "model.safetensors",
        "encoder/config.json",
    ];
    let needed = &needed[..if convert { 4 } else { 2 }];
    let needed: Vec<String> = needed.iter().map(|f| format!("{subdir}{f}")).collect();
    if let Err(error) = link_snapshot(&api, &cache, &repo, &needed) {
        tracing::warn!(
            %error,
            "could not link the cached laya files into the pinned snapshot; downloading them"
        );
    }
    let get = |file: &str| -> Result<PathBuf> {
        let file = format!("{subdir}{file}");
        let path = api.get(&file)?;
        verify(&path, &file, digest(&file)?)?;
        Ok(path)
    };
    let agent_config = get("rl_agent_config.json")?;
    let tokenizer = get("tokenizer/tokenizer.json")?;
    let gguf = match gguf {
        Some(path) if path.is_file() => path.to_path_buf(),
        Some(path) => return Err(anyhow!("GGUF {} does not exist", path.display())),
        None if convert => converted(
            &get("model.safetensors")?,
            &get("encoder/config.json")?,
            &agent_config,
            &tokenizer,
            &out,
        )?,
        None => out,
    };
    Ok(Checkpoint {
        model: model.into(),
        revision: REVISION.into(),
        agent_config,
        tokenizer,
        gguf,
    })
}

fn digest(file: &str) -> Result<&'static str> {
    DIGESTS
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, digest)| *digest)
        .ok_or_else(|| anyhow!("{file} has no pinned sha256"))
}

/// Refuse `path` (the repo's `file`) unless its sha256 is `expected`: hf-hub's
/// `get` never hashes what it downloads or finds in the cache.
fn verify(path: &Path, file: &str, expected: &str) -> Result<()> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    let found = format!("{:x}", hasher.finalize());
    ensure!(
        found == expected,
        "{file} ({}) has sha256 {found}, expected {expected} at revision {REVISION}",
        path.display()
    );
    Ok(())
}

/// Point `refs/<REVISION>` at the pinned snapshot, linking into it the blobs
/// already in the cache, as huggingface_hub does: `get` would download them
/// again (2.3 GB for the three checkpoints). Asks the Hub only while one of
/// `files` is missing from that snapshot, so once it is linked a load needs
/// no network.
fn link_snapshot(api: &ApiRepo, cache: &Cache, repo: &Repo, files: &[String]) -> Result<()> {
    let cached = cache.repo(repo.clone());
    if files.iter().all(|file| cached.get(file).is_some()) {
        return Ok(());
    }
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
    // The commit becomes the ref, and the blob ids cache paths.
    ensure!(
        head.sha == repo.revision(),
        "the Hub named commit {:?} for {}",
        head.sha,
        repo.revision()
    );
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
    use super::{DIGESTS, LAYA_REPO, MODELS, REVISION, digest, link_snapshot, verify};
    use hf_hub::api::sync::ApiBuilder;
    use hf_hub::{Cache, Repo, RepoType};

    #[test]
    fn every_file_a_load_reads_has_a_pinned_digest() {
        for subdir in ["", "multilingual/", "typed-decisions/"] {
            for file in [
                "rl_agent_config.json",
                "tokenizer/tokenizer.json",
                "model.safetensors",
                "encoder/config.json",
            ] {
                assert!(digest(&format!("{subdir}{file}")).is_ok(), "{subdir}{file}");
            }
        }
        assert_eq!(DIGESTS.len(), 4 * MODELS.len());
    }

    #[test]
    fn verify_refuses_a_file_whose_sha256_differs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rl_agent_config.json");
        std::fs::write(&path, "abc").unwrap();
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        verify(&path, "rl_agent_config.json", abc).unwrap();
        let error = verify(
            &path,
            "rl_agent_config.json",
            digest("rl_agent_config.json").unwrap(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("rl_agent_config.json") && error.contains(abc),
            "{error}"
        );
    }

    #[test]
    fn a_linked_snapshot_needs_no_hub() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path().into());
        let repo = Repo::with_revision(LAYA_REPO.into(), RepoType::Model, REVISION.into());
        // A port nothing listens on: any Hub request fails.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let api = ApiBuilder::from_cache(cache.clone())
            .with_endpoint(format!("http://127.0.0.1:{port}"))
            .with_retries(0)
            .build()
            .unwrap()
            .repo(repo.clone());
        let files = [
            "rl_agent_config.json".to_owned(),
            "tokenizer/tokenizer.json".to_owned(),
        ];
        assert!(link_snapshot(&api, &cache, &repo, &files).is_err());
        let snapshot = dir
            .path()
            .join(repo.folder_name())
            .join("snapshots")
            .join(REVISION);
        for file in &files {
            let path = snapshot.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "{}").unwrap();
        }
        cache.repo(repo.clone()).create_ref(REVISION).unwrap();
        link_snapshot(&api, &cache, &repo, &files).unwrap();
    }
}
