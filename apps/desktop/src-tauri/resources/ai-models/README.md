# Built-in speech models

Hachimi keeps bundled local AI models under one resource root and separates them by task:

- `speech-to-text/sensevoice-small`: bundled SenseVoice-Small INT8 model for
  offline Mandarin, English, Japanese, Korean, and Cantonese recognition.
- `text-to-speech/vits-melo-zh-en`: bundled bilingual Chinese/English MeloTTS
  VITS model executed in-process through sherpa-onnx. It does not require
  Python, PyTorch, or a local HTTP service.

The complete runtime files are versioned in the repository. Large ONNX files use
Git LFS, so install Git LFS and run `git lfs pull` after cloning. The build
verifies every packaged file against the pinned manifest before compiling.

Run `scripts/prepare-speech-models.ps1` from the repository root only to repair
missing or corrupt files from checksum-verified official archives. Do not
manually rename model files: the Rust runtime uses the stable paths documented
above.

The bundled MeloTTS model is distributed under the MIT license. Its exact
archive checksum and the checksum of every packaged model file are recorded in
the resource manifest.

On Windows, both STT and TTS use the bundled sherpa-onnx 1.13.4 DirectML
runtime. Hachimi enumerates non-software DXGI adapters, sorts them by dedicated
memory, and attempts `directml#<dxgi-adapter-index>` in that order. It reports
DirectML only after creating a real D3D12/DirectML device, creating the model
Session, and completing warm-up; if every adapter fails it rebuilds the Session
on the CPU provider. The runtime state exposes the selected adapter name, index,
and dedicated-memory size.
