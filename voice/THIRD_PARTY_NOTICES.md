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

The worker never redistributes model files. On first use it downloads them on the user's machine from the pages below, verifies each file's SHA-256, and stores them under `models_dir`. `voice::models::list` reports the license, author and source of every model so the console can show them.

| Model | Author | License | Source | Changes |
| --- | --- | --- | --- | --- |
| Zipformer streaming 20M (`zipformer-en-20m`) | k2-fsa, trained with icefall on LibriSpeech | Apache-2.0 | https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17 | int8 ONNX export published by the sherpa-onnx project |
| Parakeet TDT 0.6B v2 (`parakeet-tdt-0.6b-v2`) | NVIDIA | CC BY 4.0 | https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8 (export of https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2) | int8 ONNX export published by the sherpa-onnx project; the worker makes no further changes |

Attribution for the CC BY 4.0 model, to keep with any copy of its files:

> "Parakeet TDT 0.6B v2" by NVIDIA, https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2, licensed under CC BY 4.0, https://creativecommons.org/licenses/by/4.0/. Converted to ONNX and quantized to int8 by the sherpa-onnx project.

CC BY 4.0 covers the model weights. Text the model produces is not covered by the model's license.

## Started as separate processes

Read-aloud with the `host` engine runs the machine's own speech command as a child process and pipes text to it. Nothing from these programs is linked or shipped.

| Program | License | Note |
| --- | --- | --- |
| `say` (macOS) | Part of macOS | Used through its command line |
| `espeak-ng` (Linux) | GPL-3.0-or-later | Only used when the user has installed it; invoked as a separate program, never linked, so its license does not extend to the worker |

## Optional network services

With `stt.backend: openai` or `tts.backend: openai` the worker sends audio or text to the endpoint the user configures (OpenAI or any compatible server) under that service's own terms. Nothing is sent unless the user selects that engine.
