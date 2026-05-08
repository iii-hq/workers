use std::path::PathBuf;

fn workers_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn loads_workers_skill_check_yaml() {
    let path = workers_dir().join(".skill-check.yaml");
    let config = iii_skill_check::config::load(&path).expect("config should load");
    assert_eq!(config.ai_check.provider, "anthropic");
    assert_eq!(config.ai_check.model, "claude-opus-4-7");
    assert_eq!(config.ai_check.api_key_env_var, "ANTHROPIC_API_KEY");
    assert_eq!(config.ai_check.max_tokens, 4000);
    assert_eq!(config.rules.path, std::path::PathBuf::from("./project-rules"));
    assert_eq!(config.styles.path, std::path::PathBuf::from("./styles"));
}

#[test]
fn errors_when_yaml_is_missing_required_fields() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "rules:\n  path: ./x\n").unwrap();
    let result = iii_skill_check::config::load(tmp.path());
    assert!(result.is_err(), "expected an error when ai_check is missing");
}
