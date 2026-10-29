//! Turns a normalized `Event` into a call of `email::on-new-mail`. iii
//! delivers the call to every observer whose filters match it; the call does
//! not wait for their targets.

use async_trait::async_trait;
use iii_sdk::{CallAction, CallRequest, IIIClient};
use std::sync::Arc;

use super::new_mail::{NewMailEvent, ON_NEW_MAIL_FN};
use super::Event;

#[async_trait]
pub trait EventDispatcher: Send + Sync {
    async fn dispatch(&self, event: Event);
}

pub struct EngineDispatcher {
    iii: Arc<IIIClient>,
}

impl EngineDispatcher {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl EventDispatcher for EngineDispatcher {
    async fn dispatch(&self, event: Event) {
        if !self.iii.is_observed(ON_NEW_MAIL_FN) {
            return;
        }
        let (account, folder, uid) = (event.account.clone(), event.folder.clone(), event.uid);
        let payload = match serde_json::to_value(NewMailEvent::from(event)) {
            Ok(payload) => payload,
            Err(e) => {
                tracing::warn!(
                    %account,
                    %folder,
                    uid,
                    error = %e,
                    "email::on-new-mail event did not serialize"
                );
                return;
            }
        };
        let request = CallRequest {
            action: Some(CallAction::Void),
            ..CallRequest::new(ON_NEW_MAIL_FN, payload)
        };
        if let Err(e) = self.iii.call(request).await {
            tracing::warn!(
                %account,
                %folder,
                uid,
                error = %e,
                "email::on-new-mail call failed"
            );
        }
    }
}
