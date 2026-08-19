//! Differential fixtures: replay Python-captured request/response pairs
//! through the Rust ops. Outputs and error text must match exactly; unsupported
//! operations are failures, not skips.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn behavior_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/behavior")
}

#[test]
fn behavior_fixtures_match_reference() {
    let mut failures = Vec::new();
    let mut ran = 0;
    let mut stack = vec![behavior_root()];
    let mut files = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "json") {
                files.push(p);
            }
        }
    }
    files.sort();
    assert!(
        !files.is_empty(),
        "no behavior fixtures found — run gen_goldens.py behavior"
    );
    for file in files {
        let fixture: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        let fid = fixture["function"].as_str().unwrap();
        let case = format!("{fid}/{}", fixture["case"].as_str().unwrap());
        let got = browser::scrapling::dispatch_op(fid, &fixture["request"]);
        match (got, fixture.get("ok")) {
            (Ok(actual), Some(expected)) => {
                ran += 1;
                if &actual != expected {
                    failures.push(format!(
                        "{case}:\n  expected: {expected}\n  actual:   {actual}"
                    ));
                }
            }
            (Err(actual_err), None) => {
                ran += 1;
                let expected_err = fixture["err"].as_str().unwrap();
                if actual_err != expected_err {
                    failures.push(format!(
                        "{case} (error text):\n  expected: {expected_err}\n  actual:   {actual_err}"
                    ));
                }
            }
            (Ok(v), None) => {
                ran += 1;
                failures.push(format!("{case}: expected error, got {v}"));
            }
            (Err(e), Some(_)) => {
                ran += 1;
                failures.push(format!("{case}: unexpected error {e}"));
            }
        }
    }
    eprintln!("behavior fixtures run: {ran}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
