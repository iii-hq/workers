// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Worker lifecycle: container spec building.

use super::worker_manager::adapter::ContainerSpec;
use super::worker_manager::state::WorkerDef;

pub fn build_container_spec(name: &str, def: &WorkerDef, _engine_url: &str) -> ContainerSpec {
    match def {
        WorkerDef::Managed {
            image,
            env,
            resources,
        } => ContainerSpec {
            name: name.to_string(),
            image: image.clone(),
            env: {
                let mut env = env.clone();
                // The config.yaml entry name, so SDKs self-report the managed
                // identity; engine truth (`iii worker status`/`list`) matches
                // connections by this name. Not overridable via worker env —
                // identity comes from the config entry.
                env.insert("III_WORKER_NAME".to_string(), name.to_string());
                env
            },
            memory_limit: resources.as_ref().and_then(|r| r.memory.clone()),
            cpu_limit: resources.as_ref().and_then(|r| r.cpus.clone()),
        },
        WorkerDef::Binary { .. } => ContainerSpec {
            name: name.to_string(),
            image: String::new(),
            env: std::collections::HashMap::new(),
            memory_limit: None,
            cpu_limit: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::super::worker_manager::state::WorkerResources;
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn build_container_spec_maps_fields() {
        let def = WorkerDef::Managed {
            image: "ghcr.io/iii-hq/image-resize:0.1.2".to_string(),
            env: {
                let mut m = HashMap::new();
                m.insert("FOO".to_string(), "bar".to_string());
                m
            },
            resources: Some(WorkerResources {
                cpus: Some("4".to_string()),
                memory: Some("4096Mi".to_string()),
            }),
        };

        let spec = build_container_spec("my-worker", &def, "ws://localhost:49134");

        assert_eq!(spec.name, "my-worker");
        assert_eq!(spec.image, "ghcr.io/iii-hq/image-resize:0.1.2");
        assert_eq!(spec.env.get("FOO").unwrap(), "bar");
        assert_eq!(spec.env.get("III_WORKER_NAME").unwrap(), "my-worker");
        assert_eq!(spec.cpu_limit.as_deref(), Some("4"));
        assert_eq!(spec.memory_limit.as_deref(), Some("4096Mi"));
    }

    #[test]
    fn build_container_spec_binary_variant() {
        let def = WorkerDef::Binary {
            version: "0.1.2".to_string(),
            config: None,
        };

        let spec = build_container_spec("bin-worker", &def, "ws://localhost:49134");

        assert_eq!(spec.name, "bin-worker");
        assert!(
            spec.image.is_empty(),
            "Binary variant should have empty image"
        );
        assert!(spec.env.is_empty(), "Binary variant should have empty env");
        assert!(spec.memory_limit.is_none());
        assert!(spec.cpu_limit.is_none());
    }
}
