//! The built-in model catalog and the download that installs one.
//!
//! Nothing ships inside the binary: the first `voice::dictation::start` or
//! `voice::transcribe` on the `local` backend downloads the configured
//! models into `models_dir`, one file at a time, verifying every file's
//! SHA-256 before it is trusted. A second worker instance racing on the same
//! directory sees the same verified files, because a file is renamed into
//! place only after its hash matched.
//!
//! The catalog includes streaming and offline ONNX transducers, plus GGML
//! weights for a separately installed whisper-cli. GGML models download only
//! when explicitly requested; the whisper.cpp backend never fetches weights.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

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
    /// GGML weights loaded by a separate whisper-cli process.
    WhisperGgml,
    /// Neural text-to-speech loaded by the separately installed Piper process.
    PiperOnnx,
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
    /// Who trained the model; the attribution its license asks for.
    pub author: &'static str,
    /// Page the files come from, with the license text and model card.
    pub source: &'static str,
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

static ZIPFORMER_EN_LARGE_FILES: [ModelFile; 4] = [
    ModelFile {
        name: "encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-2023-06-26/resolve/main/encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
        sha256: "563fde436d16cf7607cf408cd6b30909819d03162652ef389c2450ced3f45ac1",
        size_bytes: 71_083_163,
    },
    ModelFile {
        name: "decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-2023-06-26/resolve/main/decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
        sha256: "98da299f471e38bb4e1a8df579b8cc9122d6039576a77e357b3c60f17dd83b02",
        size_bytes: 1_307_236,
    },
    ModelFile {
        name: "joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-2023-06-26/resolve/main/joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
        sha256: "d944208d660d67c8d72cd2acaeac971fa5ceb8c80e76c1968148846fedd6e297",
        size_bytes: 259_335,
    },
    ModelFile {
        name: "tokens.txt",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-2023-06-26/resolve/main/tokens.txt",
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

static CATALOG: [ModelSpec; 9] = [
    ModelSpec {
        id: "piper-pt-br-faber-medium",
        name: "Piper Faber · Brazilian Portuguese (neural)",
        kind: ModelKind::PiperOnnx,
        languages: &["pt-BR"],
        license: "CC0-1.0 (dataset; see model card)",
        author: "OHF-Voice / Piper contributors",
        source:
            "https://huggingface.co/rhasspy/piper-voices/blob/main/pt/pt_BR/faber/medium/MODEL_CARD",
        files: &PIPER_FABER_FILES,
        encoder: "",
        decoder: "",
        joiner: "",
        tokens: "",
    },
    ModelSpec {
        id: DEFAULT_MODEL,
        name: "Zipformer streaming 20M",
        kind: ModelKind::StreamingTransducer,
        languages: &["en"],
        license: "Apache-2.0",
        author: "k2-fsa (icefall)",
        source:
            "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17",
        files: &ZIPFORMER_EN_20M_FILES,
        encoder: "encoder-epoch-99-avg-1.int8.onnx",
        decoder: "decoder-epoch-99-avg-1.int8.onnx",
        joiner: "joiner-epoch-99-avg-1.int8.onnx",
        tokens: "tokens.txt",
    },
    ModelSpec {
        id: "zipformer-en-large",
        name: "Zipformer streaming large",
        kind: ModelKind::StreamingTransducer,
        languages: &["en"],
        license: "Apache-2.0",
        author: "k2-fsa (icefall)",
        source: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-2023-06-26",
        files: &ZIPFORMER_EN_LARGE_FILES,
        encoder: "encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
        decoder: "decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
        joiner: "joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx",
        tokens: "tokens.txt",
    },
    ModelSpec {
        id: DEFAULT_FINAL_MODEL,
        name: "Parakeet TDT 0.6B v2",
        kind: ModelKind::OfflineNemoTransducer,
        languages: &["en"],
        license: "CC-BY-4.0",
        author: "NVIDIA",
        source: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
        files: &PARAKEET_TDT_06B_V2_FILES,
        encoder: "encoder.int8.onnx",
        decoder: "decoder.int8.onnx",
        joiner: "joiner.int8.onnx",
        tokens: "tokens.txt",
    },
    whisper_spec("whisper-tiny", "Whisper tiny (multilingual)", &WHISPER_TINY),
    whisper_spec("whisper-base", "Whisper base (multilingual)", &WHISPER_BASE),
    whisper_spec(
        "whisper-small",
        "Whisper small (multilingual)",
        &WHISPER_SMALL,
    ),
    whisper_spec(
        "whisper-medium",
        "Whisper medium (multilingual)",
        &WHISPER_MEDIUM,
    ),
    whisper_spec(
        "whisper-large-v3-turbo",
        "Whisper large v3 turbo (multilingual)",
        &WHISPER_TURBO,
    ),
];

static PIPER_FABER_FILES: [ModelFile; 3] = [
    ModelFile {
        name: "pt_BR-faber-medium.onnx",
        url: "https://huggingface.co/rhasspy/piper-voices/resolve/main/pt/pt_BR/faber/medium/pt_BR-faber-medium.onnx",
        sha256: "858555e3a064209c57088fe6bd70c4c3dc54d03eaa00c45d5ecaf43a33f95aa7",
        size_bytes: 63_201_294,
    },
    ModelFile {
        name: "pt_BR-faber-medium.onnx.json",
        url: "https://huggingface.co/rhasspy/piper-voices/resolve/main/pt/pt_BR/faber/medium/pt_BR-faber-medium.onnx.json",
        sha256: "7e694de195ae3fc36dd732c445eb04fb49b649854893cb5506b978f0d50a1d6f",
        size_bytes: 4855,
    },
    ModelFile {
        name: "MODEL_CARD",
        url: "https://huggingface.co/rhasspy/piper-voices/resolve/main/pt/pt_BR/faber/medium/MODEL_CARD",
        sha256: "01f1a5bcfd0538782726059ae407eae9c2dd1b5b35f7298fd53a68de91afe563",
        size_bytes: 279,
    },
];

// Sizes and SHA-256 digests from ggerganov/whisper.cpp's Hugging Face LFS metadata.
static WHISPER_TINY: [ModelFile; 1] = [ModelFile {
    name: "ggml-tiny.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
    sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    size_bytes: 77_691_713,
}];
static WHISPER_BASE: [ModelFile; 1] = [ModelFile {
    name: "ggml-base.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
    sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    size_bytes: 147_951_465,
}];
static WHISPER_SMALL: [ModelFile; 1] = [ModelFile {
    name: "ggml-small.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
    sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    size_bytes: 487_601_967,
}];
static WHISPER_MEDIUM: [ModelFile; 1] = [ModelFile {
    name: "ggml-medium.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
    sha256: "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208",
    size_bytes: 1_533_763_059,
}];
static WHISPER_TURBO: [ModelFile; 1] = [ModelFile {
    name: "ggml-large-v3-turbo.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
    sha256: "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69",
    size_bytes: 1_624_555_275,
}];

const fn whisper_spec(
    id: &'static str,
    name: &'static str,
    files: &'static [ModelFile],
) -> ModelSpec {
    ModelSpec {
        id,
        name,
        kind: ModelKind::WhisperGgml,
        languages: &["multilingual"],
        license: "MIT",
        author: "OpenAI (GGML conversion by whisper.cpp)",
        source: "https://huggingface.co/ggerganov/whisper.cpp",
        files,
        // Only ONNX transducers use these fields; whisper-cli takes the GGML file.
        encoder: "",
        decoder: "",
        joiner: "",
        tokens: "",
    }
}

// Preserve existing model ids and the installed Faber directory. The remaining
// Piper voices are borrowed from a finite snapshot; no runtime network lookup.
static ALL_MODELS: LazyLock<Vec<ModelSpec>> = LazyLock::new(|| {
    let mut models = CATALOG.to_vec();
    models.extend(
        crate::piper_catalog::catalog()
            .iter()
            .filter(|m| !CATALOG.iter().any(|existing| existing.id == m.id))
            .cloned(),
    );
    models
});

pub fn catalog() -> &'static [ModelSpec] {
    &ALL_MODELS
}

pub fn find(id: &str) -> Option<&'static ModelSpec> {
    catalog().iter().find(|m| m.id == id)
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

/// Stream one file into `partial`, hashing as it lands. Returns the byte
/// count and hex digest; the caller decides what to do with the file.
async fn stream_to_partial(
    client: &reqwest::Client,
    spec: &ModelSpec,
    file: &ModelFile,
    partial: &Path,
    progress: Option<&ProgressSink>,
) -> Result<(u64, String), String> {
    let response = client
        .get(file.url)
        .send()
        .await
        .map_err(|e| format!("GET {}: {e}", file.url))?
        .error_for_status()
        .map_err(|e| format!("GET {}: {e}", file.url))?;
    let mut out = tokio::fs::File::create(partial)
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
    Ok((received, format!("{:x}", hasher.finalize())))
}

async fn download_one(
    client: &reqwest::Client,
    spec: &ModelSpec,
    file: &ModelFile,
    target: &Path,
    progress: Option<&ProgressSink>,
) -> Result<u64, String> {
    let partial = target.with_extension(format!("{}.part", uuid::Uuid::new_v4().simple()));
    let (received, digest) = match stream_to_partial(client, spec, file, &partial, progress).await {
        Ok(done) => done,
        Err(e) => {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(e);
        }
    };
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
    if let Err(e) = tokio::fs::rename(&partial, target).await {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(format!("rename {}: {e}", target.display()));
    }
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
    fn whisper_catalog_has_only_pinned_single_file_models() {
        let whisper: Vec<_> = catalog()
            .iter()
            .filter(|m| m.kind == ModelKind::WhisperGgml)
            .collect();
        assert_eq!(whisper.len(), 5);
        for model in whisper {
            assert_eq!(model.files.len(), 1);
            let file = &model.files[0];
            assert!(file.name.starts_with("ggml-") && file.name.ends_with(".bin"));
            assert!(file
                .url
                .starts_with("https://huggingface.co/ggerganov/whisper.cpp/"));
            assert_eq!(file.sha256.len(), 64);
            assert!(file.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(file.size_bytes > 70_000_000);
        }
    }

    #[tokio::test]
    async fn removing_whisper_keeps_other_models_and_custom_files() {
        let dir = tempfile::tempdir().unwrap();
        let spec = find("whisper-tiny").unwrap();
        std::fs::create_dir_all(spec.dir(dir.path())).unwrap();
        std::fs::write(spec.dir(dir.path()).join(spec.files[0].name), b"weights").unwrap();
        let custom = dir.path().join("custom.bin");
        std::fs::write(&custom, b"keep").unwrap();
        remove(spec, dir.path()).await.unwrap();
        assert!(!spec.dir(dir.path()).exists());
        assert_eq!(std::fs::read(custom).unwrap(), b"keep");
    }

    #[test]
    fn unknown_ids_are_not_found() {
        assert!(find("whisper-large").is_none());
    }
}
