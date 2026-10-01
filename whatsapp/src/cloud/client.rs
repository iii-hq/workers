//! A bounded, single-attempt client for the official Meta Graph API.
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, Method};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Config;

use super::normalize_chat_id;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_COMPONENT_BYTES: usize = 64 * 1024;

pub struct CloudClient {
    http: Client,
    #[cfg(test)]
    test_base: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Phone {
    pub id: String,
    pub display_phone_number: String,
    pub verified_name: String,
    #[serde(default)]
    pub quality_rating: Option<String>,
    #[serde(default)]
    pub code_verification_status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SubscribedApp {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConnectionCheck {
    pub phone: Phone,
    pub phone_in_business_account: bool,
    pub subscribed_apps: Vec<SubscribedApp>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub language: String,
    pub status: String,
    pub category: String,
    #[serde(default)]
    pub components: Vec<Value>,
    #[serde(default)]
    pub parameter_format: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TemplatePage {
    pub templates: Vec<Template>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplatesRequest {
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 100))]
    pub limit: usize,
    pub after: Option<String>,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextRequest {
    pub chat_id: String,
    #[schemars(length(min = 1, max = 4096))]
    pub text: String,
    #[serde(default)]
    pub preview_url: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TemplateRequest {
    pub chat_id: String,
    pub name: String,
    pub language: String,
    #[serde(default)]
    pub components: Vec<Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SentMessage {
    pub message_id: String,
    pub chat_id: String,
    /// Graph accepted the request; delivery is reported separately by webhook.
    pub status: String,
    pub message_status: Option<String>,
}

impl CloudClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: Self::builder().https_only(true).build()?,
            #[cfg(test)]
            test_base: None,
        })
    }

    fn builder() -> reqwest::ClientBuilder {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(15))
            .user_agent("iii-whatsapp-cloud/0.1")
    }

    #[cfg(test)]
    fn for_test(base: String) -> Self {
        assert!(base.starts_with("http://127.0.0.1:"));
        Self {
            http: Self::builder().build().unwrap(),
            test_base: Some(base),
        }
    }

    async fn request(
        &self,
        config: &Config,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value> {
        ensure!(config.cloud.enabled, "Cloud API is disabled");
        ensure!(
            !config.cloud.access_token.trim().is_empty(),
            "Cloud API access token is missing"
        );
        let base = "https://graph.facebook.com";
        #[cfg(test)]
        let base = self.test_base.as_deref().unwrap_or(base);
        let url = format!("{}/{}/{}", base, config.cloud.api_version, path);
        let mut request = self
            .http
            .request(method, url)
            .bearer_auth(&config.cloud.access_token)
            .timeout(Duration::from_secs(config.operation_timeout_secs))
            .query(query);
        if let Some(body) = body {
            request = request.json(body);
        }
        let mut response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                anyhow::anyhow!("Cloud API request timed out; a send may have completed; check messages before retrying")
            } else {
                anyhow::anyhow!("Cloud API request failed; a send may have completed; check messages before retrying")
            }
        })?;
        let status = response.status();
        ensure!(
            response
                .content_length()
                .is_none_or(|size| size <= MAX_RESPONSE_BYTES as u64),
            "Cloud API response exceeds the size limit"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            anyhow::anyhow!(
                "Could not read Cloud API response; check messages before retrying a send"
            )
        })? {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= MAX_RESPONSE_BYTES,
                "Cloud API response exceeds the size limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        let value = serde_json::from_slice::<Value>(&bytes);
        if !status.is_success() {
            bail!(
                "{}",
                remote_error(config, status.as_u16(), value.as_ref().ok())
            );
        }
        let value = value
            .context("Cloud API returned invalid JSON; check messages before retrying a send")?;
        if value.get("error").is_some() {
            bail!("{}", remote_error(config, status.as_u16(), Some(&value)));
        }
        Ok(value)
    }

    pub async fn check_connection(&self, config: &Config) -> Result<ConnectionCheck> {
        let phone = self
            .request(
                config,
                Method::GET,
                &config.cloud.phone_number_id,
                &[(
                    "fields",
                    "id,display_phone_number,verified_name,quality_rating,code_verification_status"
                        .into(),
                )],
                None,
            )
            .await?;
        let phone: Phone =
            serde_json::from_value(phone).context("Cloud API returned invalid phone details")?;
        ensure!(
            phone.id == config.cloud.phone_number_id,
            "Cloud API returned an unexpected phone number"
        );
        let phones = self
            .read_collection(
                config,
                &format!("{}/phone_numbers", config.cloud.business_account_id),
                Some("id"),
            )
            .await?;
        let apps = self
            .read_collection(
                config,
                &format!("{}/subscribed_apps", config.cloud.business_account_id),
                None,
            )
            .await?;
        let subscribed_apps = apps
            .into_iter()
            .map(|app| {
                serde_json::from_value(
                    app.get("whatsapp_business_api_data")
                        .cloned()
                        .context("Cloud API returned an invalid subscription")?,
                )
                .context("Cloud API returned invalid subscribed app details")
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ConnectionCheck {
            phone,
            phone_in_business_account: phones.iter().any(|item| {
                item.get("id").and_then(Value::as_str)
                    == Some(config.cloud.phone_number_id.as_str())
            }),
            subscribed_apps,
        })
    }

    async fn read_collection(
        &self,
        config: &Config,
        path: &str,
        fields: Option<&str>,
    ) -> Result<Vec<Value>> {
        let mut result = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..100 {
            let mut query = vec![("limit", "100".into())];
            if let Some(fields) = fields {
                query.push(("fields", fields.into()));
            }
            if let Some(cursor) = after.as_ref() {
                query.push(("after", cursor.clone()));
            }
            let page = self
                .request(config, Method::GET, path, &query, None)
                .await?;
            result.extend(
                page.get("data")
                    .and_then(Value::as_array)
                    .context("Cloud API returned an invalid collection")?
                    .iter()
                    .cloned(),
            );
            let next = next_cursor(&page)?;
            if next.is_none() {
                return Ok(result);
            }
            ensure!(
                next != after,
                "Cloud API returned a repeated pagination cursor"
            );
            after = next;
        }
        bail!("Cloud API collection exceeds the page limit")
    }

    pub async fn list_templates(
        &self,
        config: &Config,
        req: &TemplatesRequest,
    ) -> Result<TemplatePage> {
        ensure!((1..=100).contains(&req.limit), "limit must be 1..100");
        check_cursor(req.after.as_deref())?;
        let mut query = vec![
            (
                "fields",
                "id,name,language,status,category,components,parameter_format".into(),
            ),
            ("limit", req.limit.to_string()),
        ];
        if let Some(after) = &req.after {
            query.push(("after", after.clone()));
        }
        let page = self
            .request(
                config,
                Method::GET,
                &format!("{}/message_templates", config.cloud.business_account_id),
                &query,
                None,
            )
            .await?;
        let templates = serde_json::from_value(
            page.get("data")
                .cloned()
                .context("Cloud API returned no template list")?,
        )
        .context("Cloud API returned invalid templates")?;
        Ok(TemplatePage {
            templates,
            next_cursor: next_cursor(&page)?,
        })
    }

    pub async fn send_text(&self, config: &Config, req: &TextRequest) -> Result<SentMessage> {
        let body = text_payload(req)?;
        self.send(config, &req.chat_id, &body).await
    }

    pub async fn send_template(
        &self,
        config: &Config,
        req: &TemplateRequest,
    ) -> Result<SentMessage> {
        let body = template_payload(req)?;
        self.send(config, &req.chat_id, &body).await
    }

    async fn send(&self, config: &Config, chat: &str, body: &Value) -> Result<SentMessage> {
        let response = self
            .request(
                config,
                Method::POST,
                &format!("{}/messages", config.cloud.phone_number_id),
                &[],
                Some(body),
            )
            .await?;
        let message = response
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|v| v.first())
            .context(
                "Cloud API returned no message acknowledgement; check messages before retrying",
            )?;
        let id = message
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 512)
            .context("Cloud API returned no valid message ID; check messages before retrying")?;
        Ok(SentMessage {
            message_id: id.to_owned(),
            chat_id: normalize_chat_id(chat)?,
            status: "accepted".into(),
            message_status: message
                .get("message_status")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
}

fn recipient_payload(chat: &str) -> Result<Value> {
    let chat = normalize_chat_id(chat)?;
    let mut body = json!({"messaging_product":"whatsapp","recipient_type":"individual"});
    let field = if chat.starts_with('+') {
        "to"
    } else {
        "recipient"
    };
    body[field] = chat.into();
    Ok(body)
}

fn text_payload(req: &TextRequest) -> Result<Value> {
    ensure!(
        !req.text.trim().is_empty() && req.text.chars().count() <= 4096,
        "text must contain 1..4096 characters"
    );
    let mut body = recipient_payload(&req.chat_id)?;
    body["type"] = "text".into();
    body["text"] = json!({"body":req.text,"preview_url":req.preview_url});
    Ok(body)
}

fn template_payload(req: &TemplateRequest) -> Result<Value> {
    ensure!(
        !req.name.is_empty()
            && req.name.len() <= 512
            && req
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        "template name must contain lowercase letters, digits or underscores, at most 512 characters"
    );
    ensure!(
        !req.language.is_empty()
            && req.language.len() <= 32
            && req
                .language
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "invalid template language"
    );
    ensure!(
        req.components.iter().all(Value::is_object),
        "template components must be objects"
    );
    ensure!(
        serde_json::to_vec(&req.components)?.len() <= MAX_COMPONENT_BYTES,
        "template components exceed 65536 bytes"
    );
    let mut body = recipient_payload(&req.chat_id)?;
    body["type"] = "template".into();
    body["template"] =
        json!({"name":req.name,"language":{"code":req.language},"components":req.components});
    Ok(body)
}

fn check_cursor(cursor: Option<&str>) -> Result<()> {
    ensure!(
        cursor.is_none_or(|v| !v.is_empty() && v.len() <= 2048 && !v.chars().any(char::is_control)),
        "invalid Cloud API cursor"
    );
    Ok(())
}

fn next_cursor(page: &Value) -> Result<Option<String>> {
    // Never follow paging.next: only construct requests against the fixed Graph host.
    if page
        .pointer("/paging/next")
        .and_then(Value::as_str)
        .is_none()
    {
        return Ok(None);
    }
    let cursor = page
        .pointer("/paging/cursors/after")
        .and_then(Value::as_str)
        .context("Cloud API returned no pagination cursor")?;
    check_cursor(Some(cursor))?;
    Ok(Some(cursor.to_owned()))
}

fn remote_error(config: &Config, status: u16, value: Option<&Value>) -> String {
    let mut result = format!("Cloud API HTTP {status}");
    if let Some(error) = value.and_then(|value| value.get("error")) {
        if let Some(code) = error.get("code").and_then(Value::as_i64) {
            result.push_str(&format!(" (code {code})"));
        }
        for text in [error.get("message"), error.pointer("/error_data/details")] {
            if let Some(text) = text.and_then(Value::as_str) {
                result.push_str(": ");
                result.push_str(text);
            }
        }
    }
    for secret in [
        &config.cloud.access_token,
        &config.cloud.app_secret,
        &config.cloud.verify_token,
    ] {
        if !secret.is_empty() {
            result = result.replace(secret, "[REDACTED]");
        }
    }
    result
        .chars()
        .filter(|c| !c.is_control())
        .take(1024)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    fn config() -> Config {
        let mut config = Config::default();
        config.cloud.enabled = true;
        config.cloud.phone_number_id = "123".into();
        config.cloud.business_account_id = "456".into();
        config.cloud.access_token = "private-test-token".into();
        config.cloud.app_secret = "private-app-secret".into();
        config.cloud.verify_token = "private-verify-token".into();
        config
    }

    fn server(
        status: &str,
        headers: &str,
        body: &str,
    ) -> (CloudClient, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
            body.len()
        );
        let (tx, rx) = mpsc::channel();
        let task = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let _ = tx.send(String::from_utf8(bytes).unwrap());
            let _ = stream.write_all(response.as_bytes());
        });
        (CloudClient::for_test(base), rx, task)
    }

    #[test]
    fn payloads_preserve_unicode_and_distinguish_bsuid() {
        let req = TextRequest {
            chat_id: "+1 (202) 555-0123".into(),
            text: "😀".repeat(4096),
            preview_url: false,
        };
        let body = text_payload(&req).unwrap();
        assert_eq!(body["to"], "+12025550123");
        assert!(body.get("recipient").is_none());
        assert!(
            text_payload(&TextRequest {
                text: "a".repeat(4097),
                ..req
            })
            .is_err()
        );
        let body = template_payload(&TemplateRequest {
            chat_id: "US.123Ab".into(),
            name: "hello_world".into(),
            language: "en_US".into(),
            components: vec![],
        })
        .unwrap();
        assert_eq!(body["recipient"], "US.123Ab");
        assert!(body.get("to").is_none());
        assert_eq!(body["template"]["language"]["code"], "en_US");
    }

    #[tokio::test]
    async fn sends_once_with_header_auth_and_returns_only_acknowledgement() {
        let (client, requests, task) = server(
            "200 OK",
            "",
            r#"{"messages":[{"id":"wamid.test","message_status":"accepted"}],"contacts":[{"wa_id":"12025550123"}]}"#,
        );
        let sent = client
            .send_text(
                &config(),
                &TextRequest {
                    chat_id: "+12025550123".into(),
                    text: "test".into(),
                    preview_url: false,
                },
            )
            .await
            .unwrap();
        task.join().unwrap();
        let request = requests.recv().unwrap();
        assert!(request.starts_with("POST /v26.0/123/messages "));
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer private-test-token")
        );
        assert!(
            !request
                .lines()
                .next()
                .unwrap()
                .contains("private-test-token")
        );
        assert_eq!(sent.message_id, "wamid.test");
        assert_eq!(sent.status, "accepted");
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn redirects_are_not_followed_and_remote_secrets_are_redacted() {
        let (client, requests, task) = server(
            "302 Found",
            "Location: http://127.0.0.1:1/steal\r\n",
            r#"{"error":{"code":190,"message":"private-test-token private-app-secret private-verify-token"}}"#,
        );
        let error = client
            .list_templates(
                &config(),
                &TemplatesRequest {
                    limit: 50,
                    after: None,
                },
            )
            .await
            .unwrap_err()
            .to_string();
        task.join().unwrap();
        assert!(error.contains("HTTP 302"));
        assert!(error.contains("code 190"));
        assert!(!error.contains("private-"));
        assert_eq!(error.matches("[REDACTED]").count(), 3);
        assert!(
            requests
                .recv()
                .unwrap()
                .starts_with("GET /v26.0/456/message_templates?")
        );
    }

    #[tokio::test]
    async fn response_limit_is_enforced() {
        let (client, _, task) = server("200 OK", "", &" ".repeat(MAX_RESPONSE_BYTES + 1));
        let error = client
            .list_templates(
                &config(),
                &TemplatesRequest {
                    limit: 50,
                    after: None,
                },
            )
            .await
            .unwrap_err()
            .to_string();
        task.join().unwrap();
        assert!(error.contains("size limit"));
    }

    #[tokio::test]
    async fn template_paging_exposes_only_the_cursor() {
        let (client, _, task) = server(
            "200 OK",
            "",
            r#"{"data":[{"id":"1","name":"hello_world","language":"en_US","status":"APPROVED","category":"UTILITY","components":[]}],"paging":{"next":"https://untrusted.example/secret","cursors":{"after":"next-token"}}}"#,
        );
        let page = client
            .list_templates(
                &config(),
                &TemplatesRequest {
                    limit: 50,
                    after: Some("a+b/=".into()),
                },
            )
            .await
            .unwrap();
        task.join().unwrap();
        assert_eq!(page.next_cursor.as_deref(), Some("next-token"));
        assert_eq!(page.templates.len(), 1);
        assert!(!serde_json::to_string(&page).unwrap().contains("untrusted"));
    }
}
