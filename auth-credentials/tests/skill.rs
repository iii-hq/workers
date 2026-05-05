//! Compile-time and format checks for the registered skill.
//! Runs without an iii engine connection.
//!
//! Single-segment skill id checks. Workers in this repo all use a flat,
//! folder-name-equals-skill-id convention (see AGENTS-NEW-WORKER.md §10.1).
//! If a future worker adopts nested sub-skills, replace these tests with
//! multi-segment-aware variants.

#[test]
fn skill_md_well_formed() {
    let skill = auth_credentials::SKILL_MD;
    assert!(!skill.trim().is_empty(), "skill.md is empty");
    assert!(
        skill.len() <= 256 * 1024,
        "skill.md exceeds 256 KiB ({} bytes)",
        skill.len()
    );
    let first_non_blank = skill.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    assert!(
        first_non_blank.starts_with("# "),
        "skill.md must start with an H1, got: {first_non_blank:?}"
    );
    assert!(
        skill.lines().any(|l| l.trim() == "## Functions"),
        "skill.md must contain a `## Functions` section"
    );
}

#[test]
fn skill_id_is_valid() {
    let id = auth_credentials::SKILL_ID;
    assert!(!id.is_empty(), "SKILL_ID is empty");
    assert!(id.len() <= 64, "SKILL_ID exceeds 64 chars");
    assert_ne!(id, "fn", "SKILL_ID must not be the reserved literal `fn`");

    let first = id.chars().next().unwrap();
    assert!(
        first.is_ascii_lowercase() || first.is_ascii_digit(),
        "SKILL_ID first char must be lowercase ASCII letter or digit"
    );
    assert!(
        id.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
        "SKILL_ID has invalid characters"
    );
}
