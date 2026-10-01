use super::*;
use std::io::{self, Cursor, Seek, SeekFrom, Write};
use wacore::download::{DownloadWriter, MediaType};
use whatsapp_rust::{download::DownloadParams, media as native_media, upload::UploadResponse};

#[derive(Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    Document,
    Sticker,
}

impl MediaKind {
    pub(super) fn native(self) -> MediaType {
        match self {
            Self::Image => MediaType::Image,
            Self::Video => MediaType::Video,
            Self::Audio => MediaType::Audio,
            Self::Document => MediaType::Document,
            Self::Sticker => MediaType::Sticker,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UploadRequest {
    pub session_id: String,
    pub media_type: MediaKind,
    pub data_base64: String,
}

#[derive(Serialize, JsonSchema)]
pub struct UploadedMedia {
    pub direct_path: String,
    pub url: String,
    pub media_key_base64: String,
    pub file_sha256_base64: String,
    pub file_enc_sha256_base64: String,
    pub file_length: u64,
    pub media_key_timestamp: i64,
    pub media_type: MediaKind,
}

impl UploadedMedia {
    fn from_upload(value: UploadResponse, media_type: MediaKind) -> Self {
        Self {
            direct_path: value.direct_path,
            url: value.url,
            media_key_base64: BASE64.encode(value.media_key),
            file_sha256_base64: BASE64.encode(value.file_sha256),
            file_enc_sha256_base64: BASE64.encode(value.file_enc_sha256),
            file_length: value.file_length,
            media_key_timestamp: value.media_key_timestamp,
            media_type,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaContent {
    pub media_type: MediaKind,
    pub data_base64: String,
    pub mimetype: Option<String>,
    pub caption: Option<String>,
    pub file_name: Option<String>,
    pub thumbnail_base64: Option<String>,
    pub duration_seconds: Option<u32>,
    /// Audio is already encoded; the worker never invokes codecs.
    #[serde(default)]
    pub voice_note: bool,
    #[serde(default)]
    pub gif_playback: bool,
}

impl MediaContent {
    pub(super) fn validate(&self, max: usize) -> Result<(Vec<u8>, Option<Vec<u8>>)> {
        optional_text(self.caption.as_deref(), "caption", MAX_TEXT)?;
        optional_text(self.file_name.as_deref(), "file_name", 255)?;
        if let Some(mime) = &self.mimetype {
            validate::text(mime, "mimetype", 200)?;
            ensure!(
                mime.contains('/') && !mime.contains(['\r', '\n']),
                "invalid mimetype"
            );
            let prefix = match self.media_type {
                MediaKind::Image => Some("image/"),
                MediaKind::Video => Some("video/"),
                MediaKind::Audio => Some("audio/"),
                MediaKind::Sticker => Some("image/webp"),
                MediaKind::Document => None,
            };
            if let Some(prefix) = prefix {
                ensure!(
                    mime.starts_with(prefix),
                    "mimetype does not match media_type"
                );
            }
        }
        ensure!(
            !self.voice_note || matches!(self.media_type, MediaKind::Audio),
            "voice_note requires audio"
        );
        ensure!(
            !self.gif_playback || matches!(self.media_type, MediaKind::Video),
            "gif_playback requires video"
        );
        if matches!(self.media_type, MediaKind::Audio | MediaKind::Sticker) {
            ensure!(
                self.caption.is_none(),
                "captions are supported only for images, videos and documents"
            );
        }
        let data = validate::media(&self.data_base64, max)?;
        let thumbnail = self
            .thumbnail_base64
            .as_deref()
            .map(|t| validate::media(t, max.min(512 * 1024)))
            .transpose()?;
        Ok((data, thumbnail))
    }

    pub(super) fn message(
        &self,
        upload: UploadResponse,
        thumbnail: Option<Vec<u8>>,
    ) -> wa::Message {
        match self.media_type {
            MediaKind::Image => native_media::image_message(
                upload,
                native_media::ImageOptions {
                    caption: self.caption.clone(),
                    mimetype: self.mimetype.clone(),
                    jpeg_thumbnail: thumbnail,
                    ..Default::default()
                },
            ),
            MediaKind::Video => native_media::video_message(
                upload,
                native_media::VideoOptions {
                    caption: self.caption.clone(),
                    mimetype: self.mimetype.clone(),
                    jpeg_thumbnail: thumbnail,
                    duration_seconds: self.duration_seconds,
                    gif_playback: Some(self.gif_playback),
                    ..Default::default()
                },
            ),
            MediaKind::Audio => native_media::audio_message(
                upload,
                native_media::AudioOptions {
                    mimetype: self.mimetype.clone(),
                    duration_seconds: self.duration_seconds,
                    ptt: Some(self.voice_note),
                    ..Default::default()
                },
            ),
            MediaKind::Document => native_media::document_message(
                upload,
                native_media::DocumentOptions {
                    mimetype: self.mimetype.clone(),
                    file_name: self.file_name.clone(),
                    caption: self.caption.clone(),
                    jpeg_thumbnail: thumbnail,
                    ..Default::default()
                },
            ),
            MediaKind::Sticker => wa::Message {
                sticker_message: wa::message::StickerMessage {
                    url: Some(upload.url),
                    direct_path: Some(upload.direct_path),
                    media_key: Some(upload.media_key.to_vec()),
                    file_sha256: Some(upload.file_sha256.to_vec()),
                    file_enc_sha256: Some(upload.file_enc_sha256.to_vec()),
                    file_length: Some(upload.file_length),
                    media_key_timestamp: Some(upload.media_key_timestamp),
                    mimetype: Some("image/webp".into()),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendMediaRequest {
    pub session_id: String,
    pub chat_id: String,
    pub media: MediaContent,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DownloadRequest {
    pub session_id: String,
    pub media_type: MediaKind,
    pub direct_path: String,
    /// Omit both encryption fields only for plaintext channel media.
    pub media_key_base64: Option<String>,
    pub file_sha256_base64: String,
    pub file_enc_sha256_base64: Option<String>,
    pub file_length: u64,
}

impl DownloadRequest {
    pub(super) fn params(&self, max: usize) -> Result<DownloadParams> {
        let path = &self.direct_path;
        ensure!(
            path.len() <= 8192 && (path.starts_with("/mms/") || path.starts_with("/v/")),
            "direct_path must be a WhatsApp /mms/ or /v/ CDN path"
        );
        ensure!(
            !path.contains(['\\', '#'])
                && !path.chars().any(|c| c.is_control() || c.is_whitespace()),
            "invalid CDN direct_path"
        );
        ensure!(
            self.file_length > 0 && self.file_length <= max as u64,
            "file_length exceeds the media size limit or is zero"
        );
        let file_sha256 = fixed_bytes(&self.file_sha256_base64, "file_sha256_base64")?.to_vec();
        ensure!(
            self.media_key_base64.is_some() == self.file_enc_sha256_base64.is_some(),
            "media_key_base64 and file_enc_sha256_base64 must be supplied together"
        );
        let media_key = self
            .media_key_base64
            .as_deref()
            .map(|v| fixed_bytes(v, "media_key_base64").map(Vec::from))
            .transpose()?;
        let file_enc_sha256 = self
            .file_enc_sha256_base64
            .as_deref()
            .map(|v| fixed_bytes(v, "file_enc_sha256_base64").map(Vec::from))
            .transpose()?;
        Ok(DownloadParams {
            direct_path: path.clone(),
            media_key,
            file_sha256,
            file_enc_sha256,
            file_length: self.file_length,
            media_type: self.media_type.native(),
        })
    }
}

#[derive(Serialize, JsonSchema)]
pub struct DownloadedMedia {
    pub data_base64: String,
    pub file_length: usize,
}

/// Rejects excess bytes during streaming, even if the advertised length lies.
pub(super) struct BoundedWriter {
    buffer: Cursor<Vec<u8>>,
    max: usize,
}

impl BoundedWriter {
    pub(super) fn new(max: usize) -> Self {
        Self {
            buffer: Cursor::new(Vec::new()),
            max,
        }
    }
    pub(super) fn into_bytes(self) -> Vec<u8> {
        self.buffer.into_inner()
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.buffer.position().saturating_add(bytes.len() as u64) > self.max as u64 {
            return Err(io::Error::other("download exceeds the media size limit"));
        }
        self.buffer.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for BoundedWriter {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.buffer.position()) + i128::from(n),
            SeekFrom::End(n) => self.buffer.get_ref().len() as i128 + i128::from(n),
        };
        if position < 0 || position > self.max as i128 {
            return Err(io::Error::other("media seek exceeds the size limit"));
        }
        self.buffer.set_position(position as u64);
        Ok(position as u64)
    }
}

impl DownloadWriter for BoundedWriter {
    fn truncate(&mut self, len: u64) -> io::Result<()> {
        if len > self.max as u64 {
            return Err(io::Error::other("media truncation exceeds the size limit"));
        }
        self.buffer.get_mut().truncate(len as usize);
        Ok(())
    }
}

pub(super) fn register(registry: &mut Registry) {
    registry.add::<UploadRequest, UploadedMedia, _, _>(
        "whatsapp::media::upload",
        "Encrypt and upload bounded base64 media to WhatsApp's CDN.",
        |m, r| async move {
            let bytes = validate::media(&r.data_base64, m.config().max_media_bytes)?;
            let result = m
                .client(&r.session_id)
                .await?
                .upload(bytes, r.media_type.native(), Default::default())
                .await?;
            Ok(UploadedMedia::from_upload(result, r.media_type))
        },
    );
    registry.add::<SendMediaRequest, SentMessage, _, _>("whatsapp::media::send", "Upload and send encoded image/video/audio/document/WebP media; no URL fetching or codec processes.", |m, r| async move {
        let to = validate::jid(&r.chat_id)?;
        ensure!(to.is_pn() || to.is_lid() || to.is_group(), "encrypted media sends require a direct or group chat; use status::send_media for an explicit status audience; channel upload is not supported");
        let (data, thumbnail) = r.media.validate(m.config().max_media_bytes)?;
        let client = m.client(&r.session_id).await?;
        let upload = client.upload(data, r.media.media_type.native(), Default::default()).await?;
        let message = r.media.message(upload, thumbnail);
        Ok(client.send_message(to, message).await?.into())
    });
    registry.add::<DownloadRequest, DownloadedMedia, _, _>(
        "whatsapp::media::download",
        "Download bounded media through native CDN routing and verify its supplied hashes.",
        |m, r| async move {
            let params = r.params(m.config().max_media_bytes)?;
            let sink = BoundedWriter::new(m.config().max_media_bytes);
            let client = m.client(&r.session_id).await?;
            ensure!(
                client.http_client.supports_streaming(),
                "media download requires a streaming HTTP client"
            );
            let bytes = client
                .download_from_params_to_writer(&params, sink)
                .await?
                .into_bytes();
            ensure!(
                bytes.len() as u64 == r.file_length,
                "downloaded length differs from file_length"
            );
            Ok(DownloadedMedia {
                file_length: bytes.len(),
                data_base64: BASE64.encode(bytes),
            })
        },
    );
}
