//! How the ADE hosts its functions: always as a standby.
//!
//! A function id has one host per namespace (iii 0.26, observe v2 §7). A
//! second ADE instance (its own worker name, same namespace) gets
//! `FUNCTION_NAMESPACE_CONFLICT` for every `console::*` id the first one
//! hosts. Without `standby` the SDK drops that registration for good, so the
//! second instance never serves `/ui/*`, the workspace ring or the compose
//! updates, not even after the first leaves. With `standby` the SDK warns
//! once, registers the function again every second (±20 %), and takes over
//! once the engine accepts it: the observers the first host held go
//! `pending`, and this instance's hooks run for each of them, which rebuilds
//! the host-side state (the UI registry, the compose watch) from scratch.
//!
//! While this instance waits, nothing it registered runs here: its hooks do
//! not fire, `is_observed` is false (so the workspace ring and the compose
//! watch emit nothing), and a call of one of these ids reaches the host.
//! Every function the ADE hosts registers through [`HostFunction`], and a
//! test fails on any direct `register_function` call in the crate.

use iii_sdk::runtime::FunctionRef;
use iii_sdk::{IIIClient, RegisterFunction};

/// Registers a function the ADE hosts, as a standby.
pub trait HostFunction {
    /// [`IIIClient::register_function`] with [`RegisterFunction::standby`]:
    /// when another ADE instance hosts `id`, wait and take over when it
    /// leaves.
    fn host_function(&self, id: impl Into<String>, registration: RegisterFunction) -> FunctionRef;
}

impl HostFunction for IIIClient {
    fn host_function(&self, id: impl Into<String>, registration: RegisterFunction) -> FunctionRef {
        self.register_function(id, registration.standby())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// The crate's production code: each `.rs` file under `src/`, up to its
    /// first `#[cfg(test)]` (test stand-ins register what they like).
    fn production_sources() -> Vec<(PathBuf, String)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
            for entry in std::fs::read_dir(dir).expect("read src") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let source = std::fs::read_to_string(&path).expect("read source");
                    let production = source
                        .split("#[cfg(test)]")
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    out.push((path, production));
                }
            }
        }
        let mut out = Vec::new();
        walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
        out
    }

    /// Every function the ADE hosts is a standby: a second instance takes it
    /// over when the first leaves instead of losing it to
    /// `FUNCTION_NAMESPACE_CONFLICT`.
    #[test]
    fn every_ade_function_registers_as_a_standby() {
        let sources = production_sources();
        let direct: Vec<String> = sources
            .iter()
            .filter(|(path, _)| !path.ends_with("host.rs"))
            .flat_map(|(path, source)| {
                source
                    .lines()
                    .enumerate()
                    .filter(|(_, line)| line.contains(".register_function("))
                    .map(move |(n, line)| format!("{}:{}: {}", path.display(), n + 1, line.trim()))
            })
            .collect();
        assert!(
            direct.is_empty(),
            "register ADE functions with HostFunction::host_function (standby), not \
             register_function:\n{}",
            direct.join("\n")
        );

        // The event sources and the asset function, by name: each one is
        // registered through host_function.
        for (file, id) in [
            ("ui_assets.rs", "ASSET_FN"),
            ("ui_assets.rs", "ON_ASSETS_CHANGED_FN"),
            ("functions/workspace.rs", "ON_CHANGED_FN"),
            ("compose/mod.rs", "ON_CHANGED_FN"),
        ] {
            let hosted = sources.iter().any(|(path, source)| {
                path.ends_with(file) && source.contains(&format!(".host_function(\n        {id},"))
            });
            assert!(hosted, "{id} in {file} registers through host_function");
        }
    }
}
