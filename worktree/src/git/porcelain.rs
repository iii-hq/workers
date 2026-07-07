//! Pure parsers for the porcelain formats this worker consumes:
//! `git status --porcelain=v2 --branch` and `git worktree list --porcelain`.

/// Parsed `git status --porcelain=v2 --branch`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusV2 {
    pub staged: u64,
    pub unstaged: u64,
    pub untracked: u64,
    pub conflicted: u64,
    /// From `# branch.ab +A -B`; zero when no upstream is set.
    pub ahead: u64,
    pub behind: u64,
    pub has_upstream: bool,
    /// HEAD commit from `# branch.oid`; `None` on an unborn branch.
    pub oid: Option<String>,
}

impl StatusV2 {
    pub fn clean(&self) -> bool {
        self.staged == 0 && self.unstaged == 0 && self.untracked == 0 && self.conflicted == 0
    }
}

pub fn parse_status_v2(input: &str) -> StatusV2 {
    let mut st = StatusV2::default();
    for line in input.lines() {
        if let Some(oid) = line.strip_prefix("# branch.oid ") {
            if oid != "(initial)" {
                st.oid = Some(oid.to_string());
            }
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            st.has_upstream = true;
            for token in ab.split_whitespace() {
                if let Some(n) = token.strip_prefix('+') {
                    st.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = token.strip_prefix('-') {
                    st.behind = n.parse().unwrap_or(0);
                }
            }
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            let xy = line.split_whitespace().nth(1).unwrap_or("..");
            let mut chars = xy.chars();
            let x = chars.next().unwrap_or('.');
            let y = chars.next().unwrap_or('.');
            if x != '.' {
                st.staged += 1;
            }
            if y != '.' {
                st.unstaged += 1;
            }
        } else if line.starts_with("u ") {
            st.conflicted += 1;
        } else if line.starts_with("? ") {
            st.untracked += 1;
        }
    }
    st
}

/// One entry from `git worktree list --porcelain`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorktreeListEntry {
    pub path: String,
    pub head: Option<String>,
    /// Full ref, e.g. `refs/heads/main`. `None` when detached or bare.
    pub branch: Option<String>,
    pub locked: bool,
    pub lock_reason: Option<String>,
    pub bare: bool,
    pub detached: bool,
}

pub fn parse_worktree_list(input: &str) -> Vec<WorktreeListEntry> {
    let mut entries = Vec::new();
    let mut current: Option<WorktreeListEntry> = None;
    for line in input.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            current = Some(WorktreeListEntry {
                path: path.to_string(),
                ..Default::default()
            });
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        if let Some(head) = line.strip_prefix("HEAD ") {
            entry.head = Some(head.to_string());
        } else if let Some(branch) = line.strip_prefix("branch ") {
            entry.branch = Some(branch.to_string());
        } else if line == "locked" {
            entry.locked = true;
        } else if let Some(reason) = line.strip_prefix("locked ") {
            entry.locked = true;
            entry.lock_reason = Some(reason.to_string());
        } else if line == "bare" {
            entry.bare = true;
        } else if line == "detached" {
            entry.detached = true;
        }
    }
    if let Some(entry) = current.take() {
        entries.push(entry);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_counts_each_category() {
        let input = "\
# branch.oid abc
# branch.head main
# branch.upstream origin/main
# branch.ab +2 -1
1 M. N... 100644 100644 100644 a a staged.txt
1 .M N... 100644 100644 100644 a a unstaged.txt
1 MM N... 100644 100644 100644 a a both.txt
2 R. N... 100644 100644 100644 a a R100 new.txt\told.txt
u UU N... 100644 100644 100644 100644 a a a conflict.txt
? untracked.txt
";
        let st = parse_status_v2(input);
        assert_eq!(st.staged, 3);
        assert_eq!(st.unstaged, 2);
        assert_eq!(st.untracked, 1);
        assert_eq!(st.conflicted, 1);
        assert_eq!(st.ahead, 2);
        assert_eq!(st.behind, 1);
        assert!(st.has_upstream);
        assert_eq!(st.oid.as_deref(), Some("abc"));
        assert!(!st.clean());
    }

    #[test]
    fn status_clean_without_upstream() {
        let st = parse_status_v2("# branch.oid abc\n# branch.head main\n");
        assert!(st.clean());
        assert!(!st.has_upstream);
        assert_eq!(st.ahead, 0);
        assert_eq!(st.oid.as_deref(), Some("abc"));
        assert_eq!(parse_status_v2("# branch.oid (initial)\n").oid, None);
    }

    #[test]
    fn worktree_list_parses_lock_reason_and_flags() {
        let input = "\
worktree /repo
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree /root/proj-abc/wt_12345678
HEAD 2222222222222222222222222222222222222222
branch refs/heads/iii/wt_12345678
locked iii:worktree wt_12345678

worktree /elsewhere/detached
HEAD 3333333333333333333333333333333333333333
detached
";
        let entries = parse_worktree_list(input);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, "/repo");
        assert_eq!(entries[0].branch.as_deref(), Some("refs/heads/main"));
        assert!(!entries[0].locked);
        assert!(entries[1].locked);
        assert_eq!(
            entries[1].lock_reason.as_deref(),
            Some("iii:worktree wt_12345678")
        );
        assert!(entries[2].detached);
        assert!(entries[2].branch.is_none());
    }
}
