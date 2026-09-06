//! The local recognizer against real audio. Needs the default model on disk:
//! set `VOICE_TEST_MODELS_DIR` to a models directory that already holds
//! `zipformer-en-20m/` (or let the test download it by setting
//! `VOICE_TEST_DOWNLOAD=1`). Skipped otherwise, so CI stays offline.

use std::path::PathBuf;

use voice::audio;
use voice::config::WorkerConfig;
use voice::engine::Engine;

fn models_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("VOICE_TEST_MODELS_DIR") {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("VOICE_TEST_DOWNLOAD").map(|_| std::env::temp_dir().join("voice-test-models"))
}

/// A synthetic clip: 1.2 s of room tone, a 440 Hz tone burst, more room tone.
/// The recognizer must not invent words from it, and must not fail on it.
fn tone_clip() -> Vec<f32> {
    let mut out = Vec::new();
    let mut seed: u32 = 7;
    let mut noise = |amp: f32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        ((seed % 2001) as f32 - 1000.0) / 1000.0 * amp
    };
    for _ in 0..19_200 {
        out.push(noise(0.002));
    }
    for i in 0..8_000 {
        out.push((i as f32 * 440.0 * std::f32::consts::TAU / 16_000.0).sin() * 0.3);
    }
    for _ in 0..16_000 {
        out.push(noise(0.002));
    }
    out
}

#[tokio::test]
async fn the_local_engine_loads_and_transcribes() {
    let Some(dir) = models_dir() else {
        eprintln!("skipping: set VOICE_TEST_MODELS_DIR or VOICE_TEST_DOWNLOAD=1");
        return;
    };
    let cfg = WorkerConfig {
        models_dir: dir.to_string_lossy().into_owned(),
        ..WorkerConfig::default()
    };
    let engine = Engine::new();
    let loaded = engine
        .ensure_loaded(&cfg, None)
        .await
        .expect("model downloads and loads");
    assert!(loaded.load_ms < 30_000);
    assert!(engine.loaded_for(&cfg).await.is_some());

    let (transcript, backend, model) = engine
        .transcribe(&cfg, tone_clip(), None)
        .await
        .expect("transcribes");
    assert_eq!(backend, "local");
    assert!(
        model == cfg.stt.model || model == cfg.stt.final_model,
        "unexpected model {model}"
    );
    assert!(
        (transcript.duration_secs - 2.7).abs() < 0.05,
        "{}",
        transcript.duration_secs
    );
    assert!(
        transcript.text.split_whitespace().count() <= 2,
        "a tone should not become prose: {:?}",
        transcript.text
    );

    if let Some(clip) = std::env::var_os("VOICE_TEST_WAV") {
        let bytes = std::fs::read(clip).expect("clip readable");
        let decoded = audio::decode_wav(&bytes).expect("clip decodes");
        let (transcript, _, _) = engine
            .transcribe(&cfg, decoded.samples, None)
            .await
            .expect("transcribes the clip");
        eprintln!("clip transcript: {:?}", transcript);
        assert!(!transcript.text.is_empty());
        assert!(!transcript.segments.is_empty());
        if engine.final_loaded_for(&cfg).await.is_some() {
            assert!(
                transcript.text.contains('.') || transcript.text.contains(','),
                "the second pass should punctuate: {:?}",
                transcript.text
            );
        }
    }
}
