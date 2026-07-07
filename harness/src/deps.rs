//! Everything a function handler / loop step needs. Clients are built per
//! call from the live config snapshot so a hot-reloaded timeout takes effect
//! on the next call.

use std::sync::Arc;

use iii_sdk::IIIClient;

use crate::clients::{ContextClient, EngineClient, RouterClient, SessionClient};
use crate::config::WorkerConfig;
use crate::configuration::ConfigCell;
use crate::discovery::{FunctionsCell, FunctionsSnapshot};
use crate::events::TurnEvents;
use crate::hooks::HookRegistry;
use crate::instructions::{InstructionsCell, InstructionsSnapshot};
use crate::locks::SessionLocks;
use crate::subscriptions::SubscriptionRegistry;

#[derive(Clone)]
pub struct Deps {
    pub iii: Arc<IIIClient>,
    pub config: ConfigCell,
    pub functions: FunctionsCell,
    pub instructions: InstructionsCell,
    pub events: TurnEvents,
    pub hooks: HookRegistry,
    pub locks: SessionLocks,
    pub subscriptions: Arc<SubscriptionRegistry>,
    /// react's per-subscription fire-rate breaker (loop breaker #3).
    pub react_gate: Arc<crate::functions::react::FireGate>,
}

impl Deps {
    pub fn new(
        iii: Arc<IIIClient>,
        config: ConfigCell,
        functions: FunctionsCell,
        instructions: InstructionsCell,
        events: TurnEvents,
        hooks: HookRegistry,
    ) -> Self {
        Self {
            iii,
            config,
            functions,
            instructions,
            events,
            hooks,
            locks: SessionLocks::new(),
            subscriptions: Arc::new(SubscriptionRegistry::new()),
            react_gate: Arc::new(crate::functions::react::FireGate::default()),
        }
    }

    /// The current config snapshot (cheap `Arc` clone).
    pub async fn cfg(&self) -> Arc<WorkerConfig> {
        self.config.read().await.clone()
    }

    /// The current cached function-registry snapshot (cheap `Arc` clone). Kept
    /// live by the `engine::functions-available` trigger; see [`crate::discovery`].
    /// Carries both the callable set (`.functions`) and its `.generation`.
    pub async fn functions(&self) -> Arc<FunctionsSnapshot> {
        self.functions.read().await.clone()
    }

    /// The current instructions snapshot (cheap `Arc` clone). Kept live by the
    /// unfiltered `configuration` trigger; see [`crate::instructions`].
    pub async fn instructions(&self) -> Arc<InstructionsSnapshot> {
        self.instructions.read().await.clone()
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
