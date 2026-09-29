//! The Node compiler, embedded in the binary and materialized under
//! `data/stories/compiler/` on boot. `npm install` runs there once (and
//! again whenever the embedded package.json changes) so the worker ships as
//! one binary with no checked-in node_modules.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;

const FILES: [(&str, &str); 5] = [
    ("package.json", include_str!("../compiler/package.json")),
    ("build.mjs", include_str!("../compiler/build.mjs")),
    ("runtime.js", include_str!("../compiler/runtime.js")),
    (
        "shims/storybook-test.js",
        include_str!("../compiler/shims/storybook-test.js"),
    ),
    (
        "shims/empty-preview.js",
        include_str!("../compiler/shims/empty-preview.js"),
    ),
];

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveredProject {
    pub name: String,
    pub path: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Module {
    pub path: String,
    pub hop: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BuiltFile {
    pub file: String,
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub modules: Vec<Module>,
    #[serde(default)]
    pub manifest: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BuildOutput {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub vite: Option<Value>,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub files: Vec<BuiltFile>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Compiler {
    pub dir: PathBuf,
    node: String,
    npm: String,
}

fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

impl Compiler {
    /// Write the embedded files (only when their content differs) and make
    /// sure `node_modules/vite` exists.
    pub async fn ensure(dir: PathBuf) -> Result<Self, String> {
        let compiler = Self {
            dir,
            node: env_or("III_STORIES_NODE", "node"),
            npm: env_or(
                "III_STORIES_NPM",
                if cfg!(windows) { "npm.cmd" } else { "npm" },
            ),
        };
        let mut package_changed = false;
        for (name, content) in FILES {
            let target = compiler.dir.join(name);
            let current = std::fs::read_to_string(&target).ok();
            if current.as_deref() != Some(content) {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("creating {}: {e}", parent.display()))?;
                }
                std::fs::write(&target, content)
                    .map_err(|e| format!("writing {}: {e}", target.display()))?;
                if name == "package.json" {
                    package_changed = true;
                }
            }
        }
        let installed = compiler
            .dir
            .join("node_modules")
            .join("vite")
            .join("package.json")
            .is_file()
            && compiler
                .dir
                .join("node_modules")
                .join("@happy-dom")
                .is_dir();
        if package_changed || !installed {
            tracing::info!(dir = %compiler.dir.display(), "installing the stories compiler dependencies");
            let output = Command::new(&compiler.npm)
                .args(["install", "--no-audit", "--no-fund", "--loglevel=error"])
                .current_dir(&compiler.dir)
                .stdin(Stdio::null())
                .output()
                .await
                .map_err(|e| {
                    format!(
                        "spawning {}: {e} (set III_STORIES_NPM to the npm binary)",
                        compiler.npm
                    )
                })?;
            if !output.status.success() {
                return Err(format!(
                    "npm install failed in {}: {}",
                    compiler.dir.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }
        Ok(compiler)
    }

    async fn run(&self, cwd: &Path, args: &[String]) -> Result<Value, String> {
        let output = Command::new(&self.node)
            .arg(self.dir.join("build.mjs"))
            .args(args)
            .current_dir(cwd)
            .env("NODE_ENV", "production")
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|e| {
                format!(
                    "spawning {}: {e} (set III_STORIES_NODE to the node binary)",
                    self.node
                )
            })?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stdout
            .lines()
            .rev()
            .find(|line| line.trim_start().starts_with('{'));
        match last.and_then(|line| serde_json::from_str::<Value>(line).ok()) {
            Some(value) => {
                if !output.status.success() {
                    let error = value
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("compiler failed");
                    return Err(format!("{error}\n{}", stderr.trim()));
                }
                Ok(value)
            }
            None => Err(format!(
                "compiler produced no JSON (exit {:?}): {}",
                output.status.code(),
                stderr
                    .trim()
                    .lines()
                    .rev()
                    .take(12)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
        }
    }

    pub async fn discover(
        &self,
        workspace: &Path,
        stories: &[String],
        ignore: &[String],
    ) -> Result<Vec<DiscoveredProject>, String> {
        let mut args = vec![
            "discover".to_string(),
            "--workspace".to_string(),
            workspace.to_string_lossy().into_owned(),
        ];
        for glob in stories {
            args.push("--stories".into());
            args.push(glob.clone());
        }
        for glob in ignore {
            args.push("--ignore".into());
            args.push(glob.clone());
        }
        let value = self.run(workspace, &args).await?;
        serde_json::from_value::<Vec<DiscoveredProject>>(
            value
                .get("projects")
                .cloned()
                .unwrap_or(Value::Array(vec![])),
        )
        .map_err(|e| format!("bad discover output: {e}"))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn build(
        &self,
        project: &Path,
        out: &Path,
        files: &[String],
        preview: Option<&str>,
        config: Option<&str>,
        stories: &[String],
        ignore: &[String],
    ) -> Result<BuildOutput, String> {
        let mut args = vec![
            "build".to_string(),
            "--project".to_string(),
            project.to_string_lossy().into_owned(),
            "--out".to_string(),
            out.to_string_lossy().into_owned(),
        ];
        for file in files {
            args.push("--files".into());
            args.push(file.clone());
        }
        if let Some(preview) = preview {
            args.push("--preview".into());
            args.push(preview.to_string());
        }
        if let Some(config) = config {
            args.push("--config".into());
            args.push(config.to_string());
        }
        for glob in stories {
            args.push("--stories".into());
            args.push(glob.clone());
        }
        for glob in ignore {
            args.push("--ignore".into());
            args.push(glob.clone());
        }
        let value = self.run(project, &args).await?;
        serde_json::from_value(value).map_err(|e| format!("bad build output: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_compiler_files_are_present() {
        for (name, content) in FILES {
            assert!(!content.trim().is_empty(), "{name} is empty");
        }
        assert!(FILES[1].1.contains("discover"));
        assert!(FILES[2].1.contains("export function boot"));
    }
}
