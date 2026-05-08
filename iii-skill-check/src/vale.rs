use anyhow::Context;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Deserialize)]
struct ValeAlert {
    #[serde(rename = "Check")]
    check: String,
    #[serde(rename = "Line")]
    line: usize,
    #[serde(rename = "Message")]
    message: String,
    #[serde(rename = "Severity")]
    severity: String,
}

/// Shell out to the local `vale` binary against a list of rendered artifacts using the
/// repo's `.vale.ini`. Each Vale alert maps to one Violation. Vale's exit code is
/// ignored — non-zero merely signals "alerts found at error severity," which is
/// exactly what we surface as Violations.
pub fn run(
    artifacts: &[&Path],
    vale_config: &Path,
) -> anyhow::Result<Vec<crate::structure::Violation>> {
    if artifacts.is_empty() {
        return Ok(Vec::new());
    }

    let mut cmd = Command::new("vale");
    cmd.arg("--config").arg(vale_config);
    cmd.arg("--output=JSON");
    for a in artifacts {
        cmd.arg(a);
    }
    let output = cmd
        .output()
        .with_context(|| "running vale (is it installed and on PATH?)")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        // Vale prints nothing to stdout when there's no config-reachable file.
        // Treat as zero alerts; the caller is responsible for ensuring the files
        // match a `.vale.ini` block.
        return Ok(Vec::new());
    }

    let alerts: HashMap<String, Vec<ValeAlert>> = serde_json::from_str(trimmed).with_context(
        || format!("parsing vale JSON output (stdout was: {stdout})"),
    )?;

    let mut violations = Vec::new();
    for (file, file_alerts) in alerts {
        for a in file_alerts {
            violations.push(crate::structure::Violation {
                file: file.clone(),
                line: Some(a.line),
                message: format!("[{}] {} ({})", a.check, a.message, a.severity),
            });
        }
    }
    // Stable order so test output and diffs read consistently.
    violations.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.message.cmp(&b.message))
    });
    Ok(violations)
}
