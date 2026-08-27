//! Boot seeding of the system prompts the worker ships with. Each bundled
//! prompt is written into `<skills_folder>/system-prompts/` only when no
//! file with that name exists there — user edits always persist across
//! restarts; deleting a bundled prompt brings it back on the next boot
//! (it ships with the worker). Failures only warn: a read-only skills
//! folder must not block startup.

use std::path::Path;

use crate::config::SkillsConfig;

/// The prompts every iii-directory installation ships: `(file stem, full
/// on-disk content, frontmatter included)`.
const BUNDLED_SYSTEM_PROMPTS: &[(&str, &str)] =
    &[("iii-minimal", include_str!("../prompts/iii-minimal.md"))];

pub fn seed_bundled_system_prompts(cfg: &SkillsConfig) {
    let folder = cfg.resolved_skills_folder().join("system-prompts");
    for (name, content) in BUNDLED_SYSTEM_PROMPTS {
        if let Err(error) = seed_one(&folder, name, content) {
            tracing::warn!(%error, name, "bundled system prompt not seeded");
        }
    }
}

fn seed_one(folder: &Path, name: &str, content: &str) -> std::io::Result<()> {
    let path = folder.join(format!("{name}.md"));
    if path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(folder)?;
    // Atomic publish: never leave a half-written prompt for the scanner.
    let tmp = folder.join(format!(".{name}.md.seed"));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(dir: &Path) -> SkillsConfig {
        SkillsConfig {
            skills_folder: dir.to_string_lossy().into_owned(),
            ..SkillsConfig::default()
        }
    }

    #[test]
    fn seeds_the_bundled_prompt_once_and_keeps_user_edits() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("system-prompts/iii-minimal.md");

        seed_bundled_system_prompts(&cfg(tmp.path()));
        let seeded = std::fs::read_to_string(&path).unwrap();
        assert!(seeded.contains("name: iii-minimal"));
        assert!(seeded.contains("You are an iii agent."));

        // A user edit survives the next boot untouched.
        std::fs::write(&path, "---\ndescription: mine\n---\nEdited.\n").unwrap();
        seed_bundled_system_prompts(&cfg(tmp.path()));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "---\ndescription: mine\n---\nEdited.\n"
        );

        // A deleted bundled prompt returns on the next boot.
        std::fs::remove_file(&path).unwrap();
        seed_bundled_system_prompts(&cfg(tmp.path()));
        assert!(path.exists());
    }
}
