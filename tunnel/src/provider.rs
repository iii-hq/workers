//! Starting a tunnel and reporting what happens to it. Leases, generations,
//! deadlines, retries and persistence stay in the manager.
use std::{path::Path, sync::Arc};

use tokio::sync::mpsc;

use crate::{
    config::{Config, ProviderKind},
    prerequisites::Prerequisites,
};

pub mod cloudflared;
pub mod opentunnel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// Public URL, already validated by the provider.
    Url(String),
    Connected,
    Disconnected,
    /// Sanitized text; the manager fails the generation with it.
    Fatal(String),
}

/// One signal, stamped with the generation that produced it.
#[derive(Debug)]
pub struct Output {
    pub tunnel_id: String,
    pub generation: String,
    pub signal: Signal,
}

/// Stamps tunnel_id + generation, so a provider cannot mix generations.
#[derive(Clone)]
pub struct Sink {
    tunnel_id: String,
    generation: String,
    tx: mpsc::Sender<Output>,
}

impl Sink {
    pub fn new(tunnel_id: &str, generation: &str, tx: mpsc::Sender<Output>) -> Self {
        Self {
            tunnel_id: tunnel_id.to_owned(),
            generation: generation.to_owned(),
            tx,
        }
    }

    /// False once the manager stopped listening.
    pub async fn send(&self, signal: Signal) -> bool {
        self.tx
            .send(Output {
                tunnel_id: self.tunnel_id.clone(),
                generation: self.generation.clone(),
                signal,
            })
            .await
            .is_ok()
    }
}

#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    /// "cloudflared" | "opentunnel". Prefix of manager-side error strings.
    fn name(&self) -> &'static str;
    /// Must not block. `state_dir` is the parent of state_path. The error is
    /// sanitized text for the snapshot.
    fn start(
        &self,
        tunnel_id: &str,
        origin: &str,
        state_dir: &Path,
        sink: Sink,
    ) -> Result<Box<dyn Session>, String>;
    /// status.prerequisites. None = nothing to report.
    async fn prerequisites(&self) -> Option<Prerequisites>;
}

#[async_trait::async_trait]
pub trait Session: Send {
    /// Some(reason) once the tunnel ended by itself. Polled on every tick.
    fn exited(&mut self) -> Option<String>;
    /// Stop and wait. Must leave nothing running.
    async fn stop(self: Box<Self>);
}

/// The provider a validated config asks for.
pub fn build(config: &Config) -> Result<Arc<dyn Provider>, String> {
    Ok(match config.provider {
        ProviderKind::Cloudflared => Arc::new(cloudflared::Cloudflared::new(&config.cloudflared)),
        ProviderKind::Opentunnel => Arc::new(opentunnel::OpenTunnel::from_config(config)?),
    })
}
