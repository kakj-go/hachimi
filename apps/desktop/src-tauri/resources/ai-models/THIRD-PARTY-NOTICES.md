# Speech runtime and model notices

- `sherpa-onnx` / `sherpa-onnx-sys` 1.13.4: Apache-2.0, <https://github.com/k2-fsa/sherpa-onnx>.
- `Microsoft.ML.OnnxRuntime.DirectML` 1.14.1 and `Microsoft.AI.DirectML` 1.15.0
  are bundled for Windows DirectML execution under their respective Microsoft
  package licenses.
- `vits-melo-tts-zh_en` is bundled from the official sherpa-onnx TTS model
  release under the MIT license. It provides Chinese and English synthesis.
- SenseVoice-Small INT8 is bundled from the official sherpa-onnx ASR model
  release and is converted from `ASLP-lab/WSYue-ASR`, whose model card declares
  Apache-2.0: <https://huggingface.co/ASLP-lab/WSYue-ASR>. Its exact archive,
  file checksums and license status are recorded in the model manifest; a copy
  of Apache License 2.0 is stored beside the model.

The download URLs and SHA-256 checksums used to prepare the packaged files are recorded in each model's `manifest.json` and in `scripts/prepare-speech-models.ps1`.
