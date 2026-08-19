use std::io::{self, BufRead, Write};
use std::sync::Arc;

use browser::config::{SecurityMode, WorkerConfig};
use browser::scrapling::net::{self, Ctx};
use browser::scrapling::sessions::Registry;
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = WorkerConfig::default();
    config.scrapling.security_mode = SecurityMode::Compat;
    config.scrapling.allow_loopback = true;
    let ctx = Ctx {
        http: Registry::new(8, 900),
        config: config.into_shared(),
        iii: Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:0")),
    };
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let function = request["function"].as_str().ok_or("missing function")?;
        let payload = request.get("payload").ok_or("missing payload")?;
        let response = match net::dispatch(&ctx, function, payload).await {
            Ok(value) => json!({"ok": value}),
            Err(error) => json!({"err": error}),
        };
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}
