//! Registry reads for the page: a package's published versions and a name
//! search for the add dialog. Both use the public registry API that the
//! compose daemon resolves packages against.

use std::time::Duration;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const DEFAULT_REGISTRY: &str = "api.workers.iii.dev";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct PackageVersion {
    pub version: String,
    /// Release channels pointing at this version, such as `latest` or `next`.
    pub tags: Vec<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RegistryWorker {
    pub name: String,
    pub version: String,
    pub description: String,
    pub dependencies: Vec<String>,
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

/// `api.workers.iii.dev/database@1.0.0` → (`api.workers.iii.dev`, `database`).
/// A bare name resolves against the default registry, like the daemon does.
pub fn split_reference(reference: &str) -> Result<(String, String), String> {
    let reference = reference.trim().trim_start_matches("package://");
    let reference = reference.split('@').next().unwrap_or(reference);
    let (host, name) = match reference.trim_end_matches('/').rsplit_once('/') {
        Some((host, name)) => (host, name),
        None => (DEFAULT_REGISTRY, reference),
    };
    if !valid_segment(host) || !valid_segment(name) || name.contains(':') {
        return Err(format!("not a registry reference: {reference}"));
    }
    Ok((host.to_string(), name.to_string()))
}

async fn get_json<T: DeserializeOwned>(url: url::Url) -> Result<T, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|error| format!("{url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    response
        .json()
        .await
        .map_err(|error| format!("{url}: {error}"))
}

pub async fn versions(reference: &str) -> Result<Vec<PackageVersion>, String> {
    #[derive(Deserialize)]
    struct Body {
        versions: Vec<Raw>,
    }
    #[derive(Deserialize)]
    struct Raw {
        version: String,
        #[serde(default)]
        tags: Vec<String>,
        created_at: Option<String>,
    }
    let (host, name) = split_reference(reference)?;
    let url = url::Url::parse(&format!("https://{host}/w/{name}/versions"))
        .map_err(|error| error.to_string())?;
    let body: Body = get_json(url).await?;
    Ok(body
        .versions
        .into_iter()
        .map(|raw| PackageVersion {
            version: raw.version,
            tags: raw.tags,
            created_at: raw.created_at,
        })
        .collect())
}

pub async fn search(query: &str) -> Result<Vec<RegistryWorker>, String> {
    #[derive(Deserialize)]
    struct Body {
        workers: Vec<Raw>,
    }
    #[derive(Deserialize)]
    struct Raw {
        name: String,
        version: String,
        #[serde(default)]
        description: String,
        #[serde(rename = "type", default)]
        kind: String,
        #[serde(default)]
        dependencies: Vec<Dependency>,
    }
    #[derive(Deserialize)]
    struct Dependency {
        name: String,
    }
    let mut url = url::Url::parse(&format!("https://{DEFAULT_REGISTRY}/w"))
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut().append_pair("search", query.trim());
    let body: Body = get_json(url).await?;
    Ok(body
        .workers
        .into_iter()
        // Engine workers ship inside the engine; the daemon refuses to declare them.
        .filter(|raw| raw.kind != "engine")
        .map(|raw| RegistryWorker {
            name: raw.name,
            version: raw.version,
            description: raw.description,
            dependencies: raw.dependencies.into_iter().map(|d| d.name).collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_registry_references() {
        assert_eq!(
            split_reference("api.workers.iii.dev/database").unwrap(),
            ("api.workers.iii.dev".into(), "database".into())
        );
        assert_eq!(
            split_reference("package://registry.local:8080/web@1.2.0").unwrap(),
            ("registry.local:8080".into(), "web".into())
        );
        assert_eq!(
            split_reference("state").unwrap(),
            (DEFAULT_REGISTRY.into(), "state".into())
        );
    }

    #[test]
    fn refuses_references_that_would_escape_the_url() {
        for bad in [
            "",
            "host/../etc",
            "host/a b",
            "evil.com/x?y=1",
            "host/name#frag",
        ] {
            assert!(split_reference(bad).is_err(), "{bad} should be refused");
        }
    }
}
