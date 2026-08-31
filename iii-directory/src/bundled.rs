//! Assets bundled with the worker: system prompts and agent profiles.
//! Each ships embedded in the binary and behaves like the harness's
//! built-in default: it is always visible in its family's `list`/`get`, a
//! LOCAL file with the same name shadows it, editing it copy-on-writes that
//! local file, and deleting the local file falls back to the bundled copy
//! immediately — no file is ever seeded or resurrected on disk by the
//! worker itself.

use std::path::PathBuf;

use crate::fs_source::{self, FsAgent};

/// `(file stem, full on-disk form — frontmatter included)`.
const RAW: &[(&str, &str)] = &[("iii-minimal", include_str!("../prompts/iii-minimal.md"))];

/// Bundled agent profiles — the base identities other profiles build on
/// with `extends: <id>`. `iii` is the harness default prompt, verbatim;
/// `iii-minimal` is the minimal directory-first identity (the same file
/// that ships as the bundled system prompt of that name).
const AGENTS_RAW: &[(&str, &str)] = &[
    ("iii", include_str!("../prompts/iii.md")),
    ("iii-minimal", include_str!("../prompts/iii-minimal.md")),
];

/// One bundled prompt, split the way the read paths serve it.
pub struct BundledPrompt {
    pub name: &'static str,
    pub description: String,
    pub body: String,
    /// The full file form (frontmatter included) — what `raw: true`
    /// round-trips into the editor.
    pub raw: &'static str,
}

pub fn bundled_system_prompts() -> impl Iterator<Item = BundledPrompt> {
    RAW.iter().map(|(name, raw)| {
        let (description, body) = split(raw);
        BundledPrompt {
            name,
            description,
            body,
            raw,
        }
    })
}

pub fn bundled_system_prompt(name: &str) -> Option<BundledPrompt> {
    bundled_system_prompts().find(|prompt| prompt.name == name)
}

/// Every bundled agent profile as a scan row (`builtin: true`, empty
/// `abs_path`), parsed through the same frontmatter gate as on-disk
/// profiles. A bundled file that failed the gate would be dropped here —
/// the unit test below rules that out.
pub fn bundled_agents() -> Vec<FsAgent> {
    AGENTS_RAW
        .iter()
        .filter_map(|(id, raw)| {
            let fm = fs_source::parse_agent_frontmatter(raw).ok()?;
            Some(fs_source::agent_from_frontmatter(
                (*id).to_string(),
                fm,
                PathBuf::new(),
                true,
            ))
        })
        .collect()
}

/// The full file form of one bundled agent profile — what `raw: true`
/// round-trips into the editor, and the source an `update` copy-on-writes.
pub fn bundled_agent_raw(id: &str) -> Option<&'static str> {
    AGENTS_RAW
        .iter()
        .find(|(name, _)| *name == id)
        .map(|(_, raw)| *raw)
}

/// Minimal frontmatter split for OUR OWN build-time assets (a test pins
/// that every bundled file parses): `description:` line out of the leading
/// `---` block, everything after the block as the body.
fn split(raw: &str) -> (String, String) {
    let Some(rest) = raw.strip_prefix("---\n") else {
        return (String::new(), raw.to_string());
    };
    let Some((frontmatter, body)) = rest.split_once("\n---\n") else {
        return (String::new(), raw.to_string());
    };
    let description = frontmatter
        .lines()
        .find_map(|line| line.strip_prefix("description:"))
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    (description, body.trim_start_matches('\n').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every bundled asset must actually parse — a malformed frontmatter
    /// block would ship an unusable built-in.
    #[test]
    fn bundled_prompts_parse_and_carry_the_minimal_identity() {
        let prompts: Vec<_> = bundled_system_prompts().collect();
        assert!(!prompts.is_empty());
        for prompt in &prompts {
            assert!(!prompt.description.is_empty(), "{}", prompt.name);
            assert!(!prompt.body.trim().is_empty(), "{}", prompt.name);
            assert!(prompt.raw.starts_with("---\n"), "{}", prompt.name);
            crate::functions::prompts::validate_name(prompt.name).unwrap();
        }
        let minimal = bundled_system_prompt("iii-minimal").unwrap();
        assert!(minimal.body.starts_with("You are an iii agent."));
        assert!(bundled_system_prompt("nope").is_none());
    }

    #[test]
    fn bundled_agents_parse_and_carry_the_base_identity() {
        let agents = bundled_agents();
        assert_eq!(
            agents.len(),
            AGENTS_RAW.len(),
            "every bundled profile must pass the frontmatter gate"
        );
        for agent in &agents {
            assert!(agent.builtin, "{}", agent.name);
            assert!(agent.abs_path.as_os_str().is_empty(), "{}", agent.name);
            assert!(
                agent.extends.is_none(),
                "{}: a bundled base extends nothing",
                agent.name
            );
            assert!(!agent.description.is_empty(), "{}", agent.name);
            crate::functions::prompts::validate_name(&agent.name).unwrap();
        }
        assert!(bundled_agent_raw("iii").is_some());
        let (_, minimal) = fs_source::split_frontmatter(bundled_agent_raw("iii-minimal").unwrap());
        assert!(minimal.starts_with("You are an iii agent."));
        assert!(bundled_agent_raw("nope").is_none());
    }

    /// The bundled `iii` body IS the harness default identity, byte for byte.
    /// The include below resolves only in the monorepo layout (test-only),
    /// which is the point: the two copies cannot drift without this failing.
    #[test]
    fn bundled_iii_agent_body_is_the_harness_default_prompt() {
        let raw = bundled_agent_raw("iii").unwrap();
        let (_, body) = fs_source::split_frontmatter(raw);
        assert_eq!(body, include_str!("../../harness/prompts/default.txt"));
    }
}
