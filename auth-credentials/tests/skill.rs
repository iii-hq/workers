//! Compile-time and format checks for the registered skill bundle.

use serde_yaml::Value;

fn split_frontmatter(label: &str, body: &str) -> (Value, String) {
    let rest = body
        .strip_prefix("---\n")
        .unwrap_or_else(|| panic!("{label}: missing opening frontmatter fence"));
    let (yaml, markdown) = rest
        .split_once("\n---\n")
        .unwrap_or_else(|| panic!("{label}: missing closing frontmatter fence"));
    let fm = serde_yaml::from_str(yaml)
        .unwrap_or_else(|err| panic!("{label}: invalid YAML frontmatter: {err}"));
    (fm, markdown.to_string())
}

fn frontmatter_str<'a>(label: &str, fm: &'a Value, key: &str) -> &'a str {
    fm.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{label}: missing string frontmatter key {key:?}"))
}

fn id_is_valid(label: &str, id: &str) {
    assert!(!id.is_empty(), "{label}: id is empty");
    assert!(id.len() <= 1024, "{label}: id exceeds 1024 chars");

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

fn well_formed(label: &str, body: &str) {
    assert!(!body.trim().is_empty(), "{label}: skill is empty");
    assert!(
        body.len() <= 256 * 1024,
        "{label}: skill exceeds 256 KiB ({} bytes)",
        body.len()
    );

    let (_fm, markdown) = split_frontmatter(label, body);
    let h1 = markdown
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    assert!(
        h1.starts_with("# "),
        "{label}: skill must start with an H1, got: {h1:?}"
    );
}

fn section_position(label: &str, body: &str, heading: &str) -> usize {
    body.find(heading)
        .unwrap_or_else(|| panic!("{label}: missing required section {heading:?}"))
}

fn strip_json_comments(json: &str) -> String {
    json.lines()
        .map(|line| match line.find("//") {
            Some(idx) => &line[..idx],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn json_blocks(body: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = body;
    while let Some((_, after_open)) = rest.split_once("```json\n") {
        let Some((block, after_close)) = after_open.split_once("\n```") else {
            break;
        };
        blocks.push(block.to_string());
        rest = after_close;
    }
    blocks
}

#[test]
fn index_skill_has_index_frontmatter_and_links_to_every_how_to() {
    well_formed("index", auth_credentials::SKILL_MD);
    id_is_valid("index", auth_credentials::SKILL_ID);

    let (fm, markdown) = split_frontmatter("index", auth_credentials::SKILL_MD);
    assert_eq!(frontmatter_str("index", &fm, "type"), "index");
    assert_eq!(
        frontmatter_str("index", &fm, "title"),
        auth_credentials::SKILL_ID
    );
    assert!(markdown.contains("## How-tos"));

    for (id, _) in auth_credentials::SUB_SKILLS {
        let uri = format!("iii://{id}");
        assert!(markdown.contains(&uri), "index missing URI {uri}");
    }
}

#[test]
fn how_to_skills_use_required_frontmatter_and_path_mapping() {
    let prefix = format!("{}/", auth_credentials::SKILL_ID);
    for (id, body) in auth_credentials::SUB_SKILLS {
        well_formed(id, body);
        id_is_valid(id, id);
        assert!(
            id.starts_with(&prefix),
            "sub-skill id {id:?} must be nested under the worker id ({}/)",
            auth_credentials::SKILL_ID
        );

        let (fm, _markdown) = split_frontmatter(id, body);
        assert_eq!(frontmatter_str(id, &fm, "type"), "how-to");
        let function_id = frontmatter_str(id, &fm, "function_id");
        let expected_path = function_id.replace("::", "/");
        let actual_path = id.strip_prefix(&prefix).unwrap_or(id);
        assert_eq!(
            actual_path, expected_path,
            "{id}: skill path must mirror function namespace"
        );
        assert!(
            !frontmatter_str(id, &fm, "title").is_empty(),
            "{id}: title must be non-empty"
        );
    }
}

#[test]
fn how_to_skills_have_required_sections_in_order() {
    for (id, body) in auth_credentials::SUB_SKILLS {
        let (_fm, markdown) = split_frontmatter(id, body);
        let when = section_position(id, &markdown, "# When to use");
        let inputs = section_position(id, &markdown, "# Inputs");
        let outputs = section_position(id, &markdown, "# Outputs");
        let worked = section_position(id, &markdown, "# Worked example");
        let related = section_position(id, &markdown, "# Related");
        assert!(
            when < inputs && inputs < outputs && outputs < worked && worked < related,
            "{id}: required sections are out of order"
        );
    }
}

#[test]
fn json_examples_are_parseable_after_field_comments_are_removed() {
    for (id, body) in auth_credentials::SUB_SKILLS {
        let (_fm, markdown) = split_frontmatter(id, body);
        let blocks = json_blocks(&markdown);
        assert!(!blocks.is_empty(), "{id}: expected at least one JSON block");
        for block in blocks {
            let stripped = strip_json_comments(&block);
            serde_json::from_str::<serde_json::Value>(&stripped)
                .unwrap_or_else(|err| panic!("{id}: invalid JSON example {stripped:?}: {err}"));
        }
    }
}

#[test]
fn write_path_skills_document_side_effects() {
    for (id, body) in auth_credentials::SUB_SKILLS {
        let needs_side_effects = id.ends_with("/set_token") || id.ends_with("/delete_token");
        assert_eq!(
            body.contains("# Side effects"),
            needs_side_effects,
            "{id}: side effects section mismatch"
        );
    }
}

#[test]
fn related_bullets_use_function_id_contract() {
    for (id, body) in auth_credentials::SUB_SKILLS {
        let (_fm, markdown) = split_frontmatter(id, body);
        let related = markdown
            .split("# Related")
            .nth(1)
            .unwrap_or_else(|| panic!("{id}: missing related section"));
        for line in related.lines().filter(|line| line.starts_with("- ")) {
            assert!(
                line.contains(" — "),
                "{id}: related bullet must use en-dash separator: {line:?}"
            );
            assert!(
                line.trim_end().ends_with('.'),
                "{id}: related bullet must end with a period: {line:?}"
            );
            assert!(
                line.starts_with("- `auth::"),
                "{id}: related bullet must start with a function id in backticks: {line:?}"
            );
        }
    }
}
