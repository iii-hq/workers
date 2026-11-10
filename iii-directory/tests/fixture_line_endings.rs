//! Checkout-byte policy is independent of the local embedding runtime.

const CORPUS: &str = include_str!("local_pair_eval/corpus.json");
const LOCK: &str = include_str!("../Cargo.lock");

#[test]
fn git_checkout_preserves_frozen_bytes_with_autocrlf() {
    // Retain the isolated fixture for inspection, including on assertion failure.
    // No Git command below uses the source repository's index, config or refs.
    let root = std::env::temp_dir().join(format!("fixture-line-endings-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    eprintln!("isolated Git fixture: {}", root.display());
    let input = root.join("input");
    let exported = root.join("exported");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::create_dir_all(&exported).unwrap();
    let empty_config = root.join("empty-config");
    std::fs::write(&empty_config, b"").unwrap();
    let git = |args: &[&str]| {
        let mut command = std::process::Command::new("git");
        // Ignore inherited repository/index/config/attribute overrides.
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(key);
            }
        }
        let output = command
            .current_dir(&input)
            .env("HOME", &root)
            .env("USERPROFILE", &root)
            .env("XDG_CONFIG_HOME", &root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &empty_config)
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_INDEX_FILE", root.join("index"))
            .arg(format!("--git-dir={}", root.join("repo.git").display()))
            .arg(format!("--work-tree={}", input.display()))
            .args(["-c", "core.autocrlf=true", "-c", "core.safecrlf=false"])
            .arg("-c")
            .arg(format!("core.attributesFile={}", empty_config.display()))
            .args(args)
            .output()
            .expect("native Git is required for the checkout regression");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    git(&["init", "--quiet", "--template="]);
    std::fs::write(
        input.join(".gitattributes"),
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../.gitattributes")),
    )
    .unwrap();
    let cases = [
        (
            "iii-directory/tests/local_pair_eval/corpus.json",
            CORPUS,
            true,
        ),
        ("iii-directory/Cargo.lock", LOCK, true),
        // Identical source bytes at unrelated sibling paths must become CRLF.
        (
            "iii-directory/tests/local_pair_eval/unrelated.json",
            CORPUS,
            false,
        ),
        ("iii-directory/unrelated.lock", LOCK, false),
    ];
    for (path, original, _) in cases {
        assert!(original.contains('\n') && !original.contains('\r'));
        let file = input.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, original.as_bytes()).unwrap();
    }
    // Existing golden/binary policies must remain effective and narrowly scoped.
    let golden = "computer/tests/golden/line-endings.txt";
    let binaries = [
        "sandbox-libkrun/firmware/line-endings.bin",
        "computer/screenshots/nested/line-endings.png",
        "computer/screenshots/nested/line-endings.jpg",
        "computer/screenshots/nested/line-endings.jpeg",
        "computer/screenshots/nested/line-endings.webp",
    ];
    let binary_bytes = b"binary\0\r\nunchanged\n";
    for path in std::iter::once(golden).chain(binaries) {
        let file = input.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            file,
            if path == golden {
                b"golden\nunchanged\n".as_slice()
            } else {
                binary_bytes.as_slice()
            },
        )
        .unwrap();
    }
    git(&["add", "--", "."]);
    for (path, _, pinned) in cases {
        let attributes = git(&["check-attr", "--cached", "-z", "text", "eol", "--", path]);
        let expected = format!(
            "{path}\0text\0{}\0{path}\0eol\0{}\0",
            if pinned { "set" } else { "unspecified" },
            if pinned { "lf" } else { "unspecified" },
        );
        assert_eq!(attributes, expected.as_bytes(), "attributes for {path}");
    }
    assert_eq!(
        git(&["check-attr", "--cached", "-z", "text", "eol", "--", golden]),
        format!("{golden}\0text\0set\0{golden}\0eol\0lf\0").as_bytes()
    );
    for path in binaries {
        assert_eq!(
            git(&[
                "check-attr", "--cached", "-z", "text", "eol", "diff", "merge", "--", path,
            ]),
            format!(
                "{path}\0text\0unset\0{path}\0eol\0unspecified\0{path}\0diff\0unset\0{path}\0merge\0unset\0"
            )
            .as_bytes(),
            "binary attributes for {path}"
        );
    }
    let prefix = format!("{}/", exported.to_str().unwrap().replace('\\', "/"));
    git(&["checkout-index", "--all", &format!("--prefix={prefix}")]);
    for (path, original, pinned) in cases {
        let checkout = exported.join(path);
        let bytes = std::fs::read(&checkout).unwrap();
        // Raw Git blob hashes are independent of both attributes and evaluator
        // helpers. --no-filters prevents hashing from normalizing the evidence.
        let source_hash = git(&["hash-object", "--no-filters", "--", path]);
        let checkout_hash = git(&[
            "hash-object",
            "--no-filters",
            "--",
            checkout.to_str().unwrap(),
        ]);
        eprintln!(
            "{path}: input_blob={} exported_blob={}",
            String::from_utf8_lossy(&source_hash).trim(),
            String::from_utf8_lossy(&checkout_hash).trim()
        );
        if pinned {
            assert_eq!(bytes, original.as_bytes(), "LF bytes for {path}");
            assert_eq!(checkout_hash, source_hash, "LF hash for {path}");
        } else {
            assert_eq!(
                bytes,
                original.replace('\n', "\r\n").as_bytes(),
                "CRLF control for {path}"
            );
            assert_ne!(bytes, original.as_bytes(), "nontrivial control for {path}");
            assert_ne!(checkout_hash, source_hash, "CRLF hash for {path}");
        }
    }
    assert_eq!(
        std::fs::read(exported.join(golden)).unwrap(),
        b"golden\nunchanged\n"
    );
    for path in binaries {
        assert_eq!(
            std::fs::read(exported.join(path)).unwrap(),
            binary_bytes,
            "binary bytes for {path}"
        );
    }
}
