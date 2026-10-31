//! The example's `worker-compose.yaml` declares the stack its README runs:
//! the http worker that hosts `http::on-request` for the demo routes, this
//! worker, and the Node.js demo, each from a directory in this repository.

use std::path::{Path, PathBuf};

use serde_yaml::Value;

fn example_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("example")
}

fn compose() -> Value {
    let path = example_dir().join("worker-compose.yaml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_yaml::from_str(&text).expect("worker-compose.yaml parses")
}

/// The directory a `path://` worker names, resolved against the example.
fn worker_dir(container: &Value) -> PathBuf {
    let source = container["worker"].as_str().expect("worker source");
    let relative = source
        .strip_prefix("path://")
        .unwrap_or_else(|| panic!("{source} is a path:// worker"));
    example_dir().join(relative)
}

#[test]
fn the_example_runs_http_image_resize_and_the_demo() {
    let compose = compose();
    let containers = compose["containers"].as_mapping().expect("containers");
    let names: Vec<&str> = containers.keys().filter_map(Value::as_str).collect();
    assert_eq!(names, ["http", "image-resize", "image-resize-demo"]);

    let http = &compose["containers"]["http"];
    let http_dir = worker_dir(http);
    let manifest = std::fs::read_to_string(http_dir.join("Cargo.toml")).expect("http Cargo.toml");
    assert!(manifest.contains("name = \"iii-http\""), "{manifest}");
    assert_eq!(http["config_override"]["port"], Value::from(3111));
    assert_eq!(
        http["scripts"]["run"].as_str(),
        Some("cargo run --release --bin http")
    );

    let resize = &compose["containers"]["image-resize"];
    assert_eq!(
        worker_dir(resize).canonicalize().unwrap(),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .unwrap()
    );
    assert_eq!(
        resize["scripts"]["run"].as_str(),
        Some("cargo run --release --bin image-resize")
    );

    let demo = &compose["containers"]["image-resize-demo"];
    assert!(worker_dir(demo).join("package.json").is_file());
    assert_eq!(
        demo["start_after"],
        serde_yaml::from_str::<Value>("[http, image-resize]").unwrap()
    );
}

/// The engine of 0.26 accepts only engine workers in `config.yaml`; project
/// workers run from `worker-compose.yaml`.
#[test]
fn the_example_has_no_engine_config_yaml() {
    assert!(!example_dir().join("config.yaml").exists());
}
