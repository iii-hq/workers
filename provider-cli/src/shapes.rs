//! CLI shape table. Direct port of roster/workers/provider-cli/src/worker.ts.

#[derive(Debug, Clone, Copy)]
pub struct CliShape {
    pub tag: &'static str,
    pub bin: &'static str,
    pub args: fn(&str) -> Vec<String>,
}

fn args_print_dashdash(p: &str) -> Vec<String> {
    vec!["--print".into(), p.into()]
}
fn args_exec(p: &str) -> Vec<String> {
    vec!["exec".into(), p.into()]
}
fn args_run(p: &str) -> Vec<String> {
    vec!["run".into(), p.into()]
}
fn args_chat(p: &str) -> Vec<String> {
    vec!["chat".into(), p.into()]
}
fn args_prompt_dashdash(p: &str) -> Vec<String> {
    vec!["--prompt".into(), p.into()]
}

pub static CLI_SHAPES: &[CliShape] = &[
    CliShape {
        tag: "claude-cli",
        bin: "claude",
        args: args_print_dashdash,
    },
    CliShape {
        tag: "codex-cli",
        bin: "codex",
        args: args_exec,
    },
    CliShape {
        tag: "opencode-cli",
        bin: "opencode",
        args: args_run,
    },
    CliShape {
        tag: "openclaw-cli",
        bin: "openclaw",
        args: args_run,
    },
    CliShape {
        tag: "hermes-cli",
        bin: "hermes",
        args: args_chat,
    },
    CliShape {
        tag: "pi-cli",
        bin: "pi",
        args: args_chat,
    },
    CliShape {
        tag: "gemini-cli",
        bin: "gemini",
        args: args_prompt_dashdash,
    },
    CliShape {
        tag: "cursor-agent-cli",
        bin: "cursor-agent",
        args: args_print_dashdash,
    },
];

pub fn lookup_by_model(model: &str) -> Option<CliShape> {
    let prefix = model.split('/').next()?;
    CLI_SHAPES.iter().find(|s| s.tag == prefix).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_resolves_known_prefix() {
        let s = lookup_by_model("claude-cli/opus").unwrap();
        assert_eq!(s.bin, "claude");
        assert_eq!((s.args)("hi"), vec!["--print", "hi"]);
    }

    #[test]
    fn lookup_rejects_unknown_prefix() {
        assert!(lookup_by_model("random-cli/x").is_none());
    }
}
