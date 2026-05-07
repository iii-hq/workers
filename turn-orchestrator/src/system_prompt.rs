//! Canonical system prompt builder. Used by the provisioning state on each
//! fresh `run::start`. Override path: if the caller put a non-empty
//! `system_prompt` in the run request, it's used verbatim.
//!
//! Snapshot tests pin the wording so the move from client to server doesn't
//! drop a section. Plan-eng-review §3 flagged this as a regression risk.

const BASE_BODY: &str = "You can call any iii function on the bus through the single tool `agent_call`.\nPass `function` (e.g. \"shell::filesystem::ls\") and `payload` (its arguments).\n\nThe skills loaded into your context describe which functions exist, when to\nuse them, and what arguments they take. Read skills before you call functions\nyou haven't used yet — calling an id that doesn't exist returns\n`{error: \"function_not_found\", ...}`. Do not retry the same id; load the\nrelevant skill or ask the user.\n\nPaths must be absolute. If a tool result contains `blocked: true`, a policy\nrefused it — explain which policy and stop, do not retry.";

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
            "## Available skills\n\n{s}\n\nCall `skill::fetch` via `agent_call` to load any `iii://` URI you see above when you need its full content."
        ),
        _ => "## Available skills\n\n(Skills index not loaded — call `skill::fetch` via `agent_call` with `uri: \"iii://skills\"` to discover what's registered.)".to_string(),
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
        assert!(out.contains("`skill::fetch` via `agent_call`"));
    }

    #[test]
    fn skills_fallback_text_when_index_missing() {
        let out = build(None, Some("/tmp"), None);
        assert!(out.contains("Skills index not loaded"));
        assert!(out.contains("iii://skills"));
    }

    #[test]
    fn cwd_section_omitted_when_cwd_empty() {
        let out = build(None, None, None);
        assert!(!out.contains("## Working directory"));
    }
}
