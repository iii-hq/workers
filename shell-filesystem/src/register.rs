use iii_sdk::{RegisterFunctionMessage, Value, III};
use serde_json::json;

use crate::ops;

/// Build a `RegisterFunctionMessage` with description, JSON-Schema request
/// format, and tool-label metadata in one call. Workers that pre-date the
/// `RegisterFunction::new_async` builder still register through the raw
/// `RegisterFunctionMessage` path, so this helper keeps the call sites
/// readable.
fn tool_message(id: &str, description: &str, label: &str, schema: Value) -> RegisterFunctionMessage {
    let mut msg =
        RegisterFunctionMessage::with_id(id.into()).with_description(description.into());
    msg.request_format = Some(schema);
    msg.metadata = Some(json!({"tool": {"label": label}}));
    msg
}

pub async fn register_with_iii(iii: &III) -> anyhow::Result<()> {
    register_with_config(iii, ops::read::ReadConfig::default()).await
}

pub async fn register_with_config(
    iii: &III,
    read_config: ops::read::ReadConfig,
) -> anyhow::Result<()> {
    let iii_for_ls = iii.clone();
    iii.register_function((
        tool_message(
            ops::ls::ID,
            ops::ls::DESCRIPTION,
            "List directory",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to a directory inside the sandbox."
                    }
                },
                "required": ["path"]
            }),
        ),
        move |payload: Value| {
            let iii = iii_for_ls.clone();
            async move { ops::ls::execute(&iii, &payload).await }
        },
    ));
    let iii_for_stat = iii.clone();
    iii.register_function((
        tool_message(
            ops::stat::ID,
            ops::stat::DESCRIPTION,
            "File stat",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        ),
        move |payload: Value| {
            let iii = iii_for_stat.clone();
            async move { ops::stat::execute(&iii, &payload).await }
        },
    ));
    let iii_for_mkdir = iii.clone();
    iii.register_function((
        tool_message(
            ops::mkdir::ID,
            ops::mkdir::DESCRIPTION,
            "Make directory",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        ),
        move |payload: Value| {
            let iii = iii_for_mkdir.clone();
            async move { ops::mkdir::execute(&iii, &payload).await }
        },
    ));
    let iii_for_read = iii.clone();
    iii.register_function((
        tool_message(
            ops::read::ID,
            ops::read::DESCRIPTION,
            "Read file",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path to a file inside the sandbox."
                    }
                },
                "required": ["path"]
            }),
        ),
        move |payload: Value| {
            let iii = iii_for_read.clone();
            async move { ops::read::execute_with_config(&iii, &payload, read_config).await }
        },
    ));
    let iii_for_write = iii.clone();
    iii.register_function((
        tool_message(
            ops::write::ID,
            ops::write::DESCRIPTION,
            "Write file",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path inside the sandbox."
                    },
                    "content": {
                        "type": "string",
                        "description": "UTF-8 file contents."
                    }
                },
                "required": ["path", "content"]
            }),
        ),
        move |payload: Value| {
            let iii = iii_for_write.clone();
            async move { ops::write::execute(&iii, &payload).await }
        },
    ));
    let iii_for_rm = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(ops::rm::ID.into())
            .with_description(ops::rm::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_rm.clone();
            async move { ops::rm::execute(&iii, &payload).await }
        },
    ));
    let iii_for_chmod = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(ops::chmod::ID.into())
            .with_description(ops::chmod::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_chmod.clone();
            async move { ops::chmod::execute(&iii, &payload).await }
        },
    ));
    let iii_for_mv = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(ops::mv::ID.into())
            .with_description(ops::mv::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_mv.clone();
            async move { ops::mv::execute(&iii, &payload).await }
        },
    ));
    let iii_for_grep = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(ops::grep::ID.into())
            .with_description(ops::grep::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_grep.clone();
            async move { ops::grep::execute(&iii, &payload).await }
        },
    ));
    let iii_for_sed = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(ops::sed::ID.into())
            .with_description(ops::sed::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_sed.clone();
            async move { ops::sed::execute(&iii, &payload).await }
        },
    ));
    let iii_for_edit = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(ops::edit::ID.into())
            .with_description(ops::edit::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_edit.clone();
            async move { ops::edit::execute(&iii, &payload).await }
        },
    ));
    Ok(())
}
