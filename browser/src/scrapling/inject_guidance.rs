//! harness::hook::pre-generate guidance: teach agents the scrapling surface —
//! which fetch tier to reach for, what needs no browser at all, and which
//! capabilities this worker does NOT have. Bound with on_error: fail_open
//! (pre_generate defaults fail-CLOSED and a missing guidance line must never
//! abort a turn).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const GUIDANCE_HOOK_ID: &str = "browser::inject-guidance";
pub const GUIDANCE_HOOK_DESC: &str =
    "Internal: appends browser::* scraping and HTML parsing guidance to the agent system prompt.";

pub const GUIDANCE: &str = "\
## Showing a page vs scraping one (browser::*)
To OPEN or SHOW a page — anything the user should watch, a local app to \
demo, a form or flow to drive — start an interactive session with \
`browser::sessions::start` and the url. It renders in a real Chromium and \
appears live in the console's browser page. To reach a goal on it (fill \
and submit a form, log in, walk a flow), call `browser::run` with the end \
state as `goal` and the values to type as `inputs`; it drives the page in \
one call and returns the page it left, which you read before reporting \
success. For single steps, `browser::elements` lists the page's controls \
with refs for `browser::act`; `browser::snapshot`, `browser::navigate` and \
`browser::screenshot` read, move and capture. When the task is done, close \
the tab with `browser::sessions::stop`. The scraping surface below fetches \
content and renders nothing the user can see, so it cannot \"open\" \
something for them.
## Scraping and HTML parsing (browser::*)
Start with `browser::fetch` for static web pages, RSS/Atom feeds, and APIs; \
its `urls` input fetches concurrently. Do not refetch successful entries, and \
cite returned URLs. Escalate to `browser::dynamic-fetch` only for \
JavaScript-rendered pages, or `browser::stealthy-fetch` only for \
anti-bot/Cloudflare pages after cheaper tiers fail. Use \
`browser::screenshot-url` to capture a URL; `browser::screenshot` captures an \
existing interactive session. Use `browser::session-fetch` only for an \
already-open session whose cookies/state must be reused; \
`browser::session-open`, `browser::session-close`, and `browser::session-list` \
manage those scraping sessions, never interactive tabs. `browser::crawl` \
walks same-domain links and streams results. For HTML already in hand, use \
`browser::extract`, `browser::css`, `browser::xpath`, `browser::regex`, \
`browser::find`, `browser::find-by-text`, `browser::find-by-regex`, \
`browser::find-similar`, `browser::describe`, or `browser::to-markdown`. \
Fetching functions require approval; parse functions do not. Adaptive \
queries persist identities in SQLite. `solve_cloudflare` is supported by \
stealthy fetches; use `browser::handoff` for human-only steps in an \
interactive session.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GenerateContext {
    #[serde(default)]
    pub system_prompt: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PreGenerateEvent {
    #[serde(default)]
    pub generate: GenerateContext,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct PreGenerateMutations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreGenerateResponse {
    pub mutations: PreGenerateMutations,
}

/// Empty base → {} (preserve harness prompt); else full replacement base+guidance.
pub fn mutations_for(base: &str) -> PreGenerateResponse {
    if base.is_empty() {
        return PreGenerateResponse {
            mutations: PreGenerateMutations::default(),
        };
    }
    PreGenerateResponse {
        mutations: PreGenerateMutations {
            system_prompt: Some(format!("{base}\n\n{GUIDANCE}")),
        },
    }
}

pub async fn handle(event: PreGenerateEvent) -> Result<PreGenerateResponse, iii_sdk::Error> {
    Ok(mutations_for(&event.generate.system_prompt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_base_preserves_harness_prompt() {
        let v = serde_json::to_value(mutations_for("")).unwrap();
        assert_eq!(v, serde_json::json!({"mutations": {}}));
    }

    /// Agents search for what the guidance names: it must route goals to
    /// `browser::run`, single steps to `browser::elements` + `browser::act`,
    /// and closing a tab to `browser::sessions::stop` (not the scraping
    /// `session-close`), all of them registered functions.
    #[test]
    fn guidance_names_the_interactive_workflow_and_how_to_close_a_tab() {
        use crate::functions::{ACT_ID, ELEMENTS_ID, RUN_ID, SESSIONS_START_ID, SESSIONS_STOP_ID};
        let registered: Vec<&str> = crate::functions::catalog()
            .iter()
            .map(|spec| spec.function_id)
            .collect();
        for id in [
            SESSIONS_START_ID,
            RUN_ID,
            ELEMENTS_ID,
            ACT_ID,
            SESSIONS_STOP_ID,
        ] {
            assert!(
                GUIDANCE.contains(&format!("`{id}`")),
                "guidance never names {id}"
            );
            assert!(registered.contains(&id), "{id} is not registered");
        }
        assert!(GUIDANCE.contains("scraping sessions, never interactive tabs"));
    }

    #[test]
    fn nonempty_base_appends_guidance() {
        let v = serde_json::to_value(mutations_for("BASE")).unwrap();
        let s = v["mutations"]["system_prompt"].as_str().unwrap();
        assert!(s.starts_with("BASE\n\n## Showing a page vs scraping one"));
    }

    /// The guidance is what routes agents to these functions, so it must not
    /// promise capabilities the worker does not have, and every Scrapling id
    /// it names must actually be registered.
    #[test]
    fn guidance_names_only_real_functions_and_no_absent_capability() {
        let registered: Vec<&str> = crate::scrapling::STATIC_IDS
            .iter()
            .map(|id| id.trim_start_matches("browser::"))
            .collect();
        for word in [
            "fetch",
            "dynamic-fetch",
            "stealthy-fetch",
            "screenshot-url",
            "session-open",
            "session-fetch",
            "session-close",
            "session-list",
            "crawl",
            "extract",
            "css",
            "xpath",
            "regex",
            "find",
            "find-by-text",
            "find-by-regex",
            "find-similar",
            "describe",
            "to-markdown",
        ] {
            assert!(
                GUIDANCE.contains(word),
                "guidance never mentions `{word}`, so agents will not find it"
            );
            assert!(registered.contains(&word), "`{word}` is not registered");
        }
        assert!(
            GUIDANCE.contains("Adaptive queries persist identities in SQLite"),
            "adaptive persistence must be disclosed"
        );
        assert!(GUIDANCE.contains("`solve_cloudflare` is supported"));
        assert!(GUIDANCE.contains("browser::handoff"));
    }
}
