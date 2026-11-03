//! The README's shell examples use the `iii call` form of the 0.26 CLI:
//! `iii call <function> [key=value ...] [--json '<object>'] [--timeout-ms <ms>]`.

const README: &str = include_str!("../README.md");

#[test]
fn readme_iii_call_examples_name_the_function_first() {
    let calls: Vec<&str> = README
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("iii call "))
        .collect();
    assert!(!calls.is_empty(), "the README shows `iii call` examples");
    for line in calls {
        let function = line["iii call ".len()..]
            .split_whitespace()
            .next()
            .unwrap_or_default();
        assert!(
            function.contains("::") && !function.starts_with('-'),
            "`{line}`: `iii call` takes the function path as its first argument"
        );
    }
}

#[test]
fn readme_passes_payloads_with_the_flags_iii_call_takes() {
    for flag in ["--function-id", "--payload"] {
        assert!(
            !README.contains(flag),
            "the README passes `{flag}`; `iii call` takes `--json` and key=value pairs"
        );
    }
    assert!(README.contains("--json '"));
}
