//! Canonical system prompt builder. Used by the provisioning state on each
//! fresh `run::start`. Override path: if the caller put a non-empty
//! `system_prompt` in the run request, it's used verbatim.
//!
//! Snapshot tests pin the wording so the move from client to server doesn't
//! drop a section. Plan-eng-review §3 flagged this as a regression risk.

const BASE_BODY: &str = "You can call any iii function on the bus through the single tool `agent_call`.\nPass `function` (e.g. \"shell::fs::ls\") and `payload` (its arguments).\n\nThe skills loaded into your context describe which functions exist, when to\nuse them, and what arguments they take. Read skills before you call functions\nyou haven't used yet — calling an id that doesn't exist returns\n`{error: \"function_not_found\", ...}`. Do not retry the same id; load the\nrelevant skill or ask the user.\n\nPaths must be absolute. If a tool result contains `blocked: true`, a policy\nrefused it — explain which policy and stop, do not retry.";

/// Build the canonical system prompt. Override → verbatim. Otherwise:
/// `BASE_BODY` + working-directory section + skills index section.
///
/// `skills_index` is the raw `iii://skills` index payload; `None` triggers
/// the fallback section that tells the model to fetch it lazily.
/// `cwd` is the per-session working directory; `None` skips the section.
/// `override_prompt` is the caller-supplied prompt; non-empty → returned as-is.
pub fn build(
    skills_index: Option<&str>,
    cwd: Option<&str>,
    override_prompt: Option<&str>,
) -> String {
    if let Some(p) = override_prompt {
        if !p.is_empty() {
            return p.to_string();
        }
    }

    let cwd_section = match cwd {
        Some(c) if !c.is_empty() => format!(
            "## Working directory\n{c}\nPrefer paths under this directory. Use absolute paths.\n\n"
        ),
        _ => String::new(),
    };

    let skills_section = match skills_index {
        Some(s) if !s.is_empty() => format!(
            "## Available skills\n\n{s}\n\nThe section above already contains the skills index AND the bodies of every root-level skill — do NOT call `skill::fetch` for any `iii://<root>` URI listed above; you already have its content. Use `skill::fetch` ONLY to load deeper sub-skill URIs (e.g. `iii://resend/email/send`) that are referenced from a root body but not inlined here. If a function id isn't covered by what's loaded, call `engine::functions::list` via `agent_call` to confirm it exists and read its `request_format`. Never invent function ids."
        ),
        _ => "## Available skills\n\n(Skills index not loaded — call `skill::fetch` via `agent_call` with `uri: \"iii://skills\"` to discover what's registered. For the live function set + schemas, use `engine::functions::list`.)".to_string(),
    };

    format!("{BASE_BODY}\n\n{cwd_section}{skills_section}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_returns_verbatim_when_non_empty() {
        let out = build(Some("idx"), Some("/tmp"), Some("custom prompt"));
        assert_eq!(out, "custom prompt");
    }

    #[test]
    fn empty_override_falls_through_to_canonical() {
        let out = build(Some("idx"), Some("/tmp"), Some(""));
        assert!(out.contains("agent_call"));
        assert!(out.contains("/tmp"));
        assert!(out.contains("idx"));
    }

    #[test]
    fn canonical_includes_base_cwd_and_skills_sections() {
        let out = build(Some("- iii://skills/echo"), Some("/work/proj"), None);
        assert!(out.contains("agent_call"));
        assert!(out.contains("blocked: true"));
        assert!(out.contains("## Working directory"));
        assert!(out.contains("/work/proj"));
        assert!(out.contains("## Available skills"));
        assert!(out.contains("iii://skills/echo"));
        assert!(
            out.contains("do NOT call `skill::fetch`"),
            "must instruct against re-fetching root URIs already inlined"
        );
        assert!(
            out.contains("`skill::fetch`"),
            "must still mention `skill::fetch` for deeper sub-skill loads"
        );
    }

    /// Pins the reconciliation with `harness/docs/iii-skill.md`: skills are
    /// curated docs *over* the live function set; `engine::functions::list`
    /// is the source of truth for existence + schemas. A revert to
    /// "skills index = source of truth" would drop these substrings.
    #[test]
    fn skills_section_points_at_engine_functions_list_for_uncovered_ids() {
        let out = build(Some("- iii://skills/echo"), None, None);
        assert!(
            out.contains("engine::functions::list"),
            "skills section must direct the agent to the live function set"
        );
        assert!(
            out.contains("Never invent function ids"),
            "skills section must keep the no-invention rule"
        );
    }

    #[test]
    fn skills_fallback_text_when_index_missing() {
        let out = build(None, Some("/tmp"), None);
        assert!(out.contains("Skills index not loaded"));
        assert!(out.contains("iii://skills"));
        assert!(
            out.contains("engine::functions::list"),
            "fallback must still point at the live function set"
        );
    }

    #[test]
    fn cwd_section_omitted_when_cwd_empty() {
        let out = build(None, None, None);
        assert!(!out.contains("## Working directory"));
    }

    // ── Adversarial unit tests added per plan
    // /Users/ytallolayon/.claude/plans/let-s-implement-more-tests-refactored-flask.md
    //
    // The builder is intentionally a verbatim concatenator — input
    // sanitization is the caller's job. These tests pin that contract so
    // a future change that adds escaping/sanitization here is a deliberate
    // decision, not a silent drift.

    #[test]
    fn cwd_with_newlines_passes_through_verbatim() {
        let out = build(None, Some("/tmp/path\nmore"), None);
        assert!(
            out.contains("/tmp/path\nmore"),
            "embedded newline should reach the assembled prompt verbatim"
        );
    }

    #[test]
    fn skills_index_with_markdown_passes_through_verbatim() {
        let weird = "- iii://skills/foo\n\n```rust\nbad\n```\n";
        let out = build(Some(weird), None, None);
        assert!(
            out.contains(weird),
            "skills_index markdown should reach the prompt unchanged"
        );
    }

    #[test]
    fn large_override_prompt_returns_same_length() {
        let huge = "a".repeat(1_000_000);
        let out = build(Some("idx"), Some("/tmp"), Some(&huge));
        assert_eq!(
            out.len(),
            1_000_000,
            "override is verbatim — no implicit truncation"
        );
        assert_eq!(out, huge);
    }
}
