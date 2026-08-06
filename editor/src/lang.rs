//! Path to language id.
//!
//! The ids are Monaco's, because the console page feeds them straight to the
//! shared `CodeEditor`. An unknown id renders as plain text there, so the
//! table only needs to cover what it can name — never guess, and never invent
//! an id Monaco has not heard of.

/// Language id for `path`, or `"plaintext"` when nothing matches.
///
/// Whole-filename matches win over extensions: `Dockerfile` and `Makefile`
/// carry no extension, and `.env.local` would otherwise resolve on `local`.
pub fn for_path(path: &str) -> &'static str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);

    match name {
        "Dockerfile" | "Containerfile" => return "dockerfile",
        "Makefile" | "makefile" | "GNUmakefile" => return "makefile",
        "CMakeLists.txt" => return "cmake",
        "go.mod" | "go.sum" => return "go",
        "Cargo.lock" => return "toml",
        _ => {}
    }
    if name.starts_with(".env") {
        return "shell";
    }
    if name.starts_with(".gitignore") || name.starts_with(".dockerignore") {
        return "plaintext";
    }

    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescript",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascript",
        "py" | "pyi" => "python",
        "go" => "go",
        "rb" => "ruby",
        "php" => "php",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "scala" | "sc" => "scala",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "dart" => "dart",
        "ex" | "exs" => "elixir",
        "lua" => "lua",
        "sh" | "bash" | "zsh" => "shell",
        "sql" => "sql",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "xml" | "svg" => "xml",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" | "sass" => "scss",
        "less" => "less",
        "md" | "markdown" => "markdown",
        "vue" => "html",
        "graphql" | "gql" => "graphql",
        "ini" | "cfg" | "conf" => "ini",
        "diff" | "patch" => "diff",
        _ => "plaintext",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_lookup() {
        assert_eq!(for_path("src/main.rs"), "rust");
        assert_eq!(for_path("ui/page.tsx"), "typescript");
        assert_eq!(for_path("a/b/c.YAML"), "yaml");
    }

    #[test]
    fn whole_filename_beats_extension() {
        assert_eq!(for_path("docker/Dockerfile"), "dockerfile");
        assert_eq!(for_path("Makefile"), "makefile");
    }

    #[test]
    fn dotfiles_do_not_resolve_on_their_suffix() {
        assert_eq!(for_path(".env.local"), "shell");
        assert_eq!(for_path(".gitignore"), "plaintext");
    }

    /// The whole extensionless table, and each entry under a folder — the
    /// filename lookup runs on the last segment, not on the path.
    #[test]
    fn every_whole_filename_resolves_bare_and_nested() {
        for (name, expected) in [
            ("Dockerfile", "dockerfile"),
            ("Containerfile", "dockerfile"),
            ("Makefile", "makefile"),
            ("makefile", "makefile"),
            ("GNUmakefile", "makefile"),
            ("CMakeLists.txt", "cmake"),
            ("go.mod", "go"),
            ("go.sum", "go"),
            ("Cargo.lock", "toml"),
        ] {
            assert_eq!(for_path(name), expected, "{name}");
            assert_eq!(
                for_path(&format!("nested/dir/{name}")),
                expected,
                "{name} under a folder"
            );
        }
    }

    /// A backslash separates path segments too, so a Windows-shaped path has
    /// to resolve on its filename rather than on the whole string.
    #[test]
    fn a_backslash_separated_path_resolves_on_its_filename() {
        assert_eq!(for_path(r"src\main.rs"), "rust");
        assert_eq!(for_path(r"docker\Dockerfile"), "dockerfile");
    }

    /// The dotfile prefixes match on a prefix, so their suffixed variants have
    /// to land in the same place rather than falling through to an extension.
    #[test]
    fn suffixed_dotfiles_follow_their_prefix() {
        assert_eq!(for_path(".envrc"), "shell");
        assert_eq!(for_path(".env.production"), "shell");
        assert_eq!(for_path(".gitignore.local"), "plaintext");
        assert_eq!(for_path(".dockerignore"), "plaintext");
    }

    #[test]
    fn unknown_falls_back_to_plaintext() {
        assert_eq!(for_path("data.qqq"), "plaintext");
        assert_eq!(for_path("noextension"), "plaintext");
    }
}
