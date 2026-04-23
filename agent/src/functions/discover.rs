use std::future::Future;
use std::pin::Pin;

use iii_sdk::{IIIError, III};
use serde_json::{json, Value};

use crate::discovery;

pub fn build_handler(
    iii: III,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let iii = iii.clone();

        Box::pin(async move {
            let tools = discovery::discover_tools(&iii).await;

            let functions: Vec<Value> = tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema
                    })
                })
                .collect();

            Ok(json!({
                "functions": functions,
                "count": functions.len()
            }))
        })
    }
}
