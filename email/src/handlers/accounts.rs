use iii_sdk::{IIIError, RegisterFunction, III};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct AccountsListReq {}

pub fn register(iii: &Arc<III>, cfg: &Arc<crate::config::WorkerConfig>) {
    let cfg = cfg.clone();
    iii.register_function(
        "email::accounts::list",
        RegisterFunction::new_async(move |_: AccountsListReq| {
            let cfg = cfg.clone();
            async move {
                let accounts: Vec<_> = cfg
                    .accounts
                    .iter()
                    .map(|(k, v)| {
                        json!({
                            "name": k,
                            "provider": match v.provider {
                                crate::config::Provider::Smtp => "smtp",
                                crate::config::Provider::Imap => "imap",
                            },
                            "from": v.from,
                            "can_send": v.smtp.is_some(),
                            "can_read": v.imap.is_some(),
                            "folders": v.imap.as_ref().map(|i| i.folders.clone()).unwrap_or_default(),
                        })
                    })
                    .collect();
                Ok::<_, IIIError>(json!({ "accounts": accounts }))
            }
        })
        .description(
            "List configured email accounts. Returns { accounts: [{ name, provider, \
             from, can_send, can_read, folders }] }. Use `name` as the `account` field \
             for email::send, email::list, email::get, email::search, email::flag, \
             email::move, and email::attachment::get.",
        ),
    );
}
