//! Audio decoding for the bus: base64 PCM chunks from the mic capture, and WAV
//! files (any sample rate, mono or multi-channel) for transcription. Everything
//! comes out as 16 kHz mono `f32` samples in `[-1, 1]`, which is what the
//! recognizer eats.

use std::io::Cursor;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use sherpa_onnx::LinearResampler;

/// The recognizer's native sample rate.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Decode a base64 string of little-endian signed 16-bit PCM into samples.
pub fn decode_pcm16_base64(data: &str, max_bytes: usize) -> Result<Vec<f32>, String> {
    let bytes = BASE64_STANDARD
        .decode(data.trim())
        .map_err(|e| format!("audio chunk is not valid base64: {e}"))?;
    if bytes.len() > max_bytes {
        return Err(format!(
            "audio chunk is {} bytes, over the {max_bytes}-byte cap",
            bytes.len()
        ));
    }
    if bytes.len() % 2 != 0 {
        return Err("audio chunk has an odd byte count; expected 16-bit samples".to_string());
    }
    Ok(pcm16_to_f32(&bytes))
}

/// Little-endian signed 16-bit PCM bytes to `f32` samples.
pub fn pcm16_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32768.0)
        .collect()
}

/// Decoded audio ready for the recognizer.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    /// 16 kHz mono samples.
    pub samples: Vec<f32>,
    /// Seconds of audio.
    pub duration_secs: f32,
    /// Sample rate of the source before resampling.
    pub source_sample_rate: u32,
    /// Channel count of the source before downmixing.
    pub source_channels: u16,
}

/// Decode a WAV file (PCM 8/16/24/32-bit or IEEE float) to 16 kHz mono.
pub fn decode_wav(bytes: &[u8]) -> Result<DecodedAudio, String> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes))
        .map_err(|e| format!("not a readable WAV file: {e}"))?;
    let spec = reader.spec();
    if spec.channels == 0 {
        return Err("WAV file declares zero channels".to_string());
    }
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| format!("WAV float samples: {e}"))?,
        hound::SampleFormat::Int => {
            let scale = (1u64 << (spec.bits_per_sample.saturating_sub(1))) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
                .map_err(|e| format!("WAV integer samples: {e}"))?
        }
    };
    let mono = downmix(&interleaved, spec.channels as usize);
    let samples = resample(&mono, spec.sample_rate, TARGET_SAMPLE_RATE)?;
    Ok(DecodedAudio {
        duration_secs: samples.len() as f32 / TARGET_SAMPLE_RATE as f32,
        samples,
        source_sample_rate: spec.sample_rate,
        source_channels: spec.channels,
    })
}

/// Average interleaved channels into one.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

/// Linear resampling to the recognizer's rate. A no-op when the rates match.
pub fn resample(samples: &[f32], from_hz: u32, to_hz: u32) -> Result<Vec<f32>, String> {
    if from_hz == to_hz {
        return Ok(samples.to_vec());
    }
    if from_hz == 0 {
        return Err("audio declares a zero sample rate".to_string());
    }
    let resampler = LinearResampler::create(from_hz as i32, to_hz as i32)
        .ok_or_else(|| format!("cannot resample {from_hz} Hz to {to_hz} Hz"))?;
    Ok(resampler.resample(samples, true))
}

/// Encode 16 kHz mono samples as a WAV file (16-bit PCM), for handing audio
/// to an HTTP transcription endpoint.
pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer =
            hound::WavWriter::new(&mut cursor, spec).map_err(|e| format!("WAV writer: {e}"))?;
        for sample in samples {
            let clamped = sample.clamp(-1.0, 1.0);
            writer
                .write_sample((clamped * 32767.0) as i16)
                .map_err(|e| format!("WAV write: {e}"))?;
        }
        writer
            .finalize()
            .map_err(|e| format!("WAV finalize: {e}"))?;
    }
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm16_round_trips_through_base64() {
        let bytes: Vec<u8> = [0i16, 16384, -16384, 32767, -32768]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let encoded = BASE64_STANDARD.encode(&bytes);
        let samples = decode_pcm16_base64(&encoded, 1024).expect("decodes");
        assert_eq!(samples.len(), 5);
        assert!((samples[1] - 0.5).abs() < 1e-4);
        assert!((samples[2] + 0.5).abs() < 1e-4);
        assert!(samples[4] <= -0.999);
    }

    #[test]
    fn oversized_and_odd_chunks_are_rejected() {
        let encoded = BASE64_STANDARD.encode([0u8; 10]);
        assert!(decode_pcm16_base64(&encoded, 4)
            .unwrap_err()
            .contains("cap"));
        let odd = BASE64_STANDARD.encode([0u8; 3]);
        assert!(decode_pcm16_base64(&odd, 64).unwrap_err().contains("odd"));
    }

    #[test]
    fn wav_decodes_downmixes_and_resamples() {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
            for i in 0..48_000 {
                let v = ((i as f32 / 48.0).sin() * 10_000.0) as i16;
                writer.write_sample(v).unwrap();
                writer.write_sample(v).unwrap();
            }
            writer.finalize().unwrap();
        }
        let decoded = decode_wav(&cursor.into_inner()).expect("decodes");
        assert_eq!(decoded.source_sample_rate, 48_000);
        assert_eq!(decoded.source_channels, 2);
        assert!(
            (decoded.duration_secs - 1.0).abs() < 0.05,
            "{}",
            decoded.duration_secs
        );
        assert!(decoded.samples.iter().all(|s| s.abs() <= 1.0));
    }

    #[test]
    fn encode_wav_is_readable_again() {
        let samples: Vec<f32> = (0..1600).map(|i| (i as f32 / 10.0).sin() * 0.5).collect();
        let bytes = encode_wav(&samples, TARGET_SAMPLE_RATE).expect("encodes");
        let decoded = decode_wav(&bytes).expect("decodes");
        assert_eq!(decoded.samples.len(), 1600);
        assert!((decoded.samples[10] - samples[10]).abs() < 1e-3);
    }

    #[test]
    fn garbage_is_not_a_wav() {
        assert!(decode_wav(b"definitely not audio").is_err());
    }
}
