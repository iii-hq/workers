//! Port of laya 0.3.5 `lang.py`: dependency-free script and language detection
//! used to route a state to the checkpoint that can read it, plus the router's
//! typed-decisions workflow signatures. Numbers and thresholds are laya's.
use serde_json::Value;

/// Unicode blocks the English checkpoint (ModernBERT-large, English BPE) cannot read.
const SCRIPTS: &[(&str, &[(u32, u32)])] = &[
    ("greek", &[(0x0370, 0x03FF), (0x1F00, 0x1FFF)]),
    (
        "cyrillic",
        &[(0x0400, 0x052F), (0x2DE0, 0x2DFF), (0xA640, 0xA69F)],
    ),
    ("armenian", &[(0x0530, 0x058F)]),
    ("hebrew", &[(0x0590, 0x05FF)]),
    (
        "arabic",
        &[
            (0x0600, 0x06FF),
            (0x0750, 0x077F),
            (0x08A0, 0x08FF),
            (0xFB50, 0xFDFF),
            (0xFE70, 0xFEFF),
        ],
    ),
    ("devanagari", &[(0x0900, 0x097F), (0xA8E0, 0xA8FF)]),
    ("bengali", &[(0x0980, 0x09FF)]),
    ("gurmukhi", &[(0x0A00, 0x0A7F)]),
    ("gujarati", &[(0x0A80, 0x0AFF)]),
    ("oriya", &[(0x0B00, 0x0B7F)]),
    ("tamil", &[(0x0B80, 0x0BFF)]),
    ("telugu", &[(0x0C00, 0x0C7F)]),
    ("kannada", &[(0x0C80, 0x0CFF)]),
    ("malayalam", &[(0x0D00, 0x0D7F)]),
    ("sinhala", &[(0x0D80, 0x0DFF)]),
    ("thai", &[(0x0E00, 0x0E7F)]),
    ("lao", &[(0x0E80, 0x0EFF)]),
    ("tibetan", &[(0x0F00, 0x0FFF)]),
    ("myanmar", &[(0x1000, 0x109F)]),
    ("georgian", &[(0x10A0, 0x10FF)]),
    ("ethiopic", &[(0x1200, 0x137F)]),
    ("khmer", &[(0x1780, 0x17FF)]),
    (
        "hangul",
        &[(0x1100, 0x11FF), (0x3130, 0x318F), (0xAC00, 0xD7AF)],
    ),
    (
        "kana",
        &[(0x3040, 0x309F), (0x30A0, 0x30FF), (0x31F0, 0x31FF)],
    ),
    (
        "han",
        &[(0x3400, 0x4DBF), (0x4E00, 0x9FFF), (0xF900, 0xFAFF)],
    ),
];

/// Function words per language, in laya's order (ties go to the first).
const STOP: &[(&str, &[&str])] = &[
    (
        "en",
        &[
            "the", "and", "is", "are", "was", "were", "to", "of", "in", "for", "with", "that",
            "this", "it", "you", "have", "has", "not", "but", "on", "at", "be", "as", "from",
            "will", "can", "would", "there", "their", "what", "which", "please", "we", "i",
        ],
    ),
    (
        "fr",
        &[
            "le", "la", "les", "des", "une", "est", "pour", "dans", "que", "qui", "avec", "sur",
            "pas", "plus", "nous", "vous", "être", "cette", "mais", "sont", "ont", "aux", "ce",
        ],
    ),
    (
        "de",
        &[
            "der", "die", "das", "und", "ist", "ein", "eine", "den", "dem", "nicht", "mit", "für",
            "auf", "von", "zu", "sich", "auch", "werden", "wurde", "haben", "sind", "oder", "aber",
        ],
    ),
    (
        "es",
        &[
            "el", "los", "las", "que", "por", "con", "para", "una", "es", "se", "del", "como",
            "pero", "son", "está", "este", "esta", "todo", "más", "muy", "hay", "sus",
        ],
    ),
    (
        "pt",
        &[
            "os", "as", "que", "em", "um", "uma", "para", "com", "não", "é", "se", "do", "da",
            "dos", "das", "mas", "são", "está", "este", "esta", "muito", "pelo", "pela",
        ],
    ),
    (
        "it",
        &[
            "il", "lo", "gli", "che", "di", "per", "con", "non", "è", "si", "del", "della", "sono",
            "questo", "questa", "anche", "come", "più", "sono", "nella", "alla",
        ],
    ),
    (
        "nl",
        &[
            "het", "een", "van", "is", "op", "te", "dat", "niet", "met", "voor", "zijn", "aan",
            "door", "maar", "ook", "worden", "deze", "naar", "wordt",
        ],
    ),
    (
        "ro",
        &[
            "și", "să", "este", "sunt", "care", "pentru", "din", "dar", "după", "până", "fără",
            "ale", "lui", "în", "fost", "acum", "vreau", "trebuie", "foarte", "acest", "această",
            "acesta", "aceasta", "mi", "ți", "vă", "nu",
        ],
    ),
];

/// Letters ordinary English does not use: the signal for Latin-script languages
/// without a stopword list (Polish, Czech, Turkish, Baltic, ...).
const NON_EN_DIACRITICS: &str =
    "àâäãáåçéèêëíìîïñóòôöõøúùûüýÿßæœăâîșțşţąćęłńśźżčďěňřšťůžőűğıāēģīķļņūžđ";
/// Diacritic rate at or above which text is taken as not English.
pub const NON_EN_DIACRITIC_RATE: f64 = 0.02;
const MAX_CHARS: usize = 4000;

#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    /// `latin`, `han`, `devanagari`, ... or `unknown` without letters.
    pub script: &'static str,
    /// Best-effort ISO code for Latin text; None when undecided.
    pub language: Option<&'static str>,
    /// Whether the English checkpoint can be expected to read the state.
    pub is_english: bool,
    /// Fraction of characters that are non-English Latin letters (rounded like laya).
    pub diacritic_rate: f64,
}

fn leaves<'a>(state: &'a Value, depth: usize, out: &mut Vec<&'a str>) {
    if depth > 6 {
        return;
    }
    match state {
        Value::String(s) => out.push(s),
        Value::Object(map) => map.values().for_each(|v| leaves(v, depth + 1, out)),
        Value::Array(items) => items.iter().for_each(|v| leaves(v, depth + 1, out)),
        _ => {}
    }
}

/// The string leaves of a state joined by spaces (keys are ignored: usually English).
pub fn state_text(state: &Value) -> String {
    let mut parts = Vec::new();
    leaves(state, 0, &mut parts);
    parts.join(" ").chars().take(MAX_CHARS).collect()
}

fn script_of(ch: char) -> Option<&'static str> {
    if !ch.is_alphabetic() {
        return None;
    }
    let cp = ch as u32;
    if cp < 0x0250 || (0x1E00..=0x1EFF).contains(&cp) {
        return Some("latin");
    }
    SCRIPTS
        .iter()
        .find(|(_, ranges)| ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp)))
        .map(|(name, _)| *name)
}

/// Dominant script of `text`; laya counts non-Latin scripts first, so a tie
/// with Latin goes to the other script.
pub fn detect_script(text: &str) -> &'static str {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    let mut latin = 0;
    for name in text.chars().filter_map(script_of) {
        if name == "latin" {
            latin += 1;
        } else if let Some(entry) = counts.iter_mut().find(|(n, _)| *n == name) {
            entry.1 += 1;
        } else {
            counts.push((name, 1));
        }
    }
    counts.push(("latin", latin));
    let total: usize = counts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return "unknown";
    }
    counts
        .iter()
        .fold(("unknown", 0usize), |best, &(name, n)| {
            if n > best.1 {
                (name, n)
            } else {
                best
            }
        })
        .0
}

struct LatinProfile {
    language: Option<&'static str>,
    diacritic_rate: f64,
    looks_non_english: bool,
}

/// laya's stopword/diacritic heuristic for Latin text.
fn latin_profile(text: &str) -> LatinProfile {
    let lowered = text.to_lowercase();
    let diacritics = lowered
        .chars()
        .filter(|c| NON_EN_DIACRITICS.contains(*c))
        .count();
    let diacritic_rate = diacritics as f64 / lowered.chars().count().max(1) as f64;
    let looks_non_english = diacritic_rate >= NON_EN_DIACRITIC_RATE;
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.len() < 4 {
        return LatinProfile {
            language: None,
            diacritic_rate,
            looks_non_english,
        };
    }
    let score = |list: &[&str]| words.iter().filter(|w| list.contains(&w.as_str())).count();
    let en = score(STOP[0].1);
    let (mut best_lang, best) = STOP[1..]
        .iter()
        .map(|(lang, list)| (*lang, score(list)))
        .fold(
            (None, 0),
            |acc, (lang, n)| if n > acc.1 { (Some(lang), n) } else { acc },
        );
    if best == 0 {
        best_lang = None;
    }
    // A non-English language needs a clear margin over English function words;
    // with non-English letters present, matching English is enough (two hits).
    let language = match best_lang {
        Some(lang) if best >= 2.max(en + 2) => Some(lang),
        Some(lang) if looks_non_english && best >= 2.max(en) => Some(lang),
        _ if en > 0 && !looks_non_english => Some("en"),
        _ => None,
    };
    LatinProfile {
        language,
        diacritic_rate,
        looks_non_english,
    }
}

/// laya's `analyse`: undecided Latin text with non-English letters is not
/// English; text with no letters, or short plain-ASCII text, is.
pub fn analyse(state: &Value) -> Detection {
    let text = state_text(state);
    let script = detect_script(&text);
    if script == "unknown" {
        return Detection {
            script,
            language: None,
            is_english: true,
            diacritic_rate: 0.0,
        };
    }
    if script != "latin" {
        return Detection {
            script,
            language: None,
            is_english: false,
            diacritic_rate: 0.0,
        };
    }
    let profile = latin_profile(&text);
    let undecided = profile.language.is_none();
    Detection {
        script,
        language: profile.language,
        is_english: profile.language == Some("en") || (undecided && !profile.looks_non_english),
        diacritic_rate: (profile.diacritic_rate * 10_000.0).round_ties_even() / 10_000.0,
    }
}

/// Question-id signatures of the four typed-decisions workflows (laya's router).
pub const TYPED_DECISION_WORKFLOWS: &[(&str, &[&str])] = &[
    (
        "agent_trace_observability",
        &["action", "needs_review", "outcome", "risk", "urgency"],
    ),
    (
        "customer_service",
        &["action", "category", "churn_risk", "needs_human", "urgency"],
    ),
    (
        "invoice_processing",
        &[
            "discrepancy_severity",
            "disposition",
            "duplicate",
            "matches_order",
            "urgency",
        ],
    ),
    (
        "security_incidents",
        &[
            "credential_compromise",
            "disposition",
            "severity",
            "true_positive",
            "urgency",
        ],
    ),
];

/// The workflow whose signature equals the question ids exactly, if any.
pub fn typed_decisions_workflow<'a>(ids: impl Iterator<Item = &'a str>) -> Option<&'static str> {
    let mut ids: Vec<&str> = ids.collect();
    ids.sort_unstable();
    ids.dedup();
    TYPED_DECISION_WORKFLOWS
        .iter()
        .find(|(_, signature)| *signature == ids.as_slice())
        .map(|(name, _)| *name)
}
