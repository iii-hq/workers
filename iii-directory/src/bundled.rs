//! System prompts bundled with the worker. Each ships embedded in the
//! binary and behaves like the harness's built-in default: it is always
//! visible in `directory::system-prompts::list`/`::get`, a LOCAL file with
//! the same name shadows it, editing it copy-on-writes that local file, and
//! deleting the local file falls back to the bundled copy immediately — no
//! file is ever seeded or resurrected on disk by the worker itself.

/// `(file stem, full on-disk form — frontmatter included)`.
const RAW: &[(&str, &str)] = &[("iii-minimal", include_str!("../prompts/iii-minimal.md"))];

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
}
