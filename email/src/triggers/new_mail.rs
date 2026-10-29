//! `email::new-mail`, the trigger type, and `email::on-new-mail`, the
//! observable function next to it (iii 0.25).
//!
//! Both file their consumers in the same [`TriggerRegistry`] under
//! (account, folder): a binding by trigger instance id, an observer by
//! observer id, with the binding config as the observer's args
//! (args_format [`NewMailBindingConfig`]). The dispatcher calls bindings with
//! `iii.trigger` and observers with `obs.call`, the same payload for both.
//!
//! Events are routed in-process by the dispatcher, never by invoking the
//! function, and its handler refuses every invocation with FORBIDDEN: a
//! caller could otherwise make this worker deliver forged mail events. On an
//! engine without observe the function registers as a plain function, its
//! hooks never run, and the trigger type carries everything.

use async_trait::async_trait;
use iii_sdk::{
    errors::Error,
    runtime::FunctionRef,
    trigger::{TriggerConfig, TriggerHandler},
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use super::registry::{NewMailObserver, TriggerRegistry};
use super::NewMailEvent;

/// The trigger type, kept for old consumers on 0.24 and 0.25 engines.
pub const NEW_MAIL_TRIGGER_TYPE: &str = "email::new-mail";
/// The observable function next to [`NEW_MAIL_TRIGGER_TYPE`].
pub const ON_NEW_MAIL_FN: &str = "email::on-new-mail";

pub const NEW_MAIL_DESCRIPTION: &str =
    "Fires when IMAP IDLE pushes a new message to a configured (account, folder). \
     Payload: { account, folder, uid, message_id, from, subject, snippet, ts }. \
     If the IMAP server lacks the IDLE capability, the account fails at startup (E610).";

pub const ON_NEW_MAIL_DESCRIPTION: &str =
    "Fires when IMAP IDLE pushes a new message to a configured (account, folder). Observe it \
     with args { account, folder?, handler_timeout_ms? } (the email::new-mail config; folder \
     defaults to INBOX, handler_timeout_ms to 30000); each observer of the (account, folder) \
     receives { account, folder, uid, message_id, from, subject, snippet, ts }. Only the email \
     worker delivers it: invoking it directly is refused.";

/// Codes an observer is refused with, and the direct-invocation refusal.
pub mod codes {
    /// Any direct invocation of `email::on-new-mail`.
    pub const FORBIDDEN: &str = "FORBIDDEN";
    /// Args the binding config refuses (no `account`, a wrong type).
    pub const CONFIG_ERROR: &str = "CONFIG_ERROR";
}

/// The `email::new-mail` binding config, and the args of an
/// `email::on-new-mail` observer.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NewMailBindingConfig {
    account: String,
    #[serde(default = "default_folder")]
    folder: String,
    #[serde(default = "default_timeout")]
    handler_timeout_ms: u64,
}
fn default_folder() -> String {
    "INBOX".into()
}
fn default_timeout() -> u64 {
    30_000
}

pub struct Handler {
    pub registry: Arc<TriggerRegistry>,
}

#[async_trait]
impl TriggerHandler for Handler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let cfg: NewMailBindingConfig =
            serde_json::from_value(config.config.clone()).map_err(|e| {
                Error::Handler(
                    json!({
                        "code":"CONFIG_ERROR",
                        "message":format!("email::new-mail config: {e}")
                    })
                    .to_string(),
                )
            })?;
        self.registry.register(
            cfg.account.clone(),
            cfg.folder.clone(),
            config.id.clone(),
            config.function_id.clone(),
            config.namespace.clone(),
            cfg.handler_timeout_ms,
        );
        tracing::info!(
            instance = %config.id,
            function = %config.function_id,
            account = %cfg.account,
            folder = %cfg.folder,
            "email::new-mail trigger registered"
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.registry.unregister(&config.id);
        tracing::info!(instance = %config.id, "email::new-mail trigger unregistered");
        Ok(())
    }
}

/// Registers `email::on-new-mail`, owner-delivered, with args_format
/// [`NewMailBindingConfig`] and [`ObserveHandler`] hooks that file its
/// observers in `registry`, the registry the trigger type uses.
pub fn register_on_new_mail(iii: &IIIClient, registry: Arc<TriggerRegistry>) -> FunctionRef {
    iii.register_function(
        ON_NEW_MAIL_FN,
        RegisterFunction::new(refuse_invocation)
            .description(ON_NEW_MAIL_DESCRIPTION)
            .observe(
                Observe::owner()
                    .args_format::<NewMailBindingConfig>()
                    .handler(ObserveNewMail { registry }),
            ),
    )
}

/// The function's handler: events only come from the IDLE dispatcher. It
/// takes the [`NewMailEvent`] observers receive, so the function publishes
/// that event as its request schema.
pub fn refuse_invocation(_event: NewMailEvent) -> Result<(), Error> {
    Err(Error::Remote {
        code: codes::FORBIDDEN.to_string(),
        message: format!(
            "{ON_NEW_MAIL_FN} is an event source: observe it \
             (iii.observe({{ function_id: '{ON_NEW_MAIL_FN}', args: {{ account, folder }} }}, target)); \
             only the email worker delivers its events"
        ),
        stacktrace: None,
    })
}

/// Files each observer of `email::on-new-mail` under the (account, folder)
/// of its args, and takes it out again when the engine removes it.
pub struct ObserveNewMail {
    pub registry: Arc<TriggerRegistry>,
}

impl ObserveNewMail {
    /// Files `observer` under the (account, folder) of `args`; args the
    /// binding config refuses reject the observer with CONFIG_ERROR.
    pub fn admit(
        &self,
        args: &Value,
        observer: Arc<dyn NewMailObserver>,
    ) -> Result<(), ObserveRejection> {
        let cfg: NewMailBindingConfig = serde_json::from_value(args.clone()).map_err(|e| {
            ObserveRejection::new(codes::CONFIG_ERROR, format!("{ON_NEW_MAIL_FN} args: {e}"))
        })?;
        let id = observer.id().to_string();
        self.registry.add_observer(
            cfg.account.clone(),
            cfg.folder.clone(),
            observer,
            cfg.handler_timeout_ms,
        );
        tracing::info!(
            observer = %id,
            account = %cfg.account,
            folder = %cfg.folder,
            "email::on-new-mail observer added"
        );
        Ok(())
    }
}

#[async_trait]
impl ObserveHandler for ObserveNewMail {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        // Filed before the ack: the first event after the engine marks the
        // observer active reaches it.
        self.admit(obs.args_value(), Arc::new(obs.clone()))
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.registry.remove_observer(obs.id());
        tracing::info!(observer = %obs.id(), "email::on-new-mail observer removed");
    }
}

#[async_trait]
impl NewMailObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    async fn deliver(&self, payload: Value, timeout: Duration) -> Result<(), Error> {
        self.call_with_timeout::<Value>(payload, timeout)
            .await
            .map(|_| ())
    }
}
