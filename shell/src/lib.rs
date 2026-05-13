//! Library facade exposing the binary's modules so integration tests
//! under `tests/` can drive them at the public-API level. Both targets
//! share source files via Cargo's two-target compile.

pub mod config;
pub mod exec;
pub mod exec_dispatch;
pub mod fs;
pub mod functions;
pub mod jobs;
pub mod manifest;
pub mod target;
pub mod triggers;

/// Top-level skill id. Bundled markdown is published to the
/// iii-directory worker; clients fetch the body at `iii://shell`.
pub const SKILL_ID: &str = "shell";

/// Top-level router body — small on purpose so the LLM only loads the
/// router and drills into sub-skills via
/// `directory::skills::get`.
pub const SKILL_MD: &str = include_str!("../skill.md");

/// Sub-skill bodies, keyed by their full id. One leaf per registered
/// `shell::*` function. The router (`SKILL_MD`) links to each so the
/// agent only loads what it needs via
/// `directory::skills::get`.
pub const SUB_SKILLS: &[(&str, &str)] = &[
    ("shell/exec", include_str!("../skills/exec.md")),
    ("shell/exec_bg", include_str!("../skills/exec_bg.md")),
    ("shell/status", include_str!("../skills/status.md")),
    ("shell/kill", include_str!("../skills/kill.md")),
    ("shell/list", include_str!("../skills/list.md")),
    // The renderer flattens leaf files under skills/. The path-style ids
    // ("shell/fs/ls", …) preserve the iii:// skill namespace served by
    // the iii-directory worker; the file layout is separate.
    ("shell/fs/ls", include_str!("../skills/ls.md")),
    ("shell/fs/stat", include_str!("../skills/stat.md")),
    ("shell/fs/read", include_str!("../skills/read.md")),
    ("shell/fs/grep", include_str!("../skills/grep.md")),
    ("shell/fs/write", include_str!("../skills/write.md")),
    ("shell/fs/sed", include_str!("../skills/sed.md")),
    ("shell/fs/mkdir", include_str!("../skills/mkdir.md")),
    ("shell/fs/rm", include_str!("../skills/rm.md")),
    ("shell/fs/chmod", include_str!("../skills/chmod.md")),
    ("shell/fs/mv", include_str!("../skills/mv.md")),
];
