use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::context::E2eContext;

#[derive(Debug, Serialize)]
struct ModelsListRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct ModelsListResponse {
    models: Vec<CatalogModel>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CatalogModel {
    provider: String,
    id: String,
}

pub async fn list(context: &E2eContext, provider: Option<&str>) -> Result<Vec<CatalogModel>> {
    let mut response: ModelsListResponse = context
        .trigger("router::models::list", ModelsListRequest { provider })
        .await
        .context("list models from the running stack")?;
    response.models.sort_unstable_by(|left, right| {
        (&left.provider, &left.id).cmp(&(&right.provider, &right.id))
    });
    Ok(response.models)
}

pub fn summary(models: &[CatalogModel]) -> String {
    if models.is_empty() {
        return "No models found.\n".to_string();
    }

    let provider_width = models
        .iter()
        .map(|model| model.provider.len())
        .max()
        .unwrap_or_default()
        .max("PROVIDER".len());
    let mut output = format!("{:<provider_width$}  MODEL\n", "PROVIDER");
    for model in models {
        output.push_str(&format!(
            "{:<provider_width$}  {}\n",
            model.provider, model.id
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(provider: &str, id: &str) -> CatalogModel {
        CatalogModel {
            provider: provider.into(),
            id: id.into(),
        }
    }

    #[test]
    fn summary_aligns_provider_and_model_columns() {
        let output = summary(&[
            model("zai", "glm-5.2"),
            model("openai-codex", "codex/gpt-5.6-sol"),
        ]);

        assert_eq!(
            output,
            "\
PROVIDER      MODEL
zai           glm-5.2
openai-codex  codex/gpt-5.6-sol
"
        );
    }

    #[test]
    fn summary_handles_an_empty_catalog() {
        assert_eq!(summary(&[]), "No models found.\n");
    }

    #[test]
    fn catalog_models_have_a_stable_json_shape() {
        assert_eq!(
            serde_json::to_value([model("zai", "glm-5.2")]).unwrap(),
            serde_json::json!([{"provider": "zai", "id": "glm-5.2"}]),
        );
    }
}
