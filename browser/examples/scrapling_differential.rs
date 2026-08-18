use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let function = request["function"].as_str().ok_or("missing function")?;
        let payload = request.get("payload").ok_or("missing payload")?;
        let response = match browser::scrapling::dispatch_op(function, payload) {
            Ok(value) => json!({"ok": value}),
            Err(error) => json!({"err": error}),
        };
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}
