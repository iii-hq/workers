use iii_directory::sources::registry::{download, VersionSpec};
use iii_directory::fs_source::{scan_skills, scan_prompts};

#[tokio::main]
async fn main() -> Result<(), String> {
    let registry = std::env::var("REGISTRY")
        .unwrap_or_else(|_| "http://localhost:3111".to_string());
    let worker = std::env::var("WORKER").unwrap_or_else(|_| "hello-worker".to_string());
    let tag = std::env::var("TAG").unwrap_or_else(|_| "latest".to_string());

    let tmp = tempfile::tempdir().unwrap();
    let skills_folder = tmp.path();

    println!("→ Downloading {worker} (tag={tag}) from {registry}");
    println!("  skills_folder = {}", skills_folder.display());

    let spec = VersionSpec::Tag(tag);
    let result = download(&registry, &worker, &spec, skills_folder, 30_000).await?;

    println!("\n[download result]");
    println!("  namespace        = {}", result.namespace);
    println!("  skills_written   = {:?}", result.skills_written);
    println!("  prompts_written  = {:?}", result.prompts_written);

    let (skills, skill_skipped) = scan_skills(skills_folder);
    let (prompts, prompt_skipped) = scan_prompts(skills_folder);

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

    println!("\n[scan_prompts]");
    for p in &prompts {
        println!("  name={:<30} path={}", p.name, p.abs_path.display());
    }
    if !prompt_skipped.is_empty() {
        println!("\n[prompt skips]");
        for s in &prompt_skipped {
            println!("  {} → {}", s.path.display(), s.reason);
        }
    }

    println!(
        "\nDONE — skills scanned={}, prompts scanned={}",
        skills.len(),
        prompts.len()
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
