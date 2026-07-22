//! Run-scoped in-process service ownership.
//!
//! The service group starts each dedicated engine connection in dependency
//! order, rolls back any earlier connections when a later start fails, and
//! shuts down each connection at most once.

use std::path::PathBuf;

use anyhow::Context;

use crate::client::Client;
use crate::recorder::Recorder;
use crate::scripted_router::ScriptedRouter;
use crate::types::script::RouterScriptV1;

pub struct RunServices {
    router: Option<ScriptedRouter>,
    recorder: Option<Recorder>,
    client: Option<Client>,
}

impl RunServices {
    pub async fn start(
        ws_url: &str,
        script: RouterScriptV1,
        recorder_log: PathBuf,
    ) -> anyhow::Result<Self> {
        let mut services = Self::empty();

        services.router = Some(
            ScriptedRouter::start(ws_url, script)
                .await
                .context("start integration scripted router")?,
        );

        match Recorder::start(ws_url, recorder_log).await {
            Ok(recorder) => services.recorder = Some(recorder),
            Err(error) => {
                services.shutdown().await;
                return Err(error).context("start integration recorder");
            }
        }

        services.client = Some(Client::connect(ws_url, "integration-runner"));
        Ok(services)
    }

    pub fn router(&self) -> &ScriptedRouter {
        self.router
            .as_ref()
            .expect("run services accessed after shutdown")
    }

    pub fn recorder(&self) -> &Recorder {
        self.recorder
            .as_ref()
            .expect("run services accessed after shutdown")
    }

    pub fn client(&self) -> &Client {
        self.client
            .as_ref()
            .expect("run services accessed after shutdown")
    }

    /// Shut down in reverse start order. Taking each service before awaiting
    /// makes repeated calls no-ops and lets a cancelled call resume cleanup
    /// without shutting down an already completed service again.
    pub async fn shutdown(&mut self) {
        if let Some(client) = self.client.take() {
            client.shutdown().await;
        }
        if let Some(recorder) = self.recorder.take() {
            recorder.shutdown().await;
        }
        if let Some(router) = self.router.take() {
            router.shutdown().await;
        }
    }

    fn empty() -> Self {
        Self {
            router: None,
            recorder: None,
            client: None,
        }
    }
}
