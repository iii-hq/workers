use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use super::*;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) async fn commit(
    target: &MaterializedTargetV1,
    message: &str,
) -> Result<String, SecurityScanError> {
    run_git(
        target,
        &["-c", "core.hooksPath=/dev/null", "add", "--all"],
        None,
    )
    .await?;
    run_git(
        target,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgSign=false",
            "commit",
            "-m",
        ],
        Some(message),
    )
    .await?;
    let sha = run_git(target, &["rev-parse", "HEAD"], None).await?;
    if sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SecurityScanError::Dependency(
            "Git returned an invalid commit SHA".into(),
        ));
    }
    Ok(sha)
}

pub(super) async fn push(target: &MaterializedTargetV1) -> Result<String, SecurityScanError> {
    run_git(
        target,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "push",
            "--set-upstream",
            "origin",
            "HEAD",
        ],
        None,
    )
    .await?;
    let branch = run_git(target, &["branch", "--show-current"], None).await?;
    if branch.is_empty() {
        return Err(SecurityScanError::Dependency(
            "Git returned no current branch after push".into(),
        ));
    }
    Ok(branch)
}

async fn run_git(
    target: &MaterializedTargetV1,
    args: &[&str],
    trailing_arg: Option<&str>,
) -> Result<String, SecurityScanError> {
    let mut command = git_command(target, args, trailing_arg);
    let output = tokio::time::timeout(GIT_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            SecurityScanError::Dependency("checkout-bound Git operation timed out".into())
        })?
        .map_err(|error| {
            SecurityScanError::Dependency(format!(
                "could not start checkout-bound Git operation: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(SecurityScanError::Dependency(
            "checkout-bound Git operation failed".into(),
        ));
    }
    String::from_utf8(output.stdout)
        .map(|stdout| stdout.trim().to_string())
        .map_err(|_| SecurityScanError::Dependency("Git returned non-UTF-8 output".into()))
}

fn git_command(
    target: &MaterializedTargetV1,
    args: &[&str],
    trailing_arg: Option<&str>,
) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(&target.path)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_CEILING_DIRECTORIES")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(argument) = trailing_arg {
        command.arg(argument);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_message_is_one_data_argument() {
        let target = MaterializedTargetV1 {
            worktree_id: "wt_test".into(),
            path: "/private/action-worktree".into(),
            base_sha: "0".repeat(40),
        };
        let command = git_command(
            &target,
            &["-c", "commit.gpgSign=false", "commit", "-m"],
            Some("fix finding; touch /tmp/injected"),
        );
        let command = command.as_std();
        assert_eq!(command.get_program(), "git");
        assert_eq!(
            command
                .get_args()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            [
                "-c",
                "commit.gpgSign=false",
                "commit",
                "-m",
                "fix finding; touch /tmp/injected",
            ]
        );
        assert_eq!(
            command.get_current_dir(),
            Some(std::path::Path::new("/private/action-worktree"))
        );
    }
}
