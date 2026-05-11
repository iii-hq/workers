//! Compile-time and format checks for the registered skill set.
//! Runs without an iii engine connection.
//!
//! Asserts the platform contract from skills/README.md: H1 first (used as
//! the iii://skills index link title), then a non-heading paragraph (used
//! as the description, truncated at 140 chars). Workers in this repo
//! follow folder-name-equals-skill-id; if a future worker uses different
//! naming, adjust id_is_valid accordingly.

fn well_formed(label: &str, body: &str, require_summary: bool) {
    assert!(!body.trim().is_empty(), "{label}: skill is empty");
    assert!(
        body.len() <= 256 * 1024,
        "{label}: skill exceeds 256 KiB ({} bytes)",
        body.len()
    );

    // Skip blank lines and single-line HTML comments (e.g., the renderer's
    // generated banner). The check is in-memory only; the rendered file
    // itself is unchanged.
    let mut lines = body.lines().filter(|l| {
        let t = l.trim();
        !(t.is_empty() || t.starts_with("<!--") && t.ends_with("-->"))
    });
    let h1 = lines.next().unwrap_or("");
    assert!(
        h1.starts_with("# "),
        "{label}: skill must start with an H1, got: {h1:?}"
    );
    if require_summary {
        let summary = lines.next().unwrap_or("");
        assert!(
            !summary.starts_with('#'),
            "{label}: expected a summary paragraph after the H1, got another heading: {summary:?}"
        );
    }
}

fn id_is_valid(label: &str, id: &str) {
    assert!(!id.is_empty(), "{label}: id is empty");
    assert!(id.len() <= 1024, "{label}: id exceeds 1024 chars");

    // `fn` is the only reserved first-segment literal as of skills v0.2.0.
    let first_segment = id.split('/').next().unwrap_or("");
    assert_ne!(
        first_segment, "fn",
        "{label}: first segment must not be the reserved literal `fn`"
    );

    for segment in id.split('/') {
        assert!(
            !segment.is_empty(),
            "{label}: empty path segment in id {id:?}"
        );
        assert!(
            segment.len() <= 64,
            "{label}: segment {segment:?} exceeds 64 chars"
        );
        let first = segment.chars().next().unwrap();
        assert!(
            first.is_ascii_lowercase() || first.is_ascii_digit(),
            "{label}: segment {segment:?} must start with lowercase ASCII letter or digit"
        );
        assert!(
            segment
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
            "{label}: segment {segment:?} has invalid characters"
        );
    }
}

#[test]
fn router_well_formed() {
    well_formed("router", auth_credentials::SKILL_MD, true);
    id_is_valid("router", auth_credentials::SKILL_ID);
}

#[test]
fn sub_skills_well_formed() {
    let prefix = format!("{}/", auth_credentials::SKILL_ID);
    for (id, body) in auth_credentials::SUB_SKILLS {
        // Canonical leaves go directly from the topical H1 to ## When to use,
        // so the summary-paragraph assertion only applies to the router skill.
        well_formed(id, body, false);
        id_is_valid(id, id);
        assert!(
            id.starts_with(&prefix),
            "sub-skill id {id:?} must be nested under the worker id ({}/)",
            auth_credentials::SKILL_ID
        );
    }
}
