use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct WorkerManifest {
    pub iii: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub license: String,
    pub entrypoint: Entrypoint,
    pub capabilities: Capabilities,
    pub config: ConfigSection,
    pub resources: Resources,
}

#[derive(Debug, Serialize)]
pub struct Entrypoint {
    pub command: Vec<String>,
    pub transport: String,
    pub protocol: String,
}

#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub functions: Vec<FunctionCapability>,
}

#[derive(Debug, Serialize)]
pub struct FunctionCapability {
    pub id: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct ConfigSection {
    pub schema: Value,
}

#[derive(Debug, Serialize)]
pub struct Resources {
    pub memory: String,
    pub cpu: String,
}

pub fn build_manifest() -> WorkerManifest {
    WorkerManifest {
        iii: "v1".into(),
        name: env!("CARGO_PKG_NAME").into(),
        version: env!("CARGO_PKG_VERSION").into(),
        description: "Image resize and format conversion".into(),
        author: "iii-hq".into(),
        license: "MIT".into(),
        entrypoint: Entrypoint {
            command: vec!["/worker".into()],
            transport: "websocket".into(),
            protocol: "iii-worker-v1".into(),
        },
        capabilities: Capabilities {
            functions: vec![FunctionCapability {
                id: "image_resize::resize".into(),
                description: "Resize an image via channel I/O".into(),
                request_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "input_channel": { "type": "object" },
                        "output_channel": { "type": "object" },
                        "metadata": { "type": "object" }
                    },
                    "required": ["input_channel", "output_channel"]
                })),
                response_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "format": { "type": "string" },
                        "width": { "type": "integer" },
                        "height": { "type": "integer" }
                    }
                })),
            }],
        },
        config: ConfigSection {
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "width": { "type": "integer", "default": 200 },
                    "height": { "type": "integer", "default": 200 },
                    "strategy": {
                        "type": "string",
                        "enum": ["scale-to-fit", "crop-to-fit"],
                        "default": "scale-to-fit"
                    },
                    "quality": {
                        "type": "object",
                        "properties": {
                            "jpeg": { "type": "integer", "default": 85 },
                            "webp": { "type": "integer", "default": 80 }
                        }
                    }
                }
            }),
        },
        resources: Resources {
            memory: "256Mi".into(),
            cpu: "0.5".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_manifest_yaml() {
        let manifest = build_manifest();
        let yaml = serde_yaml::to_string(&manifest).unwrap();
        assert!(yaml.contains("iii: v1"));
        assert!(yaml.contains("name: image-resize"));
        assert!(yaml.contains("image_resize::resize"));
    }

    #[test]
    fn test_manifest_version_matches_cargo() {
        let manifest = build_manifest();
        assert_eq!(manifest.version, env!("CARGO_PKG_VERSION"));
    }
}
