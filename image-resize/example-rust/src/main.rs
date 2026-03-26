use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use iii_sdk::{
    IIIError, RegisterFunctionMessage, RegisterTriggerInput, TriggerRequest, Value,
    register_worker, InitOptions,
};
use serde_json::json;

/// Detect image format from the first bytes (magic numbers).
fn detect_format(data: &[u8]) -> &'static str {
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8 {
        return "jpeg";
    }
    if data.len() >= 4 && data[0] == 0x89 && data[1] == 0x50 && data[2] == 0x4E && data[3] == 0x47
    {
        return "png";
    }
    if data.len() >= 12
        && data[8] == 0x57
        && data[9] == 0x45
        && data[10] == 0x42
        && data[11] == 0x50
    {
        return "webp";
    }
    "jpeg" // fallback
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://localhost:49134".to_string());

    tracing::info!(url = %url, "connecting to III engine");

    let iii = register_worker(&url, InitOptions::default());

    // ── Health check ────────────────────────────────────────────────────────

    iii.register_function_with(
        RegisterFunctionMessage::with_id("api::get::/health".to_string()),
        |_input: Value| async move {
            Ok(json!({
                "status_code": 200,
                "body": { "status": "ok", "service": "image-resize-demo-rust" },
                "headers": { "Content-Type": "application/json" }
            }))
        },
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "api::get::/health".to_string(),
        config: json!({ "api_path": "/health", "http_method": "GET" }),
    })?;

    tracing::info!("registered api::get::/health");

    // ── Thumbnail endpoint ──────────────────────────────────────────────────

    let iii_clone = iii.clone();

    iii.register_function_with(
        RegisterFunctionMessage::with_id("api::post::/thumbnail".to_string())
            .with_description("Generate a thumbnail from a base64-encoded image via the image-resize module".to_string()),
        move |input: Value| {
            let iii = iii_clone.clone();
            async move {
                let body = input.get("body").cloned().unwrap_or(Value::Null);

                let image_b64 = body.get("image").and_then(|v| v.as_str());
                if image_b64.is_none() {
                    return Ok(json!({
                        "status_code": 400,
                        "body": { "error": "Missing \"image\" field (base64-encoded image data)" },
                        "headers": { "Content-Type": "application/json" }
                    }));
                }
                let image_b64 = image_b64.unwrap();

                let image_bytes = BASE64.decode(image_b64).map_err(|e| {
                    IIIError::Runtime(format!("invalid base64: {e}"))
                })?;

                let width = body.get("width").and_then(|v| v.as_u64()).unwrap_or(200);
                let height = body.get("height").and_then(|v| v.as_u64()).unwrap_or(200);
                let strategy = body
                    .get("strategy")
                    .and_then(|v| v.as_str())
                    .unwrap_or("scale-to-fit");
                let output_format = body
                    .get("outputFormat")
                    .and_then(|v| v.as_str())
                    .unwrap_or("jpeg");
                let input_format = body
                    .get("format")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| detect_format(&image_bytes).to_string());

                tracing::info!(
                    format = %input_format,
                    output_format = %output_format,
                    width = width,
                    height = height,
                    strategy = %strategy,
                    "processing thumbnail request"
                );

                // Create input + output channels for the resize worker
                let input_channel = iii.create_channel(None).await?;
                let output_channel = iii.create_channel(None).await?;

                // Write image bytes to the input channel then close it
                input_channel.writer.write(&image_bytes).await?;
                input_channel.writer.close().await?;

                // Trigger the resize function with channel refs + metadata
                let trigger_result = iii
                    .trigger(TriggerRequest {
                        function_id: "image_resize::resize".to_string(),
                        payload: json!({
                            "input_channel": input_channel.reader_ref,
                            "output_channel": output_channel.writer_ref,
                            "metadata": {
                                "format": input_format,
                                "output_format": output_format,
                                "width": 0,
                                "height": 0,
                                "target_width": width,
                                "target_height": height,
                                "strategy": strategy,
                            }
                        }),
                        action: None,
                        timeout_ms: Some(30_000),
                    })
                    .await;

                // Read thumbnail bytes from the output channel
                let mut thumbnail_bytes: Vec<u8> = Vec::new();
                loop {
                    match output_channel.reader.next_binary().await? {
                        Some(chunk) => thumbnail_bytes.extend_from_slice(&chunk),
                        None => break,
                    }
                }

                // Wait for trigger to complete
                let _result = trigger_result?;

                let thumbnail_b64 = BASE64.encode(&thumbnail_bytes);

                tracing::info!(
                    size = thumbnail_bytes.len(),
                    "thumbnail generated"
                );

                Ok(json!({
                    "status_code": 200,
                    "body": {
                        "thumbnail": thumbnail_b64,
                        "format": output_format,
                        "width": width,
                        "height": height,
                        "size": thumbnail_bytes.len(),
                    },
                    "headers": { "Content-Type": "application/json" }
                }))
            }
        },
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".to_string(),
        function_id: "api::post::/thumbnail".to_string(),
        config: json!({ "api_path": "/thumbnail", "http_method": "POST" }),
    })?;

    tracing::info!("registered api::post::/thumbnail");

    // ── Wait for shutdown ───────────────────────────────────────────────────

    tracing::info!("image-resize-demo-rust started, waiting for invocations...");

    tokio::signal::ctrl_c().await?;

    tracing::info!("shutting down");
    iii.shutdown_async().await;

    Ok(())
}
