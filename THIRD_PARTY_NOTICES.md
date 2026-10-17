# Third-party notices

## Meetings: on-device speaker detection

Murmur's meetings helper (`src-tauri/swift/MurmurMeetingsHelper`) links
FluidAudio and, on first use, downloads the models below from Hugging Face
into the app's data folder. The models are not bundled with Murmur.

### FluidAudio

FluidAudio 0.17.7, by Fluid Inference, <https://github.com/FluidInference/FluidAudio>.
Licensed under the Apache License 2.0, <https://www.apache.org/licenses/LICENSE-2.0>.

### Parakeet TDT 0.6B v3 (word timing)

`FluidInference/parakeet-tdt-0.6b-v3-coreml`: a Core ML conversion by Fluid
Inference of NVIDIA's `nvidia/parakeet-tdt-0.6b-v3`. Licensed under CC BY 4.0,
<https://creativecommons.org/licenses/by/4.0/>. Murmur uses it only to time
words; its text is never shown.

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
