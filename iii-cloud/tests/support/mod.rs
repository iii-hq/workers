//! Shared test support: the golden-file harness, a fake platform (vault and
//! events) and a context builder that points at a mock cloud.
#![allow(dead_code)]

pub mod real_engine;
pub mod repo;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_cloud::config::Config;
use iii_cloud::ctx::{Ctx, Fut, Platform};
use iii_cloud::error::Res;
use iii_cloud::store::Store;
use iii_cloud::types::ChangeEvent;
use tokio::sync::RwLock;

pub fn golden_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Compare `actual` with `tests/golden/<rel>`; `UPDATE_GOLDENS=1` rewrites it.
pub fn check_golden(rel: &str, actual: &str) -> Result<(), String> {
    let path = golden_root().join(rel);
    if std::env::var("UPDATE_GOLDENS").is_ok_and(|v| v == "1") {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        return std::fs::write(&path, actual).map_err(|e| e.to_string());
    }
    let expected = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "golden {} unreadable ({e}). Run `UPDATE_GOLDENS=1 cargo test` and review the diff.",
            path.display()
        )
    })?;
    if expected == actual {
        return Ok(());
    }
    let line = expected
        .lines()
        .zip(actual.lines())
        .position(|(e, a)| e != a)
        .unwrap_or(0)
        + 1;
    Err(format!(
        "golden mismatch: {rel} (first divergence at line {line}). Run `UPDATE_GOLDENS=1 cargo test`, review and commit."
    ))
}

pub fn assert_typed_schema(label: &str, schema: &schemars::schema::RootSchema) {
    let value = serde_json::to_value(schema).unwrap();
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("{label}: not an object"));
    let defining = [
        "type",
        "properties",
        "$ref",
        "allOf",
        "anyOf",
        "oneOf",
        "enum",
        "items",
    ];
    assert!(
        defining.iter().any(|k| obj.contains_key(*k)),
        "{label}: permissive schema"
    );
}

/// A vault in a map and the events in a list.
#[derive(Default)]
pub struct Fake {
    pub secrets: Mutex<HashMap<String, String>>,
    pub events: Mutex<Vec<ChangeEvent>>,
    /// Fake engine functions (session::*, harness::*) by id.
    pub fns: Mutex<HashMap<String, FakeFn>>,
    /// Every call made to a fake function: (id, payload).
    pub calls: Mutex<Vec<(String, serde_json::Value)>>,
}

pub type FakeFn = std::sync::Arc<dyn Fn(serde_json::Value) -> Res<serde_json::Value> + Send + Sync>;

impl Fake {
    pub fn with_secret(self: Arc<Self>, name: &str, value: &str) -> Arc<Self> {
        self.secrets
            .lock()
            .unwrap()
            .insert(name.into(), value.into());
        self
    }

    pub fn on(
        &self,
        id: &str,
        f: impl Fn(serde_json::Value) -> Res<serde_json::Value> + Send + Sync + 'static,
    ) {
        self.fns
            .lock()
            .unwrap()
            .insert(id.into(), std::sync::Arc::new(f));
    }

    pub fn kinds(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| {
                format!(
                    "{}:{}",
                    serde_json::to_value(e.kind).unwrap().as_str().unwrap(),
                    e.terminal
                )
            })
            .collect()
    }
}

impl Platform for Fake {
    fn secret_resolve<'a>(&'a self, name: &'a str) -> Fut<'a, Res<Option<String>>> {
        Box::pin(async move { Ok(self.secrets.lock().unwrap().get(name).cloned()) })
    }

    fn secret_cas<'a>(
        &'a self,
        name: &'a str,
        expected: Option<&'a str>,
        value: &'a str,
    ) -> Fut<'a, Res<bool>> {
        Box::pin(async move {
            let mut map = self.secrets.lock().unwrap();
            if map.get(name).map(String::as_str) != expected {
                return Ok(false);
            }
            map.insert(name.into(), value.into());
            Ok(true)
        })
    }

    fn emit(&self, event: ChangeEvent) {
        self.events.lock().unwrap().push(event);
    }

    fn call<'a>(
        &'a self,
        function: &'a str,
        payload: serde_json::Value,
    ) -> Fut<'a, Res<serde_json::Value>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((function.to_string(), payload.clone()));
            let f = self.fns.lock().unwrap().get(function).cloned();
            match f {
                Some(f) => f(payload),
                None => Err(iii_cloud::error::CloudError::new(
                    "ENGINE_CALL_FAILED",
                    format!("{function} is not registered"),
                )),
            }
        })
    }
}

pub struct Harness {
    pub ctx: Arc<Ctx>,
    pub fake: Arc<Fake>,
    pub dir: tempfile::TempDir,
}

/// A context against `cloud_url`, its store in a temp dir, waits of 10 ms.
pub fn harness(cloud_url: &str, fake: Arc<Fake>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        cloud_url: cloud_url.into(),
        data_dir: dir.path().join("data").to_string_lossy().into_owned(),
        ..Config::default()
    };
    let store = Arc::new(Store::open(&cfg.data_path()));
    let cell = Arc::new(RwLock::new(Arc::new(cfg)));
    let ctx = Ctx::with_tick(cell, store, fake.clone(), Some(Duration::from_millis(10)));
    Harness { ctx, fake, dir }
}
