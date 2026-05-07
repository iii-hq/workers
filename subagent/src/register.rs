use std::sync::Arc;

use iii_sdk::{RegisterFunctionMessage, Value, III};

use crate::{config::SubagentConfig, start};

pub fn register_with_iii(iii: &Arc<III>, config: &Arc<SubagentConfig>) {
    let iii_for_start = iii.clone();
    let cfg = config.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(start::ID.into())
            .with_description(start::DESCRIPTION.into()),
        move |payload: Value| {
            let iii = iii_for_start.clone();
            let cfg = cfg.clone();
            async move { start::execute(&iii, &payload, &cfg).await }
        },
    ));
}
