mod steps {
    use cucumber::{given, then, when};
    use serde_json::Value;
    use serde_yaml::Value as YamlValue;

    use crate::AuthWorld;

    #[given("an empty auth credential store")]
    fn empty_store(world: &mut AuthWorld) {
        *world = AuthWorld::new();
    }

    #[given(regex = r#"^environment variable "([^"]+)" is "([^"]*)"$"#)]
    fn env_var(world: &mut AuthWorld, name: String, value: String) {
        world.env.insert(name, value);
    }

    #[when(regex = r"^I call (auth::[a-z_]+) with payload:$")]
    async fn call_auth(world: &mut AuthWorld, function_id: String, step: &cucumber::gherkin::Step) {
        let payload = parse_payload(step);
        world.last_ok = None;
        world.last_err = None;
        match dispatch(world, &function_id, payload).await {
            Ok(value) => world.last_ok = Some(value),
            Err(err) => world.last_err = Some(err),
        }
    }

    #[then(regex = r#"^the auth credential response has api key "([^"]+)"$"#)]
    fn credential_has_key(world: &mut AuthWorld, expected: String) {
        let value = last_ok(world);
        assert_eq!(value["type"].as_str(), Some("api_key"));
        assert_eq!(value["key"].as_str(), Some(expected.as_str()));
    }

    #[then(regex = r#"^the auth OAuth response has access token "([^"]+)"$"#)]
    fn oauth_has_access_token(world: &mut AuthWorld, expected: String) {
        let value = last_ok(world);
        assert_eq!(value["type"].as_str(), Some("oauth"));
        assert_eq!(value["access_token"].as_str(), Some(expected.as_str()));
    }

    #[then(regex = r#"^the auth OAuth response has refresh token "([^"]+)"$"#)]
    fn oauth_has_refresh_token(world: &mut AuthWorld, expected: String) {
        let value = last_ok(world);
        assert_eq!(value["type"].as_str(), Some("oauth"));
        assert_eq!(value["refresh_token"].as_str(), Some(expected.as_str()));
    }

    #[then(regex = r#"^the auth OAuth response has scopes "([^"]*)"$"#)]
    fn oauth_has_scopes(world: &mut AuthWorld, csv: String) {
        let expected: Vec<&str> = csv.split(',').filter(|s| !s.is_empty()).collect();
        let got: Vec<&str> = last_ok(world)["scopes"]
            .as_array()
            .expect("scopes is an array")
            .iter()
            .map(|v| v.as_str().expect("scope is a string"))
            .collect();
        assert_eq!(got, expected);
    }

    #[then(regex = r#"^the auth status source is "([^"]+)"$"#)]
    fn status_source(world: &mut AuthWorld, expected: String) {
        let value = last_ok(world);
        assert_eq!(value["configured"].as_bool(), Some(true));
        assert_eq!(value["source"].as_str(), Some(expected.as_str()));
    }

    #[then("the auth status is unconfigured")]
    fn status_unconfigured(world: &mut AuthWorld) {
        let value = last_ok(world);
        assert_eq!(value["configured"].as_bool(), Some(false));
        assert!(value.get("source").is_none());
        assert!(value.get("label").is_none());
    }

    #[then(regex = r#"^the auth status label is "([^"]+)"$"#)]
    fn status_label(world: &mut AuthWorld, expected: String) {
        assert_eq!(last_ok(world)["label"].as_str(), Some(expected.as_str()));
    }

    #[then(regex = r#"^the auth status label starts with "([^"]+)"$"#)]
    fn status_label_starts_with(world: &mut AuthWorld, expected: String) {
        let label = last_ok(world)["label"]
            .as_str()
            .expect("status label is a string");
        assert!(
            label.starts_with(&expected),
            "expected label {label:?} to start with {expected:?}"
        );
    }

    #[then(regex = r#"^the auth provider list is "([^"]*)"$"#)]
    fn provider_list(world: &mut AuthWorld, csv: String) {
        let expected: Vec<&str> = csv.split(',').filter(|s| !s.is_empty()).collect();
        let got: Vec<&str> = last_ok(world)["providers"]
            .as_array()
            .expect("providers is an array")
            .iter()
            .map(|v| v.as_str().expect("provider is a string"))
            .collect();
        assert_eq!(got, expected);
    }

    #[then(regex = r#"^the auth response does not contain "([^"]+)"$"#)]
    fn response_does_not_contain(world: &mut AuthWorld, needle: String) {
        let rendered = serde_json::to_string(last_ok(world)).expect("render last response");
        assert!(
            !rendered.contains(&needle),
            "response leaked {needle:?}: {rendered}"
        );
    }

    #[then("the auth response is null")]
    fn response_is_null(world: &mut AuthWorld) {
        assert!(
            last_ok(world).is_null(),
            "expected null, got {:?}",
            last_ok(world)
        );
    }

    #[then(regex = r#"^the auth call fails with a message mentioning "([^"]+)"$"#)]
    fn call_fails(world: &mut AuthWorld, needle: String) {
        let err = world.last_err.as_deref().unwrap_or("");
        assert!(
            err.contains(&needle),
            "expected error to mention {needle:?}; got {err:?}; success: {:?}",
            world.last_ok
        );
    }

    #[then(regex = r#"^the auth skill index has type "([^"]+)" and title "([^"]+)"$"#)]
    fn skill_index_frontmatter(
        _world: &mut AuthWorld,
        expected_type: String,
        expected_title: String,
    ) {
        let (frontmatter, markdown) = split_frontmatter("index", auth_credentials::SKILL_MD);
        assert_eq!(
            frontmatter_str("index", &frontmatter, "type"),
            expected_type
        );
        assert_eq!(
            frontmatter_str("index", &frontmatter, "title"),
            expected_title
        );
        assert!(
            markdown.contains("## How-tos"),
            "index skill should have a How-tos section"
        );
    }

    #[then("the auth skill index links to every auth how-to")]
    fn skill_index_links_every_how_to(_world: &mut AuthWorld) {
        let (_frontmatter, markdown) = split_frontmatter("index", auth_credentials::SKILL_MD);
        for (id, _) in auth_credentials::SUB_SKILLS {
            let uri = format!("iii://{id}");
            assert!(markdown.contains(&uri), "index missing URI {uri}");
        }
    }

    #[then("every auth how-to path mirrors its function id")]
    fn every_how_to_path_mirrors_function_id(_world: &mut AuthWorld) {
        let prefix = format!("{}/", auth_credentials::SKILL_ID);
        for (id, body) in auth_credentials::SUB_SKILLS {
            let (frontmatter, _markdown) = split_frontmatter(id, body);
            assert_eq!(frontmatter_str(id, &frontmatter, "type"), "how-to");
            let function_id = frontmatter_str(id, &frontmatter, "function_id");
            let actual_path = id.strip_prefix(&prefix).unwrap_or(id);
            assert_eq!(
                actual_path,
                function_id.replace("::", "/"),
                "{id}: path must mirror function namespace"
            );
        }
    }

    #[then("every auth how-to has required sections in order")]
    fn every_how_to_has_ordered_sections(_world: &mut AuthWorld) {
        for (id, body) in auth_credentials::SUB_SKILLS {
            let (_frontmatter, markdown) = split_frontmatter(id, body);
            let when = section_position(id, &markdown, "# When to use");
            let inputs = section_position(id, &markdown, "# Inputs");
            let outputs = section_position(id, &markdown, "# Outputs");
            let worked = section_position(id, &markdown, "# Worked example");
            let related = section_position(id, &markdown, "# Related");
            assert!(
                when < inputs && inputs < outputs && outputs < worked && worked < related,
                "{id}: sections are out of order"
            );
        }
    }

    #[then("every auth how-to JSON example parses")]
    fn every_how_to_json_example_parses(_world: &mut AuthWorld) {
        for (id, body) in auth_credentials::SUB_SKILLS {
            let (_frontmatter, markdown) = split_frontmatter(id, body);
            let blocks = json_blocks(&markdown);
            assert!(!blocks.is_empty(), "{id}: expected at least one JSON block");
            for block in blocks {
                let stripped = strip_json_comments(&block);
                serde_json::from_str::<Value>(&stripped)
                    .unwrap_or_else(|err| panic!("{id}: invalid JSON example: {err}"));
            }
        }
    }

    #[then("auth write how-tos document side effects")]
    fn auth_write_how_tos_document_side_effects(_world: &mut AuthWorld) {
        for (id, body) in auth_credentials::SUB_SKILLS {
            let needs_side_effects = id.ends_with("/set_token") || id.ends_with("/delete_token");
            assert_eq!(
                body.contains("# Side effects"),
                needs_side_effects,
                "{id}: side effects section mismatch"
            );
        }
    }

    fn parse_payload(step: &cucumber::gherkin::Step) -> Value {
        let raw = step.docstring.as_deref().unwrap_or("{}");
        serde_json::from_str(raw)
            .unwrap_or_else(|err| panic!("payload docstring is not valid JSON: {raw:?}: {err}"))
    }

    fn last_ok(world: &AuthWorld) -> &Value {
        world
            .last_ok
            .as_ref()
            .unwrap_or_else(|| panic!("expected success, got error {:?}", world.last_err))
    }

    async fn dispatch(
        world: &mut AuthWorld,
        function_id: &str,
        payload: Value,
    ) -> Result<Value, String> {
        match function_id {
            "auth::get_token" => {
                let input = serde_json::from_value(payload).map_err(|err| err.to_string())?;
                let env = world.env.clone();
                let output = auth_credentials::handle_get_token(&world.store, input, |var| {
                    env.get(var).cloned()
                })
                .await
                .map_err(|err| err.to_string())?;
                serde_json::to_value(output).map_err(|err| err.to_string())
            }
            "auth::set_token" => {
                let input = serde_json::from_value(payload).map_err(|err| err.to_string())?;
                let output = auth_credentials::handle_set_token(&world.store, input)
                    .await
                    .map_err(|err| err.to_string())?;
                serde_json::to_value(output).map_err(|err| err.to_string())
            }
            "auth::delete_token" => {
                let input = serde_json::from_value(payload).map_err(|err| err.to_string())?;
                let output = auth_credentials::handle_delete_token(&world.store, input)
                    .await
                    .map_err(|err| err.to_string())?;
                serde_json::to_value(output).map_err(|err| err.to_string())
            }
            "auth::list_providers" => {
                let input = serde_json::from_value(payload).map_err(|err| err.to_string())?;
                let output = auth_credentials::handle_list_providers(&world.store, input)
                    .await
                    .map_err(|err| err.to_string())?;
                serde_json::to_value(output).map_err(|err| err.to_string())
            }
            "auth::status" => {
                let input = serde_json::from_value(payload).map_err(|err| err.to_string())?;
                let env = world.env.clone();
                let output = auth_credentials::handle_status(&world.store, input, |var| {
                    env.get(var).cloned()
                })
                .await
                .map_err(|err| err.to_string())?;
                serde_json::to_value(output).map_err(|err| err.to_string())
            }
            other => Err(format!("unknown function {other}")),
        }
    }

    fn split_frontmatter(label: &str, body: &str) -> (YamlValue, String) {
        let rest = body
            .strip_prefix("---\n")
            .unwrap_or_else(|| panic!("{label}: missing frontmatter"));
        let (yaml, markdown) = rest
            .split_once("\n---\n")
            .unwrap_or_else(|| panic!("{label}: missing closing frontmatter fence"));
        let frontmatter = serde_yaml::from_str(yaml)
            .unwrap_or_else(|err| panic!("{label}: invalid frontmatter: {err}"));
        (frontmatter, markdown.to_string())
    }

    fn frontmatter_str<'a>(label: &str, frontmatter: &'a YamlValue, key: &str) -> &'a str {
        frontmatter
            .get(key)
            .and_then(YamlValue::as_str)
            .unwrap_or_else(|| panic!("{label}: missing frontmatter string key {key:?}"))
    }

    fn section_position(label: &str, body: &str, heading: &str) -> usize {
        body.find(heading)
            .unwrap_or_else(|| panic!("{label}: missing required section {heading:?}"))
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

    fn strip_json_comments(json: &str) -> String {
        json.lines()
            .map(|line| match line.find("//") {
                Some(idx) => &line[..idx],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

use std::collections::HashMap;

use cucumber::World;
use serde_json::Value;

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct AuthWorld {
    store: auth_credentials::InMemoryStore,
    env: HashMap<String, String>,
    last_ok: Option<Value>,
    last_err: Option<String>,
}

impl AuthWorld {
    fn new() -> Self {
        Self {
            store: auth_credentials::InMemoryStore::new(),
            env: HashMap::new(),
            last_ok: None,
            last_err: None,
        }
    }
}

#[tokio::main]
async fn main() {
    AuthWorld::cucumber()
        .max_concurrent_scenarios(1)
        .run_and_exit("tests/features")
        .await;
}
