use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{
    extract_channel_refs, ChannelReader, ChannelWriter, IIIError, StreamChannelRef,
};
use serde::Deserialize;
use serde_json::Value;

use crate::config::ResizeConfig;
use crate::processing::{ImageMetadata, ThumbnailMetadata, resolve_params, process_image};

// ---------------------------------------------------------------------------
// ResizeRequest
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug)]
#[allow(dead_code)] // Used in tests; available for future typed deserialization
pub struct ResizeRequest {
    pub input_channel: StreamChannelRef,
    pub output_channel: StreamChannelRef,
    pub metadata: Option<ImageMetadata>,
}

// ---------------------------------------------------------------------------
// handle_resize
// ---------------------------------------------------------------------------

pub async fn handle_resize(
    engine_ws_base: &str,
    config: Arc<ResizeConfig>,
    input_ref: &StreamChannelRef,
    output_ref: &StreamChannelRef,
    metadata_override: Option<ImageMetadata>,
) -> Result<Value, IIIError> {
    let reader = ChannelReader::new(engine_ws_base, input_ref);
    let writer = ChannelWriter::new(engine_ws_base, output_ref);

    let (metadata, image_bytes) = if let Some(meta) = metadata_override {
        // metadata provided inline: read binary directly
        let bytes = reader.read_all().await?;
        (meta, bytes)
    } else {
        // read text message first (metadata JSON), then binary
        let metadata_holder: Arc<std::sync::Mutex<Option<String>>> =
            Arc::new(std::sync::Mutex::new(None));
        let holder_clone = metadata_holder.clone();

        reader
            .on_message(move |msg| {
                let holder = holder_clone.clone();
                // on_message callback is sync — use std::sync::Mutex
                let mut guard = holder.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(msg);
                }
            })
            .await;

        let binary = reader.next_binary().await?;
        let image_bytes = binary.ok_or_else(|| {
            IIIError::Handler("stream closed before binary frame".to_string())
        })?;

        // Retrieve the metadata text that was dispatched to the callback
        let meta_json = {
            let guard = metadata_holder.lock().unwrap();
            guard.clone().ok_or_else(|| {
                IIIError::Handler("no metadata text frame received".to_string())
            })?
        };

        let meta: ImageMetadata = serde_json::from_str(&meta_json)
            .map_err(|e| IIIError::Handler(format!("invalid metadata JSON: {e}")))?;

        (meta, image_bytes)
    };

    process_and_write(config, metadata, image_bytes, &writer, &reader).await
}

// ---------------------------------------------------------------------------
// process_and_write
// ---------------------------------------------------------------------------

pub async fn process_and_write(
    config: Arc<ResizeConfig>,
    metadata: ImageMetadata,
    image_bytes: Vec<u8>,
    writer: &ChannelWriter,
    reader: &ChannelReader,
) -> Result<Value, IIIError> {
    tracing::info!(
        format = %metadata.format,
        output_format = ?metadata.output_format,
        "received metadata"
    );

    let params = resolve_params(&config, &metadata)
        .map_err(|e| IIIError::Handler(format!("param resolution failed: {e}")))?;

    tracing::info!(
        input_format = ?params.input_format,
        output_format = ?params.output_format,
        "resolved params"
    );

    let strategy_str = match params.strategy {
        crate::config::ResizeStrategy::ScaleToFit => "scale-to-fit",
        crate::config::ResizeStrategy::CropToFit => "crop-to-fit",
    }
    .to_string();

    let out_format = match params.output_format {
        image::ImageFormat::Jpeg => "jpeg",
        image::ImageFormat::Png => "png",
        image::ImageFormat::WebP => "webp",
        _ => "unknown",
    }
    .to_string();

    let thumbnail_bytes = tokio::task::spawn_blocking(move || process_image(&image_bytes, &params))
        .await
        .map_err(|e| IIIError::Handler(format!("spawn_blocking join error: {e}")))?
        .map_err(|e| IIIError::Handler(format!("image processing failed: {e}")))?;

    // Decode thumbnail to get output dimensions
    let output_img = image::load_from_memory(&thumbnail_bytes)
        .map_err(|e| IIIError::Handler(format!("failed to decode output image: {e}")))?;

    let thumb_meta = ThumbnailMetadata {
        format: out_format,
        width: output_img.width(),
        height: output_img.height(),
        strategy: strategy_str,
    };

    // Send metadata text frame
    let meta_json = serde_json::to_string(&thumb_meta)
        .map_err(|e| IIIError::Handler(format!("failed to serialize thumbnail metadata: {e}")))?;
    writer.send_message(&meta_json).await?;

    // Send binary thumbnail frame
    writer.write(&thumbnail_bytes).await?;

    // Close both channels
    writer.close().await?;
    reader.close().await?;

    let result = serde_json::to_value(&thumb_meta)
        .map_err(|e| IIIError::Handler(format!("failed to convert metadata to Value: {e}")))?;

    Ok(result)
}

// ---------------------------------------------------------------------------
// build_handler
// ---------------------------------------------------------------------------

pub fn build_handler(
    engine_ws_base: String,
    config: Arc<ResizeConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>> + Send + Sync + 'static
{
    move |payload: Value| {
        let engine_ws_base = engine_ws_base.clone();
        let config = config.clone();

        Box::pin(async move {
            let refs = extract_channel_refs(&payload);

            let input_ref = refs
                .iter()
                .find(|(name, _)| name == "input_channel")
                .map(|(_, r)| r.clone())
                .ok_or_else(|| IIIError::Handler("missing input_channel ref".to_string()))?;

            let output_ref = refs
                .iter()
                .find(|(name, _)| name == "output_channel")
                .map(|(_, r)| r.clone())
                .ok_or_else(|| IIIError::Handler("missing output_channel ref".to_string()))?;

            let metadata: Option<ImageMetadata> = payload
                .get("metadata")
                .and_then(|v| serde_json::from_value(v.clone()).ok());

            handle_resize(&engine_ws_base, config, &input_ref, &output_ref, metadata).await
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use iii_sdk::ChannelDirection;
    use serde_json::json;

    fn make_channel_ref(id: &str, dir: &str) -> Value {
        json!({
            "channel_id": id,
            "access_key": "test-key",
            "direction": dir,
        })
    }

    #[test]
    fn test_resize_request_deserializes() {
        let payload = json!({
            "input_channel": {
                "channel_id": "ch-in-001",
                "access_key": "key-abc",
                "direction": "read"
            },
            "output_channel": {
                "channel_id": "ch-out-001",
                "access_key": "key-def",
                "direction": "write"
            }
        });

        let req: ResizeRequest = serde_json::from_value(payload).unwrap();
        assert_eq!(req.input_channel.channel_id, "ch-in-001");
        assert_eq!(req.output_channel.channel_id, "ch-out-001");
        assert!(req.metadata.is_none());
    }

    #[test]
    fn test_resize_request_with_metadata_override() {
        let payload = json!({
            "input_channel": {
                "channel_id": "ch-in-002",
                "access_key": "key-abc",
                "direction": "read"
            },
            "output_channel": {
                "channel_id": "ch-out-002",
                "access_key": "key-def",
                "direction": "write"
            },
            "metadata": {
                "format": "jpeg",
                "width": 1920,
                "height": 1080
            }
        });

        let req: ResizeRequest = serde_json::from_value(payload).unwrap();
        assert_eq!(req.input_channel.channel_id, "ch-in-002");
        assert_eq!(req.output_channel.channel_id, "ch-out-002");

        let meta = req.metadata.unwrap();
        assert_eq!(meta.format, "jpeg");
        assert_eq!(meta.width, 1920);
        assert_eq!(meta.height, 1080);
    }

    #[test]
    fn test_channel_refs_extracted_from_payload() {
        let payload = json!({
            "input_channel": make_channel_ref("ch-in-003", "read"),
            "output_channel": make_channel_ref("ch-out-003", "write"),
            "some_other_field": "value"
        });

        let refs = extract_channel_refs(&payload);

        let input = refs.iter().find(|(name, _)| name == "input_channel");
        let output = refs.iter().find(|(name, _)| name == "output_channel");

        assert!(input.is_some(), "should find input_channel ref");
        assert!(output.is_some(), "should find output_channel ref");

        let (_, input_ref) = input.unwrap();
        let (_, output_ref) = output.unwrap();

        assert_eq!(input_ref.channel_id, "ch-in-003");
        assert_eq!(output_ref.channel_id, "ch-out-003");

        assert!(matches!(input_ref.direction, ChannelDirection::Read));
        assert!(matches!(output_ref.direction, ChannelDirection::Write));
    }
}
