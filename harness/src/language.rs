//! The session's response language ([`ResponseLanguage`] on
//! `TurnOptions::response_language`).
//!
//! Left to pick the language of user-facing text itself, a model guesses: one
//! labelled every `agent_trigger` call of an English session in Spanish,
//! inferred from the username in the working directory. So the harness
//! detects the language once — from the user's first message with enough
//! prose — pins it for the session, and names it to the model in the runtime
//! context ([`runtime_line`]) and the `agent_trigger` schema
//! ([`description_field_text`]). A later explicit "answer in Portuguese" is
//! the prompt's rule to honor; nothing here re-detects.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};
use whatlang::{Lang, Script};

use crate::types::content::ContentBlock;
use crate::types::message::AgentMessage;
use crate::types::turn::ResponseLanguage;

/// Session-metadata key mirroring the pinned language, so it outlives the turn
/// record and clients can read it.
pub const METADATA_KEY: &str = "response_language";

/// Minimum prose for a verdict in space-delimited scripts: "hello", "ok" or
/// "fix the bug" leave the language unset until a longer message arrives.
const MIN_WORDS: usize = 3;
const MIN_LETTERS: usize = 12;
/// Scripts written without spaces (Han, kana, Thai…) pack a word per letter
/// or two, so they need fewer letters and no word count.
const MIN_LETTERS_UNSPACED: usize = 6;
/// Detector confidence (0..1) a verdict must reach. whatlang scales it by
/// text length, so short prompts rarely reach it: a wrong pin costs a whole
/// session, an unset one only waits for the next message. Measured on ~5k
/// short English doc sentences, 0.8 kept false verdicts to about 1 in 1000
/// (0.5 let through 1 in 300; whatlang's own `is_reliable` bar is 0.9).
const MIN_CONFIDENCE: f64 = 0.8;

/// Inline spans that are not the user's own prose: code, `#file(…)` /
/// `@fn(…)` mentions, double-quoted text and URLs.
static NON_PROSE_SPANS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"`+[^`]+`+",
        r"|[#@][A-Za-z][\w-]*\([^)\n]*\)",
        r#"|"[^"\n]*""#,
        r"|“[^”\n]*”",
        r"|„[^“”\n]*[“”]",
        r"|«[^»\n]*»",
        r"|(?i:https?://|www\.)\S+",
    ))
    .expect("static regex compiles")
});

/// The language of `message`'s prose, or `None` when it is not a user message
/// or its prose is too short or too mixed to tell.
pub fn detect(message: &AgentMessage) -> Option<ResponseLanguage> {
    let AgentMessage::User(user) = message else {
        return None;
    };
    detect_text(&ContentBlock::join_text(&user.content))
}

/// [`detect`] over raw text: strip what is not prose, then detect with
/// whatlang under the length and confidence thresholds.
pub fn detect_text(text: &str) -> Option<ResponseLanguage> {
    let prose = prose(text);
    let letters = prose.chars().filter(|c| c.is_alphabetic()).count();
    let words = prose
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .count();
    let info = whatlang::detect(&prose)?;
    let enough = if is_unspaced(info.script()) {
        letters >= MIN_LETTERS_UNSPACED
    } else {
        words >= MIN_WORDS && letters >= MIN_LETTERS
    };
    if !enough || info.confidence() < MIN_CONFIDENCE {
        return None;
    }
    Some(ResponseLanguage {
        code: info.lang().code().to_string(),
        name: english_name(info.lang()).to_string(),
    })
}

/// The language a session's metadata pins, when it holds a well-formed one.
pub fn from_metadata(metadata: &Map<String, Value>) -> Option<ResponseLanguage> {
    let stored: ResponseLanguage =
        serde_json::from_value(metadata.get(METADATA_KEY)?.clone()).ok()?;
    (!stored.code.trim().is_empty() && !stored.name.trim().is_empty()).then_some(stored)
}

/// The metadata value for `language` (the [`METADATA_KEY`] entry).
pub fn metadata_value(language: &ResponseLanguage) -> Value {
    serde_json::to_value(language).unwrap_or(Value::Null)
}

/// The runtime-context line naming the pinned response language to the
/// model. Absent while none is pinned: the prompt's "if none is named" rule
/// covers that, and an unpinned session's runtime context stays unchanged.
pub fn runtime_line(language: &ResponseLanguage) -> String {
    format!(
        "Response language: {} (from the user's first message). Write every user-facing text in \
         it — progress messages, every agent_trigger `description`, harness::ask questions and \
         the final answer — unless the user explicitly asks for another language. Never infer it \
         from names, paths or the locale.",
        language.name
    )
}

/// The `agent_trigger` `description` field's schema text for `language`.
pub fn description_field_text(language: Option<&ResponseLanguage>) -> String {
    let language = match language {
        Some(language) => format!("written in {}", language.name),
        None => "in the response language named in the session context (else the language of \
                 the user's own prose); never inferred from names, paths or locale"
            .to_string(),
    };
    format!(
        "Short user-facing description of the action, {language}. Describe the work, not the \
         function id."
    )
}

/// `text` without what is not the user's own prose: fenced code blocks,
/// blockquotes, inline code, mentions, quotes, URLs and path- or
/// identifier-like tokens.
fn prose(text: &str) -> String {
    let mut kept = Vec::new();
    let mut fence: Option<&str> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = ["```", "~~~"].into_iter().find(|f| trimmed.starts_with(f)) {
            fence = Some(open);
            continue;
        }
        if !trimmed.starts_with('>') {
            kept.push(line);
        }
    }
    let joined = kept.join("\n");
    NON_PROSE_SPANS
        .replace_all(&joined, " ")
        .split_whitespace()
        .filter(|word| !is_code_like(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A path, identifier, handle or markup token — never evidence of language.
fn is_code_like(word: &str) -> bool {
    const MARKERS: &[char] = &[
        '/', '\\', '_', '=', '<', '>', '{', '}', '[', ']', '|', '$', '@', '#', '`', '~',
    ];
    if word.contains(MARKERS) || word.contains("::") {
        return true;
    }
    // `main.rs`, `foo.bar()` — a dot between letters.
    let chars: Vec<char> = word.chars().collect();
    let dotted = chars
        .windows(3)
        .any(|w| w[1] == '.' && w[0].is_alphanumeric() && w[2].is_alphanumeric());
    // `camelCase` / `useState`.
    let camel = chars
        .windows(2)
        .any(|w| w[0].is_lowercase() && w[1].is_uppercase());
    dotted || camel
}

fn is_unspaced(script: Script) -> bool {
    matches!(
        script,
        Script::Mandarin
            | Script::Hiragana
            | Script::Katakana
            | Script::Thai
            | Script::Khmer
            | Script::Myanmar
    )
}

/// The English name a prompt reads naturally; whatlang's, except where it
/// names a variety rather than the language.
fn english_name(lang: Lang) -> &'static str {
    match lang {
        Lang::Cmn => "Chinese",
        Lang::Nob => "Norwegian",
        Lang::Slv => "Slovenian",
        Lang::Sin => "Sinhala",
        other => other.eng_name(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn name(text: &str) -> Option<String> {
        detect_text(text).map(|language| language.name)
    }

    #[test]
    fn detects_the_language_of_ordinary_prompts() {
        assert_eq!(
            name("Create a small app to track my daily tasks, with a notes field on each task."),
            Some("English".into())
        );
        assert_eq!(
            name(
                "Crie um aplicativo para controlar minhas tarefas diárias, com um campo de notas."
            ),
            Some("Portuguese".into())
        );
        assert_eq!(
            name(
                "Crea una aplicación para gestionar mis tareas diarias. Cada tarea debe tener \
                 un campo de notas y una fecha límite."
            ),
            Some("Spanish".into())
        );
        assert_eq!(
            name("Erstelle eine kleine App, um meine täglichen Aufgaben zu verwalten."),
            Some("German".into())
        );
        assert_eq!(
            name("帮我创建一个管理日常任务的应用"),
            Some("Chinese".into())
        );
        let english =
            detect_text("Explain this project and help me decide what to work on next.").unwrap();
        assert_eq!(english.code, "eng");
        assert_eq!(english.name, "English");
    }

    #[test]
    fn short_or_empty_prose_stays_undetected() {
        // Too short to tell, or too weak a verdict (whatlang reads the first
        // as Portuguese at ~0.3 confidence): left unset, never guessed.
        for text in [
            "Create a todo app with a notes field",
            "hello",
            "ok",
            "hola",
            "obrigado!",
            "fix the bug",
            "",
            "   ",
            "👍",
        ] {
            assert_eq!(detect_text(text), None, "{text:?}");
        }
    }

    #[test]
    fn code_mentions_quotes_urls_and_paths_are_not_evidence() {
        // Only the code, mention, quote, URL and path are Spanish/Portuguese.
        let text = "Please fix this function so the tests pass again\n\
                    ```rust\nfn añadir_campo() { /* añadir campo notas al modelo */ }\n```\n\
                    > Añadir campo notes al modelo y comprobar contenedores existentes\n\
                    See #file(src/añadir.rs:1-9) and @fn(coder::read-file), \
                    \"comprobar contenedores existentes\", https://ejemplo.es/añadir-campo \
                    and /home/sergio/proyectos/aplicación `añadir campo`";
        assert_eq!(name(text), Some("English".into()));
        // Nothing but non-prose left: undetected, not guessed.
        assert_eq!(
            name(
                "`comprobar contenedores existentes` #file(src/añadir_campo.rs) https://ejemplo.es"
            ),
            None
        );
        assert_eq!(name("/home/sergio/Documents/workspaces/iii"), None);
    }

    #[test]
    fn non_user_messages_are_never_detected() {
        let custom: AgentMessage = serde_json::from_value(json!({
            "role": "custom",
            "custom_type": "notification",
            "content": [{ "type": "text", "text": "Explain this project and help me decide what to work on next." }],
            "timestamp": 1
        }))
        .unwrap();
        assert_eq!(detect(&custom), None);
        let user: AgentMessage = serde_json::from_value(json!({
            "role": "user",
            "content": [{ "type": "text", "text": "Explain this project and help me decide what to work on next." }],
            "timestamp": 1
        }))
        .unwrap();
        assert_eq!(detect(&user).map(|l| l.name), Some("English".into()));
    }

    #[test]
    fn metadata_round_trips_and_rejects_malformed_values() {
        let english = ResponseLanguage {
            code: "eng".into(),
            name: "English".into(),
        };
        let mut metadata = Map::new();
        assert_eq!(from_metadata(&metadata), None);
        metadata.insert(METADATA_KEY.into(), metadata_value(&english));
        assert_eq!(from_metadata(&metadata), Some(english));
        for bad in [
            json!("English"),
            json!({ "code": "", "name": "English" }),
            json!(null),
        ] {
            metadata.insert(METADATA_KEY.into(), bad.clone());
            assert_eq!(from_metadata(&metadata), None, "{bad}");
        }
    }

    #[test]
    fn the_model_is_told_the_language_by_name() {
        let english = ResponseLanguage {
            code: "eng".into(),
            name: "English".into(),
        };
        let line = runtime_line(&english);
        assert!(line.starts_with("Response language: English (from the user's first message)."));
        assert!(line.contains("every agent_trigger `description`"));
        assert!(line.contains("unless the user explicitly asks for another language"));
        assert!(line.ends_with("Never infer it from names, paths or the locale."));

        assert_eq!(
            description_field_text(Some(&english)),
            "Short user-facing description of the action, written in English. Describe the \
             work, not the function id."
        );
        assert!(description_field_text(None).contains(
            "in the response language named in the session context (else the language of the \
             user's own prose); never inferred from names, paths or locale"
        ));
    }
}
