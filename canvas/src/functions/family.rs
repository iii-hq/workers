//! Mermaid diagram-family detection, shared by create, update, validate and
//! syntax.
//!
//! Detection is deliberately shallow: the family is named by the first
//! meaningful line of the source (mermaid's own rule), so this looks at that
//! one line and nothing else. Full parsing happens at render time in the
//! console — this module only answers "which dialect does this source claim
//! to be".

/// Every family the console's mermaid bundle renders, in the order the
/// syntax reference presents them. `graph` is accepted as input but
/// canonicalized to `flowchart`; `stateDiagram-v2` canonicalizes to
/// `stateDiagram`.
pub const FAMILIES: &[&str] = &[
    "flowchart",
    "sequenceDiagram",
    "classDiagram",
    "stateDiagram",
    "erDiagram",
    "journey",
    "gantt",
    "pie",
    "quadrantChart",
    "requirementDiagram",
    "gitGraph",
    "C4Context",
    "mindmap",
    "timeline",
    "packet",
    "kanban",
    "architecture-beta",
    "block-beta",
    "sankey-beta",
    "xychart-beta",
    "radar-beta",
    "treemap-beta",
];

/// Map a header token to its canonical family name, or `None` when the token
/// names no supported family.
fn canonicalize(token: &str) -> Option<&'static str> {
    let token = token.trim_end_matches([':', ';']);
    let canonical = match token {
        "graph" => "flowchart",
        "stateDiagram-v2" => "stateDiagram",
        "packet-beta" => "packet",
        other => other,
    };
    FAMILIES.iter().copied().find(|f| *f == canonical)
}

/// The first line that can carry the family header: not blank, not a `%%`
/// comment or directive, and past an optional `---` YAML frontmatter block.
/// Returns the line and its 1-indexed number.
pub fn first_meaningful_line(source: &str) -> Option<(&str, u32)> {
    let mut in_frontmatter = false;
    for (idx, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // The function returns at the first content line, so any `---` seen
        // here can only be a frontmatter fence, never an edge like `A --- B`.
        if line == "---" {
            in_frontmatter = !in_frontmatter;
            continue;
        }
        if in_frontmatter {
            continue;
        }
        if line.starts_with("%%") {
            continue;
        }
        return Some((line, (idx + 1) as u32));
    }
    None
}

/// Detect the mermaid family from the first meaningful line of `source`.
/// `None` when the source names no supported family — the caller decides
/// whether that is an error (validate) or just an untyped canvas (create).
pub fn detect(source: &str) -> Option<String> {
    let (line, _) = first_meaningful_line(source)?;
    let token = line.split_whitespace().next()?;
    canonicalize(token).map(str::to_string)
}

/// Resolve a caller-supplied family name to its canonical form, accepting the
/// header aliases (`graph`, `stateDiagram-v2`, `packet-beta`) and common
/// shorthands (`sequence`, `class`, `state`, `er`, `architecture`, …)
/// case-insensitively.
pub fn normalize(input: &str) -> Option<&'static str> {
    let trimmed = input.trim();
    if let Some(found) = canonicalize(trimmed) {
        return Some(found);
    }
    let lower = trimmed.to_ascii_lowercase();
    let alias = match lower.as_str() {
        "graph" => "flowchart",
        "packet-beta" => "packet",
        "sequence" => "sequenceDiagram",
        "class" => "classDiagram",
        "state" | "statediagram-v2" => "stateDiagram",
        "er" | "entity-relationship" => "erDiagram",
        "git" | "gitgraph" => "gitGraph",
        "c4" | "c4context" => "C4Context",
        "quadrant" | "quadrantchart" => "quadrantChart",
        "requirement" | "requirementdiagram" => "requirementDiagram",
        "architecture" => "architecture-beta",
        "block" => "block-beta",
        "sankey" => "sankey-beta",
        "xychart" => "xychart-beta",
        "radar" => "radar-beta",
        "treemap" => "treemap-beta",
        _ => "",
    };
    if !alias.is_empty() {
        return FAMILIES.iter().copied().find(|f| *f == alias);
    }
    FAMILIES
        .iter()
        .copied()
        .find(|f| f.to_ascii_lowercase() == lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every documented header token detects, and detects to its canonical
    /// family. This is the table the brief's detection contract lives in.
    #[test]
    fn every_header_token_detects_to_its_canonical_family() {
        let table: &[(&str, &str)] = &[
            ("flowchart TD", "flowchart"),
            ("graph LR", "flowchart"),
            ("sequenceDiagram", "sequenceDiagram"),
            ("classDiagram", "classDiagram"),
            ("stateDiagram", "stateDiagram"),
            ("stateDiagram-v2", "stateDiagram"),
            ("erDiagram", "erDiagram"),
            ("journey", "journey"),
            ("gantt", "gantt"),
            ("pie showData", "pie"),
            ("quadrantChart", "quadrantChart"),
            ("requirementDiagram", "requirementDiagram"),
            ("gitGraph", "gitGraph"),
            ("gitGraph LR:", "gitGraph"),
            ("C4Context", "C4Context"),
            ("mindmap", "mindmap"),
            ("timeline", "timeline"),
            ("packet", "packet"),
            ("packet-beta", "packet"),
            ("kanban", "kanban"),
            ("architecture-beta", "architecture-beta"),
            ("block-beta", "block-beta"),
            ("sankey-beta", "sankey-beta"),
            ("xychart-beta", "xychart-beta"),
            ("radar-beta", "radar-beta"),
            ("treemap-beta", "treemap-beta"),
        ];
        for (header, family) in table {
            let source = format!("{header}\n  A --> B\n");
            assert_eq!(
                detect(&source).as_deref(),
                Some(*family),
                "header {header:?} should detect as {family}"
            );
        }
    }

    #[test]
    fn detection_skips_blanks_comments_directives_and_frontmatter() {
        let source = "\n\n%% a comment\n%%{init: {'theme':'dark'}}%%\nflowchart TD\n  A --> B\n";
        assert_eq!(detect(source).as_deref(), Some("flowchart"));

        let with_frontmatter = "---\ntitle: Checkout\n---\nsequenceDiagram\n  A->>B: hi\n";
        assert_eq!(detect(with_frontmatter).as_deref(), Some("sequenceDiagram"));
    }

    #[test]
    fn unknown_or_empty_sources_detect_nothing() {
        assert_eq!(detect(""), None);
        assert_eq!(detect("   \n  \n"), None);
        assert_eq!(detect("%% only a comment\n"), None);
        assert_eq!(detect("plantuml\nA -> B\n"), None);
        // Family keywords are case-sensitive in mermaid itself.
        assert_eq!(detect("FLOWCHART TD\nA --> B\n"), None);
    }

    #[test]
    fn first_meaningful_line_reports_the_real_line_number() {
        let source = "\n%% note\nflowchart TD\n  A --> B\n";
        let (line, number) = first_meaningful_line(source).expect("has content");
        assert_eq!(line, "flowchart TD");
        assert_eq!(number, 3);
    }

    #[test]
    fn normalize_accepts_canonical_names_aliases_and_case_slop() {
        assert_eq!(normalize("flowchart"), Some("flowchart"));
        assert_eq!(normalize("graph"), Some("flowchart"));
        assert_eq!(normalize("sequence"), Some("sequenceDiagram"));
        assert_eq!(normalize("sequencediagram"), Some("sequenceDiagram"));
        assert_eq!(normalize("stateDiagram-v2"), Some("stateDiagram"));
        assert_eq!(normalize("ER"), Some("erDiagram"));
        assert_eq!(normalize("architecture"), Some("architecture-beta"));
        assert_eq!(normalize("C4"), Some("C4Context"));
        assert_eq!(normalize("treemap"), Some("treemap-beta"));
        assert_eq!(normalize("nonsense"), None);
    }
}
