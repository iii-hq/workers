//! Borrowed views of a finite, embedded upstream catalog snapshot.
//! Listing, selecting and validating a voice never require network access.
use crate::models::{ModelFile, ModelKind, ModelSpec};
use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Deserialize)]
struct Snapshot {
    voices: Vec<Voice>,
}
#[derive(Deserialize)]
struct Voice {
    id: String,
    name: String,
    languages: Vec<String>,
    license: String,
    author: String,
    source: String,
    files: Vec<File>,
}
#[derive(Deserialize)]
struct File {
    name: String,
    url: String,
    sha256: String,
    size_bytes: u64,
}

static DATA: LazyLock<Snapshot> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../catalog/piper-catalog.json"))
        .expect("checked embedded Piper catalog")
});

struct Resources {
    languages: Vec<&'static str>,
    files: Vec<ModelFile>,
}
static RESOURCES: LazyLock<Vec<Resources>> = LazyLock::new(|| {
    DATA.voices
        .iter()
        .map(|v| Resources {
            languages: v.languages.iter().map(String::as_str).collect(),
            files: v
                .files
                .iter()
                .map(|f| ModelFile {
                    name: &f.name,
                    url: &f.url,
                    sha256: &f.sha256,
                    size_bytes: f.size_bytes,
                })
                .collect(),
        })
        .collect()
});
static CATALOG: LazyLock<Vec<ModelSpec>> = LazyLock::new(|| {
    DATA.voices
        .iter()
        .zip(RESOURCES.iter())
        .map(|(v, r)| ModelSpec {
            id: &v.id,
            name: &v.name,
            kind: ModelKind::PiperOnnx,
            languages: &r.languages,
            license: &v.license,
            author: &v.author,
            source: &v.source,
            files: &r.files,
            encoder: "",
            decoder: "",
            joiner: "",
            tokens: "",
        })
        .collect()
});

pub fn catalog() -> &'static [ModelSpec] {
    &CATALOG
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn snapshot_covers_languages_and_preserves_unique_safe_ids() {
        let models = catalog();
        assert_eq!(models.len(), 176);
        let mut ids = HashSet::new();
        for model in models {
            assert!(ids.insert(model.id));
            assert!(model.id.starts_with("piper-"));
            assert!(
                !model.id.contains('/') && !model.id.contains("..") && !model.id.contains('\\')
            );
            assert!(!model.languages.is_empty());
            assert!(model.files[0].name.ends_with(".onnx"));
            assert!(model.files.iter().any(|f| f.name.ends_with(".onnx.json")));
            for file in model.files {
                assert!(!file.name.contains('/') && !file.name.contains(".."));
                assert!(file.url.starts_with("https://huggingface.co/rhasspy/piper-voices/resolve/1162a9173d0ce503555aed757976b7a9912eae4c/"));
                assert_eq!(file.sha256.len(), 64);
                assert!(file.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
                assert!(file.size_bytes > 0);
            }
        }
        for language in [
            "pt-BR", "pt-PT", "en-US", "en-GB", "es-ES", "fr-FR", "de-DE", "ja-JA", "zh-CN",
        ] {
            assert!(
                models.iter().any(|m| m.languages.contains(&language)),
                "missing {language}"
            );
        }
    }

    #[test]
    fn every_piper_voice_is_selectable_in_the_worker_catalog() {
        for model in catalog() {
            let found = crate::models::find(model.id).expect("downloadable voice");
            assert_eq!(found.kind, ModelKind::PiperOnnx);
            assert_eq!(found.files[0].sha256, model.files[0].sha256);
        }
        assert_eq!(
            crate::models::catalog()
                .iter()
                .filter(|m| m.kind == ModelKind::PiperOnnx)
                .count(),
            176
        );
    }
}
