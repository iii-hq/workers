use std::sync::Arc;

use futures_util::{Stream, StreamExt};
use iii_sdk::protocol::ChannelItem;
use iii_sdk::{Error, InputStream, OutputStream};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::config::{ResizeConfig, check_quality};
use crate::processing::{
    ImageMetadata, ThumbnailMetadata, invalid, process_image, resolve_params, sniff,
};
use crate::shutdown::InFlight;

/// Largest input read from `input_channel`.
pub const MAX_INPUT_BYTES: usize = 64 << 20;

#[derive(Deserialize, JsonSchema)]
pub struct ResizeRequest {
    pub input_channel: InputStream,
    pub output_channel: OutputStream,
    // A Value so a bad one is reported after the handler holds output_channel
    // and can abort it; a typed field would fail inside the SDK before that.
    #[schemars(with = "Option<ImageMetadata>")]
    pub metadata: Option<Value>,
    // Rejected in `run` so output_channel is aborted; unskipped, the schema would allow any key.
    #[serde(flatten)]
    #[schemars(skip)]
    pub rest: serde_json::Map<String, Value>,
}

/// Every binary frame of the input, up to `cap` bytes in all.
pub async fn read_image(
    mut frames: impl Stream<Item = Result<ChannelItem, Error>> + Unpin,
    cap: usize,
) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    while let Some(frame) = frames.next().await {
        match frame? {
            ChannelItem::Binary(chunk) => {
                if bytes.len() + chunk.len() > cap {
                    return Err(invalid(format!("input_channel exceeds {cap} bytes")));
                }
                bytes.extend_from_slice(&chunk);
            }
            ChannelItem::Text(_) => {
                return Err(invalid(
                    "input_channel carries only image bytes; pass options in `metadata`",
                ));
            }
        }
    }
    if bytes.is_empty() {
        return Err(invalid("input_channel carried no image bytes"));
    }
    Ok(bytes)
}

/// The `image_resize::resize` handler: on success output_channel carries the
/// result as one JSON text frame, then the image bytes. Counted in `calls`
/// until it returns.
pub async fn resize(
    config: Arc<ResizeConfig>,
    permits: Arc<Semaphore>,
    calls: Arc<InFlight>,
    req: ResizeRequest,
) -> Result<ThumbnailMetadata, Error> {
    let _call = calls.enter();
    let result = run(&config, permits, &req).await;
    if let Err(error) = &result {
        // The SDK leaves an output the handler never used open on failure; a
        // caller reading it first would wait for the engine's sweep.
        let _ = req.output_channel.abort(error.to_string()).await;
    }
    result
}

async fn run(
    config: &ResizeConfig,
    permits: Arc<Semaphore>,
    req: &ResizeRequest,
) -> Result<ThumbnailMetadata, Error> {
    // The engine adds `_`-prefixed keys such as `_caller_worker_id`.
    if let Some(key) = req.rest.keys().find(|k| !k.starts_with('_')) {
        return Err(invalid(format!(
            "unknown request key `{key}`; per-call options go in `metadata`"
        )));
    }
    let meta = match &req.metadata {
        None | Some(Value::Null) => ImageMetadata::default(),
        // Serde would also take the struct as a positional array.
        Some(value @ Value::Object(_)) => ImageMetadata::deserialize(value)
            .map_err(|e| invalid(format!("invalid metadata: {e}")))?,
        Some(_) => return Err(invalid("invalid metadata: expected an object")),
    };
    // Fail a bad override before waiting on the input stream.
    if let Some(quality) = meta.quality {
        check_quality(quality).map_err(invalid)?;
    }
    let bytes = read_image(req.input_channel.frames(), MAX_INPUT_BYTES).await?;
    let params = resolve_params(config, &meta, sniff(&bytes)?)?;

    let permit = permits
        .acquire_owned()
        .await
        .map_err(|e| Error::Handler(e.to_string()))?;
    let (done, processed) = tokio::sync::oneshot::channel();
    // Not spawn_blocking: a runtime waits for its blocking tasks when it drops,
    // so a resize a stop cuts off would hold the exit until it returns.
    std::thread::Builder::new()
        .name("image-resize".into())
        .spawn(move || {
            let _permit = permit;
            let _ = done.send(process_image(&bytes, &params));
        })
        .map_err(|e| Error::Handler(format!("could not start image processing: {e}")))?;
    let (out, result) = processed
        .await
        .map_err(|_| Error::Handler("image processing panicked".into()))??;

    req.output_channel.send(&result).await?;
    req.output_channel.write(&out).await?;
    req.output_channel.close().await?;
    tracing::debug!(
        width = result.width,
        height = result.height,
        source_width = result.source_width,
        source_height = result.source_height,
        format = ?result.format,
        strategy = ?result.strategy,
        "image resized"
    );
    Ok(result)
}
