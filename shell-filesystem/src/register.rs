use iii_sdk::{RegisterFunctionMessage, Value, III};

use crate::ops;

pub async fn register_with_iii(iii: &III) -> anyhow::Result<()> {
    let iii_for_ls = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(ops::ls::ID.into())
            .with_description(ops::ls::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_ls.clone();
            async move { ops::ls::execute(&iii, &payload).await }
        },
    ));
    let iii_for_stat = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(ops::stat::ID.into())
            .with_description(ops::stat::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_stat.clone();
            async move { ops::stat::execute(&iii, &payload).await }
        },
    ));
    let iii_for_mkdir = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(ops::mkdir::ID.into())
            .with_description(ops::mkdir::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_mkdir.clone();
            async move { ops::mkdir::execute(&iii, &payload).await }
        },
    ));
    let iii_for_read = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(ops::read::ID.into())
            .with_description(ops::read::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_read.clone();
            async move { ops::read::execute(&iii, &payload).await }
        },
    ));
    let iii_for_write = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(ops::write::ID.into())
            .with_description(ops::write::DESCRIPTION.into()),
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
