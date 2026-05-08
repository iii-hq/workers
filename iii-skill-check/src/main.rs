use anyhow::Context;
use clap::{Parser, Subcommand};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "iii-skill-check", about = "Validate worker README/skill artifacts against project rules")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render docs/* + iii.worker.yaml.name into README.md, skill.md, skills/*.md.
    Render {
        worker: PathBuf,
        /// Write rendered files to disk; without this flag, renders to memory only.
        #[arg(long)]
        write: bool,
    },
    /// Run all configured layers against a worker.
    Verify {
        worker: PathBuf,
        /// Subset of layers to run: structure,vale,ai (comma-separated).
        #[arg(long, default_value = "structure,vale,ai")]
        layers: String,
    },
    /// Re-render and diff against checked-in artifacts; non-zero on drift.
    VerifyRendered { worker: PathBuf },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Render { worker, write } => run_render(&worker, write),
        Command::Verify { worker, layers } => run_verify(&worker, &layers),
        Command::VerifyRendered { worker } => run_verify_rendered(&worker),
    }
}

fn run_render(worker: &Path, write: bool) -> anyhow::Result<()> {
    let out = iii_skill_check::render::render_worker(worker)?;
    println!(
        "rendered {} (readme {} bytes, skill {} bytes, {} leaves)",
        worker.display(),
        out.readme.len(),
        out.skill.len(),
        out.leaves.len(),
    );
    if write {
        std::fs::write(worker.join("README.md"), &out.readme)?;
        std::fs::write(worker.join("skill.md"), &out.skill)?;
        std::fs::create_dir_all(worker.join("skills"))?;
        for (leaf, body) in &out.leaves {
            std::fs::write(worker.join("skills").join(format!("{leaf}.md")), body)?;
        }
        println!("wrote artifacts to {}", worker.display());
    }
    Ok(())
}

fn run_verify(worker: &Path, layers: &str) -> anyhow::Result<()> {
    let workers_root = worker
        .parent()
        .ok_or_else(|| anyhow::anyhow!("worker dir has no parent: {}", worker.display()))?;
    let config_path = workers_root.join(".skill-check.yaml");
    let config = iii_skill_check::config::load(&config_path)?;

    let layer_set: HashSet<&str> = layers.split(',').map(|s| s.trim()).collect();
    let artifacts = enumerate_rendered_artifacts(worker);

    let mut all_violations: Vec<iii_skill_check::structure::Violation> = Vec::new();
    let mut ai_failures: Vec<(PathBuf, String)> = Vec::new();

    if layer_set.contains("structure") {
        all_violations.extend(iii_skill_check::structure::check(worker)?);
    }

    if layer_set.contains("vale") {
        let vale_config = workers_root.join(".vale.ini");
        let refs: Vec<&Path> = artifacts.iter().map(|p| p.as_path()).collect();
        all_violations.extend(iii_skill_check::vale::run(&refs, &vale_config)?);
    }

    if layer_set.contains("ai") {
        let rules_dir = workers_root.join(&config.rules.path);
        let rules = load_project_rules(&rules_dir)?;
        let prompt_path = rules_dir.join("_skill-check-prompt.md");
        let system_prompt = std::fs::read_to_string(&prompt_path)
            .with_context(|| format!("reading {}", prompt_path.display()))?;

        for art in &artifacts {
            match iii_skill_check::ai::check_artifact(
                art,
                &rules,
                &system_prompt,
                &config.ai_check.model,
                &config.ai_check.api_key_env_var,
                config.ai_check.max_tokens,
            )? {
                Ok(()) => {}
                Err(body) => ai_failures.push((art.clone(), body)),
            }
        }
    }

    if !all_violations.is_empty() {
        for v in &all_violations {
            let line = v.line.map(|l| format!(":{l}")).unwrap_or_default();
            eprintln!("{}{} — {}", v.file, line, v.message);
        }
    }
    if !ai_failures.is_empty() {
        for (path, body) in &ai_failures {
            eprintln!("\n[AI] {}\n{}", path.display(), body);
        }
    }

    let total = all_violations.len() + ai_failures.len();
    if total > 0 {
        anyhow::bail!("{total} violation(s) across layers [{layers}]");
    }
    println!("verify clean across [{layers}] for {}", worker.display());
    Ok(())
}

fn run_verify_rendered(worker: &Path) -> anyhow::Result<()> {
    let outputs = iii_skill_check::render::render_worker(worker)?;
    let mut drift: Vec<String> = Vec::new();

    let readme_disk =
        std::fs::read_to_string(worker.join("README.md")).unwrap_or_default();
    if readme_disk != outputs.readme {
        drift.push("README.md is out of date".into());
    }
    let skill_disk =
        std::fs::read_to_string(worker.join("skill.md")).unwrap_or_default();
    if skill_disk != outputs.skill {
        drift.push("skill.md is out of date".into());
    }
    for (leaf, body) in &outputs.leaves {
        let path = worker.join("skills").join(format!("{leaf}.md"));
        let disk = std::fs::read_to_string(&path).unwrap_or_default();
        if &disk != body {
            drift.push(format!("skills/{leaf}.md is out of date"));
        }
    }

    if !drift.is_empty() {
        for d in &drift {
            eprintln!("{d}");
        }
        anyhow::bail!("rendered artifacts are out of date — run `iii-skill-check render --write`");
    }
    println!("rendered artifacts match {}", worker.display());
    Ok(())
}

fn enumerate_rendered_artifacts(worker: &Path) -> Vec<PathBuf> {
    let mut out = vec![worker.join("README.md"), worker.join("skill.md")];
    let skills = worker.join("skills");
    if let Ok(entries) = std::fs::read_dir(&skills) {
        let mut leaf_paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("md"))
            .collect();
        leaf_paths.sort();
        out.extend(leaf_paths);
    }
    out
}

fn load_project_rules(rules_dir: &Path) -> anyhow::Result<String> {
    let mut combined = String::new();
    let mut entries: Vec<_> = std::fs::read_dir(rules_dir)
        .with_context(|| format!("reading {}", rules_dir.display()))?
        .filter_map(|r| r.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.ends_with(".md")
            || name.starts_with('_')
            || name == "README.md"
            || name == "SOURCE.md"
        {
            continue;
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        combined.push_str(&format!("# {name}\n\n{body}\n\n"));
    }
    Ok(combined)
}
