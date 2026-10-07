//! Port of laya 0.3.28 `lang.py`: dependency-free script and language detection
//! used to route a state to the checkpoint that can read it, plus the router's
//! typed-decisions workflow signatures. Word lists, numbers and thresholds are
//! laya's, and `tests/fixtures/lang.json` holds laya's own answers. Character
//! classes are Python's: `str.isalpha` is general category L and `re`'s `\w`
//! is L, N and `_`, which the regex crate carries for the same Unicode version
//! as Python 3.14 (16.0). `unicodedata.combining` and case come from tables of
//! Unicode 17.0 (ICU4X and the standard library); for what this module asks
//! they differ from Python's only on characters Unicode 16 leaves unassigned,
//! and on `ʕ` (see `is_lower`). Lengths and slices count characters, as
//! Python does.
use icu_properties::props::CanonicalCombiningClass;
use icu_properties::CodePointMapData;
use regex::{Captures, Regex};
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

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
/// Latin-script languages overlap heavily (`la`, `un`, `e`, `que`), so a word
/// several lists hold names none of them (`shared`). The Romance lists carry the
/// unaccented spellings too: a state that lost its accents to an ASCII-normalising
/// mail or ticket pipeline keeps no diacritic rate, and those words are the only
/// evidence left.
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
            "et", "du", "au", "ou", "je", "tu", "il", "elle", "ils", "elles", "mon", "ton", "ma",
            "ta", "sa", "mes", "tes", "ses", "ces", "deux", "trois", "très", "bien", "tout",
            "tous", "toute", "fait", "veux", "veut", "peux", "peut", "dois", "doit", "merci",
            "bonjour", "jour", "jours", "mois", "fois", "quand", "comment", "pourquoi", "alors",
            "donc",
        ],
    ),
    // `in` and `was` on purpose: counted for English alone, they outvoted short German.
    (
        "de",
        &[
            "der", "die", "das", "und", "ist", "ein", "eine", "den", "dem", "nicht", "mit", "für",
            "auf", "von", "zu", "sich", "auch", "werden", "wurde", "haben", "sind", "oder", "aber",
            "ich", "wir", "mir", "mich", "dir", "dich", "uns", "mein", "meine", "meinen", "meinem",
            "meiner", "diese", "dieser", "diesen", "dieses", "einen", "einem", "einer", "wie",
            "wo", "wann", "welche", "im", "zum", "zur", "aus", "bei", "nach", "noch", "bitte",
            "heute", "jetzt", "kann", "kannst", "habe", "gibt", "wird", "in", "was",
        ],
    ),
    // `de` and `en` stay out: common English tokens (`de facto`, `en route`).
    (
        "es",
        &[
            "el", "los", "las", "que", "por", "con", "para", "una", "es", "se", "del", "como",
            "pero", "son", "está", "este", "esta", "todo", "más", "muy", "hay", "sus", "la", "un",
            "y", "al", "lo", "le", "les", "su", "mi", "tu", "nos", "ni", "dos", "tres", "fue",
            "fueron", "ser", "tiene", "tienen", "tengo", "puede", "pueden", "quiero", "necesito",
            "hemos", "han", "sobre", "entre", "cuando", "donde", "porque", "aunque", "también",
            "ya", "eso", "esto", "esa", "ese", "nada", "algo", "aquí", "hoy", "gracias",
        ],
    ),
    // `no` stays out (English). Brazilian support words, the unaccented spellings a
    // stripped state keeps (`voce`, `nao`), chat forms (`vc`, `pra`) and the words a bug
    // report keeps once its jargon is English (`deu`, `depois`); `ate`, `bom`, `sim` and
    // `cade` stay out (English tokens too).
    (
        "pt",
        &[
            "os", "as", "que", "em", "um", "uma", "para", "com", "não", "é", "se", "do", "da",
            "dos", "das", "mas", "são", "está", "este", "esta", "muito", "pelo", "pela", "o", "e",
            "na", "nas", "nos", "ao", "aos", "por", "foi", "era", "ser", "sou", "tem", "tenho",
            "pode", "podem", "quero", "preciso", "eu", "meu", "minha", "seu", "sua", "isso",
            "isto", "aqui", "ali", "como", "quando", "onde", "porque", "mais", "já", "ainda",
            "agora", "hoje", "ontem", "dois", "três", "tudo", "nada", "obrigado", "olá", "você",
            "vocês", "voce", "voces", "vc", "vcs", "nao", "sao", "ja", "até", "tá", "pra",
            "gostaria", "obrigada", "também", "tambem", "estou", "estamos", "meus", "minhas",
            "nosso", "nossa", "consigo", "cadê", "boa", "tarde", "noite", "depois", "antes",
            "então", "entao", "ninguém", "ninguem", "alguém", "alguem", "nenhum", "nenhuma",
            "estava", "ficou", "fiz", "deu",
        ],
    ),
    // The articulated prepositions (`nel`, `dei`) are Italian-only words, so a state of
    // shared articles (`la fattura`) can still be named.
    (
        "it",
        &[
            "il", "lo", "gli", "che", "di", "per", "con", "non", "è", "si", "del", "della", "sono",
            "questo", "questa", "anche", "come", "più", "nella", "alla", "la", "le", "un", "uno",
            "una", "e", "ed", "o", "da", "su", "tra", "fra", "mi", "ci", "ne", "ho", "hai", "ha",
            "abbiamo", "avete", "hanno", "era", "stato", "stata", "devo", "deve", "devono",
            "voglio", "vorrei", "mio", "mia", "tuo", "sua", "quando", "dove", "perche", "molto",
            "poco", "sempre", "mai", "già", "ancora", "adesso", "oggi", "ieri", "grazie", "ciao",
            "scusa", "nel", "nell", "negli", "sul", "sulla", "sulle", "dal", "dalla", "dallo",
            "dagli", "dei", "delle", "dello", "degli", "agli", "alle", "col",
        ],
    ),
    (
        "nl",
        &[
            "het", "een", "van", "is", "op", "te", "dat", "niet", "met", "voor", "zijn", "aan",
            "door", "maar", "ook", "worden", "deze", "naar", "wordt",
        ],
    ),
    // Several overlap with English or German (`i`, `kan`, `har`); the ASCII spellings
    // survive a ticket pipeline that strips Swedish letters.
    (
        "sv",
        &[
            "jag",
            "är",
            "och",
            "inte",
            "att",
            "från",
            "till",
            "behöver",
            "får",
            "skulle",
            "ska",
            "vill",
            "måste",
            "också",
            "dessa",
            "detta",
            "säger",
            "upp",
            "utan",
            "mitt",
            "min",
            "om",
            "kommer",
            "här",
            "två",
            "vi",
            "nästa",
            "gör",
            "göra",
            "hjälp",
            "hjälpa",
            "mig",
            "återbetalning",
            "återbetala",
            "faktura",
            "gång",
            "gånger",
            "hittar",
            "inställningen",
            "inställningarna",
            "lösenord",
            "när",
            "öppnar",
            "spårningen",
            "aterbetalning",
            "aterbetala",
            "behover",
            "fel",
            "ganger",
            "hjalp",
            "hjalpa",
            "installningen",
            "installningarna",
            "kraschar",
            "kvittot",
            "losenord",
            "nar",
            "oppnar",
            "paket",
            "skicka",
            "sparningen",
            "tva",
            "uppdaterats",
            "blivit",
            "debiterade",
            "appen",
        ],
    ),
    // Words its Romance neighbours do not share (`la`, `o`, `un`, `de`, `pe`, `ca` stay
    // out), so `ro` cannot take their states; the diacritic rate carries the rest.
    (
        "ro",
        &[
            "și", "să", "este", "sunt", "care", "pentru", "din", "dar", "după", "până", "fără",
            "ale", "lui", "în", "fost", "acum", "vreau", "trebuie", "foarte", "acest", "această",
            "acesta", "aceasta", "mi", "ți", "vă", "nu",
        ],
    ),
    // Romanized Bangla, typed without diacritics. Words that are also English (`ache`,
    // `are`, `to`, `eta`) or that another list holds stay out.
    (
        "bn",
        &[
            "ami",
            "amar",
            "amake",
            "amra",
            "amader",
            "apni",
            "apnar",
            "apnake",
            "apnara",
            "tumi",
            "tomar",
            "tomake",
            "tomra",
            "tader",
            "ota",
            "eita",
            "oita",
            "ekta",
            "ei",
            "oi",
            "ki",
            "keno",
            "kivabe",
            "kibhabe",
            "kothay",
            "kokhon",
            "kobe",
            "koto",
            "kintu",
            "jodi",
            "tahole",
            "ar",
            "theke",
            "jonno",
            "sathe",
            "shathe",
            "diye",
            "niye",
            "moddhe",
            "kore",
            "korte",
            "korchi",
            "korsi",
            "korbo",
            "korechi",
            "koreche",
            "korun",
            "koren",
            "korlam",
            "hobe",
            "hoyeche",
            "hoise",
            "hocche",
            "hoyni",
            "chai",
            "chaina",
            "lagbe",
            "parchi",
            "parbo",
            "parchina",
            "peyechi",
            "paini",
            "dite",
            "dilam",
            "diyechi",
            "nai",
            "khub",
            "onek",
            "ekhon",
            "akhon",
            "ekhono",
            "abar",
            "ekbar",
            "duibar",
            "ajke",
            "kalke",
            "taka",
            "bhalo",
            "valo",
            "kharap",
            "shomossa",
            "somossa",
            "dhonnobad",
            "bhai",
            "shob",
            "keu",
            "kichu",
            "bolte",
            "bolun",
            "parben",
            "asbe",
            "jabe",
            "pabo",
            "ferot",
            "dorkar",
            "hoye",
            "geche",
            "gese",
        ],
    ),
    // `her`, `ne`, `men`, `de` and `ki` collide with English, French, Romanian or
    // Bangla and stay out.
    (
        "az",
        &[
            "və", "ve", "bir", "bu", "üçün", "ucun", "ilə", "ile", "olan", "olub", "olmasa", "var",
            "yox", "yoxdur", "mən", "sən", "biz", "siz", "onlar", "daha", "çox", "cox", "hər",
            "nə", "kimi", "görə", "sonra", "əgər", "eger", "deyil", "lakin", "amma", "ancaq",
            "artıq", "artiq", "də", "isə", "həm", "yalnız", "yalniz",
        ],
    ),
];
/// Danish is not listed, but these common words of its are also in the Swedish
/// list: alone they name no language, though they still count as hits.
const NORDIC_OVERLAP_WORDS: &[&str] = &[
    "hej", "ja", "nej", "jo", "tack", "mig", "min", "om", "kommer", "får", "skulle", "vi",
];
/// Specific enough to name a two- or three-word support fragment Swedish
/// (generic `fel`, `hjälp` or `appen` are not), with their ASCII spellings.
const SHORT_SWEDISH_WORDS: &[&str] = &[
    "åtkomst",
    "atkomst",
    "lösenord",
    "losenord",
    "fakturan",
    "betalningen",
    "inloggningen",
    "glömt",
    "glomt",
    "behöver",
    "behover",
    "återbetalning",
    "aterbetalning",
    "kvitto",
    "kvittot",
    "spårningen",
    "sparningen",
    "inställningen",
    "installningen",
    "felmeddelande",
    "abonnemanget",
];
/// Foreign function words that are also ordinary English words: a repeat counts
/// once ("do more, do less" is English), while every other word counts each
/// occurrence (`der ... der` stays German).
const EN_COLLISION_WORDS: &[&str] = &["come", "son", "do", "care", "todo", "im", "per", "plus"];

/// Letters that ordinary English does not use: the signal for Latin-script
/// languages without a word list (Polish, Czech, Turkish, Baltic, ...).
const NON_EN_DIACRITICS: &str =
    "àâäãáåçéèêëíìîïñóòôöõøúùûüýÿßæœăâîșțşţąćęłńśźżčďěňřšťůžőűğıāēģīķļņūžđə";
/// Diacritic rate at or above which text is taken as not English.
const NON_EN_DIACRITIC_RATE: f64 = 0.02;
/// One accented loanword or name (`café`, `José`) clears the rate above in a
/// short sentence; English function words outvote it only below this rate.
const ENGLISH_RESCUE_DIACRITIC_RATE: f64 = 0.06;
/// Non-Latin text with Latin brand names or order codes is still not for the
/// English checkpoint, though the Latin letters may be the plurality: a short
/// message needs this share of non-Latin letters, a long payload (which dilutes
/// the share) the smaller share and a sentence's worth of letters.
const NON_LATIN_FRACTION: f64 = 0.2;
const NON_LATIN_MIN_FRACTION: f64 = 0.1;
const NON_LATIN_MIN_LETTERS: usize = 10;
/// Characters of a state (and of one line) that detection reads.
const MAX_CHARS: usize = 4000;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid pattern")
}
/// One letter (`str.isalpha`).
static LETTER: LazyLock<Regex> = LazyLock::new(|| re(r"\p{L}"));
/// laya's `_WORD` (`[^\W\d_]+`): letters and letter-like numerals.
static WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[\p{L}\p{Nl}\p{No}]+"));
/// laya's `_LETTER_RUN`, a run that may be an acronym.
static LETTER_RUN: LazyLock<Regex> = LazyLock::new(|| re(r"[\p{L}\p{Nl}\p{No}]{2,}"));
/// laya's `_IDENTIFIER` (`github.com`, `user@acme.com`, `v1.2.3`) without its
/// `(?<![\w-])`: `[\w-]` and `[.@]` are disjoint, so a leftmost-first match can
/// only start where that lookbehind holds (laya #383).
static IDENTIFIER: LazyLock<Regex> =
    LazyLock::new(|| re(r"[\p{L}\p{N}_-]*(?:[.@][\p{L}\p{N}_-]+)+"));
/// laya's `_CODE_LINE`: assignment, statement, brace, bracket or call syntax.
static CODE_LINE: LazyLock<Regex> = LazyLock::new(|| re(r"[=;{}\[\]]|[\p{L}\p{N}_]\("));
/// laya's `_JOINED`: a dotted, underscored or slashed name (`os.path`, `Nav/Com`).
static JOINED: LazyLock<Regex> = LazyLock::new(|| re(r"[\p{L}\p{N}][._/\\][\p{L}\p{N}]"));

#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    /// `latin`, `han`, `devanagari`, ..., `other` (letters no listed script
    /// claims) or `unknown` without letters.
    pub script: &'static str,
    /// Best-effort ISO code; None when undecided and for non-Latin scripts.
    pub language: Option<&'static str>,
    /// Whether the English checkpoint can be expected to read the state.
    pub is_english: bool,
    /// No word list identified the language. laya's router sends Latin text
    /// that is undecided yet `is_english` to its default checkpoint: that is no
    /// evidence of English either.
    pub language_undecided: bool,
    /// Fraction of characters that are non-English Latin letters (rounded like laya).
    pub diacritic_rate: f64,
    /// Fraction of letters outside the Latin script (rounded like laya).
    pub non_latin_fraction: f64,
    /// The line or field that made a mostly English state non-English.
    pub mixed_segment: Option<String>,
}

/// Python's `round(x, 4)`. Fixed-precision formatting rounds the exact binary
/// value as Python does; `(x * 1e4).round()` does not (1/160 rounds to 0.0062
/// there and to 0.0063 in Python).
fn round4(x: f64) -> f64 {
    format!("{x:.4}").parse().expect("formatted float")
}

/// Python's `s[:n]`.
fn prefix(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(i, _)| &s[..i])
}

/// Python's `str.isspace`, which `split()` and `strip()` use: Unicode
/// White_Space plus the ASCII separators U+001C..U+001F.
fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.islower` on one character. Unicode 17 made `ʕ` (U+0295) a
/// caseless letter; Python 3.14 (Unicode 16) still calls it lowercase, and it
/// is Latin here, so it decides whether a run of capitals reads as an acronym.
fn is_lower(c: char) -> bool {
    c.is_lowercase() || c == '\u{295}'
}

/// Python's `str.isupper`: a cased letter, and none lowercase or titlecase (a
/// titlecase letter such as `ǅ` is one that uppercasing changes).
fn is_upper(s: &str) -> bool {
    s.chars().any(char::is_uppercase) && s.chars().all(|c| !is_lower(c) && c.to_uppercase().eq([c]))
}

fn letters(text: &str) -> impl Iterator<Item = char> + '_ {
    LETTER
        .find_iter(text)
        .filter_map(|m| m.as_str().chars().next())
}

fn collect_leaves<'a>(state: &'a Value, depth: usize, out: &mut Vec<&'a str>) {
    if depth > 6 {
        return;
    }
    match state {
        Value::String(s) => out.push(s),
        Value::Object(map) => map.values().for_each(|v| collect_leaves(v, depth + 1, out)),
        Value::Array(items) => items.iter().for_each(|v| collect_leaves(v, depth + 1, out)),
        _ => {}
    }
}

/// The string leaves joined by spaces, at most 4000 characters (keys are
/// ignored: usually English field names).
fn state_text(leaves: &[&str]) -> String {
    let mut parts = Vec::new();
    let mut budget = MAX_CHARS;
    for leaf in leaves {
        if budget == 0 {
            break;
        }
        let cut = prefix(leaf, budget);
        parts.push(cut);
        if cut.len() < leaf.len() {
            break;
        }
        budget = budget.saturating_sub(cut.chars().count() + 1);
    }
    prefix(&parts.join(" "), MAX_CHARS).to_owned()
}

/// Latin, IPA Extensions (Azerbaijani `ə`), Latin Extended Additional and
/// fullwidth Latin.
fn is_latin(ch: char) -> bool {
    let cp = ch as u32;
    cp < 0x02B0
        || (0x1E00..=0x1EFF).contains(&cp)
        || (0xFF21..=0xFF3A).contains(&cp)
        || (0xFF41..=0xFF5A).contains(&cp)
}

/// The named non-Latin script of any character, None for Latin and for
/// characters no range claims.
fn script_of(ch: char) -> Option<&'static str> {
    if is_latin(ch) {
        return None;
    }
    let cp = ch as u32;
    SCRIPTS
        .iter()
        .find(|(_, ranges)| ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp)))
        .map(|(name, _)| *name)
}

/// Letters per script in first-seen order, Latin last. A letter no range
/// claims counts as `other`: an unreadable script must not reach the English
/// checkpoint as `unknown`.
fn script_counts(text: &str) -> Vec<(&'static str, usize)> {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    let mut latin = 0;
    for ch in letters(text) {
        if is_latin(ch) {
            latin += 1;
            continue;
        }
        let name = script_of(ch).unwrap_or("other");
        match counts.iter_mut().find(|(n, _)| *n == name) {
            Some((_, n)) => *n += 1,
            None => counts.push((name, 1)),
        }
    }
    counts.push(("latin", latin));
    counts
}

/// The script with the most letters, the first of equals (so a named script
/// wins a tie with Latin), or `unknown` without letters.
fn dominant(counts: &[(&'static str, usize)]) -> &'static str {
    counts
        .iter()
        .filter(|(_, n)| *n > 0)
        .min_by_key(|(_, n)| Reverse(*n))
        .map_or("unknown", |(name, _)| name)
}

/// Whether `text` holds a non-Latin word rather than annotation inside English
/// prose: a run of two or more characters of one named script that is not
/// capitalised. A symbol (`α`), a name (`Дмитрий`) and a pronunciation
/// (`[vlɐˈdʲimʲɪr]`, which no range claims) are not; a combining mark belongs
/// to the letter before it and never splits a word.
fn has_non_latin_word(text: &str) -> bool {
    let is_word = |run: Option<(&str, char, usize)>| {
        run.is_some_and(|(_, first, len)| len >= 2 && !first.is_uppercase())
    };
    let ccc = CodePointMapData::<CanonicalCombiningClass>::new();
    let mut run: Option<(&'static str, char, usize)> = None;
    for ch in text
        .chars()
        .filter(|&c| ccc.get(c) == CanonicalCombiningClass::NotReordered)
    {
        let script = script_of(ch);
        if let Some((current, _, len)) = &mut run {
            if Some(*current) == script {
                *len += 1;
                continue;
            }
        }
        if is_word(run) {
            return true;
        }
        run = script.map(|s| (s, ch, 1));
    }
    is_word(run)
}

/// Each stop word with the lists that hold it (bit `i` is `STOP[i]`).
static LISTS: LazyLock<HashMap<&'static str, u16>> = LazyLock::new(|| {
    let mut lists = HashMap::new();
    for (i, (_, words)) in STOP.iter().enumerate() {
        for &word in *words {
            *lists.entry(word).or_insert(0) |= 1 << i;
        }
    }
    lists
});

fn lists(word: &str) -> u16 {
    LISTS.get(word).copied().unwrap_or(0)
}

/// A word that says "not English" without saying which language.
fn shared(word: &str) -> bool {
    lists(word).count_ones() > 1 || NORDIC_OVERLAP_WORDS.contains(&word)
}

/// An English function word no other list holds (`in`, `is`, `was` are German,
/// Dutch or Portuguese too).
fn en_only(word: &str) -> bool {
    lists(word) & 1 == 1 && !shared(word)
}

/// Whether plain-English function words outvote a marginal diacritic rate: two
/// English-only words and at most one word carrying a non-English letter (one
/// loanword is not a vocabulary; the odd `i` or `at` of a Danish sentence is
/// not English either).
fn english_rescued(words: &HashSet<&str>, diacritic_rate: f64) -> bool {
    diacritic_rate < ENGLISH_RESCUE_DIACRITIC_RATE
        && words.iter().filter(|w| en_only(w)).count() >= 2
        && words
            .iter()
            .filter(|w| w.chars().any(|c| NON_EN_DIACRITICS.contains(c)))
            .count()
            <= 1
}

struct LatinProfile {
    language: Option<&'static str>,
    diacritic_rate: f64,
    looks_non_english: bool,
}

/// laya's stopword/diacritic evidence for Latin text. A non-English language
/// is named only from a word no other list holds and with a margin over English
/// function words; `looks_non_english` can hold without a name (non-English
/// letters, or only Danish/Swedish overlap words).
fn latin_profile(text: &str) -> LatinProfile {
    // Identifiers split into pieces that collide with function words (`com`,
    // `o`); `İ` lowercases to `i` plus a combining dot, which no list holds.
    let cleaned = IDENTIFIER
        .replace_all(text, " ")
        .replace('İ', "i")
        .to_lowercase();
    let words: Vec<&str> = WORD.find_iter(&cleaned).map(|m| m.as_str()).collect();
    let lowered = text.to_lowercase();
    let diacritics = lowered
        .chars()
        .filter(|c| NON_EN_DIACRITICS.contains(*c))
        .count();
    let diacritic_rate = diacritics as f64 / lowered.chars().count().max(1) as f64;
    let non_english = diacritic_rate >= NON_EN_DIACRITIC_RATE;
    let distinct: HashSet<&str> = words.iter().copied().collect();
    let nordic_overlap = distinct.iter().any(|w| NORDIC_OVERLAP_WORDS.contains(w))
        && !distinct.iter().any(|w| en_only(w));
    let profile = |language, looks_non_english| LatinProfile {
        language,
        diacritic_rate,
        looks_non_english,
    };
    if (2..4).contains(&words.len()) && distinct.iter().any(|w| SHORT_SWEDISH_WORDS.contains(w)) {
        return profile(Some("sv"), non_english);
    }
    if words.len() < 4 {
        return profile(None, non_english || nordic_overlap);
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for word in &words {
        *counts.entry(word).or_default() += 1;
    }
    let mut scores = [0; STOP.len()];
    let mut evidenced = 0u16;
    for (word, n) in counts {
        let held = lists(word);
        let hits = if EN_COLLISION_WORDS.contains(&word) {
            1
        } else {
            n
        };
        for (i, score) in scores.iter_mut().enumerate() {
            if held >> i & 1 == 1 {
                *score += hits;
            }
        }
        if !shared(word) {
            evidenced |= held;
        }
    }
    let en = scores[0];
    // A language with only shared words is out of the running, not merely
    // behind: a lesser score with a word of its own can still be named.
    let best = (1..STOP.len())
        .filter(|i| evidenced >> i & 1 == 1)
        .min_by_key(|&i| Reverse(scores[i]));
    let best_score = best.map_or(0, |i| scores[i]);
    let best = best.map(|i| STOP[i].0);
    let language = if best.is_some() && best_score >= en + 2 {
        best
    } else if best == Some("sv")
        && distinct.contains("inte")
        && distinct.contains("kan")
        && ["kan", "jag", "vi"].contains(&words[0])
        && en <= 1
    {
        // "kan inte logga in" carries an English-shaped `in`; `kan` alone is
        // Danish and Norwegian too.
        best
    } else if best.is_some() && non_english && best_score >= en.max(2) {
        best
    } else if en > 0 && (!non_english || english_rescued(&distinct, diacritic_rate)) {
        Some("en")
    } else {
        None
    };
    profile(
        language,
        non_english || (language.is_none() && nordic_overlap),
    )
}

/// laya's `_analyse_text`: the verdict on one flattened string, without the
/// line and field scans `analyse` adds.
fn analyse_text(text: &str) -> Detection {
    let counts = script_counts(text);
    let total: usize = counts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return Detection {
            script: "unknown",
            language: None,
            is_english: true,
            language_undecided: true,
            diacritic_rate: 0.0,
            non_latin_fraction: 0.0,
            mixed_segment: None,
        };
    }
    let (named, latin) = counts.split_at(counts.len() - 1);
    let non_latin = round4(1.0 - latin[0].1 as f64 / total as f64);
    let non_latin_letters = (non_latin * total as f64).round_ties_even() as usize;
    let mut script = dominant(&counts);
    if script == "latin"
        && (non_latin >= NON_LATIN_FRACTION
            || (non_latin >= NON_LATIN_MIN_FRACTION && non_latin_letters >= NON_LATIN_MIN_LETTERS))
        && has_non_latin_word(text)
    {
        script = dominant(named);
    }
    if script != "latin" {
        return Detection {
            script,
            language: None,
            is_english: false,
            language_undecided: true,
            diacritic_rate: 0.0,
            non_latin_fraction: non_latin,
            mixed_segment: None,
        };
    }
    let profile = latin_profile(text);
    // Undecided is not English: non-English letters or a Swedish-Danish marker
    // still keep such text off the English checkpoint.
    let undecided = profile.language.is_none();
    Detection {
        script,
        language: profile.language,
        is_english: profile.language == Some("en") || (undecided && !profile.looks_non_english),
        language_undecided: undecided,
        diacritic_rate: round4(profile.diacritic_rate),
        non_latin_fraction: non_latin,
        mixed_segment: None,
    }
}

/// The non-English language one line names, if any: the evidence a whole state
/// needs (four words, a language `latin_profile` names) plus two different
/// words of that language. Code lines name none; joined tokens (`os.path`,
/// `Nav/Com`, `C:\DOS`) and acronyms inside mixed-case text (`MON`, `COM`) are
/// not words, while a line written entirely in capitals keeps its words.
fn named_prose_language(segment: &str) -> Option<&'static str> {
    if segment.trim_matches(is_space).is_empty() || CODE_LINE.is_match(segment) {
        return None;
    }
    let mut prose = segment
        .split(is_space)
        .filter(|token| !token.is_empty() && !JOINED.is_match(token))
        .collect::<Vec<_>>()
        .join(" ");
    if prose.chars().any(is_lower) {
        prose = LETTER_RUN
            .replace_all(&prose, |m: &Captures| {
                if is_upper(&m[0]) { " " } else { &m[0] }.to_owned()
            })
            .into_owned();
    }
    let tokens: Vec<&str> = WORD.find_iter(&prose).map(|m| m.as_str()).collect();
    if tokens.len() < 4 {
        return None;
    }
    let language = latin_profile(&prose).language.filter(|l| *l != "en")?;
    let (_, list) = STOP.iter().find(|(code, _)| *code == language)?;
    let own: HashSet<String> = tokens
        .iter()
        .map(|w| w.to_lowercase())
        .filter(|w| list.contains(&w.as_str()))
        .collect();
    (own.len() >= 2).then_some(language)
}

/// The first line of any leaf that names a non-English language read on its
/// own, with that line, within the first 4000 characters of the leaves.
fn non_english_segment(leaves: &[&str]) -> Option<(&'static str, String)> {
    let mut seen = 0;
    for segment in leaves.iter().flat_map(|leaf| leaf.split('\n')) {
        if seen >= MAX_CHARS {
            return None;
        }
        let segment = prefix(segment, MAX_CHARS - seen);
        seen += segment.chars().count();
        if let Some(language) = named_prose_language(segment) {
            return Some((language, segment.trim_matches(is_space).to_owned()));
        }
    }
    None
}

/// The verdict on a leaf that is itself not safe for the English checkpoint,
/// from its line with the most letters (each line read up to 4000 characters),
/// else None. One-word names, capitalised non-Latin names, code lines,
/// acronyms and joined tokens stay out, as in the line scan.
fn leaf_non_english(leaf: &str) -> Option<Detection> {
    leaf.split('\n')
        // Four words need seven characters and ten letters ten: a shorter line
        // cannot qualify, so it skips the work below.
        .filter(|line| line.chars().nth(6).is_some())
        .map(|line| prefix(line, MAX_CHARS))
        .filter(|sample| !sample.trim_matches(is_space).is_empty() && !CODE_LINE.is_match(sample))
        .filter_map(|sample| {
            let detection = analyse_text(sample);
            let qualifies = if detection.is_english {
                false
            } else if detection.language.is_some() {
                named_prose_language(sample).is_some()
            } else if detection.script != "latin" && detection.script != "unknown" {
                has_non_latin_word(sample) && letters(sample).count() >= NON_LATIN_MIN_LETTERS
            } else {
                detection.language_undecided
                    && detection.diacritic_rate >= NON_EN_DIACRITIC_RATE
                    && WORD.find_iter(sample).count() >= 4
            };
            qualifies.then(|| (letters(sample).count(), detection))
        })
        .min_by_key(|(n, _)| Reverse(*n))
        .map(|(_, detection)| detection)
}

/// laya's `analyse`: undecided Latin text with non-English letters is not
/// English; text with no letters, or short plain-ASCII text, is. String leaves
/// are what get read (depth at most 6), and one non-English line or field makes
/// the whole state non-English.
pub fn analyse(state: &Value) -> Detection {
    let mut leaves = Vec::new();
    collect_leaves(state, 0, &mut leaves);
    let mut detection = analyse_text(&state_text(&leaves));
    // A Portuguese ticket with an English stack trace, error payload or form
    // template reads as English as a whole, yet the customer's part is what a
    // question is about, and the English checkpoint answers `pt` at 0.97
    // confidence and 0.47 accuracy. A single line has no part to be outvoted by.
    if detection.script == "latin"
        && detection.is_english
        && (leaves.len() > 1 || leaves.iter().any(|leaf| leaf.contains('\n')))
    {
        if let Some((language, segment)) = non_english_segment(&leaves) {
            detection.language = Some(language);
            detection.is_english = false;
            detection.language_undecided = false;
            detection.mixed_segment = Some(segment);
        }
    }
    // A plain string was read whole. A structured state can still hide a
    // message past the 4000-character window (a long English note sorting
    // before a short German message), or in a script `latin_profile` does not name.
    if matches!(state, Value::String(_) | Value::Null) || !detection.is_english {
        return detection;
    }
    let leaf = leaves
        .iter()
        .filter_map(|leaf| {
            leaf_non_english(leaf).map(|found| (letters(prefix(leaf, MAX_CHARS)).count(), found))
        })
        .min_by_key(|(n, _)| Reverse(*n));
    if let Some((_, found)) = leaf {
        detection.language = found.language;
        detection.is_english = false;
        detection.language_undecided = found.language_undecided;
    }
    detection
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
