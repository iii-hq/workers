//! Operator-facing authentication RPCs. Every response is token-free.
use crate::session::{Ack, AuthManager, EmptyRequest, LoginIdRequest};
use crate::surface;
use iii_sdk::{errors::Error, IIIClient, RegisterFunction};
use serde_json::json;
use std::sync::Arc;

pub fn register(iii: &IIIClient, auth: Arc<AuthManager>) {
    let manager = auth.clone();
    iii.register_function(
        surface::LOGIN_START_ID,
        RegisterFunction::new_async(move |_: EmptyRequest| {
            let manager = manager.clone();
            async move { manager.start().await.map_err(|e| e.into_bus()) }
        })
        .description(surface::LOGIN_START_DESC)
        .metadata(json!({"internal":true})),
    );
    let manager = auth.clone();
    iii.register_function(
        surface::LOGIN_POLL_ID,
        RegisterFunction::new_async(move |req: LoginIdRequest| {
            let manager = manager.clone();
            async move { Ok::<_, Error>(manager.poll(&req.login_id)) }
        })
        .description(surface::LOGIN_POLL_DESC)
        .metadata(json!({"internal":true})),
    );
    let manager = auth.clone();
    iii.register_function(
        surface::LOGIN_CANCEL_ID,
        RegisterFunction::new_async(move |req: LoginIdRequest| {
            let manager = manager.clone();
            async move {
                manager.cancel(&req.login_id).await;
                Ok::<_, Error>(Ack { ok: true })
            }
        })
        .description(surface::LOGIN_CANCEL_DESC)
        .metadata(json!({"internal":true})),
    );
    let manager = auth.clone();
    iii.register_function(
        surface::AUTH_STATUS_ID,
        RegisterFunction::new_async(move |_: EmptyRequest| {
            let manager = manager.clone();
            async move { manager.status().await.map_err(|e| e.into_bus()) }
        })
        .description(surface::AUTH_STATUS_DESC)
        .metadata(json!({"internal":true})),
    );
    iii.register_function(
        surface::AUTH_LOGOUT_ID,
        RegisterFunction::new_async(move |_: EmptyRequest| {
            let manager = auth.clone();
            async move {
                manager.logout().await.map_err(|e| e.into_bus())?;
                Ok::<_, Error>(Ack { ok: true })
            }
        })
        .description(surface::AUTH_LOGOUT_DESC)
        .metadata(json!({"internal":true})),
    );
}
