use std::path::PathBuf;

fn workers_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn build_user_prompt_includes_rules_and_artifact() {
    let prompt = iii_skill_check::ai::build_user_prompt(
        "Voice: be terse.\n",
        "README.md",
        "# Hello\nworld\n",
    );
    assert!(prompt.contains("Voice: be terse."), "rules absent: {prompt}");
    assert!(prompt.contains("README.md"), "path absent: {prompt}");
    assert!(prompt.contains("# Hello"), "artifact absent: {prompt}");
}

#[test]
fn build_user_prompt_line_numbers_the_artifact() {
    let prompt = iii_skill_check::ai::build_user_prompt(
        "rules",
        "x.md",
        "alpha\nbeta\ngamma\n",
    );
    // Each artifact line should be prefixed with its 1-based line number.
    assert!(prompt.contains("1") && prompt.contains("alpha"));
    assert!(prompt.contains("2") && prompt.contains("beta"));
    assert!(prompt.contains("3") && prompt.contains("gamma"));
}

#[test]
fn parse_response_returns_ok_on_pass() {
    assert!(iii_skill_check::ai::parse_response("PASS").is_ok());
    assert!(iii_skill_check::ai::parse_response("  PASS\n").is_ok());
}

#[test]
fn parse_response_returns_err_on_fail() {
    let r = iii_skill_check::ai::parse_response("FAIL\nREADME.md:5 — voice drift — rephrase");
    assert!(r.is_err());
    let body = r.unwrap_err();
    assert!(body.contains("voice drift"));
    assert!(body.contains("README.md:5"));
}

#[test]
fn parse_response_returns_err_on_anything_other_than_pass() {
    assert!(iii_skill_check::ai::parse_response("").is_err());
    assert!(iii_skill_check::ai::parse_response("Pass.").is_err());
    assert!(iii_skill_check::ai::parse_response("Looks good!").is_err());
}

/// Live API smoke test: only runs when ANTHROPIC_API_KEY is set. Asserts that
/// the textstats README passes the AI layer against the canonical project rules.
/// Skipped in CI unless the key is wired in.
#[test]
fn ai_check_passes_textstats_readme_when_key_present() {
    if std::env::var("ANTHROPIC_API_KEY").is_err() {
        eprintln!("skipping ai_check_passes_textstats_readme: ANTHROPIC_API_KEY not set");
        return;
    }

    let workers_root = workers_dir();
    let textstats_readme = workers_root.join("textstats").join("README.md");
    let prompt_path = workers_root
        .join("project-rules")
        .join("_skill-check-prompt.md");
    let prompt = std::fs::read_to_string(&prompt_path).unwrap();

    let rules_dir = workers_root.join("project-rules");
    let mut rules = String::new();
    let mut entries: Vec<_> = std::fs::read_dir(&rules_dir).unwrap().filter_map(|r| r.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.ends_with(".md") || name.starts_with('_') || name == "README.md" || name == "SOURCE.md" {
            continue;
        }
        let body = std::fs::read_to_string(&path).unwrap();
        rules.push_str(&format!("# {name}\n\n{body}\n\n"));
    }

    let result = iii_skill_check::ai::check_artifact(
        &textstats_readme,
        &rules,
        &prompt,
        "claude-opus-4-7",
        "ANTHROPIC_API_KEY",
        4000,
    )
    .expect("API call should not error");

    assert!(
        result.is_ok(),
        "expected PASS for textstats README, got: {result:?}"
    );
}
