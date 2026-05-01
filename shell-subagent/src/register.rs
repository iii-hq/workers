use iii_sdk::{RegisterFunctionMessage, Value, III};

use crate::start;

pub async fn register_with_iii(iii: &III) -> anyhow::Result<()> {
    let iii_for_start = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(start::ID.into())
            .with_description(start::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_start.clone();
            async move { start::execute(&iii, &payload).await }
        },
    ));
    Ok(())
}
