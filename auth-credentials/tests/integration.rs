//! Behavior tests that run without an iii engine connection.

#[test]
fn api_key_credential_serializes_round_trip_with_snake_case_tag() {
    let cred = auth_credentials::Credential::ApiKey {
        key: "sk-test".into(),
    };
    let json = serde_json::to_string(&cred).expect("serialize");
    assert!(json.contains(r#""type":"api_key""#));
    let back: auth_credentials::Credential = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(cred, back);
}

#[test]
fn oauth_credential_serializes_round_trip_with_contract_tag() {
    let cred = auth_credentials::Credential::OAuth {
        access_token: "oauth-access".into(),
        refresh_token: Some("oauth-refresh".into()),
        expires_at: Some(1_893_456_000),
        scopes: vec!["models:read".into()],
        provider_extra: serde_json::json!({ "workspace": "prod" }),
    };
    let json = serde_json::to_string(&cred).expect("serialize");
    assert!(json.contains(r#""type":"oauth""#));
    assert!(!json.contains(r#""type":"o_auth""#));
    let back: auth_credentials::Credential = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(cred, back);
}

#[test]
fn credential_type_oauth_serializes_with_contract_tag() {
    let value = serde_json::to_value(auth_credentials::CredentialType::OAuth).unwrap();
    assert_eq!(value, serde_json::json!("oauth"));
}

#[test]
fn auth_source_serializes_as_snake_case() {
    let value = serde_json::to_value(auth_credentials::AuthSource::Environment).unwrap();
    assert_eq!(value, serde_json::json!("environment"));
}

#[tokio::test]
async fn handlers_round_trip_stored_credential_and_delete() -> anyhow::Result<()> {
    let store = auth_credentials::InMemoryStore::new();
    let credential = auth_credentials::Credential::ApiKey {
        key: "sk-stored".into(),
    };

    let set = auth_credentials::handle_set_token(
        &store,
        auth_credentials::SetTokenInput {
            provider: "anthropic".into(),
            credential: credential.clone(),
        },
    )
    .await?;
    assert!(set.ok);

    let got = auth_credentials::handle_get_token(
        &store,
        auth_credentials::ProviderInput {
            provider: "anthropic".into(),
        },
        |_| Some("sk-env".into()),
    )
    .await?;
    assert_eq!(got, Some(credential));

    let deleted = auth_credentials::handle_delete_token(
        &store,
        auth_credentials::ProviderInput {
            provider: "anthropic".into(),
        },
    )
    .await?;
    assert!(deleted.ok);

    let after_delete = auth_credentials::handle_get_token(
        &store,
        auth_credentials::ProviderInput {
            provider: "anthropic".into(),
        },
        |_| None,
    )
    .await?;
    assert_eq!(after_delete, None);
    Ok(())
}

#[tokio::test]
async fn set_token_overwrites_existing_credential() -> anyhow::Result<()> {
    let store = auth_credentials::InMemoryStore::new();
    auth_credentials::handle_set_token(
        &store,
        auth_credentials::SetTokenInput {
            provider: "anthropic".into(),
            credential: auth_credentials::Credential::ApiKey {
                key: "sk-old".into(),
            },
        },
    )
    .await?;
    auth_credentials::handle_set_token(
        &store,
        auth_credentials::SetTokenInput {
            provider: "anthropic".into(),
            credential: auth_credentials::Credential::ApiKey {
                key: "sk-new".into(),
            },
        },
    )
    .await?;

    let got = auth_credentials::handle_get_token(
        &store,
        auth_credentials::ProviderInput {
            provider: "anthropic".into(),
        },
        |_| None,
    )
    .await?;
    assert_eq!(
        got,
        Some(auth_credentials::Credential::ApiKey {
            key: "sk-new".into()
        })
    );
    Ok(())
}

#[tokio::test]
async fn oauth_credentials_round_trip_with_optional_fields() -> anyhow::Result<()> {
    let store = auth_credentials::InMemoryStore::new();
    let credential = auth_credentials::Credential::OAuth {
        access_token: "oauth-access-secret".into(),
        refresh_token: Some("oauth-refresh-secret".into()),
        expires_at: Some(1_893_456_000),
        scopes: vec!["models:read".into(), "messages:write".into()],
        provider_extra: serde_json::json!({ "workspace": "prod" }),
    };

    auth_credentials::handle_set_token(
        &store,
        auth_credentials::SetTokenInput {
            provider: "anthropic".into(),
            credential: credential.clone(),
        },
    )
    .await?;

    let got = auth_credentials::handle_get_token(
        &store,
        auth_credentials::ProviderInput {
            provider: "anthropic".into(),
        },
        |_| None,
    )
    .await?;
    assert_eq!(got, Some(credential));

    let status = auth_credentials::handle_status(
        &store,
        auth_credentials::ProviderInput {
            provider: "anthropic".into(),
        },
        |_| None,
    )
    .await?;
    assert_eq!(status.source, Some(auth_credentials::AuthSource::Stored));
    assert_eq!(status.label.as_deref(), Some("oauth"));
    let rendered = serde_json::to_string(&status)?;
    assert!(!rendered.contains("oauth-access-secret"));
    assert!(!rendered.contains("oauth-refresh-secret"));
    Ok(())
}

#[tokio::test]
async fn provider_is_trimmed_before_storage_lookup_and_listing() -> anyhow::Result<()> {
    let store = auth_credentials::InMemoryStore::new();
    auth_credentials::handle_set_token(
        &store,
        auth_credentials::SetTokenInput {
            provider: "  anthropic  ".into(),
            credential: auth_credentials::Credential::ApiKey {
                key: "sk-trimmed".into(),
            },
        },
    )
    .await?;

    let got = auth_credentials::handle_get_token(
        &store,
        auth_credentials::ProviderInput {
            provider: "anthropic".into(),
        },
        |_| None,
    )
    .await?;
    assert_eq!(
        got,
        Some(auth_credentials::Credential::ApiKey {
            key: "sk-trimmed".into()
        })
    );

    let output =
        auth_credentials::handle_list_providers(&store, auth_credentials::ListProvidersInput {})
            .await?;
    assert_eq!(output.providers, vec!["anthropic"]);
    Ok(())
}

#[tokio::test]
async fn get_token_uses_environment_fallback_when_store_is_empty() -> anyhow::Result<()> {
    let store = auth_credentials::InMemoryStore::new();
    let got = auth_credentials::handle_get_token(
        &store,
        auth_credentials::ProviderInput {
            provider: "openai".into(),
        },
        |var| {
            if var == "OPENAI_API_KEY" {
                Some("sk-env-openai".into())
            } else {
                None
            }
        },
    )
    .await?;
    assert_eq!(
        got,
        Some(auth_credentials::Credential::ApiKey {
            key: "sk-env-openai".into()
        })
    );
    Ok(())
}

#[tokio::test]
async fn delete_reveals_environment_fallback() -> anyhow::Result<()> {
    let store = auth_credentials::InMemoryStore::new();
    auth_credentials::handle_set_token(
        &store,
        auth_credentials::SetTokenInput {
            provider: "openai".into(),
            credential: auth_credentials::Credential::ApiKey {
                key: "sk-stored-openai".into(),
            },
        },
    )
    .await?;
    auth_credentials::handle_delete_token(
        &store,
        auth_credentials::ProviderInput {
            provider: "openai".into(),
        },
    )
    .await?;

    let got = auth_credentials::handle_get_token(
        &store,
        auth_credentials::ProviderInput {
            provider: "openai".into(),
        },
        |var| {
            if var == "OPENAI_API_KEY" {
                Some("sk-env-openai".into())
            } else {
                None
            }
        },
    )
    .await?;
    assert_eq!(
        got,
        Some(auth_credentials::Credential::ApiKey {
            key: "sk-env-openai".into()
        })
    );

    let status = auth_credentials::handle_status(
        &store,
        auth_credentials::ProviderInput {
            provider: "openai".into(),
        },
        |var| {
            if var == "OPENAI_API_KEY" {
                Some("sk-env-openai".into())
            } else {
                None
            }
        },
    )
    .await?;
    assert_eq!(
        status.source,
        Some(auth_credentials::AuthSource::Environment)
    );
    Ok(())
}

#[tokio::test]
async fn empty_environment_variable_is_ignored() -> anyhow::Result<()> {
    let store = auth_credentials::InMemoryStore::new();
    let got = auth_credentials::handle_get_token(
        &store,
        auth_credentials::ProviderInput {
            provider: "openai".into(),
        },
        |var| {
            if var == "OPENAI_API_KEY" {
                Some(String::new())
            } else {
                None
            }
        },
    )
    .await?;
    assert_eq!(got, None);

    let status = auth_credentials::handle_status(
        &store,
        auth_credentials::ProviderInput {
            provider: "openai".into(),
        },
        |var| {
            if var == "OPENAI_API_KEY" {
                Some(String::new())
            } else {
                None
            }
        },
    )
    .await?;
    assert!(!status.configured);
    assert_eq!(status.source, None);
    assert_eq!(status.label, None);
    Ok(())
}

#[tokio::test]
async fn unknown_provider_returns_null_and_unconfigured_status() -> anyhow::Result<()> {
    let store = auth_credentials::InMemoryStore::new();
    let got = auth_credentials::handle_get_token(
        &store,
        auth_credentials::ProviderInput {
            provider: "unknown-provider".into(),
        },
        |_| Some("sk-env-openai".into()),
    )
    .await?;
    assert_eq!(got, None);

    let status = auth_credentials::handle_status(
        &store,
        auth_credentials::ProviderInput {
            provider: "unknown-provider".into(),
        },
        |_| Some("sk-env-openai".into()),
    )
    .await?;
    assert!(!status.configured);
    assert_eq!(status.source, None);
    assert_eq!(status.label, None);
    Ok(())
}

#[tokio::test]
async fn status_never_serializes_full_credential_bytes() -> anyhow::Result<()> {
    let store = auth_credentials::InMemoryStore::new();
    let status = auth_credentials::handle_status(
        &store,
        auth_credentials::ProviderInput {
            provider: "openai".into(),
        },
        |var| {
            if var == "OPENAI_API_KEY" {
                Some("sk-env-openai-secret".into())
            } else {
                None
            }
        },
    )
    .await?;

    let rendered = serde_json::to_string(&status)?;
    assert!(status.configured);
    assert_eq!(
        status.source,
        Some(auth_credentials::AuthSource::Environment)
    );
    assert!(!rendered.contains("sk-env-openai-secret"));
    assert!(rendered.contains("api-key:sk-env"));
    Ok(())
}

#[tokio::test]
async fn list_providers_sorts_names_and_omits_credentials() -> anyhow::Result<()> {
    let store = auth_credentials::InMemoryStore::new();
    auth_credentials::handle_set_token(
        &store,
        auth_credentials::SetTokenInput {
            provider: "openai".into(),
            credential: auth_credentials::Credential::ApiKey {
                key: "sk-openai".into(),
            },
        },
    )
    .await?;
    auth_credentials::handle_set_token(
        &store,
        auth_credentials::SetTokenInput {
            provider: "anthropic".into(),
            credential: auth_credentials::Credential::ApiKey {
                key: "sk-anthropic".into(),
            },
        },
    )
    .await?;

    let output =
        auth_credentials::handle_list_providers(&store, auth_credentials::ListProvidersInput {})
            .await?;
    assert_eq!(output.providers, vec!["anthropic", "openai"]);

    let rendered = serde_json::to_string(&output)?;
    assert!(!rendered.contains("sk-openai"));
    assert!(!rendered.contains("sk-anthropic"));
    Ok(())
}

#[tokio::test]
async fn blank_provider_is_rejected_before_storage() {
    let store = auth_credentials::InMemoryStore::new();
    let err = auth_credentials::handle_set_token(
        &store,
        auth_credentials::SetTokenInput {
            provider: " ".into(),
            credential: auth_credentials::Credential::ApiKey {
                key: "sk-invalid".into(),
            },
        },
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("provider must be non-empty"));
}
