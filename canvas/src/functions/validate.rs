//! `canvas::validate` — check source without storing anything.
//!
//! Lets an agent verify generated mermaid (or an excalidraw scene JSON)
//! before `canvas::create`/`canvas::update`, so a broken diagram never lands
//! in the store or renders as an error card in chat.
//!
//! Honest scope: this is a cheap pre-flight, NOT a mermaid parser. It checks
//! family detection, the size cap, balanced fences (only for families where
//! bracket characters always pair — sequence arrows like `-)` and er
//! cardinalities like `o{` would false-positive), and a handful of per-family
//! lints. Full parsing happens at render time in the console; a source this
//! function passes can still fail to render.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::functions::family;
use crate::store::{CanvasFormat, Store};

pub const ID: &str = "canvas::validate";
pub const DESC: &str = "Validate canvas source without storing it: detect the mermaid diagram \
                        family, check the size cap, balanced fences and per-family lints, or \
                        check an excalidraw scene JSON's shape. A cheap pre-flight — full \
                        parsing happens at render time in the console.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct Request {
    /// Format the source claims to be: `mermaid` or `freeform`. Omit to
    /// auto-detect — a source whose first character is `{` is treated as an
    /// excalidraw scene, anything else as mermaid text.
    #[serde(default)]
    pub format: Option<CanvasFormat>,

    /// The source to validate: mermaid text, or an excalidraw scene JSON
    /// string.
    pub source: String,
}

/// One problem found in the source.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ValidationIssue {
    /// 1-indexed source line the issue points at, when known.
    pub line: Option<u32>,

    /// Human-readable description of the issue.
    pub message: String,
}

impl ValidationIssue {
    fn at(line: u32, message: impl Into<String>) -> Self {
        Self {
            line: Some(line),
            message: message.into(),
        }
    }

    fn general(message: impl Into<String>) -> Self {
        Self {
            line: None,
            message: message.into(),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// `true` when every cheap check passed. This is a pre-flight verdict,
    /// not a render guarantee — full parsing happens in the console.
    pub valid: bool,

    /// The mermaid diagram family derived from the source (`flowchart`,
    /// `sequenceDiagram`, …). `null` for freeform or when the first
    /// meaningful line names no supported family.
    pub family: Option<String>,

    /// Every issue found; empty when `valid` is `true`.
    pub issues: Vec<ValidationIssue>,
}

pub async fn handle(_store: &Store, req: Request, cfg: &WorkerConfig) -> Result<Response, String> {
    let format = req.format.unwrap_or_else(|| infer_format(&req.source));
    let (family, issues) = match format {
        CanvasFormat::Mermaid => check_mermaid(&req.source, cfg.max_source_bytes),
        CanvasFormat::Freeform => (None, check_freeform(&req.source, cfg.max_source_bytes)),
    };
    Ok(Response {
        valid: issues.is_empty(),
        family,
        issues,
    })
}

/// A source whose first non-whitespace character is `{` can only be a scene
/// JSON — no mermaid family header starts that way.
pub(crate) fn infer_format(source: &str) -> CanvasFormat {
    if source.trim_start().starts_with('{') {
        CanvasFormat::Freeform
    } else {
        CanvasFormat::Mermaid
    }
}

/// Families whose bracket characters always come in pairs, so a global
/// balance check cannot false-positive. Sequence arrows (`-)`), er
/// cardinalities (`o{`, `|{`) and free-text families are deliberately absent.
const BRACKET_CHECKED: &[&str] = &[
    "flowchart",
    "classDiagram",
    "stateDiagram",
    "mindmap",
    "architecture-beta",
    "block-beta",
    "kanban",
    "C4Context",
    "radar-beta",
];

/// All mermaid checks: size cap, family detection, fences, per-family lints.
pub(crate) fn check_mermaid(
    source: &str,
    max_source_bytes: usize,
) -> (Option<String>, Vec<ValidationIssue>) {
    let mut issues = Vec::new();
    if source.trim().is_empty() {
        issues.push(ValidationIssue::general(
            "source is empty — pass mermaid text starting with a diagram family keyword",
        ));
        return (None, issues);
    }
    check_size(source, max_source_bytes, &mut issues);

    let family = family::detect(source);
    match &family {
        None => {
            let line = family::first_meaningful_line(source).map(|(_, n)| n);
            issues.push(ValidationIssue {
                line,
                message: format!(
                    "first line must start with a supported diagram family keyword \
                     (got {:?}) — canvas::syntax lists all of them",
                    family::first_meaningful_line(source)
                        .and_then(|(l, _)| l.split_whitespace().next())
                        .unwrap_or("")
                ),
            });
        }
        Some(fam) => {
            if BRACKET_CHECKED.contains(&fam.as_str()) {
                check_fences(source, &mut issues);
            }
            check_quotes(source, &mut issues);
            lint_family(fam, source, &mut issues);
        }
    }
    (family, issues)
}

/// Excalidraw scene checks: size cap, JSON well-formedness (with the parser's
/// line number), object root, `elements` array, `type` when present.
pub(crate) fn check_freeform(source: &str, max_source_bytes: usize) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    if source.trim().is_empty() {
        issues.push(ValidationIssue::general(
            "source is empty — pass an excalidraw scene JSON object",
        ));
        return issues;
    }
    check_size(source, max_source_bytes, &mut issues);

    let scene: serde_json::Value = match serde_json::from_str(source) {
        Ok(v) => v,
        Err(e) => {
            issues.push(ValidationIssue::at(
                e.line() as u32,
                format!("scene is not valid JSON: {e}"),
            ));
            return issues;
        }
    };
    let Some(obj) = scene.as_object() else {
        issues.push(ValidationIssue::general(
            "scene JSON must be an object with an `elements` array",
        ));
        return issues;
    };
    match obj.get("elements") {
        None => issues.push(ValidationIssue::general(
            "scene has no `elements` key — an excalidraw scene carries its shapes there",
        )),
        Some(v) if !v.is_array() => issues.push(ValidationIssue::general(
            "scene `elements` must be an array",
        )),
        Some(_) => {}
    }
    if let Some(t) = obj.get("type").and_then(|v| v.as_str()) {
        if t != "excalidraw" {
            issues.push(ValidationIssue::general(format!(
                "scene `type` is {t:?}; excalidraw scenes use \"excalidraw\""
            )));
        }
    }
    issues
}

fn check_size(source: &str, max_source_bytes: usize, issues: &mut Vec<ValidationIssue>) {
    if source.len() > max_source_bytes {
        issues.push(ValidationIssue::general(format!(
            "source is {} bytes; the configured cap is {} bytes — split the diagram or \
             raise max_source_bytes",
            source.len(),
            max_source_bytes
        )));
    }
}

/// Global `()[]{}` balance over non-comment lines, quoted spans ignored.
fn check_fences(source: &str, issues: &mut Vec<ValidationIssue>) {
    let mut stack: Vec<(char, u32)> = Vec::new();
    for (idx, raw) in source.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let line = raw.trim();
        if line.starts_with("%%") {
            continue;
        }
        let mut in_quotes = false;
        for ch in line.chars() {
            match ch {
                '"' => in_quotes = !in_quotes,
                _ if in_quotes => {}
                '(' | '[' | '{' => stack.push((ch, line_no)),
                ')' | ']' | '}' => {
                    let expected = match ch {
                        ')' => '(',
                        ']' => '[',
                        _ => '{',
                    };
                    match stack.pop() {
                        Some((open, _)) if open == expected => {}
                        Some((open, open_line)) => {
                            issues.push(ValidationIssue::at(
                                line_no,
                                format!(
                                    "'{ch}' does not match the '{open}' opened on line {open_line}"
                                ),
                            ));
                            return;
                        }
                        None => {
                            issues.push(ValidationIssue::at(
                                line_no,
                                format!("'{ch}' closes nothing"),
                            ));
                            return;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if let Some((open, line)) = stack.first() {
        issues.push(ValidationIssue::at(
            *line,
            format!("'{open}' is never closed"),
        ));
    }
}

/// Double quotes never span lines in mermaid; an odd count on one line is a
/// label that swallowed the rest of the line.
fn check_quotes(source: &str, issues: &mut Vec<ValidationIssue>) {
    for (idx, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with("%%") {
            continue;
        }
        if line.chars().filter(|c| *c == '"').count() % 2 != 0 {
            issues.push(ValidationIssue::at(
                (idx + 1) as u32,
                "unbalanced double quote on this line",
            ));
            return;
        }
    }
}

/// Per-family cheap lints — each one catches a diagram that would render as
/// an empty or broken card, nothing subtler.
fn lint_family(family: &str, source: &str, issues: &mut Vec<ValidationIssue>) {
    let body: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"))
        .skip(1)
        .collect();
    match family {
        "flowchart" => {
            let has_edge = body.iter().any(|l| {
                l.contains("-->")
                    || l.contains("---")
                    || l.contains("-.-")
                    || l.contains("==>")
                    || l.contains("--o")
                    || l.contains("--x")
                    || l.contains("~~~")
            });
            if body.len() >= 2 && !has_edge {
                issues.push(ValidationIssue::general(
                    "flowchart declares several nodes but no edges (-->, ---, -.->, ==>)",
                ));
            }
        }
        "sequenceDiagram" => {
            let has_message = body.iter().any(|l| {
                l.contains("->>")
                    || l.contains("-->>")
                    || l.contains("-)")
                    || l.contains("--)")
                    || l.contains("-x")
                    || l.contains("->")
            });
            if !has_message {
                issues.push(ValidationIssue::general(
                    "sequenceDiagram has no messages (A->>B: text)",
                ));
            }
        }
        "erDiagram" => {
            let has_content = body
                .iter()
                .any(|l| l.contains("--") || l.contains("..") || l.contains('{'));
            if !has_content {
                issues.push(ValidationIssue::general(
                    "erDiagram has no relationships (A ||--o{ B : label) or entity blocks",
                ));
            }
        }
        "gantt" => {
            let has_task = body.iter().any(|l| {
                l.contains(':') && !l.starts_with("title") && !l.starts_with("dateFormat")
            });
            if !has_task {
                issues.push(ValidationIssue::general(
                    "gantt has no tasks (name : id, start, duration under a section)",
                ));
            }
        }
        "pie" => {
            let has_slice = body.iter().any(|l| l.starts_with('"') && l.contains(':'));
            if !has_slice {
                issues.push(ValidationIssue::general(
                    "pie has no slices (\"label\" : value)",
                ));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WorkerConfig {
        WorkerConfig::default()
    }

    async fn run(req: Request) -> Response {
        let store = Store::in_memory();
        handle(&store, req, &cfg()).await.expect("validate answers")
    }

    #[tokio::test]
    async fn a_clean_flowchart_is_valid_with_its_family() {
        let out = run(Request {
            format: Some(CanvasFormat::Mermaid),
            source: "flowchart TD\n  A[Start] --> B{OK?}\n  B -->|yes| C[Ship]\n".into(),
        })
        .await;
        assert!(out.valid, "issues: {:?}", out.issues);
        assert_eq!(out.family.as_deref(), Some("flowchart"));
    }

    #[tokio::test]
    async fn an_unknown_header_is_invalid_and_points_at_its_line() {
        let out = run(Request {
            format: None,
            source: "\nplantuml\nA -> B\n".into(),
        })
        .await;
        assert!(!out.valid);
        assert_eq!(out.family, None);
        assert_eq!(out.issues[0].line, Some(2));
        assert!(out.issues[0].message.contains("plantuml"));
    }

    #[tokio::test]
    async fn unbalanced_flowchart_brackets_are_reported_with_a_line() {
        let out = run(Request {
            format: None,
            source: "flowchart TD\n  A[Start --> B\n".into(),
        })
        .await;
        assert!(!out.valid);
        assert!(out
            .issues
            .iter()
            .any(|i| i.line == Some(2) && i.message.contains("never closed")));
    }

    /// Sequence arrows spell `-)` and er cardinalities spell `o{` — families
    /// whose syntax uses lone bracket characters must not be fence-checked.
    #[tokio::test]
    async fn sequence_arrows_and_er_cardinalities_do_not_false_positive() {
        let seq = run(Request {
            format: None,
            source: "sequenceDiagram\n  A-)B: async ping\n  B-->>A: pong\n".into(),
        })
        .await;
        assert!(seq.valid, "issues: {:?}", seq.issues);
        assert_eq!(seq.family.as_deref(), Some("sequenceDiagram"));

        let er = run(Request {
            format: None,
            source: "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n".into(),
        })
        .await;
        assert!(er.valid, "issues: {:?}", er.issues);
    }

    #[tokio::test]
    async fn empty_and_oversized_sources_are_invalid() {
        let empty = run(Request {
            format: None,
            source: "   \n".into(),
        })
        .await;
        assert!(!empty.valid);

        let small_cap = WorkerConfig {
            max_source_bytes: 10,
            ..WorkerConfig::default()
        };
        let store = Store::in_memory();
        let big = handle(
            &store,
            Request {
                format: None,
                source: "flowchart TD\n  A --> B\n".into(),
            },
            &small_cap,
        )
        .await
        .expect("answers");
        assert!(!big.valid);
        assert!(big.issues.iter().any(|i| i.message.contains("cap")));
    }

    #[tokio::test]
    async fn a_multi_node_flowchart_without_edges_is_flagged() {
        let out = run(Request {
            format: None,
            source: "flowchart TD\n  A[one]\n  B[two]\n".into(),
        })
        .await;
        assert!(!out.valid);
        assert!(out.issues[0].message.contains("no edges"));
    }

    #[tokio::test]
    async fn freeform_json_errors_carry_the_parser_line() {
        let out = run(Request {
            format: Some(CanvasFormat::Freeform),
            source: "{\n  \"elements\": [\n}".into(),
        })
        .await;
        assert!(!out.valid);
        assert_eq!(out.family, None);
        assert!(out.issues[0].line.is_some());
        assert!(out.issues[0].message.contains("not valid JSON"));
    }

    #[tokio::test]
    async fn freeform_shape_checks_cover_root_elements_and_type() {
        let not_object = run(Request {
            format: Some(CanvasFormat::Freeform),
            source: "[1, 2]".into(),
        })
        .await;
        assert!(!not_object.valid);

        let no_elements = run(Request {
            format: Some(CanvasFormat::Freeform),
            source: "{\"appState\": {}}".into(),
        })
        .await;
        assert!(!no_elements.valid);
        assert!(no_elements.issues[0].message.contains("elements"));

        let wrong_type = run(Request {
            format: Some(CanvasFormat::Freeform),
            source: "{\"type\": \"drawio\", \"elements\": []}".into(),
        })
        .await;
        assert!(!wrong_type.valid);

        let ok = run(Request {
            format: Some(CanvasFormat::Freeform),
            source: "{\"type\": \"excalidraw\", \"elements\": []}".into(),
        })
        .await;
        assert!(ok.valid, "issues: {:?}", ok.issues);
    }

    /// Omitting `format` auto-detects: `{`-first source is a scene, anything
    /// else is mermaid.
    #[tokio::test]
    async fn format_is_inferred_when_omitted() {
        let scene = run(Request {
            format: None,
            source: "{\"elements\": []}".into(),
        })
        .await;
        assert!(scene.valid, "issues: {:?}", scene.issues);
        assert_eq!(scene.family, None);

        let mermaid = run(Request {
            format: None,
            source: "pie title Languages\n  \"Rust\" : 60\n  \"TS\" : 40\n".into(),
        })
        .await;
        assert!(mermaid.valid, "issues: {:?}", mermaid.issues);
        assert_eq!(mermaid.family.as_deref(), Some("pie"));
    }
}
