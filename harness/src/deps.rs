//! Everything a function handler / loop step needs. Clients are built per
//! call from the live config snapshot so a hot-reloaded timeout takes effect
//! on the next call.

use std::sync::Arc;

use iii_sdk::III;

use crate::clients::{
    ContextClient, EngineClient, FunctionDescriptor, RouterClient, SessionClient,
};
use crate::config::WorkerConfig;
use crate::configuration::ConfigCell;
use crate::discovery::FunctionsCell;
use crate::events::TurnEvents;
use crate::hooks::HookRegistry;
use crate::locks::SessionLocks;

#[derive(Clone)]
pub struct Deps {
    pub iii: Arc<III>,
    pub config: ConfigCell,
    pub functions: FunctionsCell,
    pub events: TurnEvents,
    pub hooks: HookRegistry,
    pub locks: SessionLocks,
}

impl Deps {
    pub fn new(
        iii: Arc<III>,
        config: ConfigCell,
        functions: FunctionsCell,
        events: TurnEvents,
        hooks: HookRegistry,
    ) -> Self {
        Self {
            iii,
            config,
            functions,
            events,
            hooks,
            locks: SessionLocks::new(),
        }
    }

    /// The current config snapshot (cheap `Arc` clone).
    pub async fn cfg(&self) -> Arc<WorkerConfig> {
        self.config.read().await.clone()
    }

    /// The current cached function-registry snapshot (cheap `Arc` clone). Kept
    /// live by the `engine::functions-available` trigger; see [`crate::discovery`].
    pub async fn functions(&self) -> Arc<Vec<FunctionDescriptor>> {
        self.functions.read().await.clone()
    }

    pub async fn session(&self) -> SessionClient {
        let cfg = self.cfg().await;
        SessionClient::new(self.iii.clone(), cfg.session_timeout_ms)
    }

    pub async fn context(&self) -> ContextClient {
        let cfg = self.cfg().await;
        ContextClient::new(self.iii.clone(), cfg.context_timeout_ms)
    }

    pub async fn router(&self) -> RouterClient {
        let cfg = self.cfg().await;
        RouterClient::new(
            self.iii.clone(),
            cfg.router_timeout_ms,
            cfg.stream_coalesce_ms,
        )
    }

    pub async fn engine(&self) -> EngineClient {
        let cfg = self.cfg().await;
        EngineClient::new(self.iii.clone(), cfg.dispatch_timeout_ms)
    }
}
