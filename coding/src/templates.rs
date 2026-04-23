use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDef {
    pub id: String,
    pub description: String,
    pub request_format: Option<Value>,
    pub response_format: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerDef {
    pub trigger_type: String,
    pub function_id: String,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerFiles {
    pub files: Vec<GeneratedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
    pub language: String,
}

pub fn rust_worker_template(
    name: &str,
    functions: &[FunctionDef],
    triggers: &[TriggerDef],
) -> WorkerFiles {
    let snake_name = name.replace('-', "_");
    let bin_name = name.to_string();

    let cargo_toml = format!(
        r#"[workspace]

[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
publish = false

[[bin]]
name = "{name}"
path = "src/main.rs"

[dependencies]
iii-sdk = "0.11.0"
tokio = {{ version = "1", features = ["rt-multi-thread", "macros", "sync", "signal"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
serde_yaml = "0.9"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = {{ version = "0.3", features = ["fmt", "env-filter"] }}
clap = {{ version = "4", features = ["derive"] }}
"#,
        name = bin_name
    );

    let build_rs = r#"fn main() {
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
"#
    .to_string();

    let config_yaml = format!("worker_name: \"{}\"\n", bin_name);

    let config_rs = format!(
        r#"use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct {struct_name}Config {{
    #[serde(default = "default_worker_name")]
    pub worker_name: String,
}}

fn default_worker_name() -> String {{
    "{name}".to_string()
}}

impl Default for {struct_name}Config {{
    fn default() -> Self {{
        {struct_name}Config {{
            worker_name: default_worker_name(),
        }}
    }}
}}

pub fn load_config(path: &str) -> Result<{struct_name}Config> {{
    let contents = std::fs::read_to_string(path)?;
    let config: {struct_name}Config = serde_yaml::from_str(&contents)?;
    Ok(config)
}}
"#,
        name = bin_name,
        struct_name = to_pascal_case(&snake_name)
    );

    let manifest_rs = format!(
        r#"use serde::Serialize;

#[derive(Serialize)]
pub struct ModuleManifest {{
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}}

pub fn build_manifest() -> ModuleManifest {{
    ModuleManifest {{
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "III engine {name} worker".to_string(),
        default_config: serde_json::json!({{
            "class": "modules::{snake}::{pascal}Module",
            "config": {{
                "worker_name": "{name}"
            }}
        }}),
        supported_targets: vec![env!("TARGET").to_string()],
    }}
}}
"#,
        name = bin_name,
        snake = snake_name,
        pascal = to_pascal_case(&snake_name)
    );

    let mut handler_files: Vec<GeneratedFile> = Vec::new();
    let mut mod_entries: Vec<String> = Vec::new();
    let mut fn_registrations = String::new();

    for func in functions {
        let fn_snake = func.id.replace("::", "_").replace('-', "_");
        let fn_short = func
            .id
            .split("::")
            .last()
            .unwrap_or(&func.id)
            .replace('-', "_");

        mod_entries.push(format!("pub mod {};", fn_short));

        let req_json = func
            .request_format
            .as_ref()
            .map(|v| {
                format!(
                    "Some(serde_json::json!({}))",
                    serde_json::to_string_pretty(v).unwrap_or_default()
                )
            })
            .unwrap_or_else(|| "None".to_string());

        let resp_json = func
            .response_format
            .as_ref()
            .map(|v| {
                format!(
                    "Some(serde_json::json!({}))",
                    serde_json::to_string_pretty(v).unwrap_or_default()
                )
            })
            .unwrap_or_else(|| "None".to_string());

        fn_registrations.push_str(&format!(
            r#"
    let _fn_{fn_snake} = iii.register_function_with(
        RegisterFunctionMessage {{
            id: "{fn_id}".to_string(),
            description: Some("{description}".to_string()),
            request_format: {req_json},
            response_format: {resp_json},
            metadata: None,
            invocation: None,
        }},
        functions::{fn_short}::build_handler(iii_arc.clone()),
    );
"#,
            fn_snake = fn_snake,
            fn_id = func.id,
            description = func.description.replace('"', "\\\""),
            req_json = req_json,
            resp_json = resp_json,
            fn_short = fn_short,
        ));

        let handler_content = format!(
            r#"use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{{IIIError, III}};
use serde_json::Value;

pub fn build_handler(
    iii: Arc<III>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {{
    move |payload: Value| {{
        let iii = iii.clone();
        Box::pin(async move {{ handle(&iii, payload).await }})
    }}
}}

pub async fn handle(_iii: &III, _payload: Value) -> Result<Value, IIIError> {{
    Ok(serde_json::json!({{
        "status": "ok",
        "function": "{fn_id}"
    }}))
}}
"#,
            fn_id = func.id,
        );

        handler_files.push(GeneratedFile {
            path: format!("src/functions/{}.rs", fn_short),
            content: handler_content,
            language: "rust".to_string(),
        });
    }

    let mut trigger_registrations = String::new();
    for (i, trigger) in triggers.iter().enumerate() {
        let trigger_config_str =
            serde_json::to_string(&trigger.config).unwrap_or_else(|_| "{}".to_string());
        trigger_registrations.push_str(&format!(
            r#"
    let _trigger_{i} = iii.register_trigger(RegisterTriggerInput {{
        trigger_type: "{trigger_type}".to_string(),
        function_id: "{function_id}".to_string(),
        config: serde_json::json!({config}),
        metadata: None,
    }});
"#,
            i = i,
            trigger_type = trigger.trigger_type,
            function_id = trigger.function_id,
            config = trigger_config_str,
        ));
    }

    let functions_mod_rs = mod_entries.join("\n") + "\n";

    let main_rs = format!(
        r#"use anyhow::Result;
use clap::Parser;
use iii_sdk::{{register_worker, InitOptions, OtelConfig, RegisterFunctionMessage, RegisterTriggerInput}};
use std::sync::Arc;

mod config;
mod functions;
mod manifest;

#[derive(Parser, Debug)]
#[command(name = "{name}", about = "III engine {name} worker")]
struct Cli {{
    #[arg(long, default_value = "./config.yaml")]
    config: String,

    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,

    #[arg(long)]
    manifest: bool,
}}

#[tokio::main]
async fn main() -> Result<()> {{
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if cli.manifest {{
        let manifest = manifest::build_manifest();
        println!("{{}}", serde_json::to_string_pretty(&manifest).unwrap());
        return Ok(());
    }}

    let worker_config = match config::load_config(&cli.config) {{
        Ok(c) => {{
            tracing::info!(name = %c.worker_name, "loaded config from {{}}", cli.config);
            c
        }}
        Err(e) => {{
            tracing::warn!(error = %e, path = %cli.config, "failed to load config, using defaults");
            config::{pascal}Config::default()
        }}
    }};

    let _config = Arc::new(worker_config);

    tracing::info!(url = %cli.url, "connecting to III engine");

    let iii = register_worker(
        &cli.url,
        InitOptions {{
            otel: Some(OtelConfig::default()),
            ..Default::default()
        }},
    );

    let iii_arc = Arc::new(iii.clone());
{fn_registrations}{trigger_registrations}
    tracing::info!("{name} registered {fn_count} functions and {tr_count} triggers, waiting for invocations");

    tokio::signal::ctrl_c().await?;

    tracing::info!("{name} shutting down");
    iii.shutdown_async().await;

    Ok(())
}}
"#,
        name = bin_name,
        pascal = to_pascal_case(&snake_name),
        fn_registrations = fn_registrations,
        trigger_registrations = trigger_registrations,
        fn_count = functions.len(),
        tr_count = triggers.len(),
    );

    let mut files = vec![
        GeneratedFile {
            path: "Cargo.toml".to_string(),
            content: cargo_toml,
            language: "toml".to_string(),
        },
        GeneratedFile {
            path: "build.rs".to_string(),
            content: build_rs,
            language: "rust".to_string(),
        },
        GeneratedFile {
            path: "config.yaml".to_string(),
            content: config_yaml,
            language: "yaml".to_string(),
        },
        GeneratedFile {
            path: "src/main.rs".to_string(),
            content: main_rs,
            language: "rust".to_string(),
        },
        GeneratedFile {
            path: "src/config.rs".to_string(),
            content: config_rs,
            language: "rust".to_string(),
        },
        GeneratedFile {
            path: "src/manifest.rs".to_string(),
            content: manifest_rs,
            language: "rust".to_string(),
        },
        GeneratedFile {
            path: "src/functions/mod.rs".to_string(),
            content: functions_mod_rs,
            language: "rust".to_string(),
        },
    ];

    files.extend(handler_files);

    WorkerFiles { files }
}

pub fn typescript_worker_template(
    name: &str,
    functions: &[FunctionDef],
    triggers: &[TriggerDef],
) -> WorkerFiles {
    let mut files: Vec<GeneratedFile> = Vec::new();

    let package_json = format!(
        r#"{{
  "name": "{name}",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {{
    "build": "tsc",
    "start": "node dist/index.js",
    "dev": "tsx src/index.ts",
    "test": "vitest run"
  }},
  "dependencies": {{
    "iii-sdk": "^0.11.0"
  }},
  "devDependencies": {{
    "typescript": "^5.0.0",
    "tsx": "^4.0.0",
    "vitest": "^1.0.0"
  }}
}}
"#,
        name = name
    );

    let mut fn_imports = String::new();
    let mut fn_registrations = String::new();
    let mut trigger_registrations = String::new();

    for func in functions {
        let fn_short = func
            .id
            .split("::")
            .last()
            .unwrap_or(&func.id)
            .replace('-', "_");

        fn_imports.push_str(&format!(
            "import {{ handle as handle_{fn_short} }} from './functions/{fn_short}.js';\n",
            fn_short = fn_short
        ));

        let req_str = func
            .request_format
            .as_ref()
            .map(|v| serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| "undefined".to_string());

        let resp_str = func
            .response_format
            .as_ref()
            .map(|v| serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| "undefined".to_string());

        fn_registrations.push_str(&format!(
            r#"
iii.registerFunction({{
  id: '{fn_id}',
  description: '{description}',
  requestFormat: {req_str},
  responseFormat: {resp_str},
}}, handle_{fn_short});
"#,
            fn_id = func.id,
            description = func.description.replace('\'', "\\'"),
            req_str = req_str,
            resp_str = resp_str,
            fn_short = fn_short,
        ));

        let handler_content = format!(
            r#"export async function handle(payload: Record<string, unknown>): Promise<Record<string, unknown>> {{
  return {{
    status: 'ok',
    function: '{fn_id}',
  }};
}}
"#,
            fn_id = func.id,
        );

        files.push(GeneratedFile {
            path: format!("src/functions/{}.ts", fn_short),
            content: handler_content,
            language: "typescript".to_string(),
        });
    }

    for trigger in triggers {
        let config_str =
            serde_json::to_string_pretty(&trigger.config).unwrap_or_else(|_| "{}".to_string());
        trigger_registrations.push_str(&format!(
            r#"
iii.registerTrigger({{
  triggerType: '{trigger_type}',
  functionId: '{function_id}',
  config: {config},
}});
"#,
            trigger_type = trigger.trigger_type,
            function_id = trigger.function_id,
            config = config_str,
        ));
    }

    let index_ts = format!(
        r#"import {{ registerWorker }} from 'iii-sdk';
{fn_imports}
const iii = registerWorker(process.env.III_URL || 'ws://127.0.0.1:49134', {{
  otel: {{}},
}});
{fn_registrations}{trigger_registrations}
console.log('{name} registered {fn_count} functions and {tr_count} triggers');

process.on('SIGINT', async () => {{
  console.log('{name} shutting down');
  await iii.shutdown();
  process.exit(0);
}});
"#,
        name = name,
        fn_imports = fn_imports,
        fn_registrations = fn_registrations,
        trigger_registrations = trigger_registrations,
        fn_count = functions.len(),
        tr_count = triggers.len(),
    );

    let tsconfig = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "outDir": "dist",
    "rootDir": "src",
    "strict": true,
    "esModuleInterop": true,
    "declaration": true
  },
  "include": ["src"]
}
"#
    .to_string();

    files.push(GeneratedFile {
        path: "package.json".to_string(),
        content: package_json,
        language: "json".to_string(),
    });
    files.push(GeneratedFile {
        path: "tsconfig.json".to_string(),
        content: tsconfig,
        language: "json".to_string(),
    });
    files.push(GeneratedFile {
        path: "src/index.ts".to_string(),
        content: index_ts,
        language: "typescript".to_string(),
    });

    WorkerFiles { files }
}

pub fn python_worker_template(
    name: &str,
    functions: &[FunctionDef],
    triggers: &[TriggerDef],
) -> WorkerFiles {
    let mut files: Vec<GeneratedFile> = Vec::new();

    let pyproject = format!(
        r#"[project]
name = "{name}"
version = "0.1.0"
requires-python = ">=3.11"
dependencies = [
    "iii-sdk>=0.11.0",
]

[project.optional-dependencies]
dev = [
    "pytest>=7.0",
    "pytest-asyncio>=0.21",
]
"#,
        name = name
    );

    let mut fn_imports = String::new();
    let mut fn_registrations = String::new();
    let mut trigger_registrations = String::new();

    for func in functions {
        let fn_short = func
            .id
            .split("::")
            .last()
            .unwrap_or(&func.id)
            .replace('-', "_");

        fn_imports.push_str(&format!(
            "from functions.{fn_short} import handle as handle_{fn_short}\n",
            fn_short = fn_short
        ));

        let req_str = func
            .request_format
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| "None".to_string());

        let resp_str = func
            .response_format
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| "None".to_string());

        fn_registrations.push_str(&format!(
            r#"
iii.register_function(
    id="{fn_id}",
    description="{description}",
    request_format={req_str},
    response_format={resp_str},
    handler=handle_{fn_short},
)
"#,
            fn_id = func.id,
            description = func.description.replace('"', "\\\""),
            req_str = req_str,
            resp_str = resp_str,
            fn_short = fn_short,
        ));

        let handler_content = format!(
            r#"async def handle(payload: dict) -> dict:
    return {{
        "status": "ok",
        "function": "{fn_id}",
    }}
"#,
            fn_id = func.id,
        );

        files.push(GeneratedFile {
            path: format!("src/functions/{}.py", fn_short),
            content: handler_content,
            language: "python".to_string(),
        });
    }

    for trigger in triggers {
        let config_str =
            serde_json::to_string(&trigger.config).unwrap_or_else(|_| "{}".to_string());
        trigger_registrations.push_str(&format!(
            r#"
iii.register_trigger(
    trigger_type="{trigger_type}",
    function_id="{function_id}",
    config={config},
)
"#,
            trigger_type = trigger.trigger_type,
            function_id = trigger.function_id,
            config = config_str,
        ));
    }

    let init_py = "".to_string();

    let worker_py = format!(
        r#"import os
import signal
import asyncio
from iii_sdk import register_worker
{fn_imports}

async def main():
    url = os.environ.get("III_URL", "ws://127.0.0.1:49134")
    iii = register_worker(url, otel={{}})
{fn_registrations}{trigger_registrations}
    print("{name} registered {fn_count} functions and {tr_count} triggers")

    stop = asyncio.Event()
    loop = asyncio.get_event_loop()
    loop.add_signal_handler(signal.SIGINT, stop.set)

    await stop.wait()
    print("{name} shutting down")
    await iii.shutdown()

if __name__ == "__main__":
    asyncio.run(main())
"#,
        name = name,
        fn_imports = fn_imports,
        fn_registrations = fn_registrations,
        trigger_registrations = trigger_registrations,
        fn_count = functions.len(),
        tr_count = triggers.len(),
    );

    files.push(GeneratedFile {
        path: "pyproject.toml".to_string(),
        content: pyproject,
        language: "toml".to_string(),
    });
    files.push(GeneratedFile {
        path: "src/__init__.py".to_string(),
        content: init_py.clone(),
        language: "python".to_string(),
    });
    files.push(GeneratedFile {
        path: "src/functions/__init__.py".to_string(),
        content: init_py,
        language: "python".to_string(),
    });
    files.push(GeneratedFile {
        path: "src/worker.py".to_string(),
        content: worker_py,
        language: "python".to_string(),
    });

    WorkerFiles { files }
}

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    upper + chars.as_str()
                }
            }
        })
        .collect()
}

pub fn generate_single_function_rust(func: &FunctionDef) -> GeneratedFile {
    let fn_short = func
        .id
        .split("::")
        .last()
        .unwrap_or(&func.id)
        .replace('-', "_");

    let content = format!(
        r#"use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{{IIIError, III}};
use serde_json::Value;

pub fn build_handler(
    iii: Arc<III>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {{
    move |payload: Value| {{
        let iii = iii.clone();
        Box::pin(async move {{ handle(&iii, payload).await }})
    }}
}}

pub async fn handle(_iii: &III, _payload: Value) -> Result<Value, IIIError> {{
    Ok(serde_json::json!({{
        "status": "ok",
        "function": "{fn_id}"
    }}))
}}
"#,
        fn_id = func.id,
    );

    GeneratedFile {
        path: format!("src/functions/{}.rs", fn_short),
        content,
        language: "rust".to_string(),
    }
}

pub fn generate_single_function_typescript(func: &FunctionDef) -> GeneratedFile {
    let fn_short = func
        .id
        .split("::")
        .last()
        .unwrap_or(&func.id)
        .replace('-', "_");

    let content = format!(
        r#"export async function handle(payload: Record<string, unknown>): Promise<Record<string, unknown>> {{
  return {{
    status: 'ok',
    function: '{fn_id}',
  }};
}}
"#,
        fn_id = func.id,
    );

    GeneratedFile {
        path: format!("src/functions/{}.ts", fn_short),
        content,
        language: "typescript".to_string(),
    }
}

pub fn generate_single_function_python(func: &FunctionDef) -> GeneratedFile {
    let fn_short = func
        .id
        .split("::")
        .last()
        .unwrap_or(&func.id)
        .replace('-', "_");

    let content = format!(
        r#"async def handle(payload: dict) -> dict:
    return {{
        "status": "ok",
        "function": "{fn_id}",
    }}
"#,
        fn_id = func.id,
    );

    GeneratedFile {
        path: format!("src/functions/{}.py", fn_short),
        content,
        language: "python".to_string(),
    }
}

pub fn generate_trigger_code_rust(trigger: &TriggerDef) -> String {
    let config_str = serde_json::to_string(&trigger.config).unwrap_or_else(|_| "{}".to_string());
    format!(
        r#"iii.register_trigger(RegisterTriggerInput {{
    trigger_type: "{trigger_type}".to_string(),
    function_id: "{function_id}".to_string(),
    config: serde_json::json!({config}),
    metadata: None,
}});"#,
        trigger_type = trigger.trigger_type,
        function_id = trigger.function_id,
        config = config_str,
    )
}

pub fn generate_trigger_code_typescript(trigger: &TriggerDef) -> String {
    let config_str =
        serde_json::to_string_pretty(&trigger.config).unwrap_or_else(|_| "{}".to_string());
    format!(
        r#"iii.registerTrigger({{
  triggerType: '{trigger_type}',
  functionId: '{function_id}',
  config: {config},
}});"#,
        trigger_type = trigger.trigger_type,
        function_id = trigger.function_id,
        config = config_str,
    )
}

pub fn generate_trigger_code_python(trigger: &TriggerDef) -> String {
    let config_str = serde_json::to_string(&trigger.config).unwrap_or_else(|_| "{}".to_string());
    format!(
        r#"iii.register_trigger(
    trigger_type="{trigger_type}",
    function_id="{function_id}",
    config={config},
)"#,
        trigger_type = trigger.trigger_type,
        function_id = trigger.function_id,
        config = config_str,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_functions() -> Vec<FunctionDef> {
        vec![
            FunctionDef {
                id: "myworker::greet".to_string(),
                description: "Greet a user".to_string(),
                request_format: Some(json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    }
                })),
                response_format: Some(json!({
                    "type": "object",
                    "properties": {
                        "message": { "type": "string" }
                    }
                })),
            },
            FunctionDef {
                id: "myworker::compute".to_string(),
                description: "Run a computation".to_string(),
                request_format: None,
                response_format: None,
            },
        ]
    }

    fn sample_triggers() -> Vec<TriggerDef> {
        vec![
            TriggerDef {
                trigger_type: "http".to_string(),
                function_id: "myworker::greet".to_string(),
                config: json!({
                    "api_path": "myworker/greet",
                    "http_method": "POST"
                }),
            },
            TriggerDef {
                trigger_type: "cron".to_string(),
                function_id: "myworker::compute".to_string(),
                config: json!({
                    "cron": "0 */10 * * * *"
                }),
            },
        ]
    }

    #[test]
    fn test_rust_template_generates_all_files() {
        let files = rust_worker_template("my-worker", &sample_functions(), &sample_triggers());
        let paths: Vec<&str> = files.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"build.rs"));
        assert!(paths.contains(&"config.yaml"));
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"src/config.rs"));
        assert!(paths.contains(&"src/manifest.rs"));
        assert!(paths.contains(&"src/functions/mod.rs"));
        assert!(paths.contains(&"src/functions/greet.rs"));
        assert!(paths.contains(&"src/functions/compute.rs"));
    }

    #[test]
    fn test_rust_template_cargo_toml_contains_iii_sdk() {
        let files = rust_worker_template("my-worker", &sample_functions(), &sample_triggers());
        let cargo = files.files.iter().find(|f| f.path == "Cargo.toml").unwrap();
        assert!(cargo.content.contains("iii-sdk"));
        assert!(cargo.content.contains("my-worker"));
    }

    #[test]
    fn test_rust_template_main_has_register_worker() {
        let files = rust_worker_template("my-worker", &sample_functions(), &sample_triggers());
        let main = files
            .files
            .iter()
            .find(|f| f.path == "src/main.rs")
            .unwrap();
        assert!(main.content.contains("register_worker"));
        assert!(main.content.contains("register_function_with"));
        assert!(main.content.contains("register_trigger"));
        assert!(main.content.contains("shutdown_async"));
        assert!(main.content.contains("ctrl_c"));
    }

    #[test]
    fn test_rust_template_handlers_have_pin_box_pattern() {
        let files = rust_worker_template("my-worker", &sample_functions(), &sample_triggers());
        let greet = files
            .files
            .iter()
            .find(|f| f.path == "src/functions/greet.rs")
            .unwrap();
        assert!(greet
            .content
            .contains("Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>"));
        assert!(greet.content.contains("build_handler"));
        assert!(greet.content.contains("Arc<III>"));
    }

    #[test]
    fn test_typescript_template_generates_all_files() {
        let files =
            typescript_worker_template("my-worker", &sample_functions(), &sample_triggers());
        let paths: Vec<&str> = files.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"package.json"));
        assert!(paths.contains(&"tsconfig.json"));
        assert!(paths.contains(&"src/index.ts"));
        assert!(paths.contains(&"src/functions/greet.ts"));
        assert!(paths.contains(&"src/functions/compute.ts"));
    }

    #[test]
    fn test_typescript_template_index_has_register_worker() {
        let files =
            typescript_worker_template("my-worker", &sample_functions(), &sample_triggers());
        let index = files
            .files
            .iter()
            .find(|f| f.path == "src/index.ts")
            .unwrap();
        assert!(index.content.contains("registerWorker"));
        assert!(index.content.contains("registerFunction"));
        assert!(index.content.contains("registerTrigger"));
        assert!(index.content.contains("shutdown"));
    }

    #[test]
    fn test_python_template_generates_all_files() {
        let files = python_worker_template("my-worker", &sample_functions(), &sample_triggers());
        let paths: Vec<&str> = files.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"pyproject.toml"));
        assert!(paths.contains(&"src/worker.py"));
        assert!(paths.contains(&"src/__init__.py"));
        assert!(paths.contains(&"src/functions/__init__.py"));
        assert!(paths.contains(&"src/functions/greet.py"));
        assert!(paths.contains(&"src/functions/compute.py"));
    }

    #[test]
    fn test_python_template_worker_has_register() {
        let files = python_worker_template("my-worker", &sample_functions(), &sample_triggers());
        let worker = files
            .files
            .iter()
            .find(|f| f.path == "src/worker.py")
            .unwrap();
        assert!(worker.content.contains("register_worker"));
        assert!(worker.content.contains("register_function"));
        assert!(worker.content.contains("register_trigger"));
        assert!(worker.content.contains("shutdown"));
    }

    #[test]
    fn test_generate_single_function_rust() {
        let func = FunctionDef {
            id: "myworker::hello".to_string(),
            description: "Say hello".to_string(),
            request_format: None,
            response_format: None,
        };
        let file = generate_single_function_rust(&func);
        assert_eq!(file.path, "src/functions/hello.rs");
        assert_eq!(file.language, "rust");
        assert!(file.content.contains("build_handler"));
        assert!(file.content.contains("myworker::hello"));
    }

    #[test]
    fn test_generate_single_function_typescript() {
        let func = FunctionDef {
            id: "myworker::hello".to_string(),
            description: "Say hello".to_string(),
            request_format: None,
            response_format: None,
        };
        let file = generate_single_function_typescript(&func);
        assert_eq!(file.path, "src/functions/hello.ts");
        assert_eq!(file.language, "typescript");
        assert!(file.content.contains("myworker::hello"));
    }

    #[test]
    fn test_generate_single_function_python() {
        let func = FunctionDef {
            id: "myworker::hello".to_string(),
            description: "Say hello".to_string(),
            request_format: None,
            response_format: None,
        };
        let file = generate_single_function_python(&func);
        assert_eq!(file.path, "src/functions/hello.py");
        assert_eq!(file.language, "python");
        assert!(file.content.contains("myworker::hello"));
    }

    #[test]
    fn test_generate_trigger_code_rust() {
        let trigger = TriggerDef {
            trigger_type: "http".to_string(),
            function_id: "myworker::greet".to_string(),
            config: json!({ "api_path": "myworker/greet", "http_method": "POST" }),
        };
        let code = generate_trigger_code_rust(&trigger);
        assert!(code.contains("register_trigger"));
        assert!(code.contains("myworker::greet"));
        assert!(code.contains("http"));
    }

    #[test]
    fn test_generate_trigger_code_typescript() {
        let trigger = TriggerDef {
            trigger_type: "http".to_string(),
            function_id: "myworker::greet".to_string(),
            config: json!({ "api_path": "myworker/greet", "http_method": "POST" }),
        };
        let code = generate_trigger_code_typescript(&trigger);
        assert!(code.contains("registerTrigger"));
        assert!(code.contains("myworker::greet"));
    }

    #[test]
    fn test_generate_trigger_code_python() {
        let trigger = TriggerDef {
            trigger_type: "cron".to_string(),
            function_id: "myworker::compute".to_string(),
            config: json!({ "cron": "0 */5 * * * *" }),
        };
        let code = generate_trigger_code_python(&trigger);
        assert!(code.contains("register_trigger"));
        assert!(code.contains("myworker::compute"));
        assert!(code.contains("cron"));
    }

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("hello_world"), "HelloWorld");
        assert_eq!(to_pascal_case("my_worker"), "MyWorker");
        assert_eq!(to_pascal_case("single"), "Single");
    }

    #[test]
    fn test_empty_functions_produces_valid_template() {
        let files = rust_worker_template("empty-worker", &[], &[]);
        let main = files
            .files
            .iter()
            .find(|f| f.path == "src/main.rs")
            .unwrap();
        assert!(main.content.contains("register_worker"));
        assert!(main.content.contains("shutdown_async"));
    }
}
