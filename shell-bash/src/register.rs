use iii_sdk::{RegisterFunctionMessage, Value, III};

use crate::{detect_clis, exec, which};

pub async fn register_with_iii(iii: &III) -> anyhow::Result<()> {
    let iii_for_exec = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(exec::ID.into())
            .with_description(exec::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_exec.clone();
            async move { exec::execute(&iii, &payload).await }
        },
    ));
    let iii_for_which = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(which::ID.into())
            .with_description(which::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_which.clone();
            async move { which::execute(&iii, &payload).await }
        },
    ));
    let iii_for_detect = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(detect_clis::ID.into())
            .with_description(detect_clis::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_detect.clone();
            async move { detect_clis::execute(&iii, &payload).await }
        },
    ));
    Ok(())
}
