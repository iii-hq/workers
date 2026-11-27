//! The `image_resize::resize` handler, driven in-process through local streams.

use std::collections::BTreeSet;
use std::fmt::Debug;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream;
use iii_sdk::protocol::ChannelItem;
use iii_sdk::{Error, InputStream, OutputStream};
use image::{DynamicImage, ImageFormat, RgbImage};
use image_resize::config::{ResizeConfig, ResizeStrategy};
use image_resize::handler::{ResizeRequest, read_image, resize};
use image_resize::processing::{Format, ThumbnailMetadata};
use image_resize::shutdown::InFlight;
use serde_json::{Map, Value, json};
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;

fn encode(img: &DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), format).unwrap();
    buf
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = RgbImage::from_pixel(w, h, image::Rgb([10, 20, 30]));
    encode(&DynamicImage::ImageRgb8(img), ImageFormat::Png)
}

fn noise(side: u32, format: ImageFormat) -> Vec<u8> {
    let mut seed: u32 = 7;
    let img = RgbImage::from_fn(side, side, |_, _| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let [_, r, g, b] = seed.to_le_bytes();
        image::Rgb([r, g, b])
    });
    encode(&DynamicImage::ImageRgb8(img), format)
}

/// A 64x64 baseline JPEG whose APP1 holds a fake EXIF thumbnail, EOI and all.
fn jpeg_with_thumbnail() -> Vec<u8> {
    let jpeg = noise(64, ImageFormat::Jpeg);
    let app1 = [
        b"\xFF\xE1\x00\x0CExif\x00\x00".as_slice(),
        b"\xFF\xD8\xFF\xD9",
    ]
    .concat();
    [&jpeg[..2], &app1, &jpeg[2..]].concat()
}

struct Call {
    result: Result<ThumbnailMetadata, Error>,
    /// What the output's byte sink received.
    bytes: Vec<u8>,
    /// The output's Debug, which names its state ("closed", "aborted: ...").
    output: String,
}

async fn call_with(
    input: InputStream,
    metadata: Option<Value>,
    permits: Arc<Semaphore>,
    calls: Arc<InFlight>,
    rest: Map<String, Value>,
) -> Call {
    let (writer, mut reader) = tokio::io::duplex(8 << 20);
    let output_channel = OutputStream::to_writer(writer);
    let probe = output_channel.clone();
    let req = ResizeRequest {
        input_channel: input,
        output_channel,
        metadata,
        rest,
    };
    let result = resize(Arc::new(ResizeConfig::default()), permits, calls, req).await;
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), reader.read_to_end(&mut bytes))
        .await
        .expect("the handler returned with output_channel neither closed nor aborted")
        .unwrap();
    Call {
        result,
        bytes,
        output: format!("{probe:?}"),
    }
}

async fn call(input: Vec<u8>, metadata: Option<Value>) -> Call {
    call_with(
        InputStream::from_bytes(input),
        metadata,
        Arc::new(Semaphore::new(1)),
        Arc::default(),
        Map::new(),
    )
    .await
}

/// The message of an INVALID_ARGS error, which carries no stacktrace.
fn invalid_args<T: Debug>(result: Result<T, Error>) -> String {
    match result {
        Err(Error::Remote {
            code,
            message,
            stacktrace: None,
        }) if code == "INVALID_ARGS" => message,
        other => panic!("expected INVALID_ARGS, got {other:?}"),
    }
}

/// The message of a failed call, after checking that it aborted the output.
fn rejected(call: Call) -> String {
    assert!(call.output.contains("aborted"), "{}", call.output);
    invalid_args(call.result)
}

fn decoded_dims(bytes: &[u8]) -> (u32, u32) {
    // The image crate here has no WebP decoder.
    if image::guess_format(bytes).unwrap() == ImageFormat::WebP {
        let (mut w, mut h) = (0, 0);
        // SAFETY: libwebp reads at most `bytes.len()` bytes and writes only `w` and `h`.
        let ok = unsafe { libwebp_sys::WebPGetInfo(bytes.as_ptr(), bytes.len(), &mut w, &mut h) };
        assert_ne!(ok, 0, "not a WebP libwebp can read");
        return (w as u32, h as u32);
    }
    let img = image::load_from_memory(bytes).unwrap();
    (img.width(), img.height())
}

#[tokio::test]
async fn read_image_joins_every_binary_frame() {
    let frames = stream::iter(vec![
        Ok(ChannelItem::Binary(vec![1, 2])),
        Ok(ChannelItem::Binary(vec![3])),
        Ok(ChannelItem::Binary(vec![4, 5])),
    ]);
    assert_eq!(read_image(frames, 16).await.unwrap(), [1, 2, 3, 4, 5]);
}

#[tokio::test]
async fn read_image_rejects_text_frames() {
    let frames = stream::iter(vec![
        Ok(ChannelItem::Text(r#"{"format":"png"}"#.into())),
        Ok(ChannelItem::Binary(vec![1])),
    ]);
    let message = invalid_args(read_image(frames, 16).await);
    assert!(message.contains("metadata"), "{message}");
}

#[tokio::test]
async fn read_image_rejects_empty_input() {
    for frames in [vec![], vec![Ok(ChannelItem::Binary(vec![]))]] {
        let message = invalid_args(read_image(stream::iter(frames), 16).await);
        assert!(message.contains("no image bytes"), "{message}");
    }
}

#[tokio::test]
async fn read_image_enforces_cap() {
    let frames = || {
        stream::iter(vec![
            Ok(ChannelItem::Binary(vec![0; 10])),
            Ok(ChannelItem::Binary(vec![0; 7])),
        ])
    };
    let message = invalid_args(read_image(frames(), 16).await);
    assert!(message.contains("exceeds 16 bytes"), "{message}");
    assert_eq!(read_image(frames(), 17).await.unwrap().len(), 17);
}

#[tokio::test]
async fn large_input_in_64k_frames_resizes() {
    let input = noise(300, ImageFormat::Png);
    assert!(
        input.len() > 65_536,
        "{} bytes fit in one frame",
        input.len()
    );
    let call = call_with(
        InputStream::from_reader(Cursor::new(input)),
        Some(json!({ "strategy": "crop-to-fit", "target_width": 64, "target_height": 64 })),
        Arc::new(Semaphore::new(1)),
        Arc::default(),
        Map::new(),
    )
    .await;
    let result = call.result.unwrap();
    assert_eq!((result.width, result.height), (64, 64));
    assert_eq!(decoded_dims(&call.bytes), (64, 64));
}

#[tokio::test]
async fn metadata_is_optional() {
    for metadata in [None, Some(Value::Null)] {
        let call = call(png(300, 200), metadata.clone()).await;
        let result = call.result.unwrap();
        assert_eq!(result.format, Format::Png, "{metadata:?}");
        assert_eq!((result.width, result.height), (200, 133), "{metadata:?}");
        assert_eq!(decoded_dims(&call.bytes), (200, 133), "{metadata:?}");
    }
}

#[tokio::test]
async fn sniffs_input_format() {
    let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3])));
    let webp = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-fixtures/sample.webp"
    ))
    .unwrap();
    for (input, format, expected) in [
        (
            encode(&img, ImageFormat::Jpeg),
            ImageFormat::Jpeg,
            Format::Jpeg,
        ),
        (
            encode(&img, ImageFormat::Png),
            ImageFormat::Png,
            Format::Png,
        ),
        (webp, ImageFormat::WebP, Format::Webp),
    ] {
        let call = call(input, None).await;
        assert_eq!(call.result.unwrap().format, expected);
        assert_eq!(image::guess_format(&call.bytes).unwrap(), format);
    }
}

#[tokio::test]
async fn gif_input_is_invalid_args() {
    let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec();
    let message = rejected(call(gif, None).await);
    assert!(message.contains("Gif"), "{message}");
}

#[tokio::test]
async fn bad_metadata_is_invalid_args_and_aborts_output() {
    for metadata in [
        json!({ "quality": 300 }),
        json!({ "strategy": "crop" }),
        json!({ "output_format": "jpg" }),
    ] {
        let message = rejected(call(png(4, 4), Some(metadata.clone())).await);
        assert!(
            message.contains("invalid metadata"),
            "{metadata}: {message}"
        );
    }
    // No input bytes: only a check that runs before the input is read can report the metadata.
    let message = rejected(call(Vec::new(), Some(json!({ "strategy": "crop" }))).await);
    assert!(message.contains("invalid metadata"), "{message}");
    // The same holds for an out-of-range quality.
    let message = rejected(call(Vec::new(), Some(json!({ "quality": 0 }))).await);
    assert!(message.contains("quality must be 1-100"), "{message}");
}

#[tokio::test]
async fn metadata_must_be_an_object() {
    // Serde would read this array positionally as a 12x12 PNG request.
    for metadata in [json!(["png", 80, "scale-to-fit", 12, 12]), json!("png")] {
        let message = rejected(call(png(40, 40), Some(metadata.clone())).await);
        assert!(
            message.contains("invalid metadata: expected an object"),
            "{metadata}: {message}"
        );
    }
}

#[tokio::test]
async fn top_level_options_are_invalid_args() {
    for (key, value) in [("target_width", json!(64)), ("metdata", json!({}))] {
        let rest = Map::from_iter([(key.to_string(), value)]);
        let call = call_with(
            InputStream::from_bytes(png(400, 400)),
            None,
            Arc::new(Semaphore::new(1)),
            Arc::default(),
            rest,
        )
        .await;
        let message = rejected(call);
        assert!(
            message.contains(&format!("unknown request key `{key}`")),
            "{key}: {message}"
        );
    }
}

#[tokio::test]
async fn unknown_metadata_keys_are_invalid_args_naming_the_key() {
    for (key, value) in [
        ("targetWidth", json!(64)),
        ("outputFormat", json!("png")),
        ("source_width", json!(100)),
    ] {
        let message = rejected(call(png(4, 4), Some(json!({ key: value }))).await);
        assert!(
            message.contains(&format!("unknown field `{key}`")),
            "{key}: {message}"
        );
    }
}

#[tokio::test]
async fn removed_metadata_keys_are_invalid_args_naming_the_key() {
    for (key, value) in [
        ("format", json!("jpeg")),
        ("width", json!(50)),
        ("height", json!(50)),
    ] {
        let message = rejected(call(png(4, 4), Some(json!({ key: value }))).await);
        assert!(
            message.contains(&format!("unknown field `{key}`")),
            "{key}: {message}"
        );
    }
}

#[tokio::test]
async fn undecodable_input_is_invalid_args_and_aborts_output() {
    let truncated_png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[0; 32]].concat();
    let message = rejected(call(truncated_png, None).await);
    assert!(message.contains("decode"), "{message}");

    let message = rejected(call(b"not an image".to_vec(), None).await);
    assert!(message.contains("not a recognized image"), "{message}");
}

#[tokio::test]
async fn jpeg_cut_inside_its_scan_is_invalid_args() {
    for jpeg in [noise(64, ImageFormat::Jpeg), jpeg_with_thumbnail()] {
        let message = rejected(call(jpeg[..jpeg.len() / 2].to_vec(), None).await);
        assert!(message.contains("truncated"), "{message}");
    }
}

#[tokio::test]
async fn complete_jpeg_resizes_despite_trailing_bytes_or_an_eoi_in_app1() {
    let trailing = [noise(64, ImageFormat::Jpeg), b"garbage\xFF\xD8".to_vec()].concat();
    for jpeg in [trailing, jpeg_with_thumbnail()] {
        let result = call(jpeg, None).await.result.unwrap();
        assert_eq!((result.source_width, result.source_height), (64, 64));
    }
}

#[tokio::test]
async fn quality_out_of_range_is_invalid_args() {
    for quality in [0, 101] {
        let metadata = json!({ "output_format": "jpeg", "quality": quality });
        let message = rejected(call(png(4, 4), Some(metadata)).await);
        assert!(message.contains("quality must be 1-100"), "{message}");
    }
}

#[tokio::test]
async fn zero_target_is_invalid_args() {
    for key in ["target_width", "target_height", "max_pixels", "max_bytes"] {
        let message = rejected(call(Vec::new(), Some(json!({ key: 0 }))).await);
        assert!(message.contains("invalid metadata"), "{key}: {message}");
    }
}

#[tokio::test]
async fn huge_target_on_tiny_image() {
    for (strategy, dims) in [("scale-to-fit", (10, 8)), ("crop-to-fit", (8, 8))] {
        let metadata = json!({
            "strategy": strategy,
            "target_width": 4_000_000_000_u32,
            "target_height": 4_000_000_000_u32,
        });
        let call = call(png(10, 8), Some(metadata)).await;
        let result = call.result.unwrap();
        assert_eq!((result.width, result.height), dims, "{strategy}");
        assert_eq!(decoded_dims(&call.bytes), dims, "{strategy}");
    }
}

#[tokio::test]
async fn result_reports_output_and_source_dims() {
    let metadata = json!({ "target_width": 150, "target_height": 150 });
    let call = call(png(300, 200), Some(metadata)).await;
    assert_eq!(
        call.result.unwrap(),
        ThumbnailMetadata {
            format: Format::Png,
            width: 150,
            height: 100,
            source_width: 300,
            source_height: 200,
            strategy: ResizeStrategy::ScaleToFit,
            quality: None,
        }
    );
    assert_eq!(decoded_dims(&call.bytes), (150, 100));
    assert!(call.output.contains("closed"), "{}", call.output);
}

#[tokio::test]
async fn max_pixels_shrinks_past_the_box_keeping_the_aspect() {
    for (metadata, dims) in [
        (json!({ "max_pixels": 10_000 }), (115, 86)),
        (
            json!({ "target_width": 1000, "target_height": 1000, "max_pixels": 10_000 }),
            (115, 86),
        ),
        (
            json!({ "strategy": "crop-to-fit", "target_width": 100, "target_height": 100, "max_pixels": 2_500 }),
            (50, 50),
        ),
        (json!({ "max_pixels": 30_000 }), (200, 150)),
        (json!({ "max_pixels": 1 }), (1, 1)),
    ] {
        let call = call(png(400, 300), Some(metadata.clone())).await;
        let result = call.result.unwrap();
        assert_eq!((result.width, result.height), dims, "{metadata}");
        assert_eq!(decoded_dims(&call.bytes), dims, "{metadata}");
        assert_eq!((result.source_width, result.source_height), (400, 300));
    }
}

#[tokio::test]
async fn max_bytes_lowers_the_quality_then_shrinks_lossy_output() {
    let input = noise(300, ImageFormat::Jpeg);
    for (format, default_quality) in [("jpeg", 85), ("webp", 80)] {
        let plain = call(input.clone(), Some(json!({ "output_format": format }))).await;
        assert_eq!(plain.result.unwrap().quality, Some(default_quality));
        let full = plain.bytes.len();
        for (cap, shrinks) in [(full * 4 / 5, false), (full / 5, true)] {
            let metadata = json!({ "output_format": format, "max_bytes": cap });
            let call = call(input.clone(), Some(metadata.clone())).await;
            let result = call.result.unwrap();
            let size = (result.width, result.height);
            assert!(
                call.bytes.len() <= cap,
                "{metadata}: {} bytes",
                call.bytes.len()
            );
            assert_eq!(decoded_dims(&call.bytes), size, "{metadata}");
            assert!(
                result.quality < Some(default_quality),
                "{metadata}: {result:?}"
            );
            assert_eq!(size != (200, 200), shrinks, "{metadata}: {result:?}");
            assert_eq!(result.width, result.height, "{metadata}: {result:?}");
        }
    }
}

#[tokio::test]
async fn max_bytes_shrinks_png_output() {
    let input = noise(300, ImageFormat::Png);
    let full = call(input.clone(), None).await.bytes.len();
    let cap = full / 3;
    let call = call(input, Some(json!({ "max_bytes": cap }))).await;
    let result = call.result.unwrap();
    assert!(call.bytes.len() <= cap, "{} bytes", call.bytes.len());
    assert!(
        result.width < 200 && result.width == result.height,
        "{result:?}"
    );
    assert_eq!(decoded_dims(&call.bytes), (result.width, result.height));
    assert_eq!(result.quality, None);
}

#[tokio::test]
async fn max_bytes_no_output_meets_is_invalid_args() {
    for format in ["jpeg", "png", "webp"] {
        let metadata = json!({ "output_format": format, "max_bytes": 10 });
        let message = rejected(call(png(400, 300), Some(metadata)).await);
        assert!(message.contains("max_bytes (10)"), "{format}: {message}");
    }
}

#[tokio::test]
async fn permits_gate_processing() {
    let permits = Arc::new(Semaphore::new(0));
    let pending = tokio::spawn(call_with(
        InputStream::from_bytes(png(20, 20)),
        None,
        permits.clone(),
        Arc::default(),
        Map::new(),
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!pending.is_finished(), "processed without a permit");

    permits.add_permits(1);
    let call = tokio::time::timeout(Duration::from_secs(30), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call.result.unwrap().width, 20);
    assert_eq!(permits.available_permits(), 1, "the permit is given back");
}

#[tokio::test]
async fn a_call_counts_as_in_flight_until_it_returns() {
    let (permits, calls) = (Arc::new(Semaphore::new(0)), Arc::new(InFlight::default()));
    let pending = tokio::spawn(call_with(
        InputStream::from_bytes(png(20, 20)),
        None,
        permits.clone(),
        calls.clone(),
        Map::new(),
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        calls.running(),
        1,
        "a call waiting for a permit is in flight"
    );

    permits.add_permits(1);
    let call = tokio::time::timeout(Duration::from_secs(30), pending)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call.result.unwrap().width, 20);
    assert_eq!(
        calls.running(),
        0,
        "the call left the count when it returned"
    );
}

#[tokio::test]
async fn payload_with_engine_keys_deserializes() {
    let req: ResizeRequest = serde_json::from_value(json!({
        "input_channel": { "channel_id": "in-1", "access_key": "k", "direction": "read" },
        "output_channel": { "channel_id": "out-1", "access_key": "k", "direction": "write" },
        "metadata": { "target_width": 64 },
        "_caller_worker_id": "worker-1",
    }))
    .unwrap();
    assert_eq!(req.metadata, Some(json!({ "target_width": 64 })));
    assert_eq!(req.rest.keys().collect::<Vec<_>>(), ["_caller_worker_id"]);

    let call = call_with(
        InputStream::from_bytes(png(20, 20)),
        None,
        Arc::new(Semaphore::new(1)),
        Arc::default(),
        req.rest,
    )
    .await;
    assert_eq!(call.result.unwrap().width, 20);
}

#[tokio::test]
async fn registered_schema_matches_types() {
    // The settings iii-sdk registers request and response schemas with.
    let schema_of = |root| serde_json::to_value(root).unwrap();
    let settings = schemars::r#gen::SchemaSettings::draft07;
    let request = schema_of(
        settings()
            .into_generator()
            .into_root_schema_for::<ResizeRequest>(),
    );
    assert_eq!(
        request["required"],
        json!(["input_channel", "output_channel"])
    );

    let defs = &request["definitions"];
    let meta = &defs["ImageMetadata"];
    assert_eq!(meta["additionalProperties"], json!(false));
    let props = meta["properties"].as_object().unwrap();
    let keys: BTreeSet<&str> = props.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        BTreeSet::from([
            "max_bytes",
            "max_pixels",
            "output_format",
            "quality",
            "strategy",
            "target_height",
            "target_width",
        ])
    );
    assert_eq!(props["quality"]["minimum"].as_f64(), Some(1.0));
    assert_eq!(props["quality"]["maximum"].as_f64(), Some(100.0));
    assert_eq!(props["target_width"]["minimum"].as_f64(), Some(1.0));
    assert_eq!(props["target_height"]["minimum"].as_f64(), Some(1.0));
    assert_eq!(props["max_pixels"]["minimum"].as_f64(), Some(1.0));
    assert_eq!(props["max_bytes"]["minimum"].as_f64(), Some(1.0));
    assert_eq!(defs["Format"]["enum"], json!(["jpeg", "png", "webp"]));
    assert_eq!(
        defs["ResizeStrategy"]["enum"],
        json!(["scale-to-fit", "crop-to-fit"])
    );

    let response = schema_of(
        settings()
            .into_generator()
            .into_root_schema_for::<ThumbnailMetadata>(),
    );
    let required = response["required"].as_array().unwrap();
    for field in ["format", "width", "height", "source_width", "source_height"] {
        assert!(required.contains(&json!(field)), "{field}: {response}");
    }
}
