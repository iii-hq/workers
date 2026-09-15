//! Explicit smoke test: downloads checksum-verified voice files and synthesizes
//! Portuguese WAV, without opening any server audio device.
use base64::Engine as _;
use std::sync::Arc;
use voice::{
    config::{TtsBackend, WorkerConfig},
    events::{Emitter, NoopDeliverer, TriggerSets},
    models,
    tts::Speaker,
};

#[tokio::test]
#[ignore = "requires Piper installed and VOICE_TEST_PIPER_MODELS_DIR; downloads ~63 MB"]
async fn piper_portuguese_returns_browser_audio() {
    let dir =
        std::env::var("VOICE_TEST_PIPER_MODELS_DIR").expect("set an explicit models destination");
    let mut cfg = WorkerConfig {
        models_dir: dir,
        ..WorkerConfig::default()
    };
    cfg.tts.backend = TtsBackend::Piper;
    cfg.tts.piper.device = match std::env::var("VOICE_TEST_PIPER_DEVICE").as_deref() {
        Ok("cpu") => voice::config::PiperDevice::Cpu,
        _ => voice::config::PiperDevice::Auto,
    };
    cfg.tts.piper.command =
        std::env::var("VOICE_TEST_PIPER_COMMAND").unwrap_or_else(|_| "piper".into());
    let model = models::find(&cfg.tts.piper.model).unwrap();
    models::download(model, &cfg.models_path(), None)
        .await
        .unwrap();
    assert!(model.is_installed(&cfg.models_path()));
    let speaker = Speaker::new(Arc::new(Emitter::new(
        TriggerSets::new(),
        Arc::new(NoopDeliverer),
    )));
    let audio = speaker
        .speak(
            &cfg,
            "Olá! Esta é uma voz neural em português brasileiro.",
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(audio.backend, "piper");
    assert!(!audio.played);
    assert_eq!(audio.mime.as_deref(), Some("audio/wav"));
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio.audio_base64.unwrap())
        .unwrap();
    let reader = hound::WavReader::new(std::io::Cursor::new(bytes)).unwrap();
    assert!(reader.duration() > reader.spec().sample_rate);
}
