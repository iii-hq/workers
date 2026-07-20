//! `fp` binary entry: connect, register the transforms + `fp::pipe`, bind
//! the harness pre-generate guidance hook, then sleep until Ctrl+C.

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};

use fp::{guidance, pipe, util};

#[derive(Parser, Debug)]
#[command(
    name = "fp",
    about = "Lodash-style transforms and worker-side pipelines on the iii bus."
)]
struct Cli {
    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let iii = Arc::new(register_worker(
        &cli.url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                name: "fp".to_string(),
                os: std::env::consts::OS.to_string(),
                description: Some(
                    "Lodash-style transforms and worker-side pipelines on the iii bus.".to_string(),
                ),
                pid: Some(std::process::id()),
                telemetry: None,
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));

    register_functions(&iii);
    guidance::setup(&iii);

    tracing::info!("fp ready: fp::pipe + 18 transforms + guidance injection");
    tokio::signal::ctrl_c().await?;
    tracing::info!("fp shutting down");
    iii.shutdown_async().await;
    Ok(())
}

fn register_functions(iii: &Arc<IIIClient>) {
    let engine = iii.clone();
    iii.register_function(
        pipe::PIPE_ID,
        RegisterFunction::new_async(move |req: pipe::PipeRequest| {
            let iii = engine.clone();
            async move { pipe::run(&iii, req).await.map_err(Error::Handler) }
        })
        .description(pipe::PIPE_DESC),
    );

    register_util(iii, util::GET_ID, util::GET_DESC, |r: util::GetRequest| {
        util::get(&r.value, &r.path)
    });
    register_util(
        iii,
        util::PICK_ID,
        util::PICK_DESC,
        |r: util::PickRequest| util::pick(r.value, &r.paths),
    );
    register_util(
        iii,
        util::OMIT_ID,
        util::OMIT_DESC,
        |r: util::OmitRequest| util::omit(r.value, &r.paths),
    );
    register_util(
        iii,
        util::TAKE_ID,
        util::TAKE_DESC,
        |r: util::TakeRequest| util::take(r.value, r.n as usize),
    );
    register_util(
        iii,
        util::DROP_ID,
        util::DROP_DESC,
        |r: util::DropRequest| util::drop(r.value, r.n as usize),
    );
    register_util(iii, util::MAP_ID, util::MAP_DESC, |r: util::MapRequest| {
        util::map(r.value, &r.path)
    });
    register_util(
        iii,
        util::FILTER_ID,
        util::FILTER_DESC,
        |r: util::FilterRequest| util::filter(r.value, &r.matches),
    );
    register_util(
        iii,
        util::SPLIT_ID,
        util::SPLIT_DESC,
        |r: util::SplitRequest| util::split(r.value, &r.separator),
    );
    register_util(
        iii,
        util::JOIN_ID,
        util::JOIN_DESC,
        |r: util::JoinRequest| util::join(r.value, &r.separator),
    );
    register_util(
        iii,
        util::UNIQ_ID,
        util::UNIQ_DESC,
        |r: util::ValueRequest| util::uniq(r.value),
    );
    register_util(
        iii,
        util::SIZE_ID,
        util::SIZE_DESC,
        |r: util::ValueRequest| util::size(&r.value),
    );
    register_util(
        iii,
        util::COMPACT_ID,
        util::COMPACT_DESC,
        |r: util::ValueRequest| util::compact(r.value),
    );
    register_util(iii, util::NTH_ID, util::NTH_DESC, |r: util::NthRequest| {
        util::nth(r.value, r.n)
    });
    register_util(
        iii,
        util::GET_OR_ID,
        util::GET_OR_DESC,
        |r: util::GetOrRequest| util::get_or(&r.value, &r.path, r.default),
    );
    register_util(
        iii,
        util::FLATTEN_ID,
        util::FLATTEN_DESC,
        |r: util::ValueRequest| util::flatten(r.value),
    );
    register_util(
        iii,
        util::SORT_BY_ID,
        util::SORT_BY_DESC,
        |r: util::SortByRequest| util::sort_by(r.value, &r.path),
    );
    register_util(
        iii,
        util::REVERSE_ID,
        util::REVERSE_DESC,
        |r: util::ValueRequest| util::reverse(r.value),
    );

    // when returns { passed, value } instead of the UtilResponse wrapper —
    // a guard's verdict must be visible on direct calls too.
    iii.register_function(
        util::WHEN_ID,
        RegisterFunction::new_async(|req: util::WhenRequest| async move {
            util::when(&req.value, req.path.as_deref(), req.op, req.to.as_ref())
                .map(|passed| util::WhenResponse {
                    passed,
                    value: req.value,
                })
                .map_err(Error::Handler)
        })
        .description(util::WHEN_DESC),
    );
}

/// Register one transform: parse the typed request, run the pure core, wrap
/// the result as `UtilResponse { value }` (typed response schema).
fn register_util<Req, F>(iii: &Arc<IIIClient>, id: &'static str, description: &'static str, core: F)
where
    Req: serde::de::DeserializeOwned + schemars::JsonSchema + Send + 'static,
    F: Fn(Req) -> std::result::Result<serde_json::Value, String> + Send + Sync + Clone + 'static,
{
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Req| {
            let core = core.clone();
            async move {
                core(req)
                    .map(|value| util::UtilResponse { value })
                    .map_err(Error::Handler)
            }
        })
        .description(description),
    );
}
