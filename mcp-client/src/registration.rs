use anyhow::Result;
use iii_sdk::{IIIError, RegisterFunctionMessage, III};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use crate::session::Session;

pub fn slugify(uri: &str) -> String {
    uri.chars()
        .map(|c| match c {
            ':' | '/' | '.' | '?' | '#' | '&' | '=' | ' ' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

pub async fn register_all(iii: &III, session: Arc<Session>, namespace: &str) -> Result<()> {
    let caps = session.capabilities.read().await.clone().unwrap_or_default();

    if caps.tools.is_some() {
        register_tools(iii, session.clone(), namespace).await?;
    }
    if caps.resources.is_some() {
        register_resources(iii, session.clone(), namespace).await?;
    }
    if caps.prompts.is_some() {
        register_prompts(iii, session.clone(), namespace).await?;
    }

    Ok(())
}

async fn register_tools(iii: &III, session: Arc<Session>, namespace: &str) -> Result<()> {
    let tools = session.list_tools().await?;
    let mut registered = session.registered.lock().await;
    for tool in tools {
        let id = format!("{namespace}.{}::{}", session.name, tool.name);
        if registered.contains_key(&id) {
            continue;
        }

        let metadata = json!({
            "mcp.remote.server": session.name,
            "mcp.remote.tool": tool.name,
            "mcp.remote.transport": session.transport_kind(),
        });

        let sess = session.clone();
        let tool_name = tool.name.clone();
        let handler = move |input: Value| {
            let sess = sess.clone();
            let tool_name = tool_name.clone();
            async move {
                sess.tools_call(&tool_name, input)
                    .await
                    .map_err(|e| IIIError::Runtime(e.to_string()))
            }
        };

        let fn_ref = iii.register_function_with(
            RegisterFunctionMessage {
                id: id.clone(),
                description: tool.description.clone(),
                request_format: Some(tool.input_schema.clone()),
                response_format: tool.output_schema.clone(),
                metadata: Some(metadata),
                invocation: None,
            },
            handler,
        );

        registered.insert(id, fn_ref);
    }
    Ok(())
}

async fn register_resources(iii: &III, session: Arc<Session>, namespace: &str) -> Result<()> {
    let resources = session.list_resources().await?;
    let mut registered = session.registered.lock().await;
    for res in resources {
        let id = format!(
            "{namespace}.{}.resources::{}",
            session.name,
            slugify(&res.uri)
        );
        if registered.contains_key(&id) {
            continue;
        }

        let metadata = json!({
            "mcp.remote.server": session.name,
            "mcp.remote.resource": res.uri,
            "mcp.remote.transport": session.transport_kind(),
        });

        let sess = session.clone();
        let uri = res.uri.clone();
        let handler = move |_input: Value| {
            let sess = sess.clone();
            let uri = uri.clone();
            async move {
                sess.resources_read(&uri)
                    .await
                    .map_err(|e| IIIError::Runtime(e.to_string()))
            }
        };

        let fn_ref = iii.register_function_with(
            RegisterFunctionMessage {
                id: id.clone(),
                description: res.description.clone().or(res.title.clone()),
                request_format: Some(json!({ "type": "object", "properties": {} })),
                response_format: None,
                metadata: Some(metadata),
                invocation: None,
            },
            handler,
        );

        registered.insert(id, fn_ref);
    }
    Ok(())
}

async fn register_prompts(iii: &III, session: Arc<Session>, namespace: &str) -> Result<()> {
    let prompts = session.list_prompts().await?;
    let mut registered = session.registered.lock().await;
    for prompt in prompts {
        let id = format!("{namespace}.{}.prompts::{}", session.name, prompt.name);
        if registered.contains_key(&id) {
            continue;
        }

        let metadata = json!({
            "mcp.remote.server": session.name,
            "mcp.remote.prompt": prompt.name,
            "mcp.remote.transport": session.transport_kind(),
        });

        let sess = session.clone();
        let prompt_name = prompt.name.clone();
        let handler = move |input: Value| {
            let sess = sess.clone();
            let prompt_name = prompt_name.clone();
            async move {
                sess.prompts_get(&prompt_name, input)
                    .await
                    .map_err(|e| IIIError::Runtime(e.to_string()))
            }
        };

        let fn_ref = iii.register_function_with(
            RegisterFunctionMessage {
                id: id.clone(),
                description: prompt.description.clone(),
                request_format: prompt.arguments.clone(),
                response_format: None,
                metadata: Some(metadata),
                invocation: None,
            },
            handler,
        );

        registered.insert(id, fn_ref);
    }
    Ok(())
}

pub async fn reconcile(iii: &III, session: Arc<Session>, namespace: &str) -> Result<()> {
    let caps = session.capabilities.read().await.clone().unwrap_or_default();

    let mut desired: HashSet<String> = HashSet::new();

    if caps.tools.is_some() {
        for tool in session.list_tools().await? {
            desired.insert(format!("{namespace}.{}::{}", session.name, tool.name));
        }
    }
    if caps.resources.is_some() {
        for res in session.list_resources().await? {
            desired.insert(format!(
                "{namespace}.{}.resources::{}",
                session.name,
                slugify(&res.uri)
            ));
        }
    }
    if caps.prompts.is_some() {
        for prompt in session.list_prompts().await? {
            desired.insert(format!(
                "{namespace}.{}.prompts::{}",
                session.name,
                prompt.name
            ));
        }
    }

    let to_remove: Vec<String> = {
        let registered = session.registered.lock().await;
        registered
            .keys()
            .filter(|k| !desired.contains(*k))
            .cloned()
            .collect()
    };

    if !to_remove.is_empty() {
        let mut registered = session.registered.lock().await;
        for id in &to_remove {
            if let Some(fn_ref) = registered.remove(id) {
                fn_ref.unregister();
            }
        }
        drop(registered);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    register_all(iii, session, namespace).await?;
    Ok(())
}
