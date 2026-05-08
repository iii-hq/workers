use std::path::PathBuf;

/// TDD spec for the renderer.
///
/// Asserts that running `render_worker` against the in-tree `textstats` fixture
/// produces, byte-for-byte, the checked-in `README.md`, `skill.md`, and
/// `skills/*.md` artifacts.
///
/// The textstats fixture is the single source of truth for how the renderer
/// must behave; if the fixture is updated, this test is what will detect it.
#[test]
fn render_textstats_matches_golden() {
    let workers_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let textstats = workers_dir.join("textstats");

    let outputs = iii_skill_check::render::render_worker(&textstats)
        .expect("render_worker should succeed against the textstats fixture");

    let expected_readme = std::fs::read_to_string(textstats.join("README.md")).unwrap();
    similar_asserts::assert_eq!(expected_readme, outputs.readme);

    let expected_skill = std::fs::read_to_string(textstats.join("skill.md")).unwrap();
    similar_asserts::assert_eq!(expected_skill, outputs.skill);

    for leaf in ["analyze", "diff", "summarize"] {
        let expected =
            std::fs::read_to_string(textstats.join("skills").join(format!("{leaf}.md"))).unwrap();
        let actual = outputs
            .leaves
            .get(leaf)
            .unwrap_or_else(|| panic!("missing leaf '{leaf}' in render output"));
        similar_asserts::assert_eq!(expected, *actual);
    }
}
