// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0.
//! Explicit local administration, never an automatic startup/recovery hook.
use crate::worker_admission::{self, RecoveryAction, RecoveryRequest};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::io::{self, Write};

/// Inspect or explicitly provision/reconcile the persistent host admission catalog.
#[derive(Args, Debug)]
pub struct AdmissionArgs {
    #[command(subcommand)]
    command: AdmissionCommand,
}

#[derive(Subcommand, Debug)]
enum AdmissionCommand {
    /// Validate this effective user's installation without provisioning or repair.
    Check,
    /// Provision the fixed namespace for a numeric uid (requires effective uid 0).
    Bootstrap(ApplyArgs),
    /// Inspect a numeric uid's records without recovering them (requires uid 0).
    Inspect(UidArgs),
    /// Reconcile only proven postconditions; unresolved records remain fenced.
    Recover(RecordArgs),
    /// Preserve a completed record in the archive; never prune payload or evidence.
    Archive(RecordArgs),
}

#[derive(Args, Debug)]
struct UidArgs {
    #[arg(long, value_parser = parse_uid)]
    uid: u32,
}

#[derive(Args, Debug)]
struct ApplyArgs {
    #[command(flatten)]
    target: UidArgs,
    /// Explicitly request host mutation; this is not a dry run.
    #[arg(long, required = true, action = clap::ArgAction::SetTrue)]
    apply: bool,
}

#[derive(Args, Debug)]
struct RecordArgs {
    #[command(flatten)]
    target: ApplyArgs,
    /// Exact operation id returned by inspect, not a pathname or PID.
    #[arg(long, value_parser = parse_operation)]
    operation_id: String,
    /// Current record digest returned by inspect (CAS, not authorization).
    #[arg(long, value_parser = parse_digest)]
    expected_record_sha256: String,
}

fn parse_uid(value: &str) -> Result<u32, String> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("uid must be an unsigned decimal integer".into());
    }
    value.parse().map_err(|_| "uid exceeds u32".into())
}
fn parse_hex(value: &str, len: usize, label: &str) -> Result<String, String> {
    if value.len() != len
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!(
            "{label} must be {len} lowercase hexadecimal characters"
        ));
    }
    Ok(value.into())
}
fn parse_operation(value: &str) -> Result<String, String> {
    parse_hex(value, 32, "operation id")
}
fn parse_digest(value: &str) -> Result<String, String> {
    parse_hex(value, 64, "record digest")
}
fn require_apply(apply: bool) -> io::Result<()> {
    if !apply {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "explicit --apply is required",
        ));
    }
    Ok(())
}
fn recovery_request(
    args: RecordArgs,
    action: RecoveryAction,
) -> io::Result<(u32, RecoveryRequest)> {
    require_apply(args.target.apply)?;
    Ok((
        args.target.target.uid,
        RecoveryRequest {
            operation_id: args.operation_id,
            expected_record_sha256: args.expected_record_sha256,
            action,
        },
    ))
}
fn recovery_exit(report: &Value) -> i32 {
    // An Ok(report) can still say unresolved. Never expose it as successful recovery.
    if report.get("status").and_then(Value::as_str) == Some("completed") {
        0
    } else {
        3
    }
}

/// Return a process exit code after printing a structured report. No automatic retry.
pub fn run(args: AdmissionArgs) -> io::Result<i32> {
    if !cfg!(target_os = "linux") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "persistent runtime admission currently requires Linux; no host fallback",
        ));
    }
    let (report, exit) = match args.command {
        AdmissionCommand::Check => {
            worker_admission::check_installation()?;
            // SAFETY: geteuid has no preconditions and does not mutate the host.
            let uid = unsafe { libc::geteuid() };
            (
                json!({"status": "installation-checked", "uid": uid,
                "runtime_validated": false, "guest_consistent": false}),
                0,
            )
        }
        AdmissionCommand::Bootstrap(args) => {
            require_apply(args.apply)?;
            worker_admission::bootstrap_for_uid(args.target.uid)?;
            (
                json!({"status": "provisioned", "uid": args.target.uid,
                "runtime_validated": false, "guest_consistent": false}),
                0,
            )
        }
        AdmissionCommand::Inspect(args) => {
            let report = worker_admission::inspect_recovery_for_uid(args.uid)?;
            (serde_json::to_value(report).map_err(io::Error::other)?, 0)
        }
        AdmissionCommand::Recover(args) => {
            let (uid, request) = recovery_request(args, RecoveryAction::ReconcileVerified)?;
            let report = worker_admission::recover_for_uid(uid, request)?;
            let report = serde_json::to_value(report).map_err(io::Error::other)?;
            let exit = recovery_exit(&report);
            (report, exit)
        }
        AdmissionCommand::Archive(args) => {
            let (uid, request) = recovery_request(args, RecoveryAction::ArchiveCompleted)?;
            let report = worker_admission::recover_for_uid(uid, request)?;
            let report = serde_json::to_value(report).map_err(io::Error::other)?;
            let exit = recovery_exit(&report);
            (report, exit)
        }
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report).map_err(io::Error::other)?;
    writeln!(stdout)?;
    stdout.flush()?;
    Ok(exit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Commands};
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("sandbox-libkrun").chain(args.iter().copied()))
    }
    #[test]
    fn no_subcommand_still_selects_daemon_without_bootstrap() {
        assert!(parse(&[]).unwrap().command.is_none());
    }
    #[test]
    fn check_has_no_target_override_or_apply() {
        assert!(matches!(
            parse(&["admission", "check"]).unwrap().command,
            Some(Commands::Admission(AdmissionArgs {
                command: AdmissionCommand::Check
            }))
        ));
        assert!(parse(&["admission", "check", "--uid", "1000"]).is_err());
        assert!(parse(&["admission", "check", "--apply"]).is_err());
    }
    #[test]
    fn bootstrap_requires_numeric_uid_and_explicit_apply() {
        assert!(parse(&["admission", "bootstrap", "--uid", "1000", "--apply"]).is_ok());
        for args in [
            vec!["admission", "bootstrap", "--uid", "1000"],
            vec!["admission", "bootstrap", "--apply"],
            vec!["admission", "bootstrap", "--uid", "root", "--apply"],
        ] {
            assert!(parse(&args).is_err());
        }
        assert!(require_apply(false).is_err());
    }
    #[test]
    fn uid_parser_rejects_signs_spaces_and_overflow() {
        assert_eq!(parse_uid("0").unwrap(), 0);
        for bad in ["", "-1", "+1", " 1", "4294967296"] {
            assert!(parse_uid(bad).is_err());
        }
    }
    #[test]
    fn inspect_is_explicit_and_has_no_mutation_switch() {
        assert!(parse(&["admission", "inspect", "--uid", "1000"]).is_ok());
        assert!(parse(&["admission", "inspect", "--uid", "1000", "--apply"]).is_err());
    }
    #[test]
    fn record_commands_require_all_cas_fields_and_apply() {
        let id = "a".repeat(32);
        let digest = "b".repeat(64);
        for command in ["recover", "archive"] {
            let args = [
                "admission",
                command,
                "--uid",
                "1000",
                "--operation-id",
                &id,
                "--expected-record-sha256",
                &digest,
                "--apply",
            ];
            assert!(parse(&args).is_ok());
            assert!(parse(&args[..8]).is_err());
            assert!(parse(&["admission", command, "--uid", "1000", "--apply"]).is_err());
        }
    }
    #[test]
    fn operation_and_digest_are_not_paths_or_arbitrary_strings() {
        for bad in ["../record", "1", ""] {
            assert!(parse_operation(bad).is_err());
        }
        assert!(parse_operation(&"A".repeat(32)).is_err());
        assert!(parse_digest(&"z".repeat(64)).is_err());
        assert!(parse_digest(&"a".repeat(63)).is_err());
    }
    #[test]
    fn unresolved_and_unknown_reports_are_not_successful_recovery() {
        assert_eq!(recovery_exit(&json!({"status":"completed"})), 0);
        for report in [
            json!({"status":"unresolved: runtime writer requires real owned wait"}),
            json!({"status":"inspected"}),
            json!({}),
        ] {
            assert_eq!(recovery_exit(&report), 3);
        }
    }
    #[test]
    fn no_root_override_or_force_recovery_is_exposed() {
        for flag in ["--root", "--namespace", "--force"] {
            assert!(parse(&["admission", "inspect", "--uid", "1000", flag, "/tmp"]).is_err());
        }
    }
}
