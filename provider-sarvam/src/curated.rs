//! The hand-maintained Sarvam catalog. Sarvam's API exposes no
//! models-listing endpoint, so this table IS the catalog: the chat models
//! (docs.sarvam.ai getting-started/models, snapshot 2026-09) and the speech
//! models the router's transcribe and speak surfaces reach. A stale row
//! degrades limits, never routing correctness.
use crate::PROVIDER_ID;
use llm_router::types::model::{Model, SpeechModality, SpeechModel};

struct Row {
    id: &'static str,
    display: &'static str,
    context_window: u64,
    max_output_tokens: u64,
}

/// The Sarvam chat lineup. `max_output_tokens` is the Pro plan cap; Starter
/// keys are capped lower by the API and Business keys higher, and the
/// router's per-request budget still applies on top.
const CHAT_ROWS: &[Row] = &[
    Row {
        id: "sarvam-105b",
        display: "Sarvam 105B",
        context_window: 128_000,
        max_output_tokens: 8_192,
    },
    Row {
        id: "sarvam-30b",
        display: "Sarvam 30B",
        context_window: 64_000,
        max_output_tokens: 8_192,
    },
    Row {
        id: "sarvam-m",
        display: "Sarvam-M",
        context_window: 32_768,
        max_output_tokens: 8_192,
    },
];

/// The 22 scheduled Indian languages plus English, as Saaras names them.
pub const SAARAS_LANGUAGES: &[&str] = &[
    "as-IN", "bn-IN", "brx-IN", "doi-IN", "en-IN", "gu-IN", "hi-IN", "kn-IN", "kok-IN", "ks-IN",
    "mai-IN", "ml-IN", "mni-IN", "mr-IN", "ne-IN", "od-IN", "pa-IN", "sa-IN", "sat-IN", "sd-IN",
    "ta-IN", "te-IN", "ur-IN",
];

/// The languages Bulbul speaks.
pub const BULBUL_LANGUAGES: &[&str] = &[
    "bn-IN", "en-IN", "gu-IN", "hi-IN", "kn-IN", "ml-IN", "mr-IN", "od-IN", "pa-IN", "ta-IN",
    "te-IN",
];

struct SpeechRow {
    id: &'static str,
    display: &'static str,
    modality: SpeechModality,
    languages: &'static [&'static str],
}

const SPEECH_ROWS: &[SpeechRow] = &[
    SpeechRow {
        id: "saaras:v3",
        display: "Saaras v3",
        modality: SpeechModality::Stt,
        languages: SAARAS_LANGUAGES,
    },
    SpeechRow {
        id: "saaras:v4",
        display: "Saaras v4",
        modality: SpeechModality::Stt,
        languages: SAARAS_LANGUAGES,
    },
    SpeechRow {
        id: "saarika:v2.5",
        display: "Saarika v2.5 (legacy)",
        modality: SpeechModality::Stt,
        languages: SAARAS_LANGUAGES,
    },
    SpeechRow {
        id: "bulbul:v3",
        display: "Bulbul v3",
        modality: SpeechModality::Tts,
        languages: BULBUL_LANGUAGES,
    },
    SpeechRow {
        id: "bulbul:v2",
        display: "Bulbul v2 (legacy)",
        modality: SpeechModality::Tts,
        languages: BULBUL_LANGUAGES,
    },
];

/// Every model this provider serves: chat first, then speech.
pub fn models() -> Vec<Model> {
    CHAT_ROWS
        .iter()
        .map(to_chat_model)
        .chain(SPEECH_ROWS.iter().map(to_speech_model))
        .collect()
}

pub fn chat_models() -> Vec<Model> {
    CHAT_ROWS.iter().map(to_chat_model).collect()
}

fn to_chat_model(r: &Row) -> Model {
    Model {
        id: r.id.into(),
        provider: PROVIDER_ID.into(),
        display_name: Some(r.display.into()),
        context_window: r.context_window,
        max_output_tokens: r.max_output_tokens,
        input_limit: None,
        supports_thinking: Some(true),
        supports_xhigh: Some(false),
        reasoning_efforts: None,
        supports_tools: Some(true),
        supports_vision: Some(false),
        supports_cache: None,
        supports_structured_output: Some(false),
        thinking_budgets: None,
        pricing: None,
        speech: None,
    }
}

fn to_speech_model(r: &SpeechRow) -> Model {
    Model {
        id: r.id.into(),
        provider: PROVIDER_ID.into(),
        display_name: Some(r.display.into()),
        context_window: 0,
        max_output_tokens: 0,
        input_limit: None,
        supports_thinking: None,
        supports_xhigh: None,
        reasoning_efforts: None,
        supports_tools: None,
        supports_vision: None,
        supports_cache: None,
        supports_structured_output: None,
        thinking_budgets: None,
        pricing: None,
        speech: Some(SpeechModel {
            modality: r.modality,
            languages: r.languages.iter().map(|l| l.to_string()).collect(),
            streaming: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_ids_are_unique_and_owned_by_sarvam() {
        let catalog = models();
        assert!(!catalog.is_empty());
        let ids: HashSet<&str> = catalog.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids.len(), catalog.len(), "duplicate ids in catalog");
        assert!(catalog.iter().all(|m| m.provider == "sarvam"));
    }

    #[test]
    fn flagship_row_matches_docs() {
        let m = models()
            .into_iter()
            .find(|m| m.id == "sarvam-105b")
            .unwrap();
        assert_eq!(m.display_name.as_deref(), Some("Sarvam 105B"));
        assert_eq!(m.context_window, 128_000);
        assert_eq!(m.supports_thinking, Some(true));
        assert_eq!(m.supports_xhigh, Some(false));
        assert_eq!(m.supports_tools, Some(true));
        assert_eq!(m.supports_structured_output, Some(false));
        assert!(m.speech.is_none());
    }

    #[test]
    fn speech_rows_carry_their_family_and_languages() {
        let stt = models().into_iter().find(|m| m.id == "saaras:v3").unwrap();
        assert_eq!(stt.speech_modality(), Some(SpeechModality::Stt));
        assert_eq!(stt.speech.as_ref().unwrap().languages.len(), 23);
        assert_eq!(stt.context_window, 0);
        let tts = models().into_iter().find(|m| m.id == "bulbul:v3").unwrap();
        assert_eq!(tts.speech_modality(), Some(SpeechModality::Tts));
        assert_eq!(tts.speech.as_ref().unwrap().languages.len(), 11);
        assert_eq!(chat_models().len(), 3);
    }
}
