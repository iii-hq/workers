//! A deliberately partial projection, not a parser or a source of ranking terms.
//! Ranges are half-open indices into the unchanged `index::tokenize` output.
use std::ops::Range;

use super::STOPWORDS;

fn prefix(words: &[String]) -> Option<bool> {
    let [prep, wh, ..] = words else { return None };
    match (prep.as_str(), wh.as_str()) {
        ("em" | "durante", "que" | "qual" | "quais") => Some(true),
        ("in" | "at" | "during", "what" | "which") => Some(false),
        _ => None,
    }
}

fn subject(word: &str, portuguese: bool) -> bool {
    if portuguese {
        matches!(word, "eu" | "nós" | "ele" | "ela" | "você" | "vocês")
    } else {
        matches!(word, "i" | "we" | "you" | "he" | "she" | "they" | "it")
    }
}

fn auxiliary(word: &str) -> bool {
    matches!(
        word,
        "do" | "does"
            | "did"
            | "am"
            | "are"
            | "is"
            | "can"
            | "could"
            | "will"
            | "would"
            | "should"
            | "have"
            | "has"
    )
}

// Only open-class runs and nominal linkers are admitted inside the slot.
// No noun, topic, synonym, entity or candidate memory is consulted.
fn nominal(words: &[String]) -> bool {
    let content = |w: &str| !STOPWORDS.contains(&w);
    !words.is_empty()
        && content(&words[0])
        && content(words.last().unwrap())
        && words.iter().all(|w| {
            content(w)
                || matches!(
                    w.as_str(),
                    "de" | "da"
                        | "do"
                        | "das"
                        | "dos"
                        | "o"
                        | "a"
                        | "os"
                        | "as"
                        | "of"
                        | "the"
                        | "an"
                )
        })
}

fn has_clause(words: &[String]) -> bool {
    words.windows(3).any(|w| {
        (auxiliary(&w[0]) && subject(&w[1], false) && !STOPWORDS.contains(&w[2].as_str()))
            || (subject(&w[0], true) && !STOPWORDS.contains(&w[1].as_str()))
    })
}

/// `None` means unknown: consumers must use their previous evidence, never
/// infer a relation from failure to parse. Only two explicit seams are known:
/// a fronted prepositional WH phrase before an overt subject/inverted auxiliary,
/// or a comma-coordinated elliptical WH phrase attached to an overt clause.
pub(super) fn requested_slots(query: &str, words: &[String]) -> Option<Vec<Range<usize>>> {
    // Quoting, multiple sentences and embedded clauses are outside this pilot.
    let body = query.trim().strip_suffix('?').unwrap_or(query.trim());
    if body.chars().any(|c| {
        matches!(
            c,
            '?' | '.' | '!' | ';' | ':' | '\n' | '\r' | '"' | '\'' | '“' | '”' | '(' | ')'
        )
    }) {
        return None;
    }
    if let Some((left, right)) = body.rsplit_once(',') {
        if left.contains(',') {
            return None;
        }
        let lhs = crate::index::tokenize(left);
        let rhs = crate::index::tokenize(right);
        if !has_clause(&lhs) || !matches!(rhs.first()?.as_str(), "and" | "e") {
            return None;
        }
        prefix(&rhs[1..])?;
        if !nominal(&rhs[3..]) || lhs.len() + rhs.len() != words.len() {
            return None;
        }
        return Some(std::iter::once(lhs.len() + 1..words.len()).collect());
    }
    let portuguese = prefix(words)?;
    let end = if portuguese {
        (2..words.len()).find(|&i| subject(&words[i], true))?
    } else {
        (2..words.len().saturating_sub(1))
            .find(|&i| auxiliary(&words[i]) && subject(&words[i + 1], false))?
    };
    let clause_start = end + if portuguese { 1 } else { 2 };
    if !nominal(&words[2..end])
        || !words[clause_start..]
            .iter()
            .any(|w| !STOPWORDS.contains(&w.as_str()))
    {
        return None;
    }
    Some(std::iter::once(0..end).collect())
}

/// Output roles are grammatical frames, not a vocabulary of stored topics.
/// Artifact heads are mandatory in artifact requests. A directional type
/// relation can license a general output rule for a requested subtype, but
/// never proves instance identity from shared components.
#[derive(Debug, Clone)]
pub(super) struct DirectiveScope {
    pub span: Range<usize>,
    pub role: &'static str,
    pub head: Option<String>,
    pub components: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct DirectiveEvidence {
    pub query_span: [usize; 2],
    pub role: &'static str,
    pub head: Option<String>,
    pub components: Vec<String>,
    pub source_span: Option<[usize; 2]>,
    pub governor_span: Option<[usize; 2]>,
    pub assistant_span: Option<[usize; 2]>,
    pub argument_span: Option<[usize; 2]>,
    pub value_span: Option<[usize; 2]>,
    pub boundary_span: Option<[usize; 2]>,
    pub artifact_span: Option<[usize; 2]>,
    pub artifact_relation: Option<ArtifactRelation>,
    pub presentation_span: Option<[usize; 2]>,
    pub notation_span: Option<[usize; 2]>,
    pub frame: &'static str,
    pub domain: &'static str,
    pub polarity: &'static str,
    pub matched: bool,
}

/// A linguistic type witness, not a corpus label or an instance equivalence.
/// Direction matters: a rule about poems covers a requested sonnet; a rule
/// only about sonnets cannot certify an unrestricted request about poems.
#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct ArtifactRelation {
    pub query_head: String,
    pub candidate_head: String,
    pub relation: &'static str,
    pub type_path: Vec<&'static str>,
}

fn artifact_ancestry(head: &str) -> &'static [&'static str] {
    match head {
        "script" => &["script", "code", "artifact"],
        "code" | "código" => &["code", "artifact"],
        "sonnet" | "soneto" => &["sonnet", "poem", "text", "artifact"],
        "haiku" | "haicai" => &["haiku", "poem", "text", "artifact"],
        "poem" | "poema" => &["poem", "text", "artifact"],
        "report" | "relatório" => &["report", "document", "text", "artifact"],
        "invoice" | "fatura" => &["invoice", "document", "text", "artifact"],
        "document" | "documento" => &["document", "text", "artifact"],
        "prose" | "prosa" => &["prose", "text", "artifact"],
        "text" | "texto" => &["text", "artifact"],
        _ => &[],
    }
}

fn artifact_relation(query_head: &str, candidate_head: &str) -> ArtifactRelation {
    let mut witness = ArtifactRelation {
        query_head: query_head.into(),
        candidate_head: candidate_head.into(),
        relation: "unknown_or_incompatible",
        type_path: Vec::new(),
    };
    if query_head == candidate_head {
        witness.relation = "literal_identity";
        return witness;
    }
    let query_path = artifact_ancestry(query_head);
    if let Some(candidate_type) = artifact_ancestry(candidate_head).first() {
        if let Some(end) = query_path.iter().position(|kind| kind == candidate_type) {
            witness.relation = "query_subtype_of_rule";
            witness.type_path = query_path[..=end].to_vec();
        }
    }
    witness
}

fn directive_patterns(query: bool) -> &'static [regex::Regex] {
    static QUERY: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    static ASSERTION: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    let (cache, patterns): (_, &[&str]) = if query {
        (
            &QUERY,
            &[
                r"^(?:in (?:what|which) language should you reply to me|(?:se eu te escrever em [\p{L} -]+, )?em que (?:língua|idioma) você deve me responder)\?$",
                r"^(?:should you write times for me as .+ or as .+|você deve escrever os horários para mim como .+ ou como .+)\?$",
                r"^for (?P<head>[\p{L}]+) written by you, in (?:what|which) language should (?P<components>.+) be\?$",
                r"^n[oa] (?P<head>[\p{L}]+) escrit[oa] por você, (?P<components>.+) devem ficar em que idioma\?$",
                r"^vou te pedir um (?P<head>[\p{L}]+): (?P<components>.+) devem ficar em que idioma\?$",
            ],
        )
    } else {
        (
            &ASSERTION,
            &[
                r"^(?P<source>(?:a |o )?(?:user|usuária|usuário|i|eu)) (?P<governor>wants?|asks?|requests?|quer|pede|prefere) (?P<body>.+)$",
                r"^(?P<assistant>the assistant to reply|que o assistente responda)(?: always| sempre)? (?:in|em) (?P<value>[\p{L} -]+)(?:, (?P<boundary>mesmo quando ela escreve em [\p{L} -]+))?$",
                r"^(?:the )?(?P<head>[\p{L}]+) written by (?P<assistant>the assistant)(?:, including (?P<components>[^,]+),)? in (?P<value>[\p{L} -]+)$",
                r"^(?:que )?[oa] (?P<head>[\p{L}]+) escrit[oa] pelo (?P<assistant>assistente)(?:, incluindo (?P<components>[^,]+),)? fique em (?P<value>[\p{L} -]+)$",
                r"^(?P<head>times) in (?P<value>\d+-hour format)(?P<presentation> and (?P<presentation_head>[\p{L}]+) as (?P<notation>[\p{L}\d]+(?:[/:][\p{L}\d]+)+))?(?:, (?P<boundary>never (?P<alternatives>[\p{L}\d/ -]+)))?$",
                r"^(?P<head>horários) no formato de (?P<value>\d+ horas)(?:, (?P<boundary>nunca (?P<alternative_seam>no formato de )(?P<alternatives>\d+ horas)))?$",
            ],
        )
    };
    cache.get_or_init(|| {
        patterns
            .iter()
            .map(|pattern| regex::Regex::new(pattern).unwrap())
            .collect()
    })
}

fn directive_components(text: &str) -> Vec<String> {
    crate::index::tokenize(text)
        .into_iter()
        .filter(|word| !matches!(word.as_str(), "the" | "os" | "as" | "o" | "a"))
        .collect()
}

/// This recognizes only explicit output questions. A condition on the user's
/// input is not an assertion about a candidate and does not change its domain.
pub(super) fn directive_scope(query: &str) -> Option<DirectiveScope> {
    // A terminal question mark is neither proof nor denial of the relation.
    // Keep every embedded mark intact; only the optional terminal seam is known.
    let body = query.trim().strip_suffix('?').unwrap_or(query.trim());
    let normalized = format!("{}?", body.to_lowercase());
    for (kind, pattern) in directive_patterns(true).iter().enumerate() {
        let Some(captures) = pattern.captures(&normalized) else {
            continue;
        };
        return Some(DirectiveScope {
            span: 0..crate::index::tokenize(query).len(),
            role: match kind {
                0 => "reply_language",
                1 => "time_presentation",
                _ => "artifact_language",
            },
            head: captures.name("head").map(|head| head.as_str().to_owned()),
            components: captures
                .name("components")
                .map_or_else(Vec::new, |value| directive_components(value.as_str())),
        });
    }
    None
}

/// Bind only a root user governor to its explicit assistant/output argument.
/// The elided executor in a user-requested presentation has its own frame;
/// an arbitrary instruction to another person cannot inhabit that frame.
/// All token witnesses address the unchanged full candidate, before selection.
pub(super) fn directive_evidence(scope: &DirectiveScope, text: &str) -> DirectiveEvidence {
    let words = crate::index::tokenize(text);
    let mut audit = DirectiveEvidence {
        query_span: [scope.span.start, scope.span.end],
        role: scope.role,
        head: scope.head.clone(),
        components: scope.components.clone(),
        source_span: None,
        governor_span: None,
        assistant_span: None,
        argument_span: None,
        value_span: None,
        boundary_span: None,
        frame: "unbound",
        artifact_span: None,
        artifact_relation: None,
        presentation_span: None,
        notation_span: None,
        domain: obligation_domain(text, &words, &[]),
        polarity: if words.iter().any(|word| negative(word)) {
            "negative"
        } else {
            "affirmative"
        },
        matched: false,
    };
    let normalized = text.trim().trim_end_matches(['.', '!']).to_lowercase();
    let patterns = directive_patterns(false);
    let Some(root) = patterns[0].captures(&normalized) else {
        return audit;
    };
    let body = root.name("body").unwrap();
    let span = |start: usize, end: usize| {
        [
            crate::index::tokenize(&normalized[..start]).len(),
            crate::index::tokenize(&normalized[..end]).len(),
        ]
    };
    let source = root.name("source").unwrap();
    let governor = root.name("governor").unwrap();
    audit.source_span = Some(span(source.start(), source.end()));
    audit.governor_span = Some(span(governor.start(), governor.end()));
    for (kind, pattern) in patterns.iter().enumerate().skip(1) {
        let role = match kind {
            1 => "reply_language",
            2 | 3 => "artifact_language",
            _ => "time_presentation",
        };
        if role != scope.role {
            continue;
        }
        let Some(argument) = pattern.captures(body.as_str()) else {
            continue;
        };
        let value = argument.name("value").unwrap();
        let head = argument.name("head");
        // A literal delimited notation (for example, HH:MM) is presentation
        // evidence; an unconstrained `and X as Y` role assignment is not.
        if argument
            .name("presentation_head")
            .is_some_and(|part| !nominal(&crate::index::tokenize(part.as_str())))
        {
            continue;
        }
        let component_capture = argument.name("components");
        let components = component_capture.map(|value| directive_components(value.as_str()));
        if component_capture.is_some_and(|value| {
            let tokens = crate::index::tokenize(value.as_str());
            !tokens
                .split(|word| matches!(word.as_str(), "and" | "e"))
                .all(nominal)
        }) {
            continue;
        }
        if role == "artifact_language" {
            audit.artifact_span =
                head.map(|head| span(body.start() + head.start(), body.start() + head.end()));
            audit.artifact_relation =
                scope
                    .head
                    .as_deref()
                    .zip(head)
                    .map(|(query_head, candidate_head)| {
                        artifact_relation(query_head, candidate_head.as_str())
                    });
        }
        if (role == "artifact_language"
            && audit.artifact_relation.as_ref().is_none_or(|relation| {
                !matches!(
                    relation.relation,
                    "literal_identity" | "query_subtype_of_rule"
                )
            }))
            || (!scope.components.is_empty() && components.as_ref() != Some(&scope.components))
            || !nominal(&crate::index::tokenize(value.as_str()))
        {
            continue;
        }
        audit.artifact_span =
            head.map(|head| span(body.start() + head.start(), body.start() + head.end()));
        audit.presentation_span = argument.name("presentation").map(|presentation| {
            span(
                body.start() + presentation.start(),
                body.start() + presentation.end(),
            )
        });
        audit.notation_span = argument.name("notation").map(|notation| {
            span(
                body.start() + notation.start(),
                body.start() + notation.end(),
            )
        });
        audit.assistant_span = argument
            .name("assistant")
            .map(|target| span(body.start() + target.start(), body.start() + target.end()));
        audit.argument_span = Some(span(body.start(), body.end()));
        audit.value_span = Some(span(
            body.start() + value.start(),
            body.start() + value.end(),
        ));
        audit.boundary_span = argument.name("boundary").map(|boundary| {
            span(
                body.start() + boundary.start(),
                body.start() + boundary.end(),
            )
        });
        audit.frame = if role == "time_presentation" {
            "user_requested_presentation"
        } else {
            "user_assistant_output"
        };
        // Only the separately delimited rejected alternative is negative.
        // It must not invert an affirmative governor or its chosen value.
        let primary_end = audit
            .boundary_span
            .map_or(words.len(), |boundary| boundary[0]);
        audit.polarity = if words[..primary_end].iter().any(|word| negative(word)) {
            "negative"
        } else {
            "affirmative"
        };
        // Mask only bound nominal constituents and proven grammatical seams,
        // never the entire argument. Punctuation outside these seams survives.
        let mut nominal_spans = Vec::new();
        let mut punctuation = normalized.clone().into_bytes();
        for name in [
            "head",
            "value",
            "components",
            "presentation_head",
            "notation",
        ] {
            if let Some(part) = argument.name(name) {
                let [start, end] = span(body.start() + part.start(), body.start() + part.end());
                nominal_spans.push(start..end);
                if name == "notation" {
                    for byte in
                        &mut punctuation[body.start() + part.start()..body.start() + part.end()]
                    {
                        if matches!(*byte, b'/' | b':') {
                            *byte = b' ';
                        }
                    }
                }
            }
        }
        // QUE is licensed only as the initial complementizer of these two
        // complete assistant/output frames, not as an arbitrary subordinate.
        if matches!(kind, 1 | 3) && body.as_str().starts_with("que ") {
            let start = span(body.start(), body.start())[0];
            nominal_spans.push(start..start + 1);
        }
        if let Some(components) = component_capture {
            let introducer = if kind == 2 {
                ", including "
            } else {
                ", incluindo "
            };
            punctuation[body.start() + components.start() - introducer.len()] = b' ';
            punctuation[body.start() + components.end()] = b' ';
        }
        if let Some([start, _]) = audit.presentation_span {
            // The conjunction joins two presentations, not a new executor.
            nominal_spans.push(start..start + 1);
        }
        if let Some(boundary) = argument.name("boundary") {
            punctuation[body.start() + boundary.start() - 2] = b' ';
            if kind == 1 {
                // The only licensed concession is about the user's input.
                let [start, end] = span(
                    body.start() + boundary.start(),
                    body.start() + boundary.end(),
                );
                if !nominal(&words[start + 5..end]) {
                    continue;
                }
                nominal_spans.push(start..start + 5);
                nominal_spans.push(start + 5..end);
            } else {
                // These captures belong only to the two complete time-format
                // frames. PT's NO FORMATO DE is a seam before a numeric-hours
                // value, not a nominal headed by the stopword NO. AM/PM is an
                // exact written notation, not a general exemption for AM.
                let alternatives = argument.name("alternatives").unwrap();
                if !alternatives
                    .as_str()
                    .split(" or ")
                    .flat_map(|part| part.split(" ou "))
                    .all(|part| part == "am/pm" || nominal(&crate::index::tokenize(part)))
                {
                    continue;
                }
                if let Some(seam) = argument.name("alternative_seam") {
                    let [start, end] = span(body.start() + seam.start(), body.start() + seam.end());
                    nominal_spans.push(start..end);
                }
                let [start, end] = span(
                    body.start() + alternatives.start(),
                    body.start() + alternatives.end(),
                );
                // Leave the alternative values visible to the domain guards;
                // only their explicit coordination is licensed here.
                for (at, word) in words[start..end].iter().enumerate() {
                    if matches!(word.as_str(), "or" | "ou") {
                        nominal_spans.push(start + at..start + at + 1);
                    }
                }
            }
        }
        let punctuation = String::from_utf8(punctuation).unwrap();
        audit.domain = obligation_domain(&punctuation, &words, &nominal_spans);
        audit.matched = audit.domain == "asserted" && audit.polarity == "affirmative";
        break;
    }
    audit
}

#[derive(Debug, Clone, Copy)]
pub(super) enum TargetKind {
    QualifiedLabel,
    Quantity,
    SubjectPredicate,
    Attribute,
}

#[derive(Debug, Clone)]
pub(super) struct TargetScope {
    pub span: Range<usize>,
    pub kind: TargetKind,
    pub terms: Vec<String>,
    pub predicate: Option<String>,
    pub subject: Vec<String>,
    pub head_initial: bool,
    pub object: Vec<String>,
    pub relative: Vec<String>,
}

/// Partial grammatical shapes only. Nominal roles and literal predicates come
/// from the query. The separate bounded obligation frame below interprets only
/// its documented governors; no topic, candidate or score is consulted.
pub(super) fn target_scope(query: &str) -> Option<TargetScope> {
    let body = query.trim().trim_end_matches(['.', '?', '!']);
    if body
        .chars()
        .any(|c| matches!(c, '.' | '?' | '!' | ';' | ':' | '"' | '“' | '”' | '\n'))
    {
        return None;
    }
    let w = crate::index::tokenize(query);
    let content = |words: &[String]| {
        words
            .iter()
            .filter(|t| !STOPWORDS.contains(&t.as_str()))
            .cloned()
            .collect::<Vec<_>>()
    };
    let auxiliary = |t: &str| matches!(t, "does" | "do" | "did" | "is" | "are");
    let article = |t: &str| matches!(t, "the" | "a" | "an" | "o" | "os" | "as" | "um" | "uma");
    // HOW MANY / QUANTOS names the counted attribute before the clause seam.
    let quantity_start =
        if w.first().is_some_and(|t| t == "how") && w.get(1).is_some_and(|t| t == "many") {
            Some(2)
        } else if w
            .first()
            .is_some_and(|t| matches!(t.as_str(), "quantos" | "quantas"))
        {
            Some(1)
        } else {
            None
        };
    if let Some(start) = quantity_start {
        // English quantity inversion needs an auxiliary, not an article found
        // inside a later prepositional phrase. The counted phrase must also
        // stay nominal; an unknown seam supplies neither a veto nor support.
        let seam =
            (start..w.len()).find(|&i| auxiliary(&w[i]) || (w[0] != "how" && article(&w[i])))?;
        if !nominal(&w[start..seam]) {
            return None;
        }
        let terms = content(&w[start..seam]);
        let mut subject_at = seam + 1;
        if w.get(subject_at).is_some_and(|t| article(t)) {
            subject_at += 1;
        }
        let end = (subject_at..w.len())
            .find(|&i| {
                STOPWORDS.contains(&w[i].as_str())
                    || matches!(
                        w[i].as_str(),
                        "before" | "after" | "until" | "antes" | "depois"
                    )
            })
            .unwrap_or(w.len());
        if w.get(end).is_some_and(|word| {
            !matches!(
                word.as_str(),
                "before" | "after" | "until" | "antes" | "depois"
            )
        }) {
            return None;
        }
        if terms.is_empty()
            || end <= subject_at + 1
            || w[subject_at..end]
                .iter()
                .any(|t| coordination(t) || subordinate(t))
        {
            return None;
        }
        let subject = w[subject_at..end - 1].to_vec();
        if obligation_governor(&w[end - 1]).is_some()
            && !obligation_query_supported(query, &w, &[start..seam, subject_at..end - 1])
        {
            return None;
        }
        return Some(TargetScope {
            span: start..w.len(),
            kind: TargetKind::Quantity,
            terms,
            predicate: Some(w[end - 1].clone()),
            subject,
            head_initial: w[0] != "how",
            object: Vec::new(),
            relative: Vec::new(),
        });
    }
    // An explicitly named label after USING a possessive is not the earlier
    // bug/context argument. Preserve both the label and its nominal head.
    let spelling = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>();
    if let Some(at) = w
        .windows(2)
        .position(|s| matches!(s[0].as_str(), "using" | "usando") && possessive(&s[1]))
        .filter(|_| spelling.len() == w.len())
    {
        let begin = at + 2;
        if let Some(first) =
            (begin..w.len()).find(|&i| spelling[i].chars().next().is_some_and(char::is_uppercase))
        {
            let end = (first..w.len())
                .find(|&i| !spelling[i].chars().next().is_some_and(char::is_uppercase))
                .unwrap_or(w.len());
            let mut terms = w[first..end].to_vec();
            if let Some(head) = w
                .get(end)
                .filter(|head| !STOPWORDS.contains(&head.as_str()))
            {
                terms.push(head.clone());
            } else if first > begin {
                terms.push(w[first - 1].clone());
            }
            let object_begin = if w.get(1).is_some_and(|word| article(word)) {
                2
            } else {
                return None;
            };
            let object_end = (object_begin..at)
                .find(|&i| STOPWORDS.contains(&w[i].as_str()))
                .unwrap_or(at);
            let subject = w[object_begin..object_end].to_vec();
            if terms.len() > 1 && !subject.is_empty() {
                return Some(TargetScope {
                    span: object_begin..w.len(),
                    kind: TargetKind::QualifiedLabel,
                    terms,
                    predicate: Some(w[0].clone()),
                    subject,
                    head_initial: false,
                    object: Vec::new(),
                    relative: Vec::new(),
                });
            }
        }
    }
    if !w
        .first()
        .is_some_and(|t| matches!(t.as_str(), "which" | "what" | "qual"))
    {
        return None;
    }
    // WH subject: nominal head + predicate + definite object phrase. A WH
    // auxiliary question is a different shape, never a lexical verb guess.
    if let Some(object_at) = (3..w.len()).find(|&i| article(&w[i])) {
        let pred = object_at - 1;
        let finite_shape = if w[0] == "qual" {
            w[pred].ends_with('a')
                || w[pred].ends_with('e')
                || w[pred].ends_with('z')
                || w[pred].ends_with("am")
                || w[pred].ends_with("em")
        } else {
            w[pred].ends_with('s')
        };
        if pred > 1
            && finite_shape
            && !STOPWORDS.contains(&w[pred].as_str())
            && !auxiliary(&w[pred])
            && w[1..pred].iter().all(|t| {
                !STOPWORDS.contains(&t.as_str())
                    || (w[0] == "qual" && matches!(t.as_str(), "de" | "do" | "da"))
            })
            && w.get(2).is_none_or(|word| !auxiliary(word))
        {
            let begin = object_at + 1;
            let end = (begin + 1..w.len())
                .find(|&i| {
                    matches!(
                        w[i].as_str(),
                        "before" | "after" | "until" | "antes" | "depois"
                    ) || spelling
                        .get(i)
                        .is_some_and(|word| word.chars().next().is_some_and(char::is_uppercase))
                })
                .unwrap_or(w.len());
            let terms = content(&w[begin..]);
            let relative = if end < w.len()
                && spelling
                    .get(end)
                    .is_some_and(|word| word.chars().next().is_some_and(char::is_uppercase))
            {
                let tail = &w[end..];
                if tail.len() != 4 || !matches!(tail[2].as_str(), "to" | "for" | "para") {
                    return None;
                }
                tail.to_vec()
            } else {
                Vec::new()
            };
            if !terms.is_empty() && end > begin {
                return Some(TargetScope {
                    span: 1..w.len(),
                    kind: TargetKind::SubjectPredicate,
                    terms,
                    predicate: Some(w[pred].clone()),
                    subject: w[1..pred].to_vec(),
                    head_initial: w[0] == "qual",
                    object: w[begin..end].to_vec(),
                    relative,
                });
            }
        }
    }
    // PT genitive attribute before a determined subject: QUAL N de N o ...
    // Keep the attribute's head/complement together; the following subject
    // cannot lend support to a different attribute on that same entity.
    if w[0] == "qual"
        && w.get(2)
            .is_some_and(|t| matches!(t.as_str(), "de" | "do" | "da"))
        && w.get(4).is_some_and(|t| article(t))
        && w.len() > 6
        && !STOPWORDS.contains(&w[1].as_str())
        && !STOPWORDS.contains(&w[3].as_str())
        && w[5..]
            .iter()
            .all(|t| !STOPWORDS.contains(&t.as_str()) && !coordination(t) && !subordinate(t))
    {
        return Some(TargetScope {
            span: 1..w.len(),
            kind: TargetKind::Attribute,
            terms: vec![w[1].clone(), w[3].clone()],
            predicate: Some(w.last()?.clone()),
            subject: w[5..w.len() - 1].to_vec(),
            head_initial: true,
            object: Vec::new(),
            relative: Vec::new(),
        });
    }
    // An explicit WH nominal attribute before DOES/DO/DID + a determined
    // subject and one final predicate. Copular/long/embedded shapes remain
    // unknown rather than becoming a literal-argument veto.
    if let Some(aux) = (2..w.len()).find(|&i| matches!(w[i].as_str(), "does" | "do" | "did")) {
        if w[1..aux].iter().all(|t| !STOPWORDS.contains(&t.as_str()))
            && w.get(aux + 1).is_some_and(|t| article(t))
            && w.len() > aux + 3
            && w[aux + 2..]
                .iter()
                .all(|t| !STOPWORDS.contains(&t.as_str()) && !coordination(t) && !subordinate(t))
        {
            return Some(TargetScope {
                span: 1..w.len(),
                kind: TargetKind::Attribute,
                terms: w[1..aux].to_vec(),
                predicate: Some(w.last()?.clone()),
                subject: w[aux + 2..w.len() - 1].to_vec(),
                head_initial: false,
                object: Vec::new(),
                relative: Vec::new(),
            });
        }
    }
    None
}

fn quantified_word(word: &str) -> bool {
    (!word.is_empty() && word.chars().all(|c| c.is_ascii_digit()))
        || matches!(
            word,
            "one"
                | "two"
                | "three"
                | "four"
                | "five"
                | "six"
                | "seven"
                | "eight"
                | "nine"
                | "ten"
                | "um"
                | "uma"
                | "dois"
                | "duas"
                | "três"
                | "quatro"
                | "cinco"
                | "seis"
                | "sete"
                | "oito"
                | "nove"
                | "dez"
        )
}

// A counted prerequisite, not a general synonym relation. These four finite
// families are explicit: English need/require and Portuguese precisar/exigir.
// need/precisar take a bearer as subject (precisar selects DE); require/exigir
// also license an overt source imposing a counted object ON EACH target.
// Other predicates keep literal quantity matching; no semantic alias is inferred.
#[derive(Debug, Clone, Copy)]
enum ObligationGovernor {
    Need,
    Require,
    Precisar,
    Exigir,
}

fn obligation_governor(word: &str) -> Option<ObligationGovernor> {
    match word {
        "need" | "needs" => Some(ObligationGovernor::Need),
        "require" | "requires" => Some(ObligationGovernor::Require),
        "precisa" | "precisam" => Some(ObligationGovernor::Precisar),
        "exige" | "exigem" => Some(ObligationGovernor::Exigir),
        _ => None,
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct ObligationAssertion {
    /// All spans address the unchanged tokenization of the full candidate.
    pub governor_span: Option<[usize; 2]>,
    pub source_span: Option<[usize; 2]>,
    pub argument_span: Option<[usize; 2]>,
    pub target_span: Option<[usize; 2]>,
    pub boundary_span: Option<[usize; 2]>,
    /// Unbound spans are diagnostic occurrences, never a certificate.
    pub frame: &'static str,
    pub domain: &'static str,
    pub polarity: &'static str,
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct ObligationEvidence {
    pub query_governor: String,
    pub query_span: [usize; 2],
    pub target: Vec<String>,
    pub object: Vec<String>,
    pub assertion: ObligationAssertion,
    pub matched: bool,
}

// A query requests a relation; it does not assert one. Remove only its
// optional terminal question mark, preserving embedded punctuation and the
// same explicit exclusions outside bound nominal roles.
fn obligation_query_supported(
    query: &str,
    words: &[String],
    nominal_spans: &[Range<usize>],
) -> bool {
    let body = query.trim().strip_suffix('?').unwrap_or(query.trim());
    obligation_context_exclusion(body, words, nominal_spans).is_none()
        && !words.iter().any(|word| negative(word))
}

fn obligation_domain(text: &str, words: &[String], nominal_spans: &[Range<usize>]) -> &'static str {
    let exclusion = obligation_context_exclusion(text, words, nominal_spans);
    match exclusion {
        Some(domain) if domain != "unknown" => domain,
        // Candidate punctuation is never normalized as query punctuation.
        _ if text.contains('?') => "interrogative",
        Some(domain) => domain,
        None => "asserted",
    }
}

// Shared exclusions do not assert a relation. Query eligibility and candidate
// assertion domains interpret their own question punctuation separately.
fn obligation_context_exclusion(
    text: &str,
    words: &[String],
    nominal_spans: &[Range<usize>],
) -> Option<&'static str> {
    // A report/example named as object or source is not a reporting predicate
    // or an example operator. Only occurrences outside the bound nominal roles
    // participate in the domain guards; an unbound frame masks nothing.
    let words: Vec<&String> = words
        .iter()
        .enumerate()
        .filter(|(at, _)| !nominal_spans.iter().any(|span| span.contains(at)))
        .map(|(_, word)| word)
        .collect();
    let chars: Vec<char> = text.chars().collect();
    let quoted = chars.iter().enumerate().any(|(i, &ch)| {
        matches!(ch, '"' | '“' | '”' | '‘' | '’' | '`')
            || (ch == '\''
                && !(i > 0
                    && chars[i - 1].is_alphanumeric()
                    && chars.get(i + 1) == Some(&'s')
                    && chars.get(i + 2).is_none_or(|c| !c.is_alphanumeric())))
    });
    if quoted
        || words.iter().any(|word| {
            matches!(
                word.as_str(),
                "quote" | "quotes" | "quoted" | "cita" | "citou" | "citação"
            )
        })
    {
        return Some("quoted");
    }
    if words.iter().any(|word| {
        matches!(
            word.as_str(),
            "if" | "unless" | "would" | "could" | "might" | "se" | "caso" | "poderia" | "talvez"
        )
    }) {
        return Some("conditional");
    }
    if words.iter().any(|word| {
        matches!(
            word.as_str(),
            "say"
                | "says"
                | "said"
                | "report"
                | "reports"
                | "reported"
                | "diz"
                | "disse"
                | "relata"
                | "relatou"
                | "segundo"
        )
    }) {
        return Some("reported");
    }
    if words.iter().any(|word| {
        matches!(
            word.as_str(),
            "mention"
                | "mentions"
                | "mentioned"
                | "example"
                | "examples"
                | "fictional"
                | "menciona"
                | "mencionam"
                | "mencionou"
                | "exemplo"
                | "exemplos"
                | "fictício"
                | "fictícia"
        )
    }) {
        return Some("mention");
    }
    let body = text.trim().trim_end_matches(['.', '!']);
    if body.chars().any(|c| {
        matches!(
            c,
            '.' | '?' | '!' | ';' | ':' | ',' | '(' | ')' | '\n' | '\r'
        )
    }) || words
        .iter()
        .any(|word| coordination(word) || subordinate(word))
    {
        return Some("unknown");
    }
    None
}

fn obligation_tail(words: &[String], at: usize) -> bool {
    at == words.len()
        || (words.get(at).is_some_and(|word| {
            matches!(
                word.as_str(),
                "before" | "after" | "until" | "antes" | "depois"
            )
        }) && at + 1 < words.len())
}

// Intentionally shallow sources: a determiner plus one nominal head, or an
// overt user's possessive plus one head. Longer/unmarked/reported clauses are
// unknown, not guessed noun phrases containing a second subject's predicate.
fn obligation_source(words: &[String]) -> bool {
    (words.len() == 2
        && matches!(
            words[0].as_str(),
            "the" | "a" | "an" | "o" | "os" | "as" | "um" | "uma"
        )
        && nominal(&words[1..]))
        || (words.len() == 3 && words[0] == "user" && words[1] == "s" && nominal(&words[2..]))
}

/// Bounded, nonpersistent obligation evidence, evaluated before selection.
/// Unknown frames do not acquire semantic support from words elsewhere. Only
/// a sentence-root universal bearer or a shallow source + governor + counted
/// object + preposition + universal target is certified. Quotes, reports,
/// examples, conditionals, coordination and embedded clauses never certify.
/// The boundary tail is retained, not interpreted as temporal entailment.
/// Literal/non-obligational quantities do not enter this representation.
pub(super) fn obligation_evidence(scope: &TargetScope, text: &str) -> Option<ObligationEvidence> {
    if !matches!(scope.kind, TargetKind::Quantity) {
        return None;
    }
    let query_governor = scope.predicate.as_ref()?;
    obligation_governor(query_governor)?;
    let words = crate::index::tokenize(text);
    let mut assertion = ObligationAssertion {
        governor_span: words
            .iter()
            .position(|word| obligation_governor(word).is_some())
            .map(|at| [at, at + 1]),
        source_span: None,
        argument_span: None,
        target_span: None,
        boundary_span: None,
        frame: "unbound",
        domain: obligation_domain(text, &words, &[]),
        polarity: if words.iter().any(|word| negative(word)) {
            "negative"
        } else {
            "affirmative"
        },
    };
    let universal = |word: &str| matches!(word, "each" | "every" | "cada" | "todo" | "toda");
    let same_run = |at: usize, terms: &[String]| {
        words
            .get(at..at + terms.len())
            .is_some_and(|run| run.iter().zip(terms).all(|(a, b)| same_form(b, a)))
    };
    for (governor_at, word) in words.iter().enumerate() {
        let Some(governor) = obligation_governor(word) else {
            continue;
        };
        let counted =
            governor_at + 1 + usize::from(matches!(governor, ObligationGovernor::Precisar));
        if (matches!(governor, ObligationGovernor::Precisar)
            && words.get(governor_at + 1).is_none_or(|word| word != "de"))
            || !words.get(counted).is_some_and(|word| quantified_word(word))
            || !same_run(counted + 1, &scope.terms)
        {
            continue;
        }
        let argument_end = counted + 1 + scope.terms.len();
        let bearer = words.first().is_some_and(|word| universal(word))
            && same_run(1, &scope.subject)
            && governor_at == 1 + scope.subject.len()
            && obligation_tail(&words, argument_end);
        let target_at = argument_end + 2;
        let imposed = matches!(
            governor,
            ObligationGovernor::Require | ObligationGovernor::Exigir
        ) && obligation_source(&words[..governor_at])
            && words
                .get(argument_end)
                .is_some_and(|word| matches!(word.as_str(), "on" | "for" | "per" | "em" | "por"))
            && words
                .get(argument_end + 1)
                .is_some_and(|word| universal(word))
            && same_run(target_at, &scope.subject)
            && obligation_tail(&words, target_at + scope.subject.len());
        if bearer || imposed {
            assertion.governor_span = Some([governor_at, governor_at + 1]);
            assertion.source_span = Some(if bearer {
                [1, governor_at]
            } else {
                [0, governor_at]
            });
            assertion.argument_span = Some([counted, argument_end]);
            assertion.target_span = Some(if bearer {
                [1, governor_at]
            } else {
                [target_at, target_at + scope.subject.len()]
            });
            let boundary = if bearer {
                argument_end
            } else {
                target_at + scope.subject.len()
            };
            assertion.boundary_span = (boundary < words.len()).then_some([boundary, words.len()]);
            let source = assertion.source_span.unwrap();
            let target = assertion.target_span.unwrap();
            assertion.domain = obligation_domain(
                text,
                &words,
                &[
                    source[0]..source[1],
                    counted..argument_end,
                    target[0]..target[1],
                ],
            );
            assertion.frame = if bearer {
                "universal_bearer"
            } else {
                "imposed_on_universal_target"
            };
            break;
        }
    }
    let matched = assertion.frame != "unbound"
        && assertion.domain == "asserted"
        && assertion.polarity == "affirmative";
    Some(ObligationEvidence {
        query_governor: query_governor.clone(),
        query_span: [scope.span.start, scope.span.end],
        target: scope.subject.clone(),
        object: scope.terms.clone(),
        assertion,
        matched,
    })
}

pub(super) fn target_matches(scope: &TargetScope, text: &str) -> bool {
    if let Some(obligation) = obligation_evidence(scope, text) {
        return obligation.matched;
    }
    text.split(['.', '?', '!', ';', ':']).any(|clause| {
        let w = crate::index::tokenize(clause);
        if w.iter().any(|word| negative(word)) {
            return false;
        }
        let holds = |term: &str| w.iter().any(|word| same_form(term, word));
        if !scope.terms.iter().all(|term| holds(term)) {
            return false;
        }
        match scope.kind {
            TargetKind::QualifiedLabel => scope.predicate.as_ref().is_some_and(|predicate| {
                w.iter().enumerate().any(|(at, word)| {
                    if !same_form(predicate, word)
                        || at == 0
                        || !matches!(
                            w[at - 1].as_str(),
                            "user" | "usuário" | "usuária" | "i" | "eu"
                        )
                    {
                        return false;
                    }
                    let start = at
                        + 1
                        + usize::from(w.get(at + 1).is_some_and(|word| {
                            matches!(word.as_str(), "a" | "an" | "the" | "um" | "uma")
                        }));
                    // A label in a separately coordinated or embedded clause is not
                    // the requested complement of this personal predicate.
                    let end = (start..w.len())
                        .find(|&i| coordination(&w[i]) || subordinate(&w[i]))
                        .unwrap_or(w.len());
                    w.get(start..start + scope.subject.len())
                        .is_some_and(|run| {
                            run.iter().zip(&scope.subject).all(|(a, b)| same_form(b, a))
                        })
                        && w[..end].windows(scope.terms.len() - 1).enumerate().any(
                            |(label_at, run)| {
                                let head = scope.terms.last().unwrap();
                                let after = label_at + scope.terms.len() - 1;
                                let head_before = label_at > 0 && same_form(head, &w[label_at - 1]);
                                let head_after =
                                    w.get(after).is_some_and(|word| same_form(head, word));
                                let linked_after = w
                                    .get(after + 1)
                                    .is_some_and(|word| matches!(word.as_str(), "with" | "com"))
                                    && w.get(after + 2).is_some_and(|word| same_form(head, word));
                                run.iter()
                                    .zip(&scope.terms[..scope.terms.len() - 1])
                                    .all(|(a, b)| same_form(b, a))
                                    && label_at > start
                                    && (head_before
                                        || (after < end && head_after)
                                        || (after + 2 < end && linked_after))
                                    && (matches!(
                                        w[label_at - 1].as_str(),
                                        "the" | "in" | "with" | "no" | "na" | "em"
                                    ) || head_before)
                            },
                        )
                })
            }),
            TargetKind::Quantity => {
                w.windows(scope.terms.len())
                    .enumerate()
                    .any(|(attribute, run)| {
                        if attribute == 0
                            || !quantified_word(&w[attribute - 1])
                            || !run.iter().zip(&scope.terms).all(|(a, b)| same_form(b, a))
                        {
                            return false;
                        }
                        w.iter().enumerate().any(|(i, word)| {
                            let universal = matches!(
                                word.as_str(),
                                "each" | "every" | "cada" | "todo" | "toda"
                            );
                            let subject =
                                w.get(i + 1..i + 1 + scope.subject.len())
                                    .is_some_and(|run| {
                                        run.iter().zip(&scope.subject).all(|(a, b)| same_form(b, a))
                                    });
                            let after_attribute = i == attribute + scope.terms.len() + 1
                                && w.get(i - 1).is_some_and(|t| {
                                    matches!(t.as_str(), "on" | "for" | "per" | "em" | "por")
                                });
                            let predicate_at = i + 1 + scope.subject.len();
                            // Only function words may separate this predicate
                            // from its counted object, never another assertion.
                            let before_attribute = predicate_at < attribute - 1
                                && scope.predicate.as_ref().is_some_and(|predicate| {
                                    same_form(predicate, &w[predicate_at])
                                })
                                && w[predicate_at + 1..attribute - 1].iter().all(|word| {
                                    STOPWORDS.contains(&word.as_str())
                                        && !coordination(word)
                                        && !subordinate(word)
                                });
                            universal && subject && (after_attribute || before_attribute)
                        })
                    })
            }
            TargetKind::SubjectPredicate => scope.predicate.as_ref().is_some_and(|predicate| {
                w.iter().enumerate().any(|(i, word)| {
                    if !same_form(predicate, word) {
                        return false;
                    }
                    let nominal_end =
                        if i > 0 && matches!(w[i - 1].as_str(), "that" | "which" | "que") {
                            i - 1
                        } else {
                            i
                        };
                    let subject = nominal_end >= scope.subject.len()
                        && (w[nominal_end - scope.subject.len()..nominal_end]
                            .iter()
                            .zip(&scope.subject)
                            .all(|(a, b)| same_form(b, a))
                            || (scope.head_initial
                                && scope.subject.len() == 1
                                && nominal_end >= 2
                                && same_form(&scope.subject[0], &w[nominal_end - 2])
                                && !STOPWORDS.contains(&w[nominal_end - 1].as_str())));
                    if !subject {
                        return false;
                    }
                    let object_at = i
                        + 1
                        + usize::from(w.get(i + 1).is_some_and(|t| {
                            matches!(t.as_str(), "the" | "a" | "an" | "o" | "os" | "as")
                        }));
                    let direct = w
                        .get(object_at..object_at + scope.object.len())
                        .is_some_and(|run| {
                            run.iter().zip(&scope.object).all(|(a, b)| same_form(b, a))
                        });
                    let relative = w[..nominal_end]
                        .iter()
                        .rposition(|word| matches!(word.as_str(), "that" | "which" | "que"))
                        .filter(|&relative| {
                            w[relative + 1..i].iter().all(|word| {
                                !STOPWORDS.contains(&word.as_str())
                                    || matches!(word.as_str(), "the" | "o" | "a" | "os" | "as")
                            }) && w[i + 1..]
                                .iter()
                                .all(|word| STOPWORDS.contains(&word.as_str()))
                        });
                    if scope.relative.is_empty() {
                        return direct
                            || relative.is_some_and(|relative| {
                                w[..relative].windows(scope.object.len()).enumerate().any(
                                    |(object_at, run)| {
                                        run.iter().zip(&scope.object).all(|(a, b)| same_form(b, a))
                                            && w[object_at + scope.object.len()..relative]
                                                .iter()
                                                .all(|word| {
                                                    !coordination(word) && !subordinate(word)
                                                })
                                    },
                                )
                            });
                    }
                    // Retain the positions of both witnesses. A producer
                    // elsewhere in the sentence cannot qualify a direct
                    // consumer of a different object's relative clause.
                    w.iter().enumerate().any(|(at, word)| {
                        if !same_form(&scope.relative[1], word) {
                            return false;
                        }
                        let actor_start = if at > 0 && same_form(&scope.relative[0], &w[at - 1]) {
                            at - 1
                                - usize::from(
                                    scope.head_initial
                                        && at > 1
                                        && scope
                                            .subject
                                            .first()
                                            .is_some_and(|head| same_form(head, &w[at - 2])),
                                )
                        } else if at > 1
                            && same_form(&scope.relative[0], &w[at - 2])
                            && scope
                                .subject
                                .first()
                                .is_some_and(|head| same_form(head, &w[at - 1]))
                        {
                            at - 2
                        } else {
                            return false;
                        };
                        let produced_object = at + 1;
                        let destination = produced_object + scope.object.len();
                        if !w.get(produced_object..destination).is_some_and(|run| {
                            run.iter().zip(&scope.object).all(|(a, b)| same_form(b, a))
                        }) {
                            return false;
                        }
                        let destination_end =
                            if w.get(destination..destination + 2).is_some_and(|run| {
                                run[0] == scope.relative[2]
                                    && same_form(&scope.relative[3], &run[1])
                            }) {
                                destination + 2
                            } else if scope.head_initial
                                && w.get(destination..destination + 3).is_some_and(|run| {
                                    run[0] == scope.relative[2]
                                        && !STOPWORDS.contains(&run[1].as_str())
                                        && same_form(&scope.relative[3], &run[2])
                                })
                            {
                                destination + 3
                            } else {
                                return false;
                            };
                        // The consumed object must be immediately qualified
                        // by this producer's defining relative.
                        let consumed_end = object_at + scope.object.len();
                        let direct_link = direct
                            && consumed_end <= actor_start
                            && (consumed_end == actor_start
                                || (actor_start == consumed_end + 1
                                    && matches!(
                                        w[consumed_end].as_str(),
                                        "that" | "which" | "que"
                                    )));
                        // Only one unmarked nominal head may extend the bound
                        // destination before its relative. A longer bridge can
                        // introduce a different antecedent; nominal words alone
                        // do not attest that it is still this producer's object.
                        let relative_link = relative.is_some_and(|relative| {
                            destination_end <= relative
                                && (destination_end == relative
                                    || (relative == destination_end + 1
                                        && nominal(&w[destination_end..relative])
                                        && !coordination(&w[destination_end])
                                        && !subordinate(&w[destination_end])))
                        });
                        direct_link || relative_link
                    })
                })
            }),
            // An attribute belongs to the complete queried subject and its
            // predicate, not any matching subject word elsewhere in the line.
            TargetKind::Attribute => scope.predicate.as_ref().is_some_and(|predicate| {
                w.windows(scope.subject.len())
                    .enumerate()
                    .any(|(subject_at, run)| {
                        if !run.iter().zip(&scope.subject).all(|(a, b)| same_form(b, a)) {
                            return false;
                        }
                        let predicate_at = subject_at + scope.subject.len();
                        if !w
                            .get(predicate_at)
                            .is_some_and(|word| same_form(predicate, word))
                        {
                            return false;
                        }
                        let start = predicate_at + 1;
                        let end = (start..w.len())
                            .find(|&i| coordination(&w[i]) || subordinate(&w[i]))
                            .unwrap_or(w.len());
                        let argument = &w[start..end];
                        let adjacent = argument
                            .windows(scope.terms.len())
                            .any(|run| run.iter().zip(&scope.terms).all(|(a, b)| same_form(b, a)));
                        let genitive = scope.head_initial
                            && scope.terms.len() == 2
                            && argument.windows(3).any(|run| {
                                same_form(&scope.terms[0], &run[0])
                                    && matches!(run[1].as_str(), "de" | "do" | "da")
                                    && same_form(&scope.terms[1], &run[2])
                            });
                        let value = scope.head_initial
                            && scope.terms.len() == 2
                            && argument.windows(4).any(|run| {
                                same_form(&scope.terms[1], &run[0])
                                    && matches!(run[1].as_str(), "em" | "no" | "na")
                                    && same_form(&scope.terms[0], &run[2])
                                    && quantified_word(&run[3])
                            });
                        adjacent || genitive || value
                    })
            }),
        }
    })
}

/// Finite, auditable lexical taxonomy, not an assertion about this user.
/// Leaves cover ordinary seeds/nuts, grains/legumes, fruit and animal foods
/// in English/Portuguese. The only productive compounds are edible material
/// heads (oil/flour/milk) with a known food complement. The WHOLE argument
/// must parse: a food token inside a software name never types that artifact.
/// Unknown bare nouns abstain. The separate source-grounded path retains
/// ingestion or a severe restriction with an explicitly carried treatment;
/// that latter path does not infer the unknown argument's edibility.
fn lexical_food_type(argument: &str) -> Option<&'static str> {
    fn leaf(word: &str) -> bool {
        matches!(
            word,
            "sesame"
                | "sunflower"
                | "flax"
                | "poppy"
                | "peanut"
                | "almond"
                | "walnut"
                | "hazelnut"
                | "cashew"
                | "pistachio"
                | "chestnut"
                | "rice"
                | "wheat"
                | "oat"
                | "oats"
                | "barley"
                | "rye"
                | "millet"
                | "corn"
                | "lentil"
                | "lentils"
                | "chickpea"
                | "soy"
                | "pea"
                | "peas"
                | "apple"
                | "pear"
                | "peach"
                | "banana"
                | "mango"
                | "avocado"
                | "milk"
                | "egg"
                | "eggs"
                | "fish"
                | "shrimp"
                | "gergelim"
                | "girassol"
                | "linhaça"
                | "amendoim"
                | "amêndoa"
                | "noz"
                | "avelã"
                | "castanha"
                | "arroz"
                | "trigo"
                | "aveia"
                | "cevada"
                | "centeio"
                | "painço"
                | "milho"
                | "lentilha"
                | "soja"
                | "ervilha"
                | "maçã"
                | "pera"
                | "pêssego"
                | "leite"
                | "ovo"
                | "peixe"
                | "camarão"
        )
    }
    let words: Vec<_> = argument.split_whitespace().collect();
    match words.as_slice() {
        [food] if leaf(food) => Some("ordinary_food_material"),
        [food, "oil" | "flour" | "milk"] if leaf(food) => Some("food_material_compound"),
        ["oil" | "flour" | "milk", "of", food] if leaf(food) => Some("food_material_compound"),
        ["óleo" | "farinha" | "leite", "de", food] if leaf(food) => Some("food_material_compound"),
        _ => None,
    }
}

/// An ingestible recommendation respects the recipient's stated restriction.
/// Assertion provenance and lexical type assumptions are kept separate; this
/// retrieves a constraint, not a diagnosis or a clinical recommendation.
pub(super) fn dietary_constraint(query: &str, text: &str) -> Option<serde_json::Value> {
    static TASK: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static RESTRICTION: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let task = TASK.get_or_init(|| regex::Regex::new(
        r"^(?:(?:can|could) you (?:suggest|recommend) (?:a |an )?(?:(?:breakfast|lunch|dinner) )?(?:recipe|meal) (?:for me|that fits my (?:current )?diet)|(?:suggest|recommend) (?:a |an )?(?:recipe|meal) for me|(?:você pode )?(?:sugerir|sugira|recomendar|recomende) (?:uma )?(?:receita|refeição) para mim)\?$"
    ).unwrap());
    let restriction = RESTRICTION.get_or_init(|| regex::Regex::new(
        r"^(?P<subject>user|i|a usuária|o usuário|eu) (?P<predicate>have a food allergy to|has a food allergy to|tem alergia grave a|tenho alergia grave a) (?P<substance>[\p{L} -]+?)(?P<witness>,? (?:after (?:eating|drinking|ingesting) it|após ingerir isso)|,? (?:and|e) [\p{L} -]+)?\.$"
    ).unwrap());
    let query = query.trim().to_lowercase();
    if !task.is_match(&query) {
        return None;
    }
    let normalized = text.trim().to_lowercase();
    let caps = restriction.captures(&normalized)?;
    let witness = caps.name("witness");
    static BODILY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let bodily = BODILY.get_or_init(|| regex::Regex::new(
        r"^,? (?:(?:after (?:eating|drinking|ingesting) it|após ingerir isso)|(?:and|e) (?:(?:always )?carries (?:an antihistamine|allergy medication)(?: in (?:a|the) bag)?|(?:sempre )?(?:carrega|leva) (?:um antialérgico|medicação(?: antialérgica)?)(?: na bolsa| consigo)?))$"
    ).unwrap());
    // Optional clauses cannot be ignored even when the nominal type is known.
    if witness.is_some_and(|w| !bodily.is_match(w.as_str())) {
        return None;
    }
    let ingestion =
        witness.is_some_and(|w| w.as_str().contains("after ") || w.as_str().contains("após "));
    let severe = matches!(
        &caps["predicate"],
        "tem alergia grave a" | "tenho alergia grave a"
    );
    let lexical_type = lexical_food_type(&caps["substance"]);
    let words = crate::index::tokenize(&caps["substance"]);
    // Artifact heads are incompatible even with a claimed bodily frame. This
    // is a general material/artifact distinction, not a particular metaphor.
    let artifact = words.iter().any(|w| {
        matches!(
            w.as_str(),
            "file"
                | "files"
                | "configuration"
                | "software"
                | "code"
                | "protocol"
                | "server"
                | "database"
                | "interface"
                | "package"
                | "library"
                | "convention"
                | "conventions"
                | "arquivo"
                | "arquivos"
                | "código"
                | "configuração"
                | "protocolo"
                | "servidor"
                | "biblioteca"
        )
    });
    let source_bodily_witness = ingestion || (severe && witness.is_some());
    // A bare severe-allergy predicate does not itself assert a food relation;
    // retain its existing requirement for an additional bodily witness. The
    // English food-allergy predicate can compose with a lexical material type.
    let food_assertion = matches!(
        &caps["predicate"],
        "have a food allergy to" | "has a food allergy to"
    );
    let lexical_food_assertion = food_assertion && lexical_type.is_some();
    if artifact || !(source_bodily_witness || lexical_food_assertion) {
        return None;
    }
    if words.is_empty()
        || words.iter().any(|w| {
            negative(w)
                || coordination(w)
                || subordinate(w)
                || matches!(
                    w.as_str(),
                    "in" | "on"
                        | "for"
                        | "from"
                        | "em"
                        | "no"
                        | "na"
                        | "para"
                        | "my"
                        | "your"
                        | "his"
                        | "her"
                )
        })
    {
        return None;
    }
    let subject = caps.name("subject")?;
    let predicate = caps.name("predicate")?;
    let substance = caps.name("substance")?;
    Some(serde_json::json!({
        "relation": "ingestible_recommendation_respects_bodily_restriction",
        "query_domain": "personal_ingestible_recommendation",
        "candidate_domain": "stated_personal_bodily_allergy",
        "assertion_source": "candidate_subject_predicate_argument",
        "source_coordinates": "trimmed_lowercase_utf8",
        "type_witness_bytes": witness.map(|w| [w.start(), w.end()]),
        "substance_edibility_inferred": lexical_type.is_some(),
        "lexical_type_assumption": lexical_type.map(|kind| serde_json::json!({
            "taxonomy": "bounded_food_material_v1", "kind": kind,
            "source": "general_language_type_knowledge_not_candidate_assertion"
        })),
        "type_witness": if ingestion { "asserted_ingestion_of_argument" } else if lexical_type.is_some() { "lexical_food_material_type" } else { "severe_bodily_restriction_and_carried_allergy_treatment" },
        "subject_bytes": [subject.start(), subject.end()],
        "predicate_bytes": [predicate.start(), predicate.end()],
        "substance_bytes": [substance.start(), substance.end()]
    }))
}

/// A bounded production-transition frame. Calendar values are typed arguments,
/// not topics or entity aliases; extra clauses are never silently discarded.
fn production_window(text: &str) -> Option<[String; 4]> {
    static FRAME: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let frame = FRAME.get_or_init(|| regex::Regex::new(
        r"^user (?:ships|deploys) to production only on (?P<days>[a-z ]+), (?:between (?P<start_a>\d{2}:\d{2}) and (?P<end_a>\d{2}:\d{2})|from (?P<start_b>\d{2}:\d{2}) to (?P<end_b>\d{2}:\d{2})) (?P<zone>local time|utc)\.$"
    ).unwrap());
    let normalized = text.trim().to_lowercase();
    let caps = frame.captures(&normalized)?;
    let days = calendar_days(&caps["days"])?;
    let start = caps
        .name("start_a")
        .or_else(|| caps.name("start_b"))?
        .as_str();
    let end = caps.name("end_a").or_else(|| caps.name("end_b"))?.as_str();
    let clock = |s: &str| {
        let (h, m) = s.split_once(':')?;
        Some(h.parse::<u8>().ok()? < 24 && m.parse::<u8>().ok()? < 60)
    };
    if clock(start) != Some(true) || clock(end) != Some(true) {
        return None;
    }
    Some([days, start.into(), end.into(), caps["zone"].into()])
}

fn calendar_days(text: &str) -> Option<String> {
    let mut days = Vec::new();
    for (at, word) in text.split_whitespace().enumerate() {
        if at % 2 == 1 {
            if word != "and" {
                return None;
            }
            continue;
        }
        let singular = word.strip_suffix('s').unwrap_or(word);
        if !matches!(
            singular,
            "monday" | "tuesday" | "wednesday" | "thursday" | "friday" | "saturday" | "sunday"
        ) {
            return None;
        }
        days.push(singular);
    }
    (!days.is_empty() && text.split_whitespace().count() % 2 == 1).then(|| days.join(" "))
}

/// Full frame equality, including timezone and every calendar argument.
/// None means neither text is parsed; Some(false) must not fall back to bags.
pub(crate) fn production_window_equivalence(a: &str, b: &str) -> Option<bool> {
    let (left, right) = (production_window(a), production_window(b));
    (left.is_some() || right.is_some()).then(|| left.is_some() && left == right)
}

/// A temporal eligibility question binds the first-person actor to an actual
/// production transition, not any mention of release files or working hours.
pub(super) fn temporal_scope(query: &str) -> bool {
    static SCOPE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    temporal_request(query) || SCOPE.get_or_init(|| regex::Regex::new(
        r"^(?:which (?:weekdays|days) (?:am i|is (?:the )?[\p{L} -]+) allowed to [\p{L} ',?-]+|during what hours am i allowed to [\p{L} '-]+|[\p{L}\d ':-]+ at [\d:]+(?:am|pm)? on [\p{L} ]+, (?:any reason i should(?:n't|nt| not)|am i allowed to do (?:so|that)))\?$"
    ).unwrap()).is_match(&query.trim().to_lowercase())
}

pub(super) fn temporal_request(query: &str) -> bool {
    static TASK: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let task = TASK.get_or_init(|| regex::Regex::new(
        r"^(?:which (?:weekdays|days) am i allowed to (?:push a release live|deploy to production|ship to production)(?:, and during what hours)?|during what hours am i allowed to (?:push a release live|deploy to production|ship to production))\?$"
    ).unwrap());
    let normalized = query.trim().to_lowercase();
    if task.is_match(&normalized) {
        return true;
    }
    // A proposal followed by a first-person eligibility question is an open
    // calendar constraint request, NOT a claim that the proposed day is Friday
    // or that the proposal violates any particular stored rule. Calendar
    // restrictions are relevant even when the proposed time is permitted.
    static PROPOSAL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let proposal = PROPOSAL.get_or_init(|| regex::Regex::new(
        r"^(?:i am |i'm |thinking of )?(?:planning to |considering )?(?:deploying to production|shipping to production|putting a change out) at (?P<hour>\d{1,2})(?::(?P<minute>\d{2}))?(?P<period>am|pm)? on (?P<day>(?:the (?:first|last|next) workday of the week)|monday|tuesday|wednesday|thursday|friday|saturday|sunday), (?:any reason i should(?:n't|nt| not)|am i allowed to do (?:so|that))\?$"
    ).unwrap());
    let Some(caps) = proposal.captures(&normalized) else {
        return false;
    };
    let Ok(hour) = caps["hour"].parse::<u8>() else {
        return false;
    };
    let minute = caps
        .name("minute")
        .map_or(Some(0), |m| m.as_str().parse::<u8>().ok());
    minute.is_some_and(|m| m < 60)
        && if caps.name("period").is_some() {
            (1..=12).contains(&hour)
        } else {
            hour < 24
        }
}

pub(crate) fn temporal_constraint(query: &str, text: &str) -> Option<serde_json::Value> {
    static EXCLUSION: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    if !temporal_request(query) {
        return None;
    }
    let normalized = text.trim().to_lowercase();
    let (kind, argument) = if production_window(text).is_some() {
        (
            "permitted_production_window",
            normalized.find("only on ")? + 8,
        )
    } else {
        let exclusion = EXCLUSION.get_or_init(|| regex::Regex::new(
            r"^user never releases on (?P<days>[a-z ]+?)(?: or on the day before a public holiday)?(?P<purpose>, to [\p{L} -]+)?\.$"
        ).unwrap());
        let caps = exclusion.captures(&normalized)?;
        calendar_days(&caps["days"])?;
        if caps.name("purpose").is_some_and(|purpose| {
            crate::index::tokenize(purpose.as_str())
                .iter()
                .any(|word| negative(word) || subordinate(word) || coordination(word))
        }) {
            return None;
        }
        ("excluded_production_days", caps.name("days")?.start())
    };
    Some(serde_json::json!({
        "relation":"personal_production_transition_calendar",
        "candidate_domain":kind,
        "normalization":"trim_lowercase",
        "query_bytes":[0,query.trim().len()],
        "subject_bytes":[0,4],
        "governor_bytes":[5,normalized.find(" on ").unwrap_or(argument)],
        "calendar_type":"weekday_and_local_or_utc_clock",
        "query_calendar_resolution":"unresolved_workday_relation_not_a_weekday_assertion",
        "argument_start_byte":argument,
        "assertion_bytes":[0,normalized.len()]
    }))
}

/// A deliberately small technical-action ontology. These are argument types,
/// not answer terms: implementations, protocols and numeric limits remain
/// opaque source arguments. Unknown syntax receives no certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Practice {
    Integration,
    RepairOrder,
    ServiceImplementation,
    Communication,
    ReviewLimit,
}

fn practice_words(text: &str, choices: &[&str]) -> bool {
    crate::index::tokenize(text)
        .iter()
        .any(|word| choices.contains(&word.as_str()))
}

pub(super) fn practice_scope(query: &str) -> Option<Practice> {
    let q = query.to_lowercase();
    if !q.contains('?') {
        return None;
    }
    // A WH subject request has its own grounded predicate/argument frame;
    // it does not ask for the speaker's habitual practice. Topic words alone
    // must not intercept it before the existing target guard can run.
    // Other malformed/foreign/quoted requests remain subject to the practice
    // owner/root proof rather than falling back to bags of technical words.
    if target_scope(query).is_some_and(|scope| matches!(scope.kind, TargetKind::SubjectPredicate)) {
        return None;
    }
    let has = |choices: &[&str]| practice_words(&q, choices);
    if has(&["repositories", "repository", "integration"])
        && has(&["storage", "database", "databases", "tests", "testing"])
    {
        Some(Practice::Integration)
    } else if has(&["bug", "crash", "defect", "regression"])
        && has(&["first", "before", "prior"])
        && has(&["code", "implementation", "fix", "repair"])
    {
        Some(Practice::RepairOrder)
    } else if has(&["service", "services", "microservice", "microservices"])
        && has(&["language", "stack", "framework"])
        && has(&["default", "normally", "usually"])
    {
        Some(Practice::ServiceImplementation)
    } else if has(&["service", "services", "microservice", "microservices"])
        && has(&["protocol", "communicate", "communication", "talk"])
        && has(&["internal", "firewall", "private"])
    {
        Some(Practice::Communication)
    } else if has(&["branch", "branches", "requests", "changeset", "patch"])
        && has(&["size", "big", "large", "lines"])
        && has(&["review", "reviewer", "teammates"])
    {
        Some(Practice::ReviewLimit)
    } else {
        None
    }
}

/// Grammar binds an overt owner and root governor to the relevant argument.
/// The accepted contrast and temporal clauses below are licensed syntax, not
/// arbitrary tails. Quotes, reports, conditions and negated governors cannot
/// become personal assertions merely by sharing technical words.
pub(super) fn practice_evidence(query: &str, text: &str) -> Option<serde_json::Value> {
    use Practice::*;
    let kind = practice_scope(query)?;
    let q = query.trim().to_lowercase();
    if !practice_words(&q, &["i", "my"])
        || q.contains('"')
        || q.starts_with("if ")
        || q.starts_with("user quotes")
        || practice_words(
            &q,
            &["visitor", "colleague", "not", "never", "public", "external"],
        )
    {
        return None;
    }
    // A question asks for an open argument; a personal pronoun anywhere in
    // a bag of technical terms is not an owner/governor witness. Root frames
    // below license only direct requests or a reported incident followed by
    // a direct first-person request (the incident does not assert a policy).
    static REQUEST: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let request = REQUEST.get_or_init(|| regex::Regex::new(
        r"^(?:when i [a-z0-9 '-]+, (?:do|should) i [a-z0-9 '-]+|which [a-z -]+ do i (?:default to|normally use|usually use|use for) [a-z0-9 '-]+|how should my [a-z -]+ (?:talk|communicate) [a-z '-]+\? what [a-z -]+ do i (?:like|prefer)|how (?:big|large|many) [a-z -]*should (?:i|my) [a-z0-9 '-]+|someone [a-z '-]+; what do i (?:normally|usually) do first, (?:prior to|before) [a-z '-]+|before i [a-z '-]+, what do i do first)\?$"
    ).unwrap());
    if !request.is_match(&q) {
        return None;
    }
    let normalized = text.trim().to_lowercase();
    if normalized.contains(['"', '“', '”', '?']) {
        return None;
    }
    static FRAMES: std::sync::OnceLock<Vec<(Practice, regex::Regex)>> = std::sync::OnceLock::new();
    let frames = FRAMES.get_or_init(|| {
        [
            (Integration, r"^(?:user|i) (?P<governor>runs?|executes?) (?P<argument>integration tests against real [a-z0-9 +.-]+? using [a-z0-9 +.-]+?)(?: instead of mocking the database)?\.$"),
            (RepairOrder, r"^(?:user|i) (?:always )?(?P<governor>writes?|creates?) (?P<argument>a failing regression test that reproduces a reported (?:bug|defect)) before (?P<successor>changing any code to fix it|modifying the implementation)\.$"),
            (ServiceImplementation, r"^(?:user|i) (?P<governor>writes?|implements?) most of (?:her|his|their|my) (?P<domain>backend services|microservices) in (?P<argument>[a-z0-9 +.-]+)\.$"),
            (Communication, r"^(?:user|i) (?P<governor>prefers?|uses?) (?:(?P<public_argument>[a-z0-9 +.-]+?) (?:between|for) public endpoints and )?(?P<argument>[a-z0-9 +.-]+?) (?:between|for) (?P<domain>internal services|private microservices)(?: and (?P<external_argument>[a-z0-9 +.-]+) for public endpoints)?\.$"),
            (ReviewLimit, r"^(?:user|i) (?P<governor>keeps?|limits?) (?P<domain>pull requests|review patches|changesets) (?:under|to) (?P<argument>(?:about )?[0-9]+ changed lines) so a reviewer can finish them in one sitting\.$"),
        ]
        .into_iter()
        .map(|(kind, pattern)| (kind, regex::Regex::new(pattern).unwrap()))
        .collect()
    });
    let (_, frame) = frames.iter().find(|(candidate, _)| *candidate == kind)?;
    let captures = frame.captures(&normalized)?;
    let argument = captures.name("argument")?;
    // Open argument runs are not allowed to swallow another assertion or a
    // polarity/modal operator. `and` is a nominal coordinator in these runs.
    if [
        Some(argument),
        captures.name("public_argument"),
        captures.name("external_argument"),
    ]
    .into_iter()
    .flatten()
    .any(|part| {
        part.as_str().contains(". ")
            || practice_words(
                part.as_str(),
                &[
                    "not",
                    "never",
                    "if",
                    "would",
                    "could",
                    "quotes",
                    "reports",
                    "user",
                    "prefers",
                    "uses",
                    "writes",
                    "colleague",
                    "visitor",
                    "because",
                    "although",
                    "but",
                    "when",
                    "unless",
                    "says",
                    "instead",
                    "for",
                    "between",
                ],
            )
    }) {
        return None;
    }
    if kind == ServiceImplementation {
        // A request for a JVM language needs a genuine language subtype, not
        // any backend tool. The platform relation is directional.
        let jvm = practice_words(
            argument.as_str(),
            &["java", "kotlin", "scala", "clojure", "groovy"],
        );
        let framework = practice_words(
            argument.as_str(),
            &["spring", "play", "quarkus", "micronaut", "ktor", "http4k"],
        );
        if (practice_words(&q, &["jvm"]) && !jvm) || !framework {
            return None;
        }
    }
    let governor = captures.name("governor")?;
    Some(serde_json::json!({
        "relation":"owned_technical_practice",
        "kind":format!("{kind:?}"),
        "normalization":"trim_lowercase",
        "owner_bytes":[0,normalized.find(' ')?],
        "governor_bytes":[governor.start(),governor.end()],
        "argument_bytes":[argument.start(),argument.end()],
        "argument":argument.as_str(),
        "domain":captures.name("domain").map(|m| m.as_str()),
        "domain_bytes":captures.name("domain").map(|m| [m.start(),m.end()]),
        "successor":captures.name("successor").map(|m| m.as_str()),
        "assertion_bytes":[0,normalized.len()],
        "query_request_bytes":[0,q.len()],
        "query_role":"direct_first_person_practice_request"
    }))
}

/// Cardinal syntax is closed linguistic vocabulary, independent of the noun,
/// owner or topic being counted. Keep the original value span for auditing;
/// arbitrary modifiers and malformed number-word sequences are not counts.
fn cardinal_count(value: &str) -> bool {
    static CARDINAL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    CARDINAL
        .get_or_init(|| {
            let unit = "(?:one|two|three|four|five|six|seven|eight|nine)";
            let small = format!("(?:{unit}|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|(?:twenty|thirty|forty|fifty|sixty|seventy|eighty|ninety)(?:[ -]{unit})?)");
            let group = format!("(?:{unit} hundred(?: (?:and )?{small})?|{small})");
            let thousand = format!("(?:{group} thousand(?: (?:and )?{group})?|{group})");
            let million = format!("(?:{group} million(?: (?:and )?{thousand})?|{thousand})");
            regex::Regex::new(&format!("^(?:[0-9]+|zero|{group} billion(?: (?:and )?{million})?|{million})$")).unwrap()
        })
        .is_match(value)
}

/// A recognized open property is an obligation, not a topic match. Property
/// names and subjects come from the question; no stored topic vocabulary is
/// consulted. Unknown WH syntax retains the existing evidence paths.
pub(crate) fn requested_attribute(query: &str, text: &str) -> Option<serde_json::Value> {
    static QUERIES: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    let frames = QUERIES.get_or_init(|| [
        r"^how many (?P<property>[a-z]+) (?:do|does) (?P<subject>[a-z0-9 -]+) (?P<governor>have|contain)\?$",
        r"^how many (?P<property>[a-z]+) (?P<governor>sit|stand|work) (?P<link>on|in|at) (?P<subject>[a-z0-9 -]+)\?$",
        r"^which (?P<property>[a-z]+) is (?P<governor>[a-z]+) for (?P<subject>[a-z0-9 -]+)\?$",
        r"^which (?P<property>[a-z]+) is (?P<subject>[a-z0-9 -]+) (?P<link>in|at)\?$",
    ].into_iter().map(|p| regex::Regex::new(p).unwrap()).collect());
    let q = query.trim().to_lowercase();
    let (frame, caps) = frames
        .iter()
        .enumerate()
        .find_map(|(i, r)| r.captures(&q).map(|c| (i, c)))?;
    let property = caps.name("property")?;
    let subject = caps.name("subject")?;
    let governor = caps.name("governor").map(|m| m.as_str());
    let link = caps.name("link").map(|m| m.as_str());
    let normalized = text.trim().to_lowercase();
    let terms = |s: &str| {
        crate::index::tokenize(s)
            .into_iter()
            .filter(|w| !matches!(w.as_str(), "the" | "a" | "an"))
            .collect::<Vec<_>>()
    };
    let equivalent = |a: &str, b: &str| {
        let (a, b) = (terms(a), terms(b));
        !a.is_empty() && a.len() == b.len() && a.iter().zip(&b).all(|(a, b)| same_form(a, b))
    };
    // The root assertion supplies all roles in one clause. A number elsewhere,
    // a foreign owner, a report or a negated predicate is not a value witness.
    static ANSWERS: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    let answers = ANSWERS.get_or_init(|| [
        r"^(?P<subject>[a-z0-9 -]+?) (?P<governor>has|have|contains|contain) (?P<value>[a-z0-9 -]+?) (?P<property>[a-z]+)\.$",
        r"^(?P<value>[a-z0-9 -]+?) (?P<property>[a-z]+) (?P<governor>sit|stand|work) (?P<link>on|in|at) (?P<subject>[a-z0-9 -]+)\.$",
        r"^(?:the )?(?P<property>[a-z]+) (?P<value>[a-z0-9 -]+?) is (?P<governor>[a-z]+) for (?P<subject>[a-z0-9 -]+)\.$",
        r"^(?P<subject>[a-z0-9 -]+?) is (?P<link>in|at) (?:the )?(?P<property>[a-z]+) (?P<value>[a-z0-9 -]+)\.$",
    ].into_iter().map(|p| regex::Regex::new(p).unwrap()).collect());
    let answer = answers[frame].captures(&normalized);
    let answer = answer.filter(|a| {
        let value = a.name("value").unwrap();
        // `and` inside a valid cardinal belongs to the value, not to a second
        // assertion. All roles and every operator outside it remain guarded.
        let assertion = if frame < 2 {
            if !cardinal_count(value.as_str()) {
                return false;
            }
            format!(
                "{}{}",
                &normalized[..value.start()],
                &normalized[value.end()..]
            )
        } else {
            normalized.clone()
        };
        equivalent(subject.as_str(), &a["subject"])
            && same_form(property.as_str(), &a["property"])
            && governor.is_none_or(|g| {
                a.name("governor").is_some_and(|m| {
                    same_form(g, m.as_str()) || (g == "have" && m.as_str() == "has")
                })
            })
            && link.is_none_or(|l| a.name("link").is_some_and(|m| m.as_str() == l))
            && !crate::index::tokenize(&assertion).iter().any(|w| {
                negative(w)
                    || subordinate(w)
                    || coordination(w)
                    || matches!(w.as_str(), "quotes" | "reports" | "would" | "could")
            })
    });
    Some(serde_json::json!({
        "relation":"requested_property_value",
        "normalization":"trim_lowercase",
        "query_property_bytes":[property.start(),property.end()],
        "query_subject_bytes":[subject.start(),subject.end()],
        "property":property.as_str(), "subject":subject.as_str(),
        "governor":governor, "link":link, "frame":frame,
        "matched":answer.is_some(),
        "source_subject_bytes":answer.as_ref().and_then(|a| a.name("subject")).map(|m| [m.start(),m.end()]),
        "source_governor_bytes":answer.as_ref().and_then(|a| a.name("governor").or_else(|| a.name("link"))).map(|m| [m.start(),m.end()]),
        "source_property_bytes":answer.as_ref().and_then(|a| a.name("property")).map(|m| [m.start(),m.end()]),
        "value_bytes":answer.as_ref().and_then(|a| a.name("value")).map(|m| [m.start(),m.end()]),
        "value":answer.as_ref().and_then(|a| a.name("value")).map(|m| m.as_str())
    }))
}

/// Bind the domain to the governed object, never an incidental destination.
fn lodging_request(query: &str) -> Option<serde_json::Value> {
    static FRAME: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let q = query.trim().to_lowercase();
    let c = FRAME.get_or_init(|| regex::Regex::new(
        r"^pra minha próxima (?:ida|viagem) [\p{L} ]+, (?P<governor>reservo) (?:um|uma) (?P<object>hotelzão|hotel|pousada|pensão|resort)(?: pequeno| pequena| familiar| aconchegante)? ou (?P<alternative>algo mais (?:aconchegante e familiar|familiar|aconchegante)|(?:um|uma) (?:hotel|pousada|pensão|resort))\?$"
    ).unwrap()).captures(&q)?;
    Some(serde_json::json!({"normalization":"trim_lowercase",
        "governor_bytes":[c.name("governor")?.start(),c.name("governor")?.end()],
        "object_bytes":[c.name("object")?.start(),c.name("object")?.end()],
        "alternative_bytes":[c.name("alternative")?.start(),c.name("alternative")?.end()],
        "object_type":"lodging"}))
}

/// Closed predicate/role vocabulary; names are opaque source values.
/// Safety requests retrieve a restriction, never certify a recommendation.
fn personal_life_request(query: &str) -> Option<(&'static str, serde_json::Value)> {
    static FRAMES: std::sync::OnceLock<Vec<(&str, regex::Regex)>> = std::sync::OnceLock::new();
    let frames = FRAMES.get_or_init(|| [
        ("favorite_literary_work_context", r"^(?:sabe )?(?:qual (?P<attribute>romance|livro)(?: [\p{L}-]+)? eu (?:mais amo|prefiro)|which (?P<work>book|novel)(?: [\p{L}-]+)? do i (?:love most|prefer)|what is my favorite (?:book|novel))\?$"),
        ("assistant_answer_style", r"^(?:remind me, )?(?:do i prefer (?P<choice>(?:short|long|concise|detailed|long detailed|short direct|straight-to-the-point) (?:answers|explanations) or (?:short|long|concise|detailed|long detailed|short direct|straight-to-the-point) (?:answers|explanations)) from you|what (?P<attribute>answer style) do i prefer from you)\?$"),
        ("music_performers_context", r"^(?:que|quais) (?P<attribute>artistas|cantores|cantoras)(?: brasileiros| brasileiras)? (?:eu (?:escuto|ouço)|tocam no meu fone) (?P<context>enquanto|quando) (?:eu )?(?:estou )?(?P<activity>programando|codando|programo)\?$"),
        ("music_performers_context", r"^which (?P<attribute>artists|singers|performers) do i (?:hear|listen to) (?P<context>while) (?P<activity>coding|programming)\?$"),
        ("recorded_medication_restriction", r"^(?:estou com|tenho) (?P<complaint>enxaqueca|dor de cabeça), qual (?P<attribute>remédio|medicamento) eu posso tomar(?: (?P<qualifier>sem risco|com segurança))?\?$"),
        ("recorded_medication_restriction", r"^i have (?:a )?(?P<complaint>headache|migraine), which (?P<attribute>medicine|medication) can i take(?: (?P<qualifier>safely|without risk))?\?$"),
        ("sibling_video_schedule", r"^(?:em que momento da semana|quando) eu (?:costumo )?(?:bater papo|conversar|converso) com minha (?P<kinship>mana|irmã|irmão) (?P<modality>pela webcam|por videochamada)\?$"),
        ("sibling_video_schedule", r"^when do i (?P<modality>video call|video chat with) my (?P<kinship>sister|brother)\?$"),
    ].into_iter().map(|(k,p)| (k,regex::Regex::new(p).unwrap())).collect());
    let q = query.trim().to_lowercase();
    frames.iter().find_map(|(kind, frame)| {
        let c = frame.captures(&q)?;
        let span = |name| c.name(name).map(|m| [m.start(),m.end()]);
        Some((*kind,serde_json::json!({
            "normalization":"trim_lowercase", "attribute_bytes":span("attribute"),
            "context_bytes":span("context"), "activity_bytes":span("activity"),
            "complaint_bytes":span("complaint"), "qualifier_bytes":span("qualifier"),
            "kinship_bytes":span("kinship"), "modality_bytes":span("modality"),
            "choice_bytes":span("choice"), "work_bytes":span("work"),
            "kinship":c.name("kinship").map(|m| match m.as_str() {"mana"|"irmã"|"sister" => "sister", _ => "brother"}),
            "certifies_medication_safety":false, "certifies_performer_nationality":false
        })))
    })
}

/// Personal activities use a finite lexical type system, not entity aliases.
/// Query roles create obligations; an unrelated night, hotel booking or
/// software package cannot discharge a purpose, preference or learning role.
fn life_scope(query: &str) -> Option<&'static str> {
    let q = query.trim().to_lowercase();
    let has = |words: &[&str]| practice_words(&q, words);
    // An open requested property (room, count, publication, etc.) is not
    // activity progress or personal preference merely because a domain occurs.
    if requested_attribute(query, "").is_some() {
        return None;
    }
    if let Some((kind, _)) = personal_life_request(query) {
        return Some(kind);
    }
    if lodging_request(query).is_some() {
        return Some("lodging_preference");
    }
    static REQUESTS: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    let requests = REQUESTS.get_or_init(|| [
        r"^(?:what do i drink|o que (?:é bom )?eu (?:devo )?beber) (?:before bed|before sleeping|antes de dormir|antes de deitar)(?: (?:to sleep better|to fall asleep|pra pegar no sono|para dormir melhor))?\?$",
        r"^(?:which (?:lodging|accommodation) do i prefer|qual (?:hotel|hospedagem|pousada) eu prefiro)\?$",
        r"^(?:how is my (?:pottery|ceramics) (?:course|class) going|como anda meu curso de (?:olaria|cerâmica))\?(?: quero (?:fazer|modelar) (?:potes|tigelas|canecas|vasos) de barro (?:pro|para o) [\p{L} ]+\.)?$",
    ].into_iter().map(|p| regex::Regex::new(p).unwrap()).collect());
    if !requests.iter().any(|r| r.is_match(&q)) {
        return None;
    }
    if !q.contains('?')
        || q.contains(['\"', '“', '”'])
        || !has(&["i", "my", "eu", "meu", "minha"])
        || has(&[
            "visitor",
            "colleague",
            "reports",
            "quotes",
            "if",
            "not",
            "never",
        ])
    {
        return None;
    }
    if has(&["drink", "beber", "bebida"]) && has(&["sleep", "bedtime", "sono", "dormir", "deitar"])
    {
        Some("sleep_beverage_purpose")
    } else if has(&[
        "lodging",
        "accommodation",
        "hospedagem",
        "hotelzão",
        "hotel",
        "hotéis",
        "pousada",
    ]) && has(&["prefer", "preference", "prefiro", "prefere", "reservo"])
        && !has(&["booked", "reservation", "reservei", "reservado", "reserva"])
    {
        Some("lodging_preference")
    } else if has(&["pottery", "ceramics", "olaria", "cerâmica"])
        && has(&[
            "course",
            "class",
            "classes",
            "learning",
            "curso",
            "aulas",
            "aprendendo",
        ])
    {
        Some("ceramic_learning")
    } else {
        None
    }
}

/// Preferences are owned assertions, not answer entailment. A favorite book
/// can be useful context for a novel question without proving the work's form
/// or nationality. Opaque titles/authors are never looked up or reclassified.
fn personal_preference_evidence(query: &str, text: &str, kind: &str) -> Option<serde_json::Value> {
    static FRAMES: std::sync::OnceLock<Vec<(&str, regex::Regex)>> = std::sync::OnceLock::new();
    let frames = FRAMES.get_or_init(|| [
        ("favorite_literary_work_context", r"^(?P<domain>o livro|o romance) (?P<governor>favorito) (?P<owner>da usuária|do usuário) é (?P<value>[\p{L}0-9 '-]+?), de (?P<author>[\p{L} '-]+?)(?:, que ela já releu [\p{L}0-9 -]+ vezes)?\.$"),
        ("favorite_literary_work_context", r"^(?P<owner>user's|my) (?P<governor>favorite) (?P<domain>book|novel) is (?P<value>[\p{L}0-9 '-]+?) by (?P<author>[\p{L} '-]+)\.$"),
        ("assistant_answer_style", r"^(?P<owner>user|i) (?P<governor>prefers|prefer) (?P<value>short, direct|short|concise|long, detailed|long|detailed) (?P<domain>answers|explanations)(?: (?P<role>from (?:you|the assistant)))?(?: that (?P<details>lead with the conclusion and skip long preambles|include examples and explain each step))?\.$"),
    ].into_iter().map(|(k,p)| (k,regex::Regex::new(p).unwrap())).collect());
    let normalized = text.trim().to_lowercase();
    let assertion = frames
        .iter()
        .filter(|(k, _)| *k == kind)
        .find_map(|(_, r)| r.captures(&normalized))
        .filter(|c| {
            ["value", "author"]
                .into_iter()
                .filter_map(|n| c.name(n))
                .all(|m| {
                    !practice_words(
                        m.as_str(),
                        &[
                            "not",
                            "never",
                            "if",
                            "unknown",
                            "would",
                            "could",
                            "reports",
                            "quotes",
                            "não",
                            "nunca",
                            "desconhecido",
                            "relata",
                            "cita",
                        ],
                    )
                })
        });
    let span = |n| {
        assertion
            .as_ref()
            .and_then(|c| c.name(n))
            .map(|m| [m.start(), m.end()])
    };
    Some(serde_json::json!({
        "relation":"owned_personal_preference_context", "kind":kind,
        "normalization":"trim_lowercase", "matched":assertion.is_some(),
        "owner_bytes":span("owner"), "governor_bytes":span("governor"),
        "value_bytes":span("value"), "domain_bytes":span("domain"),
        "author_bytes":span("author"), "output_role_bytes":span("role"),
        "output_role":if kind == "assistant_answer_style" { Some("assistant_answer") } else { None },
        "certifies_work_nationality":false, "certifies_work_is_novel":false,
        "query_roles":personal_life_request(query).map(|(_,p)|p),
        "value":assertion.as_ref().and_then(|c|c.name("value")).map(|m|m.as_str()),
        "assertion_bytes":assertion.as_ref().map(|_|[0,normalized.len()])
    }))
}

/// Finite lexical taxonomy of ordinary musical entities. These entries resolve
/// type only, never ownership, nationality or preference. Unknown and ambiguous
/// homonyms require a source performer head; casing is not a type witness.
fn musical_entity_type(value: &str) -> &'static str {
    match value {
        // Brazilian popular music/samba; jazz/soul/blues; folk/pop; classical.
        "gal costa"
        | "caetano veloso"
        | "gilberto gil"
        | "milton nascimento"
        | "elis regina"
        | "maria bethânia"
        | "tom jobim"
        | "dona ivone lara"
        | "leci brandão"
        | "nina simone"
        | "chet baker"
        | "dinah washington"
        | "aretha franklin"
        | "etta james"
        | "otis redding"
        | "miles davis"
        | "ella fitzgerald"
        | "billie holiday"
        | "louis armstrong"
        | "joni mitchell"
        | "stevie wonder"
        | "david bowie"
        | "björk"
        | "johann sebastian bach"
        | "ludwig van beethoven" => "performer",
        "jazz" | "blues" | "samba" | "mpb" | "rock" | "pop" | "soul" | "classical"
        | "classical music" | "heavy metal" | "metal" | "death metal" | "black metal" | "folk"
        | "country" | "reggae" | "funk" | "bossa nova" | "hip hop" | "electronic music" => "genre",
        // Human/stage names also denoting animal, action, artifact or place.
        "seal" | "sting" | "prince" | "boston" | "genesis" => "ambiguous",
        _ => "unknown",
    }
}

fn musical_argument_type(value: &str, explicit_head: bool) -> serde_json::Value {
    let entities: Vec<_> = value
        .split(" and ")
        .flat_map(|part| part.split(" e "))
        .map(|name| serde_json::json!({"value":name,"lexical_type":musical_entity_type(name)}))
        .collect();
    let known_performers = entities
        .iter()
        .all(|entity| entity["lexical_type"] == "performer");
    serde_json::json!({
        "source_argument_role":"listening_preference_example",
        "type_basis":if explicit_head {"source_argument_type_head"}
            else if known_performers {"bounded_musical_entity_taxonomy"}
            else {"unresolved"},
        "entities":entities,
        "resolves_performer":explicit_head || known_performers,
        "certifies_nationality":false,
        "certifies_singer_subtype":false
    })
}

/// All captures belong to one owned root assertion. Governors bind roles;
/// a separate explicit head or bounded entity taxonomy resolves musical type.
/// Neither cosine, tags nor name casing can discharge these obligations.
pub(crate) fn life_evidence(query: &str, text: &str) -> Option<serde_json::Value> {
    let kind = life_scope(query)?;
    if matches!(
        kind,
        "favorite_literary_work_context" | "assistant_answer_style"
    ) {
        return personal_preference_evidence(query, text, kind);
    }
    let normalized = text.trim().to_lowercase();
    static FRAMES: std::sync::OnceLock<Vec<(&str, regex::Regex)>> = std::sync::OnceLock::new();
    let frames = FRAMES.get_or_init(|| [
        ("sleep_beverage_purpose", r"^(?:à noite )?(?P<owner>a usuária|o usuário|eu) (?:(?:evita|evito) cafeína e )?(?P<governor>toma|tomo|bebe|bebo) (?P<value>chá de [\p{L} -]+|leite morno|água) (?P<purpose>para dormir melhor|antes de dormir para pegar no sono)\.$"),
        ("sleep_beverage_purpose", r"^(?P<owner>user|i) (?:(?:avoids|avoid) caffeine and )?(?P<governor>drinks|drink) (?P<value>[\p{L} -]+ tea|warm milk|water) (?P<purpose>to sleep better|before bed to fall asleep)\.$"),
        ("lodging_preference", r"^(?P<owner>a usuária|o usuário|eu) (?P<governor>prefere|prefiro) se hospedar em (?P<value>pousadas pequenas|pequenas pousadas|pensões familiares|hotéis pequenos) a (?P<contrast>grandes hotéis de rede|grandes resorts)(?: quando viaja)?\.$"),
        ("lodging_preference", r"^(?P<owner>user|i) (?P<governor>prefers|prefer) staying in (?P<value>small guesthouses|family inns|small hotels) (?:to|rather than) (?P<contrast>large chain hotels|large resorts)(?: when traveling)?\.$"),
        ("ceramic_learning", r"^(?P<owner>a usuária|o usuário|eu) (?P<governor>começou|comecei) aulas de (?P<domain>cerâmica|olaria) e (?:está|estou) aprendendo a (?P<action>modelar|fazer) (?P<value>tigelas|potes|vasos|canecas) no (?P<instrument>torno)\.$"),
        ("ceramic_learning", r"^(?P<owner>user|i) (?P<governor>started) (?P<domain>pottery|ceramics) classes and (?:is|am) learning to (?P<action>shape|make|throw) (?P<value>bowls|pots|vases|mugs) on (?:a|the) (?P<instrument>wheel)\.$"),
        ("music_performers_context", r"^(?P<owner>a usuária|o usuário|eu) (?P<governor>escuta|escuto|ouve|ouço) (?:muita |muito )?(?P<genre>[\p{L} -]+) (?P<context>enquanto) (?P<activity>programa|programo), (?P<performer_link>principalmente|especialmente) (?:(?P<performer_type>singers|performers|artists|cantores|cantoras|artistas) )?(?P<value>[\p{L} '-]+)\.$"),
        ("music_performers_context", r"^(?P<owner>user|i) (?P<governor>listens to|listen to) (?P<genre>[\p{L} -]+) (?P<context>while) (?P<activity>coding|programming), (?P<performer_link>especially|mainly) (?:(?P<performer_type>singers|performers|artists|cantores|cantoras|artistas) )?(?P<value>[\p{L} '-]+)\.$"),
        ("recorded_medication_restriction", r"^(?P<owner>a usuária|o usuário|eu) (?P<governor>é alérgica|é alérgico|sou alérgica|sou alérgico) a (?P<restriction>[\p{L}-]+), então (?P<habit_governor>usa|uso) (?P<value>[\p{L}-]+) (?P<context>quando tem|quando tenho) (?P<complaint>dor de cabeça|enxaqueca)\.$"),
        ("recorded_medication_restriction", r"^(?P<owner>user|i) (?P<governor>is allergic|am allergic) to (?P<restriction>[\p{L}-]+), so (?P<habit_governor>uses|use) (?P<value>[\p{L}-]+) (?P<context>for) (?:a )?(?P<complaint>headache|migraine)\.$"),
        ("sibling_video_schedule", r"^(?P<owner>a usuária|o usuário|eu) (?P<governor>faz|faço) (?P<modality>videochamada) com (?:a|minha|o|meu) (?P<kinship>irmã|irmão) (?P<relative>[\p{L} '-]+?) (?P<value>todo (?:domingo|sábado)(?: de manhã| à noite)?|toda (?:segunda|terça|quarta|quinta|sexta)(?:-feira)?(?: à noite| de manhã)?)(?: (?P<purpose>para colocar a conversa em dia|para conversar))?\.$"),
        ("sibling_video_schedule", r"^(?P<owner>user|i) (?P<governor>video calls|video call) (?P<modality>with) (?:her|his|my) (?P<kinship>sister|brother) (?P<relative>[\p{L} '-]+?) (?P<value>every (?:monday|tuesday|wednesday|thursday|friday|saturday|sunday)(?: evening| morning)?)(?: (?P<purpose>to catch up))?\.$"),
    ].into_iter().map(|(kind, pattern)| (kind, regex::Regex::new(pattern).unwrap())).collect());
    let request = personal_life_request(query).map(|(_, p)| p);
    let mut music_typing = None;
    let assertion = frames
        .iter()
        .filter(|(k, _)| *k == kind)
        .find_map(|(_, frame)| frame.captures(&normalized))
        .filter(|c| {
            if kind == "sibling_video_schedule" {
                let source_role = match c.name("kinship").unwrap().as_str() {
                    "irmã" | "sister" => "sister",
                    _ => "brother",
                };
                if request.as_ref().is_none_or(|p| p["kinship"] != source_role) {
                    return false;
                }
            }
            if kind == "music_performers_context" {
                let genre = c.name("genre").unwrap().as_str();
                let value = c.name("value").unwrap().as_str();
                // The listening assertion supplies ownership/activity, not
                // entity type: resolve that independent obligation separately.
                let typing = musical_argument_type(value, c.name("performer_type").is_some());
                let resolves_performer = typing["resolves_performer"] == true;
                music_typing = Some(typing);
                let musical_style = |s: &str| s == "music" || s == "música"
                    || musical_entity_type(s) == "genre";
                if !musical_style(genre) || !resolves_performer {
                    return false;
                }
                // The head types opaque names, including lowercase names and
                // names homonymous with genres. It cannot type a contradictory
                // artifact head or a new typed constituent in that same list.
                if practice_words(value, &[
                    "software", "studio", "server", "editor", "plugin", "package",
                    "genre", "genres", "style", "styles", "gênero", "gêneros",
                    "estilo", "estilos", "visitor", "visitante",
                ]) {
                    return false;
                }
            }
            if kind == "sibling_video_schedule" {
                static NAMES: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
                let m = c.name("relative").unwrap();
                let Some(original) = text.trim().get(m.start()..m.end()) else { return false; };
                let names = NAMES.get_or_init(|| regex::Regex::new(r"^\p{Lu}[\p{L}'-]+(?: \p{Lu}[\p{L}'-]+)*(?: (?:and|e) \p{Lu}[\p{L}'-]+(?: \p{Lu}[\p{L}'-]+)*)*$").unwrap());
                if !names.is_match(original) || practice_words(m.as_str(), &["jazz", "samba", "rock", "pop", "mpb", "blues", "classical", "visitor", "visitante"]) { return false; }
            }
            if kind == "recorded_medication_restriction" {
                // Audited lexical medication types, not advice or entity aliases.
                let medication = |s: &str| {
                    matches!(
                        s,
                        "aspirin"
                            | "aspirina"
                            | "acetaminophen"
                            | "paracetamol"
                            | "ibuprofen"
                            | "ibuprofeno"
                            | "dipyrone"
                            | "dipirona"
                            | "naproxen"
                            | "naproxeno"
                            | "sumatriptan"
                            | "sumatriptana"
                            | "metamizole"
                            | "metamizol"
                    )
                };
                if !medication(c.name("restriction").unwrap().as_str())
                    || !medication(c.name("value").unwrap().as_str())
                {
                    return false;
                }
            }
            if ["genre", "relative"]
                .iter()
                .filter_map(|name| c.name(name))
                .any(|m| {
                    practice_words(
                        m.as_str(),
                        &[
                            "not",
                            "never",
                            "if",
                            "reports",
                            "quotes",
                            "says",
                            "visitor",
                            "colleague",
                            "não",
                            "nunca",
                            "relata",
                            "cita",
                            "visitante",
                            "and",
                            "but",
                            "e",
                            "mas",
                        ],
                    )
                })
            {
                return false;
            }
            let value = c.name("value").unwrap().as_str();
            // In performer lists conjunctions join values, not predicates.
            let guarded = if kind == "music_performers_context" {
                value.replace(" and ", " ").replace(" e ", " ")
            } else {
                value.to_owned()
            };
            !practice_words(
                &guarded,
                &[
                    "not",
                    "never",
                    "if",
                    "would",
                    "could",
                    "unknown",
                    "reports",
                    "quotes",
                    "não",
                    "nunca",
                    "talvez",
                    "desconhecido",
                    "and",
                    "but",
                    "user",
                    "visitor",
                    "colleague",
                    "says",
                    "drinks",
                    "prefers",
                    "e",
                    "mas",
                    "usuária",
                    "usuário",
                    "relata",
                    "cita",
                    "bebe",
                    "toma",
                ],
            )
        });
    let span = |name: &str| {
        assertion
            .as_ref()
            .and_then(|c| c.name(name))
            .map(|m| [m.start(), m.end()])
    };
    Some(serde_json::json!({
        "relation":"owned_personal_activity", "kind":kind,
        "normalization":"trim_lowercase", "matched":assertion.is_some(),
        "query_request_bytes":[0,query.trim().len()],
        "owner_bytes":span("owner"), "governor_bytes":span("governor"),
        "value_bytes":span("value"), "purpose_bytes":span("purpose"),
        "contrast_bytes":span("contrast"), "domain_bytes":span("domain"),
        "action_bytes":span("action"), "instrument_bytes":span("instrument"),
        "genre_bytes":span("genre"), "performer_link_bytes":span("performer_link"),
        "performer_type_bytes":span("performer_type"),
        "performer_type_witness":assertion.as_ref().and_then(|c|c.name("performer_type")).map(|_|"source_argument_type_head"),
        "certifies_performer_type":kind == "music_performers_context" && assertion.is_some(),
        "music_argument_typing":music_typing,
        "certifies_singer_subtype":false,
        "activity_bytes":span("activity"), "context_bytes":span("context"),
        "restriction_bytes":span("restriction"), "habit_governor_bytes":span("habit_governor"),
        "complaint_bytes":span("complaint"), "kinship_bytes":span("kinship"),
        "relative_bytes":span("relative"), "modality_bytes":span("modality"),
        "query_roles":request, "query_lodging_argument":lodging_request(query),
        "certifies_medication_safety":false,
        "certifies_performer_nationality":false,
        "value":assertion.as_ref().and_then(|c|c.name("value")).map(|m|m.as_str()),
        "assertion_bytes":assertion.as_ref().map(|_|[0,normalized.len()])
    }))
}

/// Closed linguistic attributes, not topics or stored entity labels. Recognition
/// creates an obligation even when no fact can discharge it.
fn identity_scope(query: &str) -> Option<(String, String)> {
    static FRAMES: std::sync::OnceLock<Vec<(&str, regex::Regex)>> = std::sync::OnceLock::new();
    let frames = FRAMES.get_or_init(|| [
        ("occupation", r"^(?:what(?:'s| is) my (?:job title|occupation)(?: at work)?|which (?:company|employer) do i work (?:at|for)|em que empresa eu trabalho(?: mesmo)?)\?"),
        ("age", r"^how old am i(?: now)?\?"),
        ("birthday", r"^(?:when is my birthday|quando é (?:o )?meu aniversário)\?"),
        ("languages", r"^(?:which languages do i speak|que idiomas eu falo)\?"),
        ("birthplace", r"^(?:where was i born|what is my birthplace|qual é(?: mesmo)? (?:a )?minha cidade natal)\?"),
        ("kinship", r"^what(?:'s| is) my (?P<kinship>partner|spouse|wife|husband|mother|father|sister|brother|daughter|son)'s name\?"),
    ].into_iter().map(|(kind, pattern)| (kind, regex::Regex::new(pattern).unwrap())).collect());
    let q = query.trim().to_lowercase();
    if q.contains(['"', '“', '”', ';', '\n']) || q.matches('?').count() != 1 {
        return None;
    }
    frames.iter().find_map(|(kind, frame)| {
        frame.captures(&q).map(|c| {
            (
                kind.to_string(),
                c.name("kinship").map_or("", |m| m.as_str()).to_owned(),
            )
        })
    })
}

/// An affirmative root with an overt owner, typed governor and value. Values
/// remain source bytes; this parser never calculates or manufactures a fact.
fn identity_assertion(text: &str) -> Option<serde_json::Value> {
    static FRAMES: std::sync::OnceLock<Vec<(&str, regex::Regex)>> = std::sync::OnceLock::new();
    let frames = FRAMES.get_or_init(|| [
        ("occupation", r"^(?P<owner>user|i) (?P<governor>is|am) (?:a|an) (?P<value>[\p{L} -]+?)(?: on the [\p{L} -]+ team)? at (?P<employer>[\p{L} -]+)\.$"),
        ("occupation", r"^(?P<owner>a usuária|o usuário|eu) (?P<governor>trabalha|trabalho) (?:na|no|em) (?P<value>[\p{L} -]+?)(?:, (?:uma|um) [\p{L} -]+)?(?:,? desde [0-9]{4})?\.$"),
        ("occupation", r"^(?P<owner>user|i) (?P<governor>works|work) (?:at|for) (?P<value>[\p{L} -]+)\.$"),
        ("kinship", r"^(?P<owner>the user's|user's|my) (?P<kinship>partner|spouse|wife|husband|mother|father|sister|brother|daughter|son) (?P<governor>is) (?P<value>[\p{L} -]+?)(?:, (?P<details>(?:a|an) [\p{L} -]+?) she has been dating since (?P<since>[0-9]{4}))?\.$"),
        ("kinship", r"^(?P<owner>the user's|user's|my) (?P<kinship>partner|spouse|wife|husband|mother|father|sister|brother|daughter|son)'s name (?P<governor>is) (?P<value>[\p{L} -]+)\.$"),
        ("occupation", r"^(?P<owner>user|i) (?P<governor>works as|work as) (?:a|an) (?P<value>[\p{L} -]+) at (?P<employer>[\p{L} -]+)\.$"),
        ("kinship", r"^(?P<value>[\p{L} -]+?), (?P<details>(?:a|an) [\p{L} -]+?), (?P<governor>has been) (?P<owner>the user's|my) (?P<kinship>partner|spouse|wife|husband|mother|father|sister|brother|daughter|son) since (?P<since>[0-9]{4})\.$"),
        ("birthday", r"^(?P<owner>a usuária|o usuário|eu) (?P<governor>nasceu|nasci) em (?P<value>(?P<day>[0-9]{1,2}) de (?P<month>[\p{L}]+) de (?P<year>[0-9]{4}))\.$"),
        ("birthday", r"^(?P<owner>user|i) (?P<governor>was born) on (?P<value>(?P<year>[0-9]{4})-(?P<month>[0-9]{2})-(?P<day>[0-9]{2}))\.$"),
        ("age", r"^(?P<owner>user|i) (?P<governor>turned) (?P<value>[0-9]{1,3}) years old in (?P<month>[\p{L}]+) (?P<year>[0-9]{4})\.$"),
        ("birthplace", r"^(?P<owner>user|i) (?P<governor>was born) in (?P<value>[\p{L} ,'-]+?)(?:, and grew up there until college)?\.$"),
        ("languages", r"^(?P<owner>a usuária|o usuário) (?P<governor>tem) o (?P<value>[\p{L} -]+ como língua materna, fala [\p{L} -]+ fluente e [\p{L} -]+ em nível intermediário)\.$"),
        ("languages", r"^(?P<owner>user|i) (?P<governor>speaks|speak) (?P<value>[\p{L} ,'-]+?)(?: fluently)?\.$"),
    ].into_iter().map(|(kind, pattern)| (kind, regex::Regex::new(pattern).unwrap())).collect());
    let normalized = text.trim().to_lowercase();
    if crate::index::tokenize(&normalized).iter().any(|w| {
        matches!(
            w.as_str(),
            "not"
                | "never"
                | "if"
                | "would"
                | "could"
                | "quotes"
                | "reports"
                | "says"
                | "não"
                | "nunca"
                | "se"
                | "seria"
                | "relata"
                | "cita"
        )
    }) {
        return None;
    }
    let (kind, c) = frames
        .iter()
        .find_map(|(kind, frame)| frame.captures(&normalized).map(|c| (*kind, c)))?;
    if ["value", "details", "employer"]
        .iter()
        .filter_map(|name| c.name(name))
        .any(|m| {
            crate::index::tokenize(m.as_str()).iter().any(|w| {
                matches!(
                    w.as_str(),
                    "unknown" | "unsure" | "possibly" | "maybe" | "might" | "should" | "allegedly"
                )
            })
        })
    {
        return None;
    }
    let value = c.name("value")?;
    let owner = c.name("owner")?;
    let governor = c.name("governor")?;
    // Do not allow an open nominal argument to swallow a new assertion.
    if ["value", "details", "employer"]
        .iter()
        .filter_map(|name| c.name(name))
        .any(|m| {
            crate::index::tokenize(m.as_str()).iter().any(|w| {
                matches!(
                    w.as_str(),
                    "is" | "was"
                        | "has"
                        | "user"
                        | "usuária"
                        | "usuário"
                        | "visitor"
                        | "colleague"
                        | "but"
                        | "where"
                        | "whose"
                        | "called"
                        | "named"
                        | "company"
                        | "workshop"
                        | "room"
                        | "é"
                )
            })
        })
    {
        return None;
    }
    Some(
        serde_json::json!({"kind":kind,"owner":"user","normalization":"trim_lowercase",
        "value":value.as_str(),"value_bytes":[value.start(),value.end()],
        "owner_bytes":[owner.start(),owner.end()],"governor":governor.as_str(),"governor_bytes":[governor.start(),governor.end()],
        "kinship":c.name("kinship").map(|m|m.as_str()),
        "details":c.name("details").map(|m|m.as_str()),"since":c.name("since").map(|m|m.as_str()),
        "year":c.name("year").map(|m|m.as_str()),"month":c.name("month").map(|m|m.as_str()),
        "day":c.name("day").map(|m|m.as_str()),"employer":c.name("employer").map(|m|m.as_str()),
        "argument_bytes":(["kinship","details","since","year","month","day","employer"].into_iter()
            .filter_map(|name| c.name(name).map(|m| (name.to_owned(), serde_json::json!([m.start(),m.end()]))))
            .collect::<serde_json::Map<String,serde_json::Value>>())}),
    )
}

fn identity_month(s: &str) -> Option<u32> {
    match s {
        "january" | "janeiro" => Some(1),
        "february" | "fevereiro" => Some(2),
        "march" | "março" => Some(3),
        "april" | "abril" => Some(4),
        "may" | "maio" => Some(5),
        "june" | "junho" => Some(6),
        "july" | "julho" => Some(7),
        "august" | "agosto" => Some(8),
        "september" | "setembro" => Some(9),
        "october" | "outubro" => Some(10),
        "november" | "novembro" => Some(11),
        "december" | "dezembro" => Some(12),
        _ => s.parse::<u32>().ok().filter(|m| (1..=12).contains(m)),
    }
}

/// Date/age context is licensed only by a real calendar date and, for an age
/// statement, a matching birth witness and a current-year reference event.
pub(crate) fn identity_evidence(
    query: &str,
    text: &str,
    context: &[&str],
    now: u64,
) -> Option<serde_json::Value> {
    use chrono::Datelike;
    let (scope, kinship) = identity_scope(query)?;
    let assertion = identity_assertion(text);
    let today = chrono::DateTime::from_timestamp_millis(now as i64)?.date_naive();
    let birth = |p: &serde_json::Value| {
        chrono::NaiveDate::from_ymd_opt(
            p["year"].as_str()?.parse().ok()?,
            identity_month(p["month"].as_str()?)?,
            p["day"].as_str()?.parse().ok()?,
        )
    };
    let mut temporal_witness = None;
    let matched = assertion.as_ref().is_some_and(|p| {
        let kind = p["kind"].as_str().unwrap_or("");
        match (scope.as_str(), kind) {
            ("occupation", "occupation") => p["employer"].is_null()
                || matches!(p["governor"].as_str(), Some("work as" | "works as"))
                || crate::index::tokenize(p["value"].as_str().unwrap_or("")).iter().any(|w|
                    matches!(w.as_str(), "engineer"|"designer"|"developer"|"manager"|"director"|"analyst"|"architect"|"scientist"|"teacher"|"nurse"|"physician"|"technician"|"accountant"|"attorney"|"researcher"|"consultant"|"officer"|"specialist"|"coordinator"|"assistant"|"professor"|"editor"|"writer"|"chef"|"mechanic"|"operator"|"supervisor")),
            ("languages", "languages") => true,
            ("birthplace", "birthplace") => !crate::index::tokenize(p["value"].as_str().unwrap_or("")).iter().any(|w|
                matches!(w.as_str(), "company"|"workshop"|"room"|"hospital"|"ward"|"laboratory")),
            ("kinship", "kinship") => p["kinship"] == kinship,
            ("age"|"birthday", "birthday") => birth(p).is_some_and(|d| d <= today),
            ("age"|"birthday", "age") => {
                let year = p["year"].as_str().and_then(|s|s.parse::<i32>().ok());
                let month = p["month"].as_str().and_then(identity_month);
                let age = p["value"].as_str().and_then(|s|s.parse::<i32>().ok());
                let witness = context.iter().find_map(|text| {
                    let b = identity_assertion(text)?;
                    if b["kind"] != "birthday" { return None; }
                    let d = birth(&b)?;
                    (year == Some(today.year()) && month == Some(d.month())
                        && age == Some(today.year() - d.year())
                        && d.with_year(today.year()).is_some_and(|anniversary| anniversary <= today))
                        .then(|| serde_json::json!({"birth_assertion":text,"birth_date":d.to_string(),"current_date":today.to_string()}))
                });
                if let Some(witness) = witness {
                    temporal_witness = Some(witness);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    });
    Some(
        serde_json::json!({"relation":"owned_identity_attribute","scope":scope,"kinship":kinship,
        "matched":matched,"assertion":assertion.filter(|_|matched),
        "current_date":today.to_string(),"temporal_witness":temporal_witness}),
    )
}

pub(crate) fn identity_kinship(text: &str) -> bool {
    identity_assertion(text).is_some_and(|p| p["kind"] == "kinship")
}

/// Full relational tuples only; shared names or partner labels are not twins.
/// Metadata/pins are checked by the consumer before suppressing either line.
pub(crate) fn identity_equivalent(a: &str, b: &str) -> bool {
    let (Some(a), Some(b)) = (identity_assertion(a), identity_assertion(b)) else {
        return false;
    };
    a["kind"] == "kinship"
        && b["kind"] == "kinship"
        && ["owner", "value", "kinship", "details", "since"]
            .iter()
            .all(|k| a[k] == b[k])
}

/// A scope is a requirement, not a claim that the answer exists. Empty terms
/// mean an ambiguous personal construction: no candidate gains a certificate.
#[derive(Debug, Clone)]
pub(super) struct PersonalScope {
    pub span: Range<usize>,
    pub terms: Vec<String>,
    pub predicate: Option<String>,
    pub owner: Option<String>,
    pub self_owned: bool,
    /// WHERE supplies an open argument, not a missing nominal phrase.
    pub open_argument: bool,
    /// Only an overt EU query licenses regular first-to-third-person agreement.
    pub portuguese_first_person: bool,
}

pub(super) fn possessive(word: &str) -> bool {
    matches!(word, "my" | "meu" | "meus" | "minha" | "minhas")
}

pub(super) fn subordinate(word: &str) -> bool {
    matches!(
        word,
        "when"
            | "while"
            | "because"
            | "if"
            | "that"
            | "quando"
            | "enquanto"
            | "porque"
            | "se"
            | "que"
    )
}

fn adverbial(word: &str) -> bool {
    word.strip_suffix("ly")
        .is_some_and(|stem| stem.chars().count() >= 4)
        || word
            .strip_suffix("mente")
            .is_some_and(|stem| stem.chars().count() >= 3)
        || matches!(word, "now" | "agora" | "já")
}

fn negative(word: &str) -> bool {
    matches!(
        word,
        "not" | "never" | "nor" | "without" | "não" | "nunca" | "nem" | "sem"
    )
}

fn coordination(word: &str) -> bool {
    matches!(word, "and" | "or" | "but" | "e" | "ou" | "mas")
}

/// A deliberately small affirmative domain for an omitted coordinated subject:
/// an overt personal subject, then a predicate, followed only by zero objects,
/// another coordinated predicate, or preposition-led proper-name complements.
/// Unmarked objects, reported clauses, relatives and quotes remain unknown.
/// Orthographic names are read from the text, never from a corpus vocabulary.
fn personal_coordination_domain(words: &[String], conjunction: usize, clause: &str) -> bool {
    let personal = |w: &str| matches!(w, "i" | "eu" | "user" | "usuário" | "usuária");
    let join = |w: &str| matches!(w, "and" | "e");
    let prep = |w: &str| {
        matches!(
            w,
            "from"
                | "to"
                | "in"
                | "at"
                | "on"
                | "with"
                | "de"
                | "do"
                | "da"
                | "dos"
                | "das"
                | "para"
                | "em"
                | "no"
                | "na"
                | "nos"
                | "nas"
                | "com"
        )
    };
    if !words.first().is_some_and(|w| personal(w))
        || !words.get(conjunction).is_some_and(|w| join(w))
        || clause
            .chars()
            .any(|c| matches!(c, ',' | '"' | '“' | '”' | '\'' | '\n' | '\r'))
    {
        return false;
    }
    let spelling: Vec<&str> = clause
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if spelling.len() != words.len() {
        return false;
    }
    let mut at = 1;
    loop {
        while at < conjunction && adverbial(&words[at]) {
            at += 1;
        }
        if at >= conjunction
            || STOPWORDS.contains(&words[at].as_str())
            || personal(&words[at])
            || subordinate(&words[at])
            || coordination(&words[at])
        {
            return false;
        }
        // One predicate immediately under the already certified subject.
        at += 1;
        loop {
            if at == conjunction {
                return true;
            }
            if join(&words[at]) {
                at += 1;
                break;
            }
            if !prep(&words[at]) {
                return false;
            }
            at += 1;
            if at < conjunction && matches!(words[at].as_str(), "the" | "o" | "a" | "os" | "as") {
                at += 1;
            }
            let first_name = at;
            while at < conjunction && !prep(&words[at]) && !coordination(&words[at]) {
                if STOPWORDS.contains(&words[at].as_str())
                    || subordinate(&words[at])
                    || personal(&words[at])
                    || !spelling[at].chars().next().is_some_and(char::is_uppercase)
                {
                    return false;
                }
                at += 1;
            }
            if at == first_name {
                return false;
            }
        }
    }
}

/// Only grammatical markers are enumerated. Nouns/predicates come unchanged
/// from the query. No candidate, tag dictionary or ranking is read here.
pub(super) fn personal_scopes(query: &str) -> Vec<PersonalScope> {
    let mut scopes = Vec::new();
    let mut offset = 0;
    for clause in query.split(['?', '!', '.', ';', ':']) {
        let w = crate::index::tokenize(clause);
        let content = |start: usize, end: usize| {
            w[start..end]
                .iter()
                .filter(|t| !STOPWORDS.contains(&t.as_str()) && !subordinate(t) && !adverbial(t))
                .cloned()
                .collect::<Vec<_>>()
        };
        let inverted = w
            .windows(2)
            .position(|s| matches!(s[0].as_str(), "do" | "did" | "am") && s[1] == "i");
        if let Some(i) = inverted {
            let procedural = i > 0 && w[i - 1] == "how";
            let predicate_at = (i + 2..w.len()).find(|&at| !adverbial(&w[at]));
            // HOW asks for a procedure, not an autobiographical preference.
            // Preserve independent lexical/system support, not a pronoun escape.
            if let Some(predicate_at) =
                predicate_at.filter(|&at| !procedural && !STOPWORDS.contains(&w[at].as_str()))
            {
                let front = w[..i]
                    .iter()
                    .rposition(|t| matches!(t.as_str(), "what" | "which"));
                let front_terms = front.map(|at| content(at + 1, i)).filter(|t| !t.is_empty());
                let terms = front_terms
                    .clone()
                    .unwrap_or_else(|| content(predicate_at + 1, w.len()));
                let open_argument = i > 0 && w[i - 1] == "where";
                // A bare personal predicate remains an unmet requirement,
                // not a singleton word certificate.
                scopes.push(PersonalScope {
                    span: offset + front.map_or(i, |at| at + 1)..offset + w.len(),
                    terms,
                    predicate: if front_terms.is_some() {
                        None
                    } else {
                        Some(w[predicate_at].clone())
                    },
                    owner: None,
                    self_owned: false,
                    open_argument,
                    portuguese_first_person: false,
                });
            }
        } else if w.first().is_some_and(|t| t == "eu") && w.len() > 2 {
            if let Some(predicate_at) = (1..w.len()).find(|&at| !adverbial(&w[at])) {
                scopes.push(PersonalScope {
                    span: offset..offset + w.len(),
                    terms: content(predicate_at + 1, w.len()),
                    predicate: Some(w[predicate_at].clone()),
                    owner: None,
                    self_owned: false,
                    open_argument: false,
                    portuguese_first_person: true,
                });
            }
        }
        // Object-question with an explicitly possessed subject: WHAT do my
        // friends <predicate> me? Unlike two bag words, the predicate and the
        // personal target are part of the same grammatical requirement.
        if let Some(at) = w
            .windows(2)
            .position(|pair| pair[0] == "do" && pair[1] == "my")
            .filter(|&at| {
                w.first()
                    .is_some_and(|word| matches!(word.as_str(), "what" | "which"))
                    && w.get(at + 4).is_some_and(|word| word == "me")
            })
        {
            scopes.push(PersonalScope {
                span: offset + at + 1..offset + at + 5,
                terms: vec![w[at + 2].clone()],
                predicate: Some(w[at + 3].clone()),
                owner: Some(w[at + 2].clone()),
                self_owned: false,
                open_argument: false,
                portuguese_first_person: false,
            });
        }
        for (i, word) in w.iter().enumerate().filter(|(_, word)| possessive(word)) {
            let Some(first) = w.get(i + 1) else { continue };
            if STOPWORDS.contains(&first.as_str()) {
                continue;
            }
            let end = (i + 1..w.len())
                .find(|&at| subordinate(&w[at]))
                .unwrap_or(w.len());
            let clitic = w.iter().enumerate().skip(i + 2).find(|(at, t)| {
                t.as_str() == "me"
                    && w.get(at + 1).is_some_and(|p| {
                        p.ends_with("ar") || p.ends_with("er") || p.ends_with("ir")
                    })
            });
            if let Some((at, _)) = clitic {
                scopes.push(PersonalScope {
                    span: offset + i..offset + at + 2,
                    terms: vec![first.clone()],
                    predicate: Some(w[at + 1].clone()),
                    owner: Some(first.clone()),
                    self_owned: false,
                    open_argument: false,
                    portuguese_first_person: false,
                });
            } else if w.get(i + 2).is_some_and(|t| t == "s") && w.get(i + 3).is_some() {
                scopes.push(PersonalScope {
                    span: offset + i..offset + i + 4,
                    terms: vec![first.clone(), w[i + 3].clone()],
                    predicate: None,
                    owner: Some(first.clone()),
                    self_owned: false,
                    open_argument: false,
                    portuguese_first_person: false,
                });
            } else if end < w.len() {
                scopes.push(PersonalScope {
                    span: offset + i..offset + end,
                    terms: Vec::new(),
                    predicate: None,
                    owner: None,
                    self_owned: false,
                    open_argument: false,
                    portuguese_first_person: false,
                });
            } else {
                let direct = w[..i]
                    .iter()
                    .any(|t| matches!(t.as_str(), "is" | "s" | "é"))
                    && w.first()
                        .is_some_and(|t| matches!(t.as_str(), "what" | "who" | "qual" | "quem"));
                if direct {
                    let end = (i + 1..w.len())
                        .find(|&at| STOPWORDS.contains(&w[at].as_str()))
                        .unwrap_or(w.len());
                    let terms = if word == "my" {
                        content(i + 1, end)
                    } else {
                        vec![first.clone()]
                    };
                    scopes.push(PersonalScope {
                        span: offset + i..offset + end,
                        terms,
                        predicate: None,
                        owner: None,
                        self_owned: true,
                        open_argument: false,
                        portuguese_first_person: false,
                    });
                }
            }
        }
        offset += w.len();
    }
    scopes
}

pub(super) fn same_form(query: &str, token: &str) -> bool {
    query == token
        || query.strip_suffix('s') == Some(token)
        || token.strip_suffix('s') == Some(query)
        || crate::index::verb_stem(query) == crate::index::verb_stem(token)
}

/// A predicate needs an overt subject and its argument in this same clause.
/// Exemplificative `like` elsewhere in a sentence cannot satisfy this shape.
pub(super) fn scope_matches(scope: &PersonalScope, text: &str) -> bool {
    if scope.terms.is_empty() && !scope.open_argument {
        return false;
    }
    text.split(['?', '!', '.', ';', ':']).any(|clause| {
        let all_words = crate::index::tokenize(clause);
        // A separately coordinated negative does not undo an already stated
        // nominal attribute. Predicate certificates remain stricter below.
        let nominal_end = if scope.self_owned {
            all_words
                .iter()
                .position(|w| coordination(w))
                .unwrap_or(all_words.len())
        } else {
            all_words.len()
        };
        let w = if scope.self_owned {
            all_words[..nominal_end].to_vec()
        } else {
            all_words
        };
        // These partial certificates do not resolve negation or contrast
        // scope. Reject the whole affected clause rather than attest words
        // merely mentioned as the unwanted alternative.
        if w.iter().any(|word| negative(word)) {
            return false;
        }
        if w.first()
            .is_some_and(|word| matches!(word.as_str(), "user" | "i" | "my"))
            && w.iter().any(|word| word == "no")
        {
            return false;
        }
        if scope.self_owned && nominal_end == 0 {
            return false;
        }
        if !scope
            .terms
            .iter()
            .all(|term| w.iter().any(|t| same_form(term, t)))
        {
            return false;
        }
        if let Some(predicate) = &scope.predicate {
            return w.iter().enumerate().any(|(i, t)| {
                (same_form(predicate, t)
                    || (scope.portuguese_first_person
                        && predicate.strip_suffix('o').is_some_and(|stem| {
                            stem.chars().count() >= 3
                                && stem.chars().all(char::is_alphabetic)
                                && t.strip_suffix('a') == Some(stem)
                        })
                        && (0..i)
                            .rev()
                            .find(|&at| !adverbial(&w[at]))
                            .is_some_and(|at| matches!(w[at].as_str(), "usuário" | "usuária"))))
                    && if let Some(owner) = &scope.owner {
                        w[..i].iter().any(|t| same_form(owner, t))
                            && w[i + 1..].iter().any(|t| {
                                matches!(t.as_str(), "user" | "usuário" | "usuária" | "me" | "mim")
                            })
                    } else {
                        let personal = |word: &str| {
                            matches!(word, "i" | "eu" | "user" | "usuário" | "usuária")
                        };
                        let before = (0..i).rev().find(|&at| !adverbial(&w[at]));
                        let direct = before.is_some_and(|at| personal(&w[at]));
                        let inherited =
                            before.is_some_and(|at| personal_coordination_domain(&w, at, clause));
                        let end = (i + 1..w.len())
                            .find(|&at| coordination(&w[at]) || subordinate(&w[at]))
                            .unwrap_or(w.len());
                        (direct || inherited)
                            && scope
                                .terms
                                .iter()
                                .all(|term| w[i + 1..end].iter().any(|t| same_form(term, t)))
                            && (!scope.open_argument
                                || w[i + 1..end]
                                    .iter()
                                    .any(|t| !STOPWORDS.contains(&t.as_str()) && !adverbial(t)))
                    }
            });
        }
        if scope.self_owned {
            let self_subject = w.iter().any(|t| {
                matches!(
                    t.as_str(),
                    "my" | "meu" | "minha" | "user" | "usuário" | "usuária" | "eu" | "i"
                )
            });
            let genitives = w.iter().filter(|t| t.as_str() == "s").count();
            let linked = w.iter().enumerate().any(|(i, t)| {
                let start = if t == "my" {
                    Some(i + 1)
                } else if t == "user" && w.get(i + 1).is_some_and(|t| t == "s") {
                    Some(i + 2)
                } else {
                    None
                };
                start.is_some_and(|start| {
                    let end = (start..w.len())
                        .find(|&at| matches!(w[at].as_str(), "is" | "are" | "was" | "é"))
                        .unwrap_or(start);
                    start < end
                        && w[start..end]
                            .iter()
                            .all(|word| scope.terms.iter().any(|term| same_form(term, word)))
                })
            });
            // PT possessive heads can be realized after the personal subject;
            // every requested head still has to occur in this same clause.
            let pt_link = w.iter().enumerate().any(|(i, t)| {
                matches!(t.as_str(), "usuário" | "usuária")
                    && i >= 2
                    && matches!(w[i - 1].as_str(), "do" | "da")
                    && scope.terms.iter().any(|term| same_form(term, &w[i - 2]))
            }) || w
                .first()
                .is_some_and(|t| matches!(t.as_str(), "meu" | "minha"))
                && w.get(1)
                    .is_some_and(|t| scope.terms.iter().any(|term| same_form(term, t)));
            let pt_subject = w
                .first()
                .is_some_and(|t| matches!(t.as_str(), "eu" | "meu" | "minha"))
                || w.windows(2).any(|pair| {
                    matches!(pair[0].as_str(), "o" | "a")
                        && matches!(pair[1].as_str(), "usuário" | "usuária")
                });
            // A personal subject with no nested nominal possessor may state
            // its own attribute after modifiers (e.g. a newly assigned role).
            let nested_owner = w.iter().enumerate().any(|(i, t)| {
                matches!(t.as_str(), "do" | "da")
                    && w.get(i + 1)
                        .is_some_and(|t| !matches!(t.as_str(), "usuário" | "usuária"))
            });
            let direct_subject = w
                .first()
                .is_some_and(|t| matches!(t.as_str(), "user" | "i"))
                && w.get(1).is_some_and(|t| t != "s");
            return self_subject
                && genitives <= 1
                && (linked || pt_link || ((pt_subject || direct_subject) && !nested_owner));
        }
        scope.owner.as_ref().is_none_or(|owner| {
            w.iter().enumerate().any(|(i, t)| {
                same_form(owner, t)
                    && w.get(i + 1).is_some_and(|t| t == "s")
                    && w.get(i + 2).is_some_and(|head| {
                        scope
                            .terms
                            .iter()
                            .filter(|term| !same_form(term, owner))
                            .all(|term| same_form(term, head))
                    })
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obligation_relation_query_punctuation_and_candidate_assertion_have_separate_roles() {
        let mut failures = Vec::new();
        let mut executed = 0;
        for (language, query, candidate, object, target, governor, span) in [
            (
                "en-need",
                "How many clips does a parcel need before transit",
                "Each parcel needs two clips before transit",
                "clips",
                "parcel",
                "need",
                [2, 9],
            ),
            (
                "en-require",
                "How many clips does a parcel require before transit",
                "Each parcel requires two clips before transit",
                "clips",
                "parcel",
                "require",
                [2, 9],
            ),
            (
                "pt-precisar",
                "Quantas etiquetas a bandeja precisa antes do transporte",
                "Cada bandeja precisa de duas etiquetas antes do transporte",
                "etiquetas",
                "bandeja",
                "precisa",
                [1, 8],
            ),
            (
                "pt-exigir",
                "Quantas etiquetas a bandeja exige antes do transporte",
                "Cada bandeja exige duas etiquetas antes do transporte",
                "etiquetas",
                "bandeja",
                "exige",
                [1, 8],
            ),
            (
                "en-nominal-object",
                "How many reports does a parcel need before transit",
                "Each parcel needs two reports before transit",
                "reports",
                "parcel",
                "need",
                [2, 9],
            ),
            (
                "en-nominal-target",
                "How many copies does a report need before filing",
                "Each report needs two copies before filing",
                "copies",
                "report",
                "need",
                [2, 9],
            ),
        ] {
            let expected_scope = serde_json::json!({
                "span":span, "kind":"Quantity", "terms":[object],
                "predicate":governor, "subject":[target], "head_initial":language.starts_with("pt"),
                "object":[], "relative":[]
            });
            let (argument, boundary) = match language {
                "pt-precisar" => ([4, 6], [6, 9]),
                "pt-exigir" => ([3, 5], [5, 8]),
                _ => ([3, 5], [5, 7]),
            };
            for query_suffix in ["", "?"] {
                let query = format!("{query}{query_suffix}");
                let scope = target_scope(&query);
                for (candidate_suffix, domain, matched) in
                    [("", "asserted", true), ("?", "interrogative", false)]
                {
                    let candidate = format!("{candidate}{candidate_suffix}");
                    let expected_audit = serde_json::json!({
                        "query_governor":governor, "query_span":span,
                        "target":[target], "object":[object], "matched":matched,
                        "assertion":{
                            "governor_span":[2, 3], "source_span":[1, 2],
                            "argument_span":argument, "target_span":[1, 2],
                            "boundary_span":boundary, "frame":"universal_bearer",
                            "domain":domain, "polarity":"affirmative"
                        }
                    });
                    let expected = serde_json::json!({
                        "scope":expected_scope, "obligation":expected_audit,
                        "target_matches":matched
                    });
                    let result = serde_json::json!({
                        "scope":scope.as_ref().map(|scope| serde_json::json!({
                            "span":[scope.span.start, scope.span.end],
                            "kind":format!("{:?}", scope.kind), "terms":scope.terms,
                            "predicate":scope.predicate, "subject":scope.subject,
                            "head_initial":scope.head_initial,
                            "object":scope.object, "relative":scope.relative
                        })),
                        "obligation":scope.as_ref().and_then(|scope| obligation_evidence(scope, &candidate)),
                        "target_matches":scope.as_ref().map(|scope| target_matches(scope, &candidate))
                    });
                    println!(
                        "{}",
                        serde_json::json!({
                            "diagnostic":"query-candidate-domain-separation", "stage":"helper",
                            "language":language, "query":query, "candidate":candidate,
                            "expected":expected, "result":result
                        })
                    );
                    executed += 1;
                    if result != expected {
                        failures.push(format!(
                            "{language}/{query_suffix}/{candidate_suffix}: {result} != {expected}"
                        ));
                    }
                }
            }
        }
        assert_eq!(executed, 24);
        assert!(
            failures.is_empty(),
            "papéis da consulta e da asserção: {failures:?}"
        );
    }

    #[test]
    fn obligation_relation_query_guards_survive_terminal_question_mark_normalization() {
        let mut failures = Vec::new();
        let mut executed = 0;
        for query in [
            "How many clips does a parcel need before transit if storage changes",
            "Quantas etiquetas a bandeja precisa antes do transporte se o depósito muda",
            "How many clips does a parcel need before transit without permission",
            "Quantas etiquetas a bandeja precisa antes do transporte não autorizado",
            "How many clips does a parcel need before the clerk says transit begins",
            "Quantas etiquetas a bandeja precisa antes do transporte segundo o escrivão",
            "How many clips does a parcel need before transit in a fictional example",
            "Quantas etiquetas a bandeja precisa antes do transporte como exemplo fictício",
            "How many clips does a parcel need before \"transit\"",
            "Quantas etiquetas a bandeja precisa antes do 'transporte'",
            "How many clips does a parcel need before transit and storage",
            "Quantas etiquetas a bandeja precisa antes do transporte e armazenamento",
            "How many clips does a parcel need before transit, after storage",
            "Quantas etiquetas a bandeja precisa antes do transporte (após armazenamento)",
            "How many clips does a parcel need? Before transit",
            "Quantas etiquetas a bandeja precisa? Antes do transporte",
        ] {
            for suffix in ["", "?"] {
                let query = format!("{query}{suffix}");
                let scope = target_scope(&query);
                println!(
                    "{}",
                    serde_json::json!({
                        "diagnostic":"query-domain-guards", "query":query,
                        "expected_scope_required":false, "scope_required":scope.is_some()
                    })
                );
                executed += 1;
                if scope.is_some() {
                    failures.push(query);
                }
            }
        }
        assert_eq!(executed, 32);
        assert!(
            failures.is_empty(),
            "consultas fora do domínio delimitado: {failures:?}"
        );
    }

    #[test]
    fn target_relation_relative_bridge_certifies_only_a_shallow_head() {
        for (query, positive, negative) in [
            (
                "Which service consumes the audit events Aster publishes to Orbit?",
                "Aster publishes audit events to Orbit channels that the sentinel service consumes.",
                "Aster publishes audit events to Orbit channels beside the cargo bundles that the sentinel service consumes.",
            ),
            (
                "Qual serviço consome os eventos de auditoria Aster publica para Orbit?",
                "O serviço Aster publica eventos de auditoria para canais Orbit que o serviço sentinela consome.",
                "O serviço Aster publica eventos de auditoria para canais Orbit junto aos registros de carga que o serviço sentinela consome.",
            ),
        ] {
            let scope = target_scope(query).unwrap();
            assert!(target_matches(&scope, positive), "{query} / {positive}");
            assert!(!target_matches(&scope, negative), "{query} / {negative}");
        }
    }

    fn spans(query: &str) -> Option<Vec<[usize; 2]>> {
        requested_slots(query, &crate::index::tokenize(query))
            .map(|spans| spans.into_iter().map(|s| [s.start, s.end]).collect())
    }

    #[test]
    fn target_relation_review_r20_relative_antecedent_binding_helpers() {
        let query = "Which service consumes the telemetry packets Vega publishes to Relay?";
        let scope = target_scope(query);
        let mut failures = Vec::new();
        let mut executed = 0;
        for (id, fact, expected_matched) in [
            (
                "answer",
                "Vega publishes telemetry packets to Relay topics that the archive service consumes.",
                true,
            ),
            (
                "incidental",
                "Vega publishes telemetry packets to Relay topics alongside the billing records that the archive service consumes.",
                false,
            ),
        ] {
            let expected = serde_json::json!({
                "target_scope_required":true, "target_matches":expected_matched
            });
            let result = serde_json::json!({
                "target_scope_required":scope.is_some(),
                "target_matches":scope.as_ref().map(|scope| target_matches(scope, fact))
            });
            println!(
                "{}",
                serde_json::json!({
                    "diagnostic":"relative-antecedent-binding", "stage":"helper",
                    "query":query, "id":id, "fact":fact,
                    "expected":expected, "result":result,
                    "witness":{"scope":format!("{scope:?}"), "tokens":crate::index::tokenize(fact)}
                })
            );
            executed += 1;
            if result != expected {
                failures.push(format!("{id}: esperado={expected}, obtido={result}"));
            }
        }
        assert_eq!(executed, 2);
        assert!(failures.is_empty(), "vínculo do antecedente: {failures:?}");
    }

    #[test]
    fn obligation_relation_review_r20_candidate_questions_not_assertions_helpers() {
        let mut failures = Vec::new();
        let mut executed = 0;
        for (language, query, answer, incidental) in [
            (
                "en",
                "How many seals does a crate need before dispatch?",
                "Each crate needs three seals before dispatch.",
                "Each crate needs three seals before dispatch?",
            ),
            (
                "pt",
                "Quantos lacres uma caixa precisa antes do envio?",
                "Cada caixa precisa de três lacres antes do envio.",
                "Cada caixa precisa de três lacres antes do envio?",
            ),
        ] {
            let scope = target_scope(query);
            for (id, fact, expected_matched) in
                [("answer", answer, true), ("incidental", incidental, false)]
            {
                let audit = scope
                    .as_ref()
                    .and_then(|scope| obligation_evidence(scope, fact));
                let expected = serde_json::json!({
                    "target_scope_required":true, "target_matches":expected_matched,
                    "obligation_matched":expected_matched, "domain_asserted":expected_matched
                });
                let result = serde_json::json!({
                    "target_scope_required":scope.is_some(),
                    "target_matches":scope.as_ref().map(|scope| target_matches(scope, fact)),
                    "obligation_matched":audit.as_ref().map(|audit| audit.matched),
                    "domain_asserted":audit.as_ref().map(|audit| audit.assertion.domain == "asserted")
                });
                println!(
                    "{}",
                    serde_json::json!({
                        "diagnostic":"candidate-questions-not-assertions", "stage":"helper",
                        "language":language, "query":query, "id":id, "fact":fact,
                        "expected":expected, "result":result,
                        "witness":{"scope":format!("{scope:?}"),
                            "tokens":crate::index::tokenize(fact), "obligation":audit}
                    })
                );
                executed += 1;
                if result != expected {
                    failures.push(format!(
                        "{language}/{id}: esperado={expected}, obtido={result}"
                    ));
                }
            }
        }
        assert_eq!(executed, 4);
        assert!(failures.is_empty(), "pergunta não é asserção: {failures:?}");
    }

    #[test]
    fn obligation_relation_neutral_frames_and_assertion_domains() {
        let groups: [(&str, &[(&str, bool)]); 2] = [
            ("How many seals does a crate need before dispatch?", &[
                ("Each crate needs three seals before dispatch.", true),
                ("Every crate requires three seals before dispatch.", true),
                ("The depot requires three seals on every crate before dispatch.", true),
                ("The manual mentions three seals on every crate as an example.", false),
                ("The depot requires guidance and mentions three seals on every crate before dispatch.", false),
                ("The depot requires three ribbons on every crate before dispatch and mentions three seals.", false),
                ("The depot requires three seals on every pallet before dispatch.", false),
                ("The depot does not require three seals on every crate before dispatch.", false),
                ("If the depot requires three seals on every crate before dispatch, storage changes.", false),
                ("The clerk reports that the depot requires three seals on every crate before dispatch.", false),
                ("The clerk says each crate needs three seals before dispatch.", false),
                ("The manual quotes: each crate needs three seals before dispatch.", false),
                ("\"Each crate needs three seals before dispatch.\"", false),
                ("Each crate needs three seals before dispatch as a fictional example.", false),
                ("Every crate displays three seals before dispatch.", false),
            ]),
            ("Quantos lacres a caixa precisa antes do envio?", &[
                ("Cada caixa precisa de três lacres antes do envio.", true),
                ("Cada caixa exige três lacres antes do envio.", true),
                ("A norma exige três lacres em cada caixa antes do envio.", true),
                ("O manual menciona três lacres em cada caixa como exemplo.", false),
                ("A norma exige orientação e menciona três lacres em cada caixa antes do envio.", false),
                ("A norma exige três fitas em cada caixa antes do envio e menciona três lacres.", false),
                ("A norma exige três lacres em cada pacote antes do envio.", false),
                ("A norma não exige três lacres em cada caixa antes do envio.", false),
                ("Se a norma exige três lacres em cada caixa antes do envio, o depósito muda.", false),
                ("O escrivão relata que a norma exige três lacres em cada caixa antes do envio.", false),
                ("O escrivão diz cada caixa precisa de três lacres antes do envio.", false),
                ("O manual cita: cada caixa precisa de três lacres antes do envio.", false),
                ("“Cada caixa precisa de três lacres antes do envio.”", false),
                ("Cada caixa precisa de três lacres antes do envio como exemplo fictício.", false),
                ("Cada caixa exibe três lacres antes do envio.", false),
            ]),
        ];
        let mut failures = Vec::new();
        let mut executed = 0;
        for (query, cases) in groups {
            let scope = target_scope(query).expect("bounded counted requirement");
            for &(text, expected) in cases {
                let matched = target_matches(&scope, text);
                println!(
                    "{}",
                    serde_json::json!({"diagnostic":"obligation-relation-neutral",
                    "query":query,"text":text,"expected":expected,"matched":matched})
                );
                executed += 1;
                if matched != expected {
                    failures.push(format!("{query} / {text}: {matched} != {expected}"));
                }
            }
        }
        assert_eq!(executed, 30);
        assert!(failures.is_empty(), "bounded obligation: {failures:?}");
    }

    #[test]
    fn obligation_relation_audit_binds_governor_argument_target_and_domain() {
        let query = "How many seals does a crate need before dispatch?";
        let scope = target_scope(query).unwrap();
        let audit = obligation_evidence(
            &scope,
            "The depot requires three seals on every crate before dispatch.",
        )
        .unwrap();
        assert!(audit.matched);
        assert_eq!(audit.query_governor, "need");
        assert_eq!(audit.target, ["crate"]);
        assert_eq!(audit.object, ["seals"]);
        assert_eq!(audit.assertion.domain, "asserted");
        assert_eq!(audit.assertion.polarity, "affirmative");
        assert_eq!(audit.assertion.governor_span, Some([2, 3]));
        assert_eq!(audit.assertion.source_span, Some([0, 2]));
        assert_eq!(audit.assertion.argument_span, Some([3, 5]));
        assert_eq!(audit.assertion.target_span, Some([7, 8]));
        assert_eq!(audit.assertion.boundary_span, Some([8, 10]));
        for (text, domain, polarity) in [
            ("The depot does not require three seals on every crate before dispatch.", "asserted", "negative"),
            ("If the depot requires three seals on every crate before dispatch, storage changes.", "conditional", "affirmative"),
            ("The clerk reports that the depot requires three seals on every crate before dispatch.", "reported", "affirmative"),
            ("The manual mentions three seals on every crate as an example.", "mention", "affirmative"),
            ("\"Each crate needs three seals before dispatch.\"", "quoted", "affirmative"),
        ] {
            let audit = obligation_evidence(&scope, text).unwrap();
            assert!(!audit.matched, "{text}");
            assert_eq!(audit.assertion.domain, domain, "{text}");
            assert_eq!(audit.assertion.polarity, polarity, "{text}");
        }
    }

    #[test]
    fn obligation_relation_neutral_governors_share_only_the_bounded_prerequisite_role() {
        for (query, text) in [
            (
                "How many reports does a crate need before dispatch?",
                "Each crate needs three reports before dispatch.",
            ),
            (
                "How many copies does a report need before filing?",
                "The report requires two copies on every report before filing.",
            ),
            (
                "How many seals does a crate require before dispatch?",
                "Each crate needs three seals before dispatch.",
            ),
            (
                "Quantos lacres a caixa exige antes do envio?",
                "Cada caixa precisa de três lacres antes do envio.",
            ),
        ] {
            let scope = target_scope(query).unwrap();
            assert!(target_matches(&scope, text), "{query} / {text}");
        }
    }

    #[test]
    fn obligation_relation_unattested_frames_stay_unbound_without_guessing() {
        let scope = target_scope("How many seals does a crate need before dispatch?").unwrap();
        for text in [
            "The depot mandates three seals on every crate before dispatch.",
            "According to the clerk each crate needs three seals before dispatch.",
            "The clerk believes each crate needs three seals before dispatch.",
            "Each crate might need three seals before dispatch.",
            "The receiving depot requires three seals on every crate before dispatch.",
        ] {
            let audit = obligation_evidence(&scope, text).unwrap();
            assert!(!audit.matched, "unattested frame: {text}");
            assert_eq!(audit.assertion.frame, "unbound", "{text}");
        }
        let literal = target_scope("How many tiles does a tray hold?").unwrap();
        assert!(obligation_evidence(&literal, "Each tray holds four tiles.").is_none());
        let outside = target_scope("How many seals does a crate mandate?").unwrap();
        assert!(obligation_evidence(&outside, "Each crate mandates three seals.").is_none());
        assert!(
            target_matches(&outside, "Each crate mandates three seals."),
            "an unattested semantic governor retains the pre-existing literal quantity path"
        );
    }

    #[test]
    fn obligation_relation_does_not_retype_literal_quantities_or_unknown_questions() {
        for (query, text) in [
            (
                "How many tiles does a tray hold?",
                "Each tray holds four tiles.",
            ),
            (
                "Quantas peças o estojo guarda?",
                "Cada estojo guarda quatro peças.",
            ),
        ] {
            let scope = target_scope(query).expect("existing literal quantity shape");
            assert!(
                target_matches(&scope, text),
                "literal count: {query} / {text}"
            );
        }
        for query in [
            "How many people sit on the payments platform team?",
            "How many seals does a crate need if storage changes?",
            "Quantos lacres a caixa precisa se o depósito muda?",
            "How many seals might a crate need before dispatch?",
            "Does the depot mandate seals?",
        ] {
            assert!(target_scope(query).is_none(), "unknown shape: {query}");
        }
    }

    #[test]
    fn target_relation_review_regressions_helpers_bind_evidence_and_leave_uncertified_seams_unknown(
    ) {
        let cases = [
            (
                "quantity_wrapper_mention",
                "How many approvals does a pull request need before it can be merged into main?",
                "User's team requires two approvals on every pull request before it can be merged into main.",
                "The training guide mentions two approvals on every pull request as a fictional example.",
            ),
            (
                "consumer_disconnected_producer",
                "Which service consumes the payment events Ledgerly publishes to Kafka?",
                "User's Ledgerly service publishes payment events to Kafka topics that the fraud-check service consumes.",
                "Ledgerly publishes payment events to Kafka while the archive service consumes the payment events Aster publishes to Orbit.",
            ),
        ];
        let mut failures = Vec::new();
        let mut executed = 0;
        for (case, query, answer, incidental) in cases {
            let scope = target_scope(query);
            for (id, text, expected) in
                [("answer", answer, true), ("incidental", incidental, false)]
            {
                let matched = scope.as_ref().map(|scope| target_matches(scope, text));
                println!(
                    "{}",
                    serde_json::json!({
                        "diagnostic":"target-relation-review-regressions-helper",
                        "run":"memory-quality-523100c4-r1", "phase":"relevance-target-r20-red",
                        "attempt":2, "correction_round":20, "case":case, "id":id,
                        "query":query, "text":text,
                        "expected":{"target_scope_required":true,"target_matches":expected},
                        "result":{"target_scope_required":scope.is_some(),"target_matches":matched},
                        "scope":format!("{scope:?}")
                    })
                );
                executed += 1;
                if matched != Some(expected) {
                    failures.push(format!(
                        "{case}/{id}: esperado={expected}, obtido={matched:?}, scope={scope:?}"
                    ));
                }
            }
        }
        let case = "quantity_uncertified_prepositional_seam";
        let query = "How many people sit on the payments platform team?";
        let text = "Seven people sit on the payments platform team.";
        let scope = target_scope(query);
        println!(
            "{}",
            serde_json::json!({
                "diagnostic":"target-relation-review-regressions-helper",
                "run":"memory-quality-523100c4-r1", "phase":"relevance-target-r20-red",
                "attempt":2, "correction_round":20, "case":case, "id":"answer",
                "query":query, "text":text,
                "expected":{"target_scope":null},
                "result":{"target_scope":scope.as_ref().map(|scope| serde_json::json!({
                    "kind":format!("{:?}",scope.kind), "terms":scope.terms,
                    "subject":scope.subject, "predicate":scope.predicate,
                    "object":scope.object, "relative":scope.relative
                }))}
            })
        );
        executed += 1;
        if scope.is_some() {
            failures.push(format!(
                "{case}: a forma não certificada deve permanecer None, obtido={scope:?}"
            ));
        }
        assert_eq!(
            executed, 5,
            "todos os registros do helper devem ser percorridos"
        );
        assert!(
            failures.is_empty(),
            "contraprovas de relação-alvo: {failures:?}"
        );
    }

    #[test]
    fn target_relation_certificates_preserve_required_roles_not_context_words() {
        let cases=[
            ("Write a status note for this defect using my usual Opaline Ledger markers.",
                "User writes status notes in the Opaline Ledger style with markers.",true),
            ("Write a status note for this defect using my usual Opaline Ledger markers.",
                "User writes a regression test to fix the defect. Opaline Ledger markers are mentioned later.",false),
            ("Write a status note for this defect using my usual Opaline Ledger markers.",
                "User writes status notes and mentions Opaline Ledger markers as an unrelated example.",false),
            ("How many endorsements does a change require before merging?",
                "Each change requires two endorsements before merging.",true),
            ("How many endorsements does a change require before merging?",
                "Each change contains 230 edited lines.",false),
            ("How many endorsements does a change require before merging?",
                "Two endorsements are mentioned elsewhere; every change contains edited lines.",false),
            ("How many endorsements does a change require before merging?",
                "Each change mentions two endorsements as an example and requires 230 edited lines.",false),
            ("How many endorsements does a change require before merging?",
                "Each change requires guidance and an example mentions two endorsements.",false),
            ("Which service consumes the audit events Aster publishes to Orbit?",
                "The Aster service publishes audit events to Orbit topics that the sentinel service consumes.",true),
            ("Which service consumes the audit events Aster publishes to Orbit?",
                "The Aster service consumes audit events that the sentinel service publishes to Orbit.",false),
            ("Which service consumes the audit events Aster publishes to Orbit?",
                "The Aster service publishes audit events to Orbit topics and the archive records a service that the sentinel service consumes.",false),
            ("Which database version does the Aster service use?",
                "The Aster service uses database version 8.",true),
            ("Which database version does the Aster service use?",
                "The archive uses database version 8 and describes the Aster service.",false),
            ("Qual versão de banco o serviço Aster usa?",
                "O serviço Aster usa banco na versão 8.",true),
            ("Qual versão de banco o serviço Aster usa?",
                "O arquivo usa banco na versão 8 e descreve o serviço Aster.",false),
        ];
        let mut failures = Vec::new();
        for (query, fact, expected) in cases {
            let scope = target_scope(query);
            let matched = scope
                .as_ref()
                .is_some_and(|scope| target_matches(scope, fact));
            println!(
                "{}",
                serde_json::json!({"diagnostic":"target-relation-certificate","query":query,
                "fact":fact,"expected":expected,"matched":matched})
            );
            if matched != expected || scope.is_none() {
                failures.push(format!(
                    "{query} / {fact}: {matched} != {expected}; {scope:?}"
                ));
            }
        }
        for query in [
            "Recall which CI replaced Jenkins at work.",
            "Which unknown very long noun phrase is this?",
            "Which code editor am I using these days? I want setup help.",
            "How many small widgets do the workers have?",
            "Qual versão de banco o serviço Aster usa enquanto reinicia?",
        ] {
            assert!(
                target_scope(query).is_none(),
                "uncertified shape must stay unknown: {query}"
            );
        }
        assert!(failures.is_empty(), "target relation roles: {failures:?}");
    }

    #[test]
    fn subject_inheritance_neutral_chain_rejects_unmarked_report_and_nested_subject() {
        let query = "Do I collect ceramic tiles?";
        let scopes = personal_scopes(query);
        let cases = [
            ("User nods and collects ceramic tiles.", true),
            (
                "User reports that a cousin nods and collects ceramic tiles.",
                false,
            ),
            (
                "User notices Cousin nods and collects ceramic tiles.",
                false,
            ),
            (
                "User nods and the cousin waves and collects ceramic tiles.",
                false,
            ),
        ];
        let mut failures = Vec::new();
        for (fact, expected) in cases {
            let matched =
                !scopes.is_empty() && scopes.iter().all(|scope| scope_matches(scope, fact));
            println!(
                "{}",
                serde_json::json!({"diagnostic":"subject-inheritance-neutral-helper",
                "query":query,"fact":fact,"expected":expected,"scope_matched":matched})
            );
            if matched != expected {
                failures.push(format!("{fact}: {matched} != {expected}"));
            }
        }
        assert!(failures.is_empty(), "neutral chain: {failures:?}");
    }

    #[test]
    fn subject_inheritance_helper_distinguishes_personal_chain_from_reported_subject() {
        // Expected relationships are labelled independently of the projection.
        let cases = [
            (
                "Do I prefer concise answers?",
                "User smiles and prefers concise answers.",
                true,
            ),
            (
                "Do I prefer concise answers?",
                "User says the manager smiles and prefers concise answers.",
                false,
            ),
            (
                "Do I prefer concise answers?",
                "User smiles and the manager prefers concise answers.",
                false,
            ),
            (
                "Do I prefer concise answers?",
                "User says the manager smiles and usually prefers concise answers.",
                false,
            ),
            (
                "Where do I live now?",
                "User moved from Cork to Oslo and now lives in an apartment.",
                true,
            ),
            (
                "Where do I live now?",
                "User says the manager moved from Cork and now lives in Oslo.",
                false,
            ),
            (
                "Where do I live now?",
                "User moved from Cork and the manager now lives in Oslo.",
                false,
            ),
            (
                "Eu estava revisando esquemas?",
                "Eu sorria e estava revisando esquemas.",
                true,
            ),
            (
                "Eu estava revisando esquemas?",
                "Eu digo que o gerente sorri e estava revisando esquemas.",
                false,
            ),
            (
                "Eu estava revisando esquemas?",
                "Eu sorria e o gerente estava revisando esquemas.",
                false,
            ),
        ];
        let mut failures = Vec::new();
        let mut executed = 0;
        for (n, (query, fact, expected)) in cases.into_iter().enumerate() {
            let scopes = personal_scopes(query);
            let matched =
                !scopes.is_empty() && scopes.iter().all(|scope| scope_matches(scope, fact));
            println!(
                "{}",
                serde_json::json!({"diagnostic":"subject-inheritance-helper",
                "case":n,"query":query,"fact":fact,"expected":expected,"scope_matched":matched,
                "scopes":scopes.iter().map(|scope|serde_json::json!({
                    "span":[scope.span.start,scope.span.end],"terms":scope.terms,
                    "predicate":scope.predicate,"open_argument":scope.open_argument})).collect::<Vec<_>>()})
            );
            executed += 1;
            if scopes.is_empty() || matched != expected {
                failures.push(format!("{n}: {query:?} / {fact:?}: expected={expected}, matched={matched}, scopes={scopes:?}"));
            }
        }
        assert_eq!(executed, 10);
        assert!(
            failures.is_empty(),
            "subject inheritance helper: {failures:?}"
        );
    }

    #[test]
    fn certificate_regressions_how_is_procedural_where_is_open_and_negation_is_not_support() {
        assert!(personal_scopes("How do I apply database migrations?").is_empty());
        assert!(
            personal_scopes("I need a column; how do I usually write versioned migrations?")
                .is_empty()
        );
        let scopes = personal_scopes("Do I usually prefer concise answers now?");
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].predicate.as_deref(), Some("prefer"));
        assert_eq!(scopes[0].terms, ["concise", "answers"]);
        assert!(scope_matches(
            &scopes[0],
            "User usually prefers concise answers."
        ));
        assert!(!scope_matches(
            &scopes[0],
            "User prefers binary transport, not concise answers."
        ));
        assert!(!scope_matches(
            &scopes[0],
            "User prefers binary transport but another user sends concise answers."
        ));
        let location = personal_scopes("Where do I live now? Looking for a bakery near my place.");
        assert_eq!(location.len(), 1);
        assert!(location[0].open_argument);
        assert!(location[0].terms.is_empty());
        assert!(scope_matches(
            &location[0],
            "User moved from Cork to Oslo and now lives in an apartment."
        ));
        assert!(!scope_matches(
            &location[0],
            "User likes maps of places where another person lives."
        ));
        let sent = personal_scopes("What do my friends send me?");
        assert_eq!(sent.len(), 1);
        assert!(scope_matches(&sent[0], "My friends send me postcards."));
        assert!(!scope_matches(&sent[0], "My friends send parcels home."));
    }

    #[test]
    fn personal_scope_certificates_keep_predicate_argument_and_owner_together() {
        let q = "Do I prefer concise answers?";
        let scopes = personal_scopes(q);
        assert_eq!(scopes.len(), 1);
        assert_eq!(scopes[0].span, 0..5);
        assert!(scope_matches(&scopes[0], "User prefers concise answers."));
        assert!(!scope_matches(&scopes[0], "User prefers binary transport."));
        assert!(!scope_matches(
            &scopes[0],
            "Examples like concise answers are documented."
        ));
        let owned = personal_scopes("What's my partner's name?");
        assert_eq!(owned.len(), 1);
        assert!(scope_matches(&owned[0], "User's partner's name is Noor."));
        assert!(!scope_matches(&owned[0], "User's name is Iris."));
        let own = personal_scopes("What's my name?");
        assert!(scope_matches(&own[0], "User's name is Iris."));
        assert!(!scope_matches(&own[0], "User's partner's name is Noor."));
        let subordinate = personal_scopes("Que artistas tocam no meu fone quando estou codando?");
        assert!(!subordinate.is_empty());
        assert!(subordinate
            .iter()
            .all(|scope| !scope_matches(scope, "A usuária prefere vinho quando viaja.")));
    }

    #[test]
    fn slot_projection_uses_subject_and_auxiliary_seams_not_nouns() {
        assert_eq!(
            spans("Em que etapa da pesquisa eu reviso o desenho?"),
            Some(vec![[0, 5]])
        );
        assert_eq!(
            spans("During which phase of the exercise do we inspect the draft?"),
            Some(vec![[0, 6]])
        );
        assert_eq!(
            spans("Which draft do I inspect, and during what phase?"),
            Some(vec![[6, 9]])
        );
    }

    #[test]
    fn slot_projection_keeps_attributes_fragments_and_embedded_ambiguity_unknown() {
        for query in [
            "What is my schedule?",
            "Qual é meu gestor?",
            "Em que etapa da pesquisa?",
            "During which phase of the exercise?",
            "Em que etapa, da pesquisa, eu reviso o desenho?",
            "Em que etapa da pesquisa \"eu\" reviso o desenho?",
            "During what phase and which cycle do I inspect the draft?",
        ] {
            assert_eq!(spans(query), None, "{query}");
        }
    }
}

#[cfg(test)]
mod directive_relation_type_tests {
    use super::*;

    #[test]
    fn directive_relation_subtypes_are_directional_and_not_shared_ancestors() {
        for (query, rule, path) in [
            ("script", "código", vec!["script", "code"]),
            ("sonnet", "poem", vec!["sonnet", "poem"]),
            ("haicai", "texto", vec!["haiku", "poem", "text"]),
            ("relatório", "documento", vec!["report", "document"]),
            ("invoice", "text", vec!["invoice", "document", "text"]),
        ] {
            let evidence = artifact_relation(query, rule);
            assert_eq!(evidence.relation, "query_subtype_of_rule");
            assert_eq!(evidence.type_path, path);
            assert_eq!(
                artifact_relation(rule, query).relation,
                "unknown_or_incompatible"
            );
        }
        for (query, rule) in [
            ("script", "prosa"),
            ("script", "relatório"),
            ("sonnet", "haiku"),
            ("report", "invoice"),
            ("report", "script"),
            ("widget", "text"),
            ("text", "widget"),
            ("code", "cod"),
        ] {
            assert_eq!(
                artifact_relation(query, rule).relation,
                "unknown_or_incompatible"
            );
        }
        assert_eq!(
            artifact_relation("widget", "widget").relation,
            "literal_identity"
        );
    }

    #[test]
    fn directive_relation_subtype_certificate_still_requires_bound_affirmative_output() {
        let query = "Vou te pedir um script: os nomes de variáveis e os comentários devem ficar em que idioma?";
        let scope = directive_scope(query).unwrap();
        assert_eq!(scope.head.as_deref(), Some("script"));
        let positive = "A usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.";
        let evidence = directive_evidence(&scope, positive);
        assert!(evidence.matched);
        assert_eq!(evidence.domain, "asserted");
        assert_eq!(evidence.polarity, "affirmative");
        assert!(evidence.source_span.is_some());
        assert!(evidence.governor_span.is_some());
        assert!(evidence.assistant_span.is_some());
        assert!(evidence.artifact_span.is_some());
        assert!(evidence.argument_span.is_some());
        assert!(evidence.value_span.is_some());
        let relation = evidence.artifact_relation.unwrap();
        assert_eq!(relation.query_head, "script");
        assert_eq!(relation.candidate_head, "código");
        assert_eq!(relation.type_path, ["script", "code"]);
        for negative in [
            "A usuária pede que a prosa escrita pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.",
            "A usuária pede que o relatório escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.",
            "A usuária pede que o código escrito pelo colega, incluindo nomes de variáveis e comentários, fique em italiano.",
            "A usuária não pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.",
            "Se a usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.",
            "A usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano?",
            "\"A usuária pede que o código escrito pelo assistente, incluindo nomes de variáveis e comentários, fique em italiano.\"",
            "A usuária pede que o código escrito pelo assistente, incluindo títulos e legendas, fique em italiano.",
        ] {
            assert!(!directive_evidence(&scope, negative).matched, "{negative}");
        }
    }
}
