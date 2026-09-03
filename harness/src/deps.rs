//! Everything a function handler / loop step needs. Clients are built per
//! call from the live config snapshot so a hot-reloaded timeout takes effect
//! on the next call.

use std::sync::Arc;

use iii_sdk::IIIClient;

use crate::bindings::BindingStore;
use crate::clients::{ContextClient, EngineClient, RouterClient, SessionClient};
use crate::config::WorkerConfig;
use crate::configuration::ConfigCell;
use crate::discovery::{FunctionsCell, FunctionsSnapshot};
use crate::events::TurnEvents;
use crate::hooks::HookRegistry;
use crate::locks::{SessionLocks, TurnCancels};
use crate::projects::ProjectStore;
use crate::skills::{SkillsCell, SkillsSnapshot};

#[derive(Clone)]
pub struct Deps {
    pub iii: Arc<IIIClient>,
    pub config: ConfigCell,
    pub functions: FunctionsCell,
    pub skills: SkillsCell,
    pub events: TurnEvents,
    pub hooks: HookRegistry,
    pub locks: SessionLocks,
    pub cancels: TurnCancels,
    /// Harness-owned JSON project catalog, serialized across concurrent
    /// console requests while allowing its configured file path to hot-reload.
    pub projects: ProjectStore,
    /// SDK handles for the delivery triggers this process registered; see
    /// [`crate::bindings::TriggerHandles`].
    pub trigger_handles: crate::bindings::TriggerHandles,
}

impl Deps {
    pub fn new(
        iii: Arc<IIIClient>,
        config: ConfigCell,
        functions: FunctionsCell,
        skills: SkillsCell,
        events: TurnEvents,
        hooks: HookRegistry,
    ) -> Self {
        Self {
            iii,
            config,
            functions,
            skills,
            events,
            hooks,
            locks: SessionLocks::new(),
            cancels: TurnCancels::new(),
            projects: ProjectStore::default(),
            trigger_handles: crate::bindings::TriggerHandles::default(),
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

    pub async fn skills(&self) -> Arc<SkillsSnapshot> {
        self.skills.read().await.clone()
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

    /// The durable trigger-binding store. Built per call like the other
    /// clients so a hot-reloaded timeout applies to the next read.
    pub async fn bindings(&self) -> BindingStore {
        let cfg = self.cfg().await;
        BindingStore::new(
            self.iii.clone(),
            cfg.session_timeout_ms,
            self.events.clone(),
        )
    }
}
