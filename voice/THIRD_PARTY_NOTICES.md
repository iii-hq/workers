# Third-party notices

The voice worker itself is Apache-2.0 (see the repository `LICENSE`). This file lists everything the worker links into its binary, downloads at build time or run time, or starts as a separate process, with the license each one carries and where it comes from. No GPL, LGPL or AGPL code is linked into the binary.

## Linked into the binary

The build downloads the `-static-no-tts-lib` archives from the sherpa-onnx GitHub releases, checks each against a SHA-256 pinned in the vendored build script, and links these static libraries. The vendored `vendor/sherpa-onnx-sys` crate (Apache-2.0, license file kept) is the upstream `sherpa-onnx-sys` 1.13.7 crate with one change: its link list omits `espeak-ng`, `piper_phonemize` and `ucd`, so the GPL-3.0 text-to-speech pieces that ship in the full archives are never linked.

| Component | License | Source | Role |
| --- | --- | --- | --- |
| sherpa-onnx (`sherpa-onnx-c-api`, `sherpa-onnx-core`, `sherpa-onnx-fst`, `sherpa-onnx-fstfar`, `sherpa-onnx-kaldifst-core`) and the `sherpa-onnx` Rust crate | Apache-2.0 | https://github.com/k2-fsa/sherpa-onnx | Speech recognition runtime |
| kaldi-decoder | Apache-2.0 | https://github.com/k2-fsa/kaldi-decoder | Transducer decoding |
| kaldifst / OpenFst | Apache-2.0 | https://github.com/k2-fsa/kaldifst | FST utilities used by the decoder |
| kaldi-native-fbank | Apache-2.0 | https://github.com/csukuangfj/kaldi-native-fbank | Filterbank features |
| KISS FFT | BSD-3-Clause | https://github.com/mborgerding/kissfft | FFT inside the feature extractor |
| ONNX Runtime | MIT | https://github.com/microsoft/onnxruntime | Model inference |
| simple-sentencepiece (`ssentencepiece_core`) | Apache-2.0 | https://github.com/pkufool/simple-sentencepiece | Token handling |

Every Rust crate the worker depends on is under a permissive license (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0, CDLA-Permissive-2.0 or a choice that includes one of those); `cargo metadata` in `voice/` lists them.

## Models downloaded at run time

The worker never redistributes model files. On first use (bundled ONNX models) or explicit download (Whisper and Piper voices) it downloads them on the user's machine from the pages below, verifies each file's SHA-256, and stores them under `models_dir`. `voice::models::list` reports the license, author and source of every model so the console can show them.

| Model | Author | License | Source | Changes |
| --- | --- | --- | --- | --- |
| Zipformer streaming 20M (`zipformer-en-20m`) | k2-fsa, trained with icefall on LibriSpeech | Apache-2.0 | https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17 | int8 ONNX export published by the sherpa-onnx project |
| Parakeet TDT 0.6B v2 (`parakeet-tdt-0.6b-v2`) | NVIDIA | CC BY 4.0 | https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 (export of https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2) | int8 ONNX export published by the sherpa-onnx project; the worker makes no further changes |

| Whisper tiny, base, small, medium, large-v3-turbo (`whisper-*`) | OpenAI; GGML conversion by whisper.cpp | MIT | https://huggingface.co/ggerganov/whisper.cpp (original models: https://github.com/openai/whisper) | Published multilingual GGML files, downloaded unchanged and SHA-256 verified |

Attribution for the CC BY 4.0 model, to keep with any copy of its files:

> "Parakeet TDT 0.6B v2" by NVIDIA, https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2, licensed under CC BY 4.0, https://creativecommons.org/licenses/by/4.0/. Converted to ONNX and quantized to int8 by the sherpa-onnx project.

CC BY 4.0 covers the model weights. Text the model produces is not covered by the model's license.

## Started as separate processes

Read-aloud with the `host` engine and transcription with the `whisper_cpp` engine run commands already installed on the machine as child processes. No program code from these commands is linked, downloaded, or shipped by the worker. Optional Whisper model weights are downloaded separately as documented above.

| Program | License | Note |
| --- | --- | --- |
| `say` (macOS) | Part of macOS | Used through its command line |
| `espeak-ng` (Linux) | GPL-3.0-or-later | Only used when the user has installed it; invoked as a separate program, never linked, so its license does not extend to the worker |
| Documented `whisper-cli` integration (whisper.cpp) | MIT | Optional speech-to-text process supplied by the user. Custom command replacements and GGML models require separate license review. |

## Piper neural synthesis

The optional `piper` TTS backend invokes the user's `piper-tts` installation
as a separate process (https://github.com/OHF-Voice/piper1-gpl, GPL-3.0); it is
not linked into or shipped with the worker. Piper Faber pt-BR medium is
downloaded separately from
https://huggingface.co/rhasspy/piper-voices/tree/main/pt/pt_BR/faber/medium.
Its model card identifies the training dataset as CC0 and notes fine-tuning
from Lessac medium. The catalog downloads and preserves that card alongside
the original ONNX/config files; consult it before redistributing weights.
The CC0 label describes the dataset, not a blanket license grant for derived
weights.

### Other Piper voices

The expanded catalog imports 176 entries from the upstream Piper voices index
at commit `1162a9173d0ce503555aed757976b7a9912eae4c`. It embeds metadata only;
weights and model cards are downloaded solely on explicit user request. There
is no blanket CC0 license assertion for these voices: each entry links to its
own pinned MODEL_CARD, downloaded unchanged alongside ONNX and configuration.
The importer pins SHA-256 for every file, using LFS metadata for weights and
Git-blob-verified downloads for small metadata. Review the individual card and
dataset terms before redistribution or use.

## Markdown speech preparation

The worker uses `pulldown-cmark` 0.13 (MIT) to extract spoken prose from
CommonMark/GFM input. This is a parser only: no Markdown HTML is rendered, no
scripts are executed and no linked resources are fetched.
Source: https://github.com/pulldown-cmark/pulldown-cmark.

## Optional network services

With `stt.backend: openai` or `tts.backend: openai` the worker sends audio or text to the endpoint the user configures (OpenAI or any compatible server) under that service's own terms. Nothing is sent unless the user selects that engine.
