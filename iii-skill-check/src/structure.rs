use anyhow::Context;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Violation {
    pub file: String,
    pub line: Option<usize>,
    pub message: String,
}

/// Run Layer 1 deterministic structure checks against a worker dir.
///
/// Reads `<dir>/iii.worker.yaml`, `<dir>/README.md`, `<dir>/skill.md`, and any
/// `<dir>/skills/*.md` files. Returns one Violation per finding; empty Vec means
/// the artifacts are clean at this layer.
pub fn check(dir: &Path) -> anyhow::Result<Vec<Violation>> {
    let manifest = crate::introspect::read_manifest(dir)?;
    let name = &manifest.name;

    let readme_path = dir.join("README.md");
    let skill_path = dir.join("skill.md");
    let skills_dir = dir.join("skills");

    let readme = std::fs::read_to_string(&readme_path)
        .with_context(|| format!("reading {}", readme_path.display()))?;
    let skill = std::fs::read_to_string(&skill_path)
        .with_context(|| format!("reading {}", skill_path.display()))?;
    let leaves = list_existing_leaves(&skills_dir)?;

    let mut violations = Vec::new();
    violations.extend(check_required_sections(&readme));
    violations.extend(check_install_line(&readme, name));
    violations.extend(check_no_source_build(&readme));

    let known_leaves: HashSet<String> = leaves.iter().map(|(n, _)| n.clone()).collect();

    let mut artifacts: Vec<(String, &str)> =
        vec![("README.md".to_string(), readme.as_str()), ("skill.md".to_string(), skill.as_str())];
    for (leaf, body) in &leaves {
        artifacts.push((format!("skills/{leaf}.md"), body.as_str()));
    }

    for (label, content) in &artifacts {
        violations.extend(check_llm_only_balance(label, content));
        violations.extend(check_iii_links(label, content, name, &known_leaves));
    }

    Ok(violations)
}

fn check_required_sections(readme: &str) -> Vec<Violation> {
    let required = ["## Install", "## Quickstart", "## Configuration"];
    let mut violations = Vec::new();
    let mut last_pos: Option<usize> = None;
    for header in &required {
        match readme.find(header) {
            Some(pos) => {
                if let Some(prev) = last_pos {
                    if pos < prev {
                        violations.push(Violation {
                            file: "README.md".to_string(),
                            line: None,
                            message: format!("section {header} appears out of expected order"),
                        });
                    }
                }
                last_pos = Some(pos);
            }
            None => {
                violations.push(Violation {
                    file: "README.md".to_string(),
                    line: None,
                    message: format!("missing required section {header}"),
                });
            }
        }
    }
    violations
}

fn check_install_line(readme: &str, name: &str) -> Vec<Violation> {
    let expected = format!("iii worker add {name}");
    if readme.contains(&expected) {
        return Vec::new();
    }
    vec![Violation {
        file: "README.md".to_string(),
        line: None,
        message: format!(
            "install command should be `{expected}` matching iii.worker.yaml.name"
        ),
    }]
}

fn check_no_source_build(readme: &str) -> Vec<Violation> {
    let blocked = ["cargo build", "cargo install"];
    let mut violations = Vec::new();
    for (i, line) in readme.lines().enumerate() {
        let lower = line.to_lowercase();
        for needle in &blocked {
            if lower.contains(needle) {
                violations.push(Violation {
                    file: "README.md".to_string(),
                    line: Some(i + 1),
                    message: format!(
                        "source-build instruction `{needle}` is forbidden in published README"
                    ),
                });
            }
        }
    }
    violations
}

fn check_llm_only_balance(label: &str, content: &str) -> Vec<Violation> {
    let starts = content.matches("<!-- llm-only:start -->").count();
    let ends = content.matches("<!-- llm-only:end -->").count();
    if starts == ends {
        return Vec::new();
    }
    vec![Violation {
        file: label.to_string(),
        line: None,
        message: format!(
            "unbalanced llm-only blocks: {starts} start markers, {ends} end markers"
        ),
    }]
}

fn check_iii_links(
    label: &str,
    content: &str,
    worker_name: &str,
    known_leaves: &HashSet<String>,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let prefix = format!("iii://{worker_name}/");
    for (i, line) in content.lines().enumerate() {
        let mut start = 0;
        while let Some(rel) = line[start..].find(&prefix) {
            let abs = start + rel;
            let after = &line[abs + prefix.len()..];
            let leaf: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '/')
                .collect();
            if !leaf.is_empty() && !known_leaves.contains(&leaf) {
                violations.push(Violation {
                    file: label.to_string(),
                    line: Some(i + 1),
                    message: format!(
                        "iii://{worker_name}/{leaf} points to a leaf that is not registered (no skills/{leaf}.md)"
                    ),
                });
            }
            start = abs + prefix.len();
        }
    }
    violations
}

fn list_existing_leaves(skills_dir: &Path) -> anyhow::Result<Vec<(String, String)>> {
    let mut leaves: Vec<(String, String)> = Vec::new();
    if !skills_dir.exists() {
        return Ok(leaves);
    }
    for entry in std::fs::read_dir(skills_dir)
        .with_context(|| format!("reading {}", skills_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("invalid skill filename: {}", path.display()))?
            .to_string();
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        leaves.push((name, body));
    }
    leaves.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(leaves)
}
