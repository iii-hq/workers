//! Bash-command arity dictionary, ported from opencode's
//! `packages/opencode/src/permission/arity.ts`.
//!
//! Maps a command prefix string (e.g. `"git"`, `"npm run"`, `"docker compose"`)
//! to the number of tokens that constitute its human-meaningful identity.
//! Used by [`crate::config::ShellConfig::allowlist_contains`] so operators
//! can write `"git checkout"` or `"npm run dev"` as multi-token allowlist
//! entries and have them match the right slice of an incoming argv.
//!
//! Rules (from the upstream prompt):
//!   1. Each entry is a command-prefix string → token count.
//!   2. Flags never count as tokens; only subcommands do.
//!   3. Longest matching prefix wins.
//!   4. Only include a longer prefix when its arity differs from what the
//!      shorter prefix already implies (e.g. `git` arity 2 implies
//!      `git checkout` arity 2, so `git checkout` is omitted; `git config`
//!      arity 3 IS included because it differs).

use std::collections::HashMap;
use std::sync::OnceLock;

fn arity_table() -> &'static HashMap<&'static str, usize> {
    static TABLE: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let entries: &[(&str, usize)] = &[
            ("cat", 1),
            ("cd", 1),
            ("chmod", 1),
            ("chown", 1),
            ("cp", 1),
            ("echo", 1),
            ("env", 1),
            ("export", 1),
            ("grep", 1),
            ("kill", 1),
            ("killall", 1),
            ("ln", 1),
            ("ls", 1),
            ("mkdir", 1),
            ("mv", 1),
            ("ps", 1),
            ("pwd", 1),
            ("rm", 1),
            ("rmdir", 1),
            ("sleep", 1),
            ("source", 1),
            ("tail", 1),
            ("touch", 1),
            ("unset", 1),
            ("which", 1),
            ("aws", 3),
            ("az", 3),
            ("bazel", 2),
            ("brew", 2),
            ("bun", 2),
            ("bun run", 3),
            ("bun x", 3),
            ("cargo", 2),
            ("cargo add", 3),
            ("cargo run", 3),
            ("cdk", 2),
            ("cf", 2),
            ("cmake", 2),
            ("composer", 2),
            ("consul", 2),
            ("consul kv", 3),
            ("crictl", 2),
            ("deno", 2),
            ("deno task", 3),
            ("doctl", 3),
            ("docker", 2),
            ("docker builder", 3),
            ("docker compose", 3),
            ("docker container", 3),
            ("docker image", 3),
            ("docker network", 3),
            ("docker volume", 3),
            ("eksctl", 2),
            ("eksctl create", 3),
            ("firebase", 2),
            ("flyctl", 2),
            ("gcloud", 3),
            ("gh", 3),
            ("git", 2),
            ("git config", 3),
            ("git remote", 3),
            ("git stash", 3),
            ("go", 2),
            ("gradle", 2),
            ("helm", 2),
            ("heroku", 2),
            ("hugo", 2),
            ("ip", 2),
            ("ip addr", 3),
            ("ip link", 3),
            ("ip netns", 3),
            ("ip route", 3),
            ("kind", 2),
            ("kind create", 3),
            ("kubectl", 2),
            ("kubectl kustomize", 3),
            ("kubectl rollout", 3),
            ("kustomize", 2),
            ("make", 2),
            ("mc", 2),
            ("mc admin", 3),
            ("minikube", 2),
            ("mongosh", 2),
            ("mysql", 2),
            ("mvn", 2),
            ("ng", 2),
            ("npm", 2),
            ("npm exec", 3),
            ("npm init", 3),
            ("npm run", 3),
            ("npm view", 3),
            ("nvm", 2),
            ("nx", 2),
            ("openssl", 2),
            ("openssl req", 3),
            ("openssl x509", 3),
            ("pip", 2),
            ("pipenv", 2),
            ("pnpm", 2),
            ("pnpm dlx", 3),
            ("pnpm exec", 3),
            ("pnpm run", 3),
            ("poetry", 2),
            ("podman", 2),
            ("podman container", 3),
            ("podman image", 3),
            ("psql", 2),
            ("pulumi", 2),
            ("pulumi stack", 3),
            ("pyenv", 2),
            ("python", 2),
            ("rake", 2),
            ("rbenv", 2),
            ("redis-cli", 2),
            ("rustup", 2),
            ("serverless", 2),
            ("sfdx", 3),
            ("skaffold", 2),
            ("sls", 2),
            ("sst", 2),
            ("swift", 2),
            ("systemctl", 2),
            ("terraform", 2),
            ("terraform workspace", 3),
            ("tmux", 2),
            ("turbo", 2),
            ("ufw", 2),
            ("vault", 2),
            ("vault auth", 3),
            ("vault kv", 3),
            ("vercel", 2),
            ("volta", 2),
            ("wp", 2),
            ("yarn", 2),
            ("yarn dlx", 3),
            ("yarn run", 3),
        ];
        entries.iter().copied().collect()
    })
}

/// Return the human-meaningful command-identity prefix of `argv`.
///
/// Walks lengths from longest to shortest looking for the joined prefix in
/// the [`arity_table`]. On a hit, returns `argv[..arity]` (clamped to the
/// argv length so we never panic on a too-short input that happens to match
/// a longer-arity prefix). On a miss, returns `argv[..1]` — the single
/// program-name token — or an empty slice if argv was empty.
///
/// Matches the semantics of opencode's `prefix()` exactly so the rule
/// surface is portable between the two implementations.
pub fn prefix(argv: &[String]) -> Vec<String> {
    let table = arity_table();
    for len in (1..=argv.len()).rev() {
        let candidate = argv[..len].join(" ");
        if let Some(&arity) = table.get(candidate.as_str()) {
            let take = arity.min(argv.len());
            return argv[..take].to_vec();
        }
    }
    if argv.is_empty() {
        Vec::new()
    } else {
        argv[..1].to_vec()
    }
}

/// Normalize argv[0] from a full path (e.g. `/usr/bin/ls`) to its basename
/// before arity matching. Preserves the rest of argv untouched. Used so
/// allowlist matching is path-agnostic (existing behavior of
/// [`crate::config::ShellConfig::allowlist_contains`]).
pub fn normalize_argv_head(argv: &[String]) -> Vec<String> {
    let mut out: Vec<String> = argv.to_vec();
    if let Some(first) = out.first_mut() {
        if let Some(base) = std::path::Path::new(first.as_str())
            .file_name()
            .and_then(|s| s.to_str())
        {
            *first = base.to_string();
        }
    }
    out
}

/// True iff the arity-aware prefix of `argv` matches `entry`. An entry can
/// be a single token (`"ls"`) or a multi-token prefix (`"git checkout"`,
/// `"npm run dev"`); the match is token-aligned (so `"git"` does not match
/// argv beginning with `git-lfs`).
pub fn prefix_matches(argv: &[String], entry: &str) -> bool {
    let normalized = normalize_argv_head(argv);
    let pfx = prefix(&normalized);
    if pfx.is_empty() {
        return false;
    }
    let joined = pfx.join(" ");
    if joined == entry {
        return true;
    }
    // Token-aligned prefix match: the entry covers a leading subset of the
    // prefix tokens. Compare token-by-token to avoid false positives on
    // substrings (e.g. entry "git" should match prefix "git checkout" but
    // not "git-lfs push" — already filtered by the basename step above).
    let entry_tokens: Vec<&str> = entry.split_whitespace().collect();
    if entry_tokens.is_empty() || entry_tokens.len() > pfx.len() {
        return false;
    }
    entry_tokens
        .iter()
        .zip(pfx.iter())
        .all(|(e, p)| *e == p.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_argv_returns_empty_prefix() {
        assert!(prefix(&[]).is_empty());
    }

    #[test]
    fn unknown_command_returns_first_token() {
        assert_eq!(prefix(&s(&["foobar", "--flag"])), s(&["foobar"]));
    }

    #[test]
    fn single_token_arity_one_command() {
        assert_eq!(prefix(&s(&["ls", "-la"])), s(&["ls"]));
        assert_eq!(prefix(&s(&["touch", "x.txt"])), s(&["touch"]));
    }

    #[test]
    fn git_subcommand_picked_up_via_arity_two() {
        assert_eq!(
            prefix(&s(&["git", "checkout", "main"])),
            s(&["git", "checkout"])
        );
        assert_eq!(prefix(&s(&["git", "commit", "-am", "x"])), s(&["git", "commit"]));
    }

    #[test]
    fn longer_arity_wins_when_present() {
        // "npm run" is arity 3 even though "npm" is arity 2.
        assert_eq!(
            prefix(&s(&["npm", "run", "dev", "--watch"])),
            s(&["npm", "run", "dev"])
        );
    }

    #[test]
    fn arity_clamps_when_argv_too_short() {
        // "git" wants arity 2 but argv only has 1 token. Don't panic; return
        // what we have.
        assert_eq!(prefix(&s(&["git"])), s(&["git"]));
    }

    #[test]
    fn docker_compose_multitoken_prefix() {
        assert_eq!(
            prefix(&s(&["docker", "compose", "up", "-d"])),
            s(&["docker", "compose", "up"])
        );
    }

    #[test]
    fn unknown_command_with_no_args() {
        assert_eq!(prefix(&s(&["netstat"])), s(&["netstat"]));
    }

    #[test]
    fn normalize_strips_path_from_head() {
        let argv = s(&["/usr/bin/ls", "-la"]);
        assert_eq!(normalize_argv_head(&argv), s(&["ls", "-la"]));
    }

    #[test]
    fn normalize_leaves_bare_command_alone() {
        let argv = s(&["ls", "-la"]);
        assert_eq!(normalize_argv_head(&argv), s(&["ls", "-la"]));
    }

    #[test]
    fn prefix_matches_single_token_entry() {
        assert!(prefix_matches(&s(&["ls", "-la"]), "ls"));
        assert!(!prefix_matches(&s(&["lsattr"]), "ls"));
    }

    #[test]
    fn prefix_matches_multi_token_entry() {
        assert!(prefix_matches(
            &s(&["git", "checkout", "main"]),
            "git checkout"
        ));
        assert!(!prefix_matches(
            &s(&["git", "rebase", "-i"]),
            "git checkout"
        ));
    }

    #[test]
    fn prefix_matches_shorter_entry_against_longer_prefix() {
        // entry "git" should match argv that resolves to ["git", "checkout"]
        assert!(prefix_matches(&s(&["git", "checkout", "main"]), "git"));
    }

    #[test]
    fn prefix_matches_handles_full_path_argv_head() {
        assert!(prefix_matches(&s(&["/usr/bin/ls", "-la"]), "ls"));
        assert!(prefix_matches(
            &s(&["/opt/homebrew/bin/git", "checkout", "main"]),
            "git checkout"
        ));
    }

    #[test]
    fn prefix_does_not_match_token_boundary_collision() {
        // basename normalization makes "git-lfs" survive as its own token,
        // so entry "git" cannot match argv ["git-lfs", "push"].
        assert!(!prefix_matches(&s(&["git-lfs", "push"]), "git"));
    }

    #[test]
    fn prefix_matches_returns_false_for_empty_argv() {
        assert!(!prefix_matches(&[], "ls"));
    }

    #[test]
    fn prefix_matches_returns_false_for_empty_entry() {
        assert!(!prefix_matches(&s(&["ls"]), ""));
    }
}
