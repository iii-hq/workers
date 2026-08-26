use iii_directory::fs_source::{scan_agents, scan_skills, scan_system_prompts};
use iii_directory::sources::registry::{download, VersionSpec};

#[tokio::main]
async fn main() -> Result<(), String> {
    let registry =
        std::env::var("REGISTRY").unwrap_or_else(|_| "http://localhost:3111".to_string());
    let worker = std::env::var("WORKER").unwrap_or_else(|_| "hello-worker".to_string());
    let tag = std::env::var("TAG").unwrap_or_else(|_| "latest".to_string());

    let tmp = tempfile::tempdir().unwrap();
    let skills_folder = tmp.path().join("skills");
    let agents_folder = tmp.path().join("agents");

    println!("→ Downloading {worker} (tag={tag}) from {registry}");
    println!("  skills_folder = {}", skills_folder.display());
    println!("  agents_folder = {}", agents_folder.display());

    let spec = VersionSpec::Tag(tag);
    let result = download(
        &registry,
        &worker,
        &spec,
        &skills_folder,
        &agents_folder,
        30_000,
    )
    .await?;

    println!("\n[download result]");
    println!("  namespace        = {}", result.namespace);
    println!("  skills_written   = {:?}", result.skills_written);
    println!(
        "  system_prompts_written = {:?}",
        result.system_prompts_written
    );
    println!("  agents_written   = {:?}", result.agents_written);

    let (skills, skill_skipped) = scan_skills(&skills_folder);
    let (system_prompts, prompt_skipped) = scan_system_prompts(&skills_folder);
    let (agents, agent_skipped) = scan_agents(&agents_folder);

    println!("\n[scan_skills]");
    for s in &skills {
        println!("  id={:<40} path={}", s.id, s.abs_path.display());
    }
    if !skill_skipped.is_empty() {
        println!("\n[skill skips]");
        for s in &skill_skipped {
            println!("  {} → {}", s.path.display(), s.reason);
        }
    }

    println!("\n[scan_system_prompts]");
    for p in &system_prompts {
        println!("  name={:<30} path={}", p.name, p.abs_path.display());
    }
    if !prompt_skipped.is_empty() {
        println!("\n[prompt skips]");
        for s in &prompt_skipped {
            println!("  {} → {}", s.path.display(), s.reason);
        }
    }

    println!("\n[scan_agents]");
    for agent in &agents {
        println!("  id={:<30} path={}", agent.name, agent.abs_path.display());
    }
    if !agent_skipped.is_empty() {
        println!("\n[agent skips]");
        for skipped in &agent_skipped {
            println!("  {} → {}", skipped.path.display(), skipped.reason);
        }
    }

    println!(
        "\nDONE — skills scanned={}, system prompts scanned={}, agents scanned={}",
        skills.len(),
        system_prompts.len(),
        agents.len()
    );

    // Soft assertions so the example doubles as a smoke test.
    let any_index = skills.iter().any(|s| s.id.ends_with("/index"));
    if !any_index {
        eprintln!("FAIL: no skill with id ending in /index was scanned");
        std::process::exit(1);
    }
    for s in &skills {
        if s.id.chars().any(|c| c.is_ascii_uppercase()) {
            eprintln!("FAIL: skill id leaked uppercase: {}", s.id);
            std::process::exit(1);
        }
    }
    println!("smoke: at least one /index id present, no uppercase leaks");
    Ok(())
}
