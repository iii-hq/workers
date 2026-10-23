//! The public `tmux::*` functions. Mutations poke the live feed so the
//! console reflects them at once.

pub mod panes;
pub mod sessions;
pub mod subagents;
pub mod windows;

use std::sync::{Arc, Mutex};

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::collector::Collector;
use crate::events::Monitor;
use crate::model::{PortsSnapshot, SessionsSnapshot, TmuxInfo};
use crate::tmux_cli::Tmux;
use crate::transcript::Transcripts;

/// What every handler needs.
pub struct Ctx {
    pub collector: Arc<Collector>,
    pub monitor: Arc<Monitor>,
    /// The sub-agent transcripts viewers follow.
    pub transcripts: Arc<Mutex<Transcripts>>,
}

impl Ctx {
    pub async fn tmux(&self) -> Result<Tmux, String> {
        self.collector.tmux().await
    }
}

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

fn schema_of<T: JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

/// No parameters.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct EmptyInput {}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct KilledOutput {
    /// Always true.
    pub killed: bool,
}

pub const STATUS: &str = "tmux::status";
pub const SESSIONS_LIST: &str = "tmux::sessions::list";
pub const PORTS_LIST: &str = "tmux::ports::list";

const STATUS_DESC: &str = "Whether tmux is installed, its version and path, and whether a tmux server is running on the configured socket.";
const SESSIONS_LIST_DESC: &str = "Every tmux session with its windows and panes: foreground command, cwd, title, size, the agent CLI running in the pane (claude, codex, … with state working / waiting / idle), the `iii compose` daemon it runs and the ports its processes listen on.";
const PORTS_LIST_DESC: &str = "Every TCP port this user's processes listen on, with process, iii role (engine / worker / compose), worker name, compose namespace and the tmux pane the process runs in; plus each running `iii compose` stack with its engine WebSocket port and workers' ports.";

pub async fn status(ctx: &Ctx, _input: EmptyInput) -> Result<TmuxInfo, String> {
    Ok(ctx.collector.info().await)
}

pub async fn sessions_list(ctx: &Ctx, _input: EmptyInput) -> Result<SessionsSnapshot, String> {
    Ok(ctx.collector.overview().await.sessions)
}

pub async fn ports_list(ctx: &Ctx, _input: EmptyInput) -> Result<PortsSnapshot, String> {
    Ok(ctx.collector.overview().await.ports)
}

macro_rules! register_fn {
    ($iii:expr, $ctx:expr, $id:expr, $desc:expr, $input:ty, $handler:path) => {{
        let ctx = $ctx.clone();
        $iii.register_function(
            $id,
            iii_sdk::RegisterFunction::new_async(move |input: $input| {
                let ctx = ctx.clone();
                async move {
                    $handler(&ctx, input)
                        .await
                        .map_err(iii_sdk::errors::Error::Handler)
                }
            })
            .description($desc),
        );
    }};
}
pub(crate) use register_fn;

/// Every public function, in registration order.
pub fn catalog() -> Vec<FunctionSpec> {
    let mut specs = vec![
        spec::<EmptyInput, TmuxInfo>(STATUS, STATUS_DESC),
        spec::<EmptyInput, SessionsSnapshot>(SESSIONS_LIST, SESSIONS_LIST_DESC),
    ];
    specs.extend(sessions::catalog());
    specs.extend(windows::catalog());
    specs.extend(panes::catalog());
    specs.extend(subagents::catalog());
    specs.push(spec::<EmptyInput, PortsSnapshot>(
        PORTS_LIST,
        PORTS_LIST_DESC,
    ));
    specs
}

pub fn register_all(iii: &IIIClient, ctx: Arc<Ctx>) {
    register_fn!(iii, ctx, STATUS, STATUS_DESC, EmptyInput, status);
    register_fn!(
        iii,
        ctx,
        SESSIONS_LIST,
        SESSIONS_LIST_DESC,
        EmptyInput,
        sessions_list
    );
    sessions::register(iii, &ctx);
    windows::register(iii, &ctx);
    panes::register(iii, &ctx);
    subagents::register(iii, &ctx);
    register_fn!(
        iii,
        ctx,
        PORTS_LIST,
        PORTS_LIST_DESC,
        EmptyInput,
        ports_list
    );
}
