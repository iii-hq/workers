use iii_sdk::{errors::Error, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct AccountsListReq {}

#[derive(Debug, Serialize, JsonSchema)]
struct AccountsListResp {
    accounts: Vec<AccountInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct AccountInfo {
    name: String,
    provider: String,
    from: String,
    can_send: bool,
    can_read: bool,
    folders: Vec<String>,
}

pub fn register(iii: &Arc<IIIClient>, cfg: &Arc<crate::config::WorkerConfig>) {
    let cfg = cfg.clone();
    iii.register_function(
        "email::accounts::list",
        RegisterFunction::new_async(move |_: AccountsListReq| {
            let cfg = cfg.clone();
            async move {
                let accounts: Vec<_> = cfg
                    .accounts
                    .iter()
                    .map(|(name, account)| AccountInfo {
                        name: name.clone(),
                        provider: match account.provider {
                            crate::config::Provider::Smtp => "smtp".to_string(),
                            crate::config::Provider::Imap => "imap".to_string(),
                        },
                        from: account.from.clone(),
                        can_send: account.smtp.is_some(),
                        can_read: account.imap.is_some(),
                        folders: account
                            .imap
                            .as_ref()
                            .map(|imap| imap.folders.clone())
                            .unwrap_or_default(),
                    })
                    .collect();
                Ok::<_, Error>(AccountsListResp { accounts })
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
