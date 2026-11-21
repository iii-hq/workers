//! Prompt functions: what a `prompts/list` entry registers and how arguments
//! are sent.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::host::FunctionDef;
use crate::reconcile::{SkipKind, Skipped, Target, Wanted, WantedSet, check_name};

#[derive(Deserialize)]
pub struct PromptArg {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub required: Option<bool>,
}

#[derive(Deserialize)]
pub struct PromptInfo {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub arguments: Option<Vec<PromptArg>>,
}

pub fn request_schema(args: &[PromptArg]) -> Value {
    let mut props = Map::new();
    for a in args {
        let mut p = json!({"type": "string"});
        for (k, v) in [("title", &a.title), ("description", &a.description)] {
            if let Some(v) = v {
                p[k] = json!(v);
            }
        }
        props.insert(a.name.clone(), p);
    }
    let mut schema = json!({"type": "object", "properties": props, "additionalProperties": false});
    let required: Vec<&str> = args
        .iter()
        .filter(|a| a.required == Some(true))
        .map(|a| a.name.as_str())
        .collect();
    if !required.is_empty() {
        schema["required"] = json!(required);
    }
    schema
}

pub fn description(server: &str, prompt: &PromptInfo) -> String {
    prompt
        .description
        .clone()
        .or_else(|| prompt.title.clone())
        .unwrap_or_else(|| format!("Prompt `{}` from the `{server}` MCP server.", prompt.name))
}

/// `prompts/get` takes string arguments only: numbers and booleans go as
/// their JSON text, `null` is left out.
pub fn coerce(args: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for (k, v) in args {
        match v {
            Value::String(_) => out.insert(k.clone(), v.clone()),
            Value::Number(_) | Value::Bool(_) => out.insert(k.clone(), json!(v.to_string())),
            Value::Null => None,
            _ => return Err(format!("invalid_argument: {k} must be a string")),
        };
    }
    Ok(out)
}

pub fn response_schema() -> Value {
    json!({"type": "object", "properties": {
        "description": {"type": "string"},
        "messages": {"type": "array", "items": {"type": "object",
            "properties": {"role": {"type": "string", "enum": ["user", "assistant"]}, "content": {"type": "array"}},
            "required": ["role", "content"]}}},
        "required": ["messages"]})
}

/// The functions a prompt listing wants and the prompts it skips. The first
/// of two prompts with the same name wins.
pub fn prompt_wanted(server: &str, prompts: &[PromptInfo]) -> (WantedSet, Vec<Skipped>) {
    let prefix = format!("mcp__{server}__prompts__");
    let mut wanted = WantedSet::new();
    let mut skipped = Vec::new();
    for p in prompts {
        let id = format!("mcp::{server}::prompts::{}", p.name);
        let fails = check_name(&prefix, &p.name)
            .err()
            .or_else(|| wanted.contains_key(&id).then(|| "duplicate name".into()));
        if let Some(reason) = fails {
            skipped.push(Skipped {
                kind: SkipKind::Prompt,
                name: p.name.clone(),
                reason,
            });
            continue;
        }
        let args = p.arguments.as_deref().unwrap_or(&[]);
        let mut mcp = json!({"server": server, "prompt": p.name,
            "arguments": args.iter().map(|a| a.name.as_str()).collect::<Vec<_>>()});
        if let Some(title) = &p.title {
            mcp["title"] = json!(title);
        }
        let def = FunctionDef {
            id: id.clone(),
            description: description(server, p),
            request_format: request_schema(args),
            response_format: response_schema(),
            metadata: json!({ "mcp": mcp }),
        };
        let target = Target::Prompt {
            name: p.name.clone(),
        };
        wanted.insert(id, Wanted { def, target });
    }
    (wanted, skipped)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    use super::*;
    use crate::host::FunctionDef;
    use crate::reconcile::{SkipKind, Target, WantedSet};

    fn infos(v: Value) -> Vec<PromptInfo> {
        serde_json::from_value(v).unwrap()
    }
    fn defs(w: &WantedSet) -> BTreeMap<String, FunctionDef> {
        w.iter().map(|(k, v)| (k.clone(), v.def.clone())).collect()
    }

    #[test]
    fn request_schema_from_arguments() {
        assert_eq!(
            request_schema(&[]),
            json!({"type": "object", "properties": {}, "additionalProperties": false})
        );
        let args: Vec<PromptArg> = serde_json::from_value(json!([
            {"name": "language", "description": "Language", "required": true},
            {"name": "focus", "title": "Focus"},
            {"name": "depth", "required": true}]))
        .unwrap();
        assert_eq!(
            request_schema(&args),
            json!({"type": "object", "properties": {
                "language": {"type": "string", "description": "Language"},
                "focus": {"type": "string", "title": "Focus"},
                "depth": {"type": "string"}},
                "required": ["language", "depth"], "additionalProperties": false})
        );
    }

    #[test]
    fn description_falls_back_to_title_then_a_fixed_sentence() {
        let p = infos(json!([
            {"name": "a", "description": "D", "title": "T"},
            {"name": "b", "title": "T"},
            {"name": "c"}]));
        assert_eq!(description("fake", &p[0]), "D");
        assert_eq!(description("fake", &p[1]), "T");
        assert_eq!(
            description("fake", &p[2]),
            "Prompt `c` from the `fake` MCP server."
        );
    }

    #[test]
    fn coerce_sends_strings() {
        let m = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(
            coerce(&m(
                json!({"s": "x", "n": 3, "f": 1.5, "b": true, "z": null})
            ))
            .unwrap(),
            m(json!({"s": "x", "n": "3", "f": "1.5", "b": "true"}))
        );
        assert_eq!(
            coerce(&m(json!({"o": {}}))),
            Err("invalid_argument: o must be a string".to_string())
        );
        assert_eq!(
            coerce(&m(json!({"a": [1]}))),
            Err("invalid_argument: a must be a string".to_string())
        );
    }

    #[test]
    fn prompt_wanted_ids_metadata_and_skips() {
        let (w, skipped) = prompt_wanted(
            "fake",
            &infos(json!([
                {"name": "code_review", "title": "Review", "description": "Review code",
                 "arguments": [{"name": "language", "required": true}, {"name": "focus"}]},
                {"name": "bad.name"}, {"name": "a__b"}, {"name": "code_review"}])),
        );
        assert_eq!(
            w.keys().collect::<Vec<_>>(),
            ["mcp::fake::prompts::code_review"]
        );
        let e = &w["mcp::fake::prompts::code_review"];
        assert!(matches!(&e.target, Target::Prompt { name } if name == "code_review"));
        assert_eq!(e.def.description, "Review code");
        assert_eq!(e.def.response_format, response_schema());
        assert_eq!(
            e.def.metadata,
            json!({"mcp": {"server": "fake", "prompt": "code_review", "title": "Review",
                "arguments": ["language", "focus"]}})
        );
        assert_eq!(
            skipped.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["bad.name", "a__b", "code_review"]
        );
        assert!(
            skipped
                .iter()
                .all(|s| s.kind == SkipKind::Prompt && !s.reason.is_empty())
        );
        assert_eq!(skipped[2].reason, "duplicate name");
    }

    #[test]
    fn the_encoded_prompt_name_is_at_most_64_characters() {
        let server = "a".repeat(24);
        let (w, s) = prompt_wanted(&server, &infos(json!([{"name": "p".repeat(24)}])));
        assert_eq!((w.len(), s.len()), (1, 0));
        let (w, s) = prompt_wanted(&server, &infos(json!([{"name": "p".repeat(25)}])));
        assert_eq!((w.len(), s.len()), (0, 1));
    }

    #[test]
    fn changed_arguments_are_a_changed_function() {
        let wanted = |args: Value| {
            defs(&prompt_wanted("fake", &infos(json!([{"name": "p", "arguments": args}]))).0)
        };
        let p = crate::reconcile::plan(
            &wanted(json!([{"name": "a"}])),
            &wanted(json!([{"name": "a"}, {"name": "b"}])),
        );
        assert_eq!(
            (p.remove.len(), p.change, p.add.len()),
            (0, vec!["mcp::fake::prompts::p".to_string()], 0)
        );
    }
}
