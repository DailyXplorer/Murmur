# Third-party notices

## Local transcription

Murmur transcribes on this Mac with Parakeet Ultra by Moondream
(`moondream/parakeet-ultra`), a post-trained version of NVIDIA's Parakeet TDT
0.6B v3 (`nvidia/parakeet-tdt-0.6b-v3`). Both are licensed under CC BY 4.0,
<https://creativecommons.org/licenses/by/4.0/>. Murmur downloads a converted
copy from Hugging Face, at a pinned commit and checked by SHA-256; it is not
bundled with Murmur. Murmur removes the `<unk>` tokens the model writes in
place of characters its tokenizer cannot spell.

- Apple silicon: `FluidInference/parakeet-ultra-coreml`, converted to Core ML
  by Fluid Inference, run by FluidAudio in the meetings helper (see below).
- Intel Macs and Windows: `handy-computer/parakeet-ultra-gguf`, converted to
  GGUF and quantised to 8 bits by handy-computer, run on the CPU by
  transcribe.cpp.

### transcribe.cpp

transcribe.cpp 0.4.1 (Rust crates `transcribe-cpp` and `transcribe-cpp-sys`),
<https://github.com/handy-computer/transcribe.cpp>, built into the Intel Mac
and Windows apps. Licensed under the MIT License; copyright (c) 2026 The
transcribe.cpp authors. It includes ggml (MIT, copyright (c) 2023-2026 The
ggml authors), Silero VAD (MIT) and miniz (MIT), whose licence texts ship
with its source.

## Meetings: on-device speaker detection

Murmur's meetings helper (`src-tauri/swift/MurmurMeetingsHelper`) links
FluidAudio to run the models below. On first use, Murmur downloads them from
Hugging Face, at pinned commits and checked by SHA-256, into the app's data
folder. The models are not bundled with Murmur.

### FluidAudio

FluidAudio 0.17.7, by Fluid Inference, <https://github.com/FluidInference/FluidAudio>.
Licensed under the Apache License 2.0, <https://www.apache.org/licenses/LICENSE-2.0>.

### Parakeet Ultra (word timing)

`FluidInference/parakeet-ultra-coreml`: a Core ML conversion by Fluid
Inference of Moondream's `moondream/parakeet-ultra`, itself a post-trained
version of NVIDIA's `nvidia/parakeet-tdt-0.6b-v3`. Licensed under CC BY 4.0,
<https://creativecommons.org/licenses/by/4.0/>. Meetings use it to time
words, and to transcribe the parts Codex refuses.

### Speaker diarization Community-1

`FluidInference/speaker-diarization-coreml`, offline artifact set:

> Segmentation.mlmodelc, FBank.mlmodelc, Embedding.mlmodelc, PLDA.mlmodelc,
> PldaRho.mlmodelc, plda-parameters.json, and xvector-transform.json are
> modified Core ML conversions of components from the pyannote Community-1
> speaker diarization pipeline, published by pyannote under CC-BY-4.0
> (<https://huggingface.co/pyannote/speaker-diarization-community-1>). The
> pipeline's speaker embedding component relates to WeSpeaker. The PLDA
> parameters and x-vector transform originate from BUT Speech@FIT (Brno
> University of Technology), who license them under CC-BY-4.0, including
> commercial use. Fluid Inference modified these files by converting the
> PyTorch components to Core ML, using fixed and enumerated input shapes,
> applying mixed-precision storage where recorded in model metadata,
> separating the FBank frontend from the embedding backend, and compiling
> packages for Apple platforms. These files are licensed under CC-BY-4.0
> (<https://creativecommons.org/licenses/by/4.0/>).

## Meetings on Windows: speaker detection

The Windows app tells the voices of a meeting apart with the same model as the
Mac, pyannote Community-1, run on the CPU inside the app by ONNX Runtime. The
pipeline around the networks (windowing, features, PLDA, agglomerative and VBx
clustering, assignment, reconstruction) is Murmur's own Rust re-implementation
of the offline pipeline of FluidAudio 0.17.7 (Apache License 2.0, see above),
with the same parameters. Murmur downloads the model files when Meetings are
on, from Hugging Face at pinned commits and checked by SHA-256; they are not
bundled with Murmur.

### Speaker diarization Community-1 (ONNX)

- `altunenes/speaker-diarization-community-1-onnx`: the segmentation network
  (`segmentation-community-1.onnx`), exported to ONNX by altunenes.
- `welcomyou/pyannote-community-1-onnx-split`: the embedding network split
  before its statistics pooling (`embedding_encoder.onnx`) with the weight and
  bias of its last layer (`resnet_seg_1_weight.npy`, `resnet_seg_1_bias.npy`),
  derived by welcomyou from altunenes's export. The pooling that weighs each
  frame by one speaker's activity is done by Murmur.
- `FluidInference/speaker-diarization-coreml`: only `plda-parameters.json` and
  `xvector-transform.json`, the same files the Mac uses.

> These files are modified conversions of components of the pyannote
> Community-1 speaker diarization pipeline, published by pyannote under
> CC-BY-4.0 (<https://huggingface.co/pyannote/speaker-diarization-community-1>).
> The pipeline's speaker embedding component relates to WeSpeaker. The PLDA
> parameters and x-vector transform originate from BUT Speech@FIT (Brno
> University of Technology), who license them under CC-BY-4.0, including
> commercial use. The PyTorch components were converted to ONNX, and the
> embedding network was split in two parts. These files are licensed under
> CC-BY-4.0 (<https://creativecommons.org/licenses/by/4.0/>).

### ONNX Runtime

ONNX Runtime, by Microsoft, <https://github.com/microsoft/onnxruntime>,
licensed under the MIT License. It is linked into the Windows app as the
prebuilt CPU binaries that the Rust crate `ort` (pyke, MIT or Apache License
2.0) downloads at build time and checks against the hash the crate pins. No
other execution provider is built in, and no DLL ships beside the app.
