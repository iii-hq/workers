//! The name rule, the functions a server's listing wants, the diff between two
//! sets of definitions, and the registry that applies it to the engine.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::host::{FunctionDef, Handler, Host, Registration};

pub fn tool_response_format() -> Value {
    json!({"type": "object", "properties": {"content": {"type": "array"}}, "required": ["content"]})
}

pub fn function_id(server: &str, name: &str) -> String {
    format!("mcp::{server}::{name}")
}

/// The rule for tool and prompt names. `encoded_prefix` is `mcp__<server>__`
/// or `mcp__<server>__prompts__`; a name that fails is skipped, never renamed.
pub fn check_name(encoded_prefix: &str, name: &str) -> Result<(), String> {
    if name.is_empty() {
        Err("name is empty".into())
    } else if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Err("name has characters outside `[A-Za-z0-9_-]`".into())
    } else if name.contains("__") {
        Err("name contains `__`".into())
    } else if encoded_prefix.len() + name.len() > 64 {
        Err("encoded name is longer than 64 characters".into())
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkipKind {
    Tool,
    Prompt,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Skipped {
    pub kind: SkipKind,
    pub name: String,
    pub reason: String,
}

/// One tool of a `tools/list` reply.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Value,
    pub annotations: Option<Value>,
}

/// What a registered function calls on the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Tool { name: String },
    Instructions,
    ResourceList,
    ResourceRead,
    ResourceTemplates,
    Prompt { name: String },
}

#[derive(Clone)]
pub struct Wanted {
    pub def: FunctionDef,
    pub target: Target,
}

pub type WantedSet = BTreeMap<String, Wanted>;

/// The functions a tool listing wants, in id order, and the tools it skips.
/// The first of two tools with the same name wins.
pub fn tool_wanted(
    server: &str,
    tools: &[ToolInfo],
    has_instructions: bool,
) -> (WantedSet, Vec<Skipped>) {
    let prefix = format!("mcp__{server}__");
    let mut wanted = WantedSet::new();
    let mut skipped = Vec::new();
    for tool in tools {
        let id = function_id(server, &tool.name);
        let fails = check_name(&prefix, &tool.name)
            .err()
            .or_else(|| wanted.contains_key(&id).then(|| "duplicate name".into()));
        if let Some(reason) = fails {
            skipped.push(Skipped {
                kind: SkipKind::Tool,
                name: tool.name.clone(),
                reason,
            });
            continue;
        }
        let mut request_format = tool.input_schema.clone();
        if let Some(schema) = request_format.as_object_mut() {
            schema.remove("$schema");
        }
        let mut mcp = json!({"server": server, "tool": tool.name});
        if let Some(title) = &tool.title {
            mcp["title"] = json!(title);
        }
        if let Some(annotations) = &tool.annotations {
            mcp["annotations"] = annotations.clone();
        }
        let def = FunctionDef {
            id: id.clone(),
            description: tool.description.clone().unwrap_or_default(),
            request_format,
            response_format: tool_response_format(),
            metadata: json!({ "mcp": mcp }),
        };
        let target = Target::Tool {
            name: tool.name.clone(),
        };
        wanted.insert(id, Wanted { def, target });
    }
    let id = function_id(server, "instructions");
    if has_instructions && !wanted.contains_key(&id) {
        let def = FunctionDef {
            id: id.clone(),
            description: format!(
                "Usage instructions from the `{server}` MCP server. Read them before calling its tools."
            ),
            request_format: json!({"type": "object", "properties": {}, "additionalProperties": false}),
            response_format: tool_response_format(),
            metadata: json!({"mcp": {"server": server, "instructions": true}}),
        };
        let target = Target::Instructions;
        wanted.insert(id, Wanted { def, target });
    }
    (wanted, skipped)
}

/// What a reconcile does, each list in id order.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub remove: Vec<String>,
    pub change: Vec<String>,
    pub add: Vec<String>,
    /// The changed ids whose server took back `readOnlyHint: true`.
    pub lost_read_only: Vec<String>,
}

pub fn plan(old: &BTreeMap<String, FunctionDef>, new: &BTreeMap<String, FunctionDef>) -> Plan {
    let mut plan = Plan::default();
    for (id, old_def) in old {
        match new.get(id) {
            None => plan.remove.push(id.clone()),
            Some(new_def) if new_def != old_def => {
                plan.change.push(id.clone());
                if lost_read_only(old_def, new_def) {
                    plan.lost_read_only.push(id.clone());
                }
            }
            Some(_) => {}
        }
    }
    plan.add = new
        .keys()
        .filter(|id| !old.contains_key(*id))
        .cloned()
        .collect();
    plan
}

pub fn lost_read_only(old: &FunctionDef, new: &FunctionDef) -> bool {
    let read_only =
        |d: &FunctionDef| d.metadata.pointer("/mcp/annotations/readOnlyHint") == Some(&json!(true));
    read_only(old) && !read_only(new)
}

type Entry = (FunctionDef, Target, Box<dyn Registration>);

/// The functions one server has registered.
#[derive(Default)]
pub struct Registry {
    live: Mutex<BTreeMap<String, Entry>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes the registered set match `wanted`. An id absent from `wanted`
    /// for which `untouched` holds stays as it is. Registering a live id
    /// panics in the SDK, so a change unregisters before it registers.
    pub async fn apply(
        &self,
        host: &dyn Host,
        wanted: WantedSet,
        untouched: &(dyn Fn(&str) -> bool + Sync),
        handler_for: &(dyn Fn(&Wanted) -> Handler + Sync),
    ) -> Plan {
        let mut live = self.live.lock().await;
        let old = live
            .iter()
            .filter(|(id, _)| wanted.contains_key(*id) || !untouched(id))
            .map(|(id, (def, _, _))| (id.clone(), def.clone()))
            .collect();
        let new = wanted
            .iter()
            .map(|(id, w)| (id.clone(), w.def.clone()))
            .collect();
        let plan = plan(&old, &new);
        for id in &plan.remove {
            if let Some((_, _, registration)) = live.remove(id) {
                registration.unregister();
            }
        }
        for id in plan.change.iter().chain(&plan.add) {
            if let Some((_, _, registration)) = live.remove(id) {
                registration.unregister();
            }
            let w = &wanted[id];
            let registration = host.register(&w.def, handler_for(w));
            live.insert(id.clone(), (w.def.clone(), w.target.clone(), registration));
        }
        plan
    }

    /// Unregisters every function of this registry.
    pub async fn clear(&self) {
        for (_, (_, _, registration)) in std::mem::take(&mut *self.live.lock().await) {
            registration.unregister();
        }
    }

    pub async fn defs(&self) -> BTreeMap<String, FunctionDef> {
        let live = self.live.lock().await;
        live.iter()
            .map(|(id, (def, _, _))| (id.clone(), def.clone()))
            .collect()
    }

    pub async fn target(&self, id: &str) -> Option<Target> {
        self.live.lock().await.get(id).map(|(_, t, _)| t.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tools(v: Value) -> Vec<ToolInfo> {
        serde_json::from_value(v).unwrap()
    }

    fn def(id: &str, description: &str) -> FunctionDef {
        FunctionDef {
            id: id.into(),
            description: description.into(),
            request_format: json!({"type": "object"}),
            response_format: tool_response_format(),
            metadata: json!({"mcp": {"server": "s", "tool": id}}),
        }
    }

    fn set<const N: usize>(defs: [FunctionDef; N]) -> BTreeMap<String, FunctionDef> {
        defs.into_iter().map(|d| (d.id.clone(), d)).collect()
    }

    const CHARS: &str = "name has characters outside `[A-Za-z0-9_-]`";
    const UNDERSCORES: &str = "name contains `__`";
    const LONG: &str = "encoded name is longer than 64 characters";

    #[test]
    fn check_name_applies_the_rules_in_order() {
        let p = "mcp__linear__";
        assert_eq!(
            function_id("linear", "save_issue"),
            "mcp::linear::save_issue"
        );
        assert_eq!(check_name(p, "get-issue_2"), Ok(()));
        assert_eq!(check_name(p, ""), Err("name is empty".into()));
        for bad in ["a.b", "a/b", "a b", "ação"] {
            assert_eq!(check_name(p, bad), Err(CHARS.into()), "{bad}");
        }
        assert_eq!(check_name(p, "a__b"), Err(UNDERSCORES.into()));
        // A name that breaks several rules gets the first reason.
        assert_eq!(check_name(p, "a.__b"), Err(CHARS.into()));
        assert_eq!(check_name(p, &"a.".repeat(40)), Err(CHARS.into()));
        assert_eq!(check_name(p, &"a__".repeat(20)), Err(UNDERSCORES.into()));

        // The longest server name (24 characters) leaves 33 characters for a
        // tool and 24 for a prompt.
        let server = "s".repeat(24);
        let tool_prefix = format!("mcp__{server}__");
        let prompt_prefix = format!("mcp__{server}__prompts__");
        assert_eq!(check_name(&tool_prefix, &"t".repeat(33)), Ok(()));
        assert_eq!(check_name(&tool_prefix, &"t".repeat(34)), Err(LONG.into()));
        assert_eq!(check_name(&prompt_prefix, &"p".repeat(24)), Ok(()));
        assert_eq!(
            check_name(&prompt_prefix, &"p".repeat(25)),
            Err(LONG.into())
        );
    }

    #[test]
    fn tool_wanted_builds_defs_from_wire_tools() {
        assert_eq!(
            tool_response_format(),
            json!({"type":"object","properties":{"content":{"type":"array"}},"required":["content"]})
        );
        let list = tools(json!([
            {
                "name": "save_issue",
                "title": "Save issue",
                "description": "Create or update an issue.",
                "inputSchema": {
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "type": "object",
                    "properties": {"title": {"type": "string"}, "_meta": {"type": "object"}},
                    "additionalProperties": false
                },
                "annotations": {"readOnlyHint": false, "title": "Save"},
                "outputSchema": {"type": "object", "properties": {"id": {"type": "string"}}},
                "_meta": {"vendor": "x"}
            },
            {"name": "whoami", "inputSchema": {"type": "object"}},
            {"name": "bad.name", "inputSchema": {"type": "object"}}
        ]));
        let (wanted, skipped) = tool_wanted("linear", &list, true);
        assert_eq!(
            wanted.keys().collect::<Vec<_>>(),
            [
                "mcp::linear::instructions",
                "mcp::linear::save_issue",
                "mcp::linear::whoami"
            ]
        );

        let save = &wanted["mcp::linear::save_issue"];
        assert_eq!(save.def.id, "mcp::linear::save_issue");
        assert_eq!(save.def.description, "Create or update an issue.");
        assert_eq!(
            save.def.request_format,
            json!({
                "type": "object",
                "properties": {"title": {"type": "string"}, "_meta": {"type": "object"}},
                "additionalProperties": false
            })
        );
        assert_eq!(save.def.response_format, tool_response_format());
        assert_eq!(
            save.def.metadata,
            json!({"mcp": {"server": "linear", "tool": "save_issue", "title": "Save issue", "annotations": {"readOnlyHint": false, "title": "Save"}}})
        );
        assert_eq!(
            save.target,
            Target::Tool {
                name: "save_issue".into()
            }
        );

        let whoami = &wanted["mcp::linear::whoami"];
        assert_eq!(whoami.def.description, "");
        assert_eq!(whoami.def.request_format, json!({"type": "object"}));
        assert_eq!(
            whoami.def.metadata,
            json!({"mcp": {"server": "linear", "tool": "whoami"}})
        );
        assert_eq!(
            whoami.target,
            Target::Tool {
                name: "whoami".into()
            }
        );

        let instructions = &wanted["mcp::linear::instructions"];
        assert_eq!(instructions.target, Target::Instructions);
        assert_eq!(
            instructions.def,
            FunctionDef {
                id: "mcp::linear::instructions".into(),
                description: "Usage instructions from the `linear` MCP server. Read them before calling its tools.".into(),
                request_format: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                response_format: tool_response_format(),
                metadata: json!({"mcp": {"server": "linear", "instructions": true}}),
            }
        );

        assert_eq!(
            skipped,
            vec![Skipped {
                kind: SkipKind::Tool,
                name: "bad.name".into(),
                reason: CHARS.into()
            }]
        );
        assert_eq!(serde_json::to_value(&skipped[0]).unwrap()["kind"], "tool");
    }

    #[test]
    fn a_tool_named_instructions_wins_over_the_instructions_function() {
        let list = tools(json!([{"name": "instructions", "inputSchema": {"type": "object"}}]));
        let (wanted, skipped) = tool_wanted("linear", &list, true);
        assert_eq!(wanted.len(), 1);
        assert_eq!(
            wanted["mcp::linear::instructions"].target,
            Target::Tool {
                name: "instructions".into()
            }
        );
        assert!(skipped.is_empty());

        let (wanted, _) = tool_wanted("linear", &[], false);
        assert!(wanted.is_empty());
    }

    #[test]
    fn plan_diffs_by_value() {
        let old = set([def("a", "1"), def("b", "1"), def("c", "1")]);
        let new = set([def("b", "1"), def("c", "2"), def("d", "1")]);
        assert_eq!(
            plan(&old, &new),
            Plan {
                remove: vec!["a".into()],
                change: vec!["c".into()],
                add: vec!["d".into()],
                lost_read_only: vec![],
            }
        );
        assert_eq!(plan(&old, &old), Plan::default());

        let mut annotated = def("a", "1");
        annotated.metadata["mcp"]["annotations"] = json!({"destructiveHint": true});
        assert_eq!(plan(&set([def("a", "1")]), &set([annotated])).change, ["a"]);

        let prompt = |request_format: Value| FunctionDef {
            id: "mcp::s::prompts::p".into(),
            description: "Prompt".into(),
            request_format,
            response_format: json!({"type": "object"}),
            metadata: json!({"mcp": {"server": "s", "prompt": "p"}}),
        };
        let none = prompt(json!({"type": "object", "properties": {}}));
        let one = prompt(json!({"type": "object", "properties": {"q": {"type": "string"}}}));
        assert_eq!(
            plan(&set([none]), &set([one])).change,
            ["mcp::s::prompts::p"]
        );
    }

    #[test]
    fn lost_read_only_sees_the_hint_go() {
        let hinted = |hint: bool| {
            let mut d = def("a", "1");
            d.metadata["mcp"]["annotations"] = json!({"readOnlyHint": hint});
            d
        };
        let (read, write, plain) = (hinted(true), hinted(false), def("a", "1"));
        assert!(lost_read_only(&read, &write));
        assert!(lost_read_only(&read, &plain));
        assert!(!lost_read_only(&read, &read));
        assert!(!lost_read_only(&plain, &read));
        assert!(!lost_read_only(&write, &plain));

        assert_eq!(
            plan(&set([read.clone()]), &set([write])).lost_read_only,
            ["a"]
        );
        assert!(
            plan(&set([read.clone()]), &set([read.clone()]))
                .lost_read_only
                .is_empty()
        );
        // A removed id is never in it.
        assert!(
            plan(&set([read]), &BTreeMap::new())
                .lost_read_only
                .is_empty()
        );
    }
}
