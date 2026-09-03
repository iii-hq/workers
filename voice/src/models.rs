//! The built-in model catalog and the download that installs one.
//!
//! Nothing ships inside the binary: the first `voice::dictation::start` or
//! `voice::transcribe` on the `local` backend downloads the configured
//! models into `models_dir`, one file at a time, verifying every file's
//! SHA-256 before it is trusted. A second worker instance racing on the same
//! directory sees the same verified files, because a file is renamed into
//! place only after its hash matched.
//!
//! Two kinds of model live here: a small streaming transducer that produces
//! live partial text, and a large offline transducer that re-decodes each
//! finished utterance for the final text with punctuation and casing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

/// Streaming model installed and used when the configuration names none.
pub const DEFAULT_MODEL: &str = "zipformer-en-20m";
/// Second-pass model used when the configuration names none.
pub const DEFAULT_FINAL_MODEL: &str = "parakeet-tdt-0.6b-v2";

/// How the recognizer loads a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// Streaming zipformer transducer: partial text as audio arrives.
    StreamingTransducer,
    /// NeMo offline transducer: one decode per finished utterance.
    OfflineNemoTransducer,
}

/// One downloadable file of a model.
#[derive(Debug, Clone)]
pub struct ModelFile {
    /// File name inside the model directory.
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
}

/// A model the local engine can load.
#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: ModelKind,
    pub languages: &'static [&'static str],
    pub license: &'static str,
    pub files: &'static [ModelFile],
    pub encoder: &'static str,
    pub decoder: &'static str,
    pub joiner: &'static str,
    pub tokens: &'static str,
}

impl ModelSpec {
    pub fn size_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size_bytes).sum()
    }

    /// Directory holding this model's files.
    pub fn dir(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.id)
    }

    /// `true` when every file is present with the expected size. The hash is
    /// checked at download time; re-hashing hundreds of megabytes on every
    /// boot is not worth the seconds it costs.
    pub fn is_installed(&self, models_dir: &Path) -> bool {
        let dir = self.dir(models_dir);
        self.files.iter().all(|f| {
            std::fs::metadata(dir.join(f.name))
                .is_ok_and(|m| m.is_file() && m.len() == f.size_bytes)
        })
    }
}

#[cfg(test)]
const ZIPFORMER_EN_20M_BASE: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/resolve/main/";

static ZIPFORMER_EN_20M_FILES: [ModelFile; 4] = [
    ModelFile {
        name: "encoder-epoch-99-avg-1.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/resolve/main/encoder-epoch-99-avg-1.int8.onnx",
        sha256: "3810755ce7c3ab26b42a8bcf39d191308fa27fb0f53358823ba46141d03b7eb3",
        size_bytes: 42_845_182,
    },
    ModelFile {
        name: "decoder-epoch-99-avg-1.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/resolve/main/decoder-epoch-99-avg-1.int8.onnx",
        sha256: "21e2a2acd961b3ac72f55be2f10f1a285e1b0b0ba010d7c0b6eab141411b163c",
        size_bytes: 539_499,
    },
    ModelFile {
        name: "joiner-epoch-99-avg-1.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/resolve/main/joiner-epoch-99-avg-1.int8.onnx",
        sha256: "e085d73b593cf9b0707f370dbd656d58327d3fe36d80d849202ef81df02cb01e",
        size_bytes: 259_572,
    },
    ModelFile {
        name: "tokens.txt",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17/resolve/main/tokens.txt",
        sha256: "49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb",
        size_bytes: 5_048,
    },
];

static PARAKEET_TDT_06B_V2_FILES: [ModelFile; 4] = [
    ModelFile {
        name: "encoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/main/encoder.int8.onnx",
        sha256: "a32b12d17bbbc309d0686fbbcc2987b5e9b8333a7da83fa6b089f0a2acd651ab",
        size_bytes: 652_184_296,
    },
    ModelFile {
        name: "decoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/main/decoder.int8.onnx",
        sha256: "b6bb64963457237b900e496ee9994b59294526439fbcc1fecf705b31a15c6b4e",
        size_bytes: 7_257_753,
    },
    ModelFile {
        name: "joiner.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/main/joiner.int8.onnx",
        sha256: "7946164367946e7f9f29a122407c3252b680dbae9a51343eb2488d057c3c43d2",
        size_bytes: 1_739_080,
    },
    ModelFile {
        name: "tokens.txt",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/main/tokens.txt",
        sha256: "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d",
        size_bytes: 9_384,
    },
];

static CATALOG: [ModelSpec; 2] = [
    ModelSpec {
        id: DEFAULT_MODEL,
        name: "Streaming Zipformer, English, 20M parameters (int8)",
        kind: ModelKind::StreamingTransducer,
        languages: &["en"],
        license: "Apache-2.0",
        files: &ZIPFORMER_EN_20M_FILES,
        encoder: "encoder-epoch-99-avg-1.int8.onnx",
        decoder: "decoder-epoch-99-avg-1.int8.onnx",
        joiner: "joiner-epoch-99-avg-1.int8.onnx",
        tokens: "tokens.txt",
    },
    ModelSpec {
        id: DEFAULT_FINAL_MODEL,
        name: "Parakeet TDT 0.6B v2, English, punctuation and casing (int8)",
        kind: ModelKind::OfflineNemoTransducer,
        languages: &["en"],
        license: "CC-BY-4.0",
        files: &PARAKEET_TDT_06B_V2_FILES,
        encoder: "encoder.int8.onnx",
        decoder: "decoder.int8.onnx",
        joiner: "joiner.int8.onnx",
        tokens: "tokens.txt",
    },
];

pub fn catalog() -> &'static [ModelSpec] {
    &CATALOG
}

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    CATALOG.iter().find(|m| m.id == id)
}

/// Download progress, one report per megabyte and a final `done`.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct Progress {
    pub id: String,
    pub file: String,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub type ProgressSink = Arc<dyn Fn(Progress) + Send + Sync>;

/// Install every missing file of `spec` under `models_dir`. Files already
/// present with the right size are kept. Returns the bytes written.
pub async fn download(
    spec: &ModelSpec,
    models_dir: &Path,
    progress: Option<ProgressSink>,
) -> Result<u64, String> {
    let dir = spec.dir(models_dir);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("create {}: {e}", dir.display()))?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("iii-voice/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let mut written = 0u64;
    for file in spec.files {
        let target = dir.join(file.name);
        if tokio::fs::metadata(&target)
            .await
            .is_ok_and(|m| m.is_file() && m.len() == file.size_bytes)
        {
            continue;
        }
        let result = download_one(&client, spec, file, &target, progress.as_ref()).await;
        if let Err(e) = &result {
            if let Some(sink) = &progress {
                sink(Progress {
                    id: spec.id.to_string(),
                    file: file.name.to_string(),
                    received_bytes: 0,
                    total_bytes: file.size_bytes,
                    done: true,
                    error: Some(e.clone()),
                });
            }
        }
        written += result?;
    }
    Ok(written)
}

async fn download_one(
    client: &reqwest::Client,
    spec: &ModelSpec,
    file: &ModelFile,
    target: &Path,
    progress: Option<&ProgressSink>,
) -> Result<u64, String> {
    let partial = target.with_extension("part");
    let response = client
        .get(file.url)
        .send()
        .await
        .map_err(|e| format!("GET {}: {e}", file.url))?
        .error_for_status()
        .map_err(|e| format!("GET {}: {e}", file.url))?;
    let mut out = tokio::fs::File::create(&partial)
        .await
        .map_err(|e| format!("create {}: {e}", partial.display()))?;
    let mut hasher = Sha256::new();
    let mut received = 0u64;
    let mut stream = response.bytes_stream();
    let mut last_report = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("read {}: {e}", file.url))?;
        hasher.update(&chunk);
        out.write_all(&chunk)
            .await
            .map_err(|e| format!("write {}: {e}", partial.display()))?;
        received += chunk.len() as u64;
        if let Some(sink) = progress {
            if received - last_report >= 1_048_576 || received == file.size_bytes {
                last_report = received;
                sink(Progress {
                    id: spec.id.to_string(),
                    file: file.name.to_string(),
                    received_bytes: received,
                    total_bytes: file.size_bytes,
                    done: false,
                    error: None,
                });
            }
        }
    }
    out.flush().await.map_err(|e| format!("flush: {e}"))?;
    drop(out);
    let digest = format!("{:x}", hasher.finalize());
    if digest != file.sha256 {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(format!(
            "{} failed its checksum (got {digest}, expected {}); the download was discarded",
            file.name, file.sha256
        ));
    }
    if received != file.size_bytes {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(format!(
            "{} is {received} bytes, expected {}",
            file.name, file.size_bytes
        ));
    }
    tokio::fs::rename(&partial, target)
        .await
        .map_err(|e| format!("rename {}: {e}", target.display()))?;
    if let Some(sink) = progress {
        sink(Progress {
            id: spec.id.to_string(),
            file: file.name.to_string(),
            received_bytes: received,
            total_bytes: file.size_bytes,
            done: true,
            error: None,
        });
    }
    Ok(received)
}

/// Remove an installed model's directory.
pub async fn remove(spec: &ModelSpec, models_dir: &Path) -> Result<(), String> {
    let dir = spec.dir(models_dir);
    match tokio::fs::remove_dir_all(&dir).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("remove {}: {e}", dir.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_models_are_in_the_catalog() {
        let spec = find(DEFAULT_MODEL).expect("default streaming model listed");
        assert_eq!(spec.kind, ModelKind::StreamingTransducer);
        assert_eq!(spec.files.len(), 4);
        assert!(spec.size_bytes() > 40_000_000);
        for file in spec.files {
            assert_eq!(file.sha256.len(), 64, "{} hash", file.name);
            assert!(file.url.starts_with(ZIPFORMER_EN_20M_BASE), "{}", file.url);
        }
        let final_spec = find(DEFAULT_FINAL_MODEL).expect("default second-pass model listed");
        assert_eq!(final_spec.kind, ModelKind::OfflineNemoTransducer);
        assert!(final_spec.size_bytes() > 600_000_000);
        for file in final_spec.files {
            assert_eq!(file.sha256.len(), 64, "{} hash", file.name);
        }
    }

    #[test]
    fn installed_means_every_file_at_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let spec = find(DEFAULT_MODEL).unwrap();
        assert!(!spec.is_installed(dir.path()));
        let model_dir = spec.dir(dir.path());
        std::fs::create_dir_all(&model_dir).unwrap();
        for file in spec.files {
            let f = std::fs::File::create(model_dir.join(file.name)).unwrap();
            f.set_len(file.size_bytes).unwrap();
        }
        assert!(spec.is_installed(dir.path()));
        std::fs::File::create(model_dir.join(spec.tokens))
            .unwrap()
            .set_len(1)
            .unwrap();
        assert!(!spec.is_installed(dir.path()));
    }

    #[test]
    fn unknown_ids_are_not_found() {
        assert!(find("whisper-large").is_none());
    }
}
