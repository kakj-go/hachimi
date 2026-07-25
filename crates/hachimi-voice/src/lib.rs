//! Local speech runtime: SenseVoice-Small capture and native VITS playback.

mod catalog;
mod speech_model;
mod vits;

pub use catalog::{
    BUILTIN_ARCHIVE_SHA256, BUILTIN_VOICE_ID, InspectedVoiceModel, VoiceAsset, VoiceAssetPaths,
    VoiceCatalog, VoiceCatalogError, inspect_voice_archive,
};
pub use vits::{
    LipSyncProvider, SentenceSegment, SentenceSegmenter, VoiceEventSink, VoiceRuntime,
    VoiceRuntimeEventSinks, VoiceRuntimeStateSink, VoiceTurnEventSink, valid_phoneme_timeline,
};

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use cpal::{
    FromSample, Sample, SampleFormat, SizedSample,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use hachimi_core::FeatureAvailability;
use hachimi_protocol::{
    SpeechRecognitionRuntimeState, VoiceComputeBackend, VoiceComputeDevice, VoiceComputeMode,
};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig};
use thiserror::Error;

#[must_use]
pub const fn availability() -> FeatureAvailability {
    FeatureAvailability::Available
}

#[derive(Debug, Error)]
pub enum VoiceError {
    #[error("语速必须在 50%–200% 之间")]
    InvalidSpeed,
    #[error("VITS 模型不可用：{0}")]
    VitsConfiguration(String),
    #[error("VITS 合成失败：{0}")]
    VitsSynthesis(String),
    #[error("SenseVoice-Small 模型不可用")]
    SpeechRecognizerUnavailable,
    #[error("麦克风不可用或未授予访问权限：{0}")]
    MicrophoneUnavailable(String),
    #[error("没有听到清晰语音，请重试")]
    SpeechNotRecognized,
    #[error("本地语音识别失败：{0}")]
    SpeechRecognition(String),
}

#[derive(Clone)]
pub struct SpeechRecognizerRuntime {
    model_dir: PathBuf,
    recognizer: Arc<Mutex<Option<SpeechRecognizerSession>>>,
    compute_mode: Arc<Mutex<VoiceComputeMode>>,
    loading: Arc<AtomicBool>,
    fallback_reason: Arc<Mutex<Option<String>>>,
    error: Arc<Mutex<Option<String>>>,
}

struct SpeechRecognizerSession {
    recognizer: OfflineRecognizer,
    backend: VoiceComputeBackend,
    compute_device: Option<VoiceComputeDevice>,
}

impl std::fmt::Debug for SpeechRecognizerRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SpeechRecognizerRuntime")
            .field("available", &self.available())
            .field("model_dir", &self.model_dir)
            .finish_non_exhaustive()
    }
}

impl SpeechRecognizerRuntime {
    #[must_use]
    pub fn new(model_dir: PathBuf, compute_mode: VoiceComputeMode) -> Self {
        Self {
            model_dir: native_compatible_path(&model_dir),
            recognizer: Arc::new(Mutex::new(None)),
            compute_mode: Arc::new(Mutex::new(compute_mode)),
            loading: Arc::new(AtomicBool::new(false)),
            fallback_reason: Arc::new(Mutex::new(None)),
            error: Arc::new(Mutex::new(None)),
        }
    }

    #[must_use]
    pub fn available(&self) -> bool {
        self.model_dir.join("model.int8.onnx").is_file()
            && self.model_dir.join("tokens.txt").is_file()
    }

    #[must_use]
    pub fn state(&self) -> SpeechRecognitionRuntimeState {
        let compute_mode = *self
            .compute_mode
            .lock()
            .unwrap_or_else(|value| value.into_inner());
        let (backend, compute_device) = {
            let recognizer = self
                .recognizer
                .lock()
                .unwrap_or_else(|value| value.into_inner());
            recognizer.as_ref().map_or((None, None), |session| {
                (Some(session.backend), session.compute_device.clone())
            })
        };
        let fallback_reason = self
            .fallback_reason
            .lock()
            .unwrap_or_else(|value| value.into_inner())
            .clone();
        let error = self
            .error
            .lock()
            .unwrap_or_else(|value| value.into_inner())
            .clone();
        SpeechRecognitionRuntimeState {
            installed: self.available(),
            installing: false,
            bundled: true,
            model_name: speech_model::DEFAULT_SPEECH_MODEL_NAME.into(),
            provider: "sherpa-onnx 1.13.4".into(),
            languages: speech_model::DEFAULT_SPEECH_MODEL_LANGUAGES
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            size_bytes: speech_model::installed_size(&self.model_dir),
            compute_mode,
            backend,
            compute_device,
            fallback_reason,
            loading: self.loading.load(Ordering::SeqCst),
            error,
        }
    }

    pub fn update_compute_mode(
        &self,
        compute_mode: VoiceComputeMode,
    ) -> Result<SpeechRecognitionRuntimeState, VoiceError> {
        *self
            .compute_mode
            .lock()
            .unwrap_or_else(|value| value.into_inner()) = compute_mode;
        *self
            .recognizer
            .lock()
            .unwrap_or_else(|value| value.into_inner()) = None;
        self.initialize()
    }

    pub fn initialize(&self) -> Result<SpeechRecognitionRuntimeState, VoiceError> {
        if !self.available() {
            return Err(VoiceError::SpeechRecognizerUnavailable);
        }
        if self
            .loading
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(self.state());
        }
        let mode = *self
            .compute_mode
            .lock()
            .unwrap_or_else(|value| value.into_inner());
        let result = load_sense_voice_with_fallback(&self.model_dir, mode);
        self.loading.store(false, Ordering::SeqCst);
        match result {
            Ok((session, fallback_reason)) => {
                tracing::info!(
                    backend = ?session.backend,
                    fallback_reason = fallback_reason.as_deref(),
                    "SenseVoice Session warm-up completed"
                );
                *self
                    .recognizer
                    .lock()
                    .unwrap_or_else(|value| value.into_inner()) = Some(session);
                *self
                    .fallback_reason
                    .lock()
                    .unwrap_or_else(|value| value.into_inner()) = fallback_reason;
                *self.error.lock().unwrap_or_else(|value| value.into_inner()) = None;
                Ok(self.state())
            }
            Err(error) => {
                *self.error.lock().unwrap_or_else(|value| value.into_inner()) = Some(error.clone());
                Err(VoiceError::SpeechRecognition(error))
            }
        }
    }

    pub fn recognize_once(&self) -> Result<String, VoiceError> {
        if !self.available() {
            return Err(VoiceError::SpeechRecognizerUnavailable);
        }
        let (samples, sample_rate) = capture_microphone_utterance()?;
        let samples = resample_mono(&samples, sample_rate, 16_000);
        if samples.len() < 3_200 {
            return Err(VoiceError::SpeechNotRecognized);
        }
        let mut recognizer = self
            .recognizer
            .lock()
            .map_err(|_| VoiceError::SpeechRecognition("识别器状态不可用".into()))?;
        if recognizer.is_none() {
            drop(recognizer);
            self.initialize()?;
            recognizer = self
                .recognizer
                .lock()
                .map_err(|_| VoiceError::SpeechRecognition("识别器状态不可用".into()))?;
        }
        let recognizer = &recognizer
            .as_ref()
            .expect("recognizer was initialized")
            .recognizer;
        let stream = recognizer.create_stream();
        stream.accept_waveform(16_000, &samples);
        recognizer.decode(&stream);
        let result = stream
            .get_result()
            .ok_or_else(|| VoiceError::SpeechRecognition("SenseVoice 没有返回识别结果".into()))?;
        let text = clean_sense_voice_text(&result.text);
        if text.is_empty() {
            Err(VoiceError::SpeechNotRecognized)
        } else {
            Ok(text)
        }
    }
}

fn create_sense_voice_session(
    model_dir: &Path,
    provider: &str,
) -> Result<OfflineRecognizer, String> {
    let model = native_compatible_path(&model_dir.join("model.int8.onnx"));
    let tokens = native_compatible_path(&model_dir.join("tokens.txt"));
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model.to_string_lossy().into_owned()),
        language: Some("auto".into()),
        use_itn: true,
    };
    config.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
    config.model_config.num_threads = cpu_thread_count();
    config.model_config.provider = Some(provider.into());
    config.model_config.debug = false;
    OfflineRecognizer::create(&config)
        .ok_or_else(|| format!("无法使用 {provider} 创建 SenseVoice Session"))
}

fn warm_up_sense_voice(recognizer: &OfflineRecognizer) -> Result<(), String> {
    let stream = recognizer.create_stream();
    let silence = vec![0.0_f32; 16_000];
    stream.accept_waveform(16_000, &silence);
    recognizer.decode(&stream);
    stream
        .get_result()
        .map(|_| ())
        .ok_or_else(|| "SenseVoice 热身没有返回结果".to_owned())
}

fn load_sense_voice_with_fallback(
    model_dir: &Path,
    mode: VoiceComputeMode,
) -> Result<(SpeechRecognizerSession, Option<String>), String> {
    if matches!(mode, VoiceComputeMode::Auto | VoiceComputeMode::DirectMl) {
        let directml_result = directml_adapters().and_then(|adapters| {
            let mut failures = Vec::new();
            for adapter in adapters {
                let provider = directml_provider_name(adapter.device_id);
                match create_sense_voice_session(model_dir, &provider).and_then(|recognizer| {
                    warm_up_sense_voice(&recognizer)?;
                    Ok(recognizer)
                }) {
                    Ok(recognizer) => return Ok((recognizer, adapter)),
                    Err(error) => failures.push(format!("{}：{error}", adapter.name)),
                }
            }
            Err(format!(
                "所有 DirectML 适配器热身失败：{}",
                failures.join("；")
            ))
        });
        match directml_result {
            Ok((recognizer, compute_device)) => {
                return Ok((
                    SpeechRecognizerSession {
                        recognizer,
                        backend: VoiceComputeBackend::DirectMl,
                        compute_device: Some(compute_device),
                    },
                    None,
                ));
            }
            Err(reason) => {
                let recognizer = create_sense_voice_session(model_dir, "cpu")?;
                warm_up_sense_voice(&recognizer)?;
                return Ok((
                    SpeechRecognizerSession {
                        recognizer,
                        backend: VoiceComputeBackend::Cpu,
                        compute_device: None,
                    },
                    Some(format!("DirectML 不可用，已回退 CPU：{reason}")),
                ));
            }
        }
    }
    let recognizer = create_sense_voice_session(model_dir, "cpu")?;
    warm_up_sense_voice(&recognizer)?;
    Ok((
        SpeechRecognizerSession {
            recognizer,
            backend: VoiceComputeBackend::Cpu,
            compute_device: None,
        },
        None,
    ))
}

fn cpu_thread_count() -> i32 {
    std::thread::available_parallelism()
        .map_or(2, usize::from)
        .div_ceil(2)
        .clamp(2, 4) as i32
}

fn directml_provider_name(device_id: u32) -> String {
    format!("directml#{device_id}")
}

#[cfg(windows)]
fn directml_adapters() -> Result<Vec<VoiceComputeDevice>, String> {
    use windows::Win32::{
        AI::MachineLearning::DirectML::{DML_CREATE_DEVICE_FLAG_NONE, DMLCreateDevice, IDMLDevice},
        Graphics::{
            Direct3D::D3D_FEATURE_LEVEL_11_0,
            Direct3D12::{D3D12CreateDevice, ID3D12Device},
            Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1},
        },
    };

    unsafe {
        let factory: IDXGIFactory1 =
            CreateDXGIFactory1().map_err(|error| format!("无法创建 DXGI Factory：{error}"))?;
        let mut adapters = Vec::new();
        let mut rejected = Vec::new();
        for device_id in 0..64_u32 {
            let Ok(adapter) = factory.EnumAdapters1(device_id) else {
                break;
            };
            let description = adapter
                .GetDesc1()
                .map_err(|error| format!("无法读取 GPU Adapter {device_id}：{error}"))?;
            let name_end = description
                .Description
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(description.Description.len());
            let name = String::from_utf16_lossy(&description.Description[..name_end]);
            if description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                rejected.push(format!("{name} 是软件适配器"));
                continue;
            }
            let mut d3d_device: Option<ID3D12Device> = None;
            if let Err(error) = D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut d3d_device)
            {
                rejected.push(format!("{name} 无法创建 D3D12 Device：{error}"));
                continue;
            }
            let Some(d3d_device) = d3d_device else {
                rejected.push(format!("{name} 返回空 D3D12 Device"));
                continue;
            };
            let mut dml_device: Option<IDMLDevice> = None;
            if let Err(error) =
                DMLCreateDevice(&d3d_device, DML_CREATE_DEVICE_FLAG_NONE, &mut dml_device)
            {
                rejected.push(format!("{name} 无法创建 DirectML Device：{error}"));
                continue;
            }
            if dml_device.is_none() {
                rejected.push(format!("{name} 返回空 DirectML Device"));
                continue;
            }
            let memory_mb =
                (description.DedicatedVideoMemory / (1024 * 1024)).min(u32::MAX as usize) as u32;
            adapters.push(VoiceComputeDevice {
                device_id,
                name,
                dedicated_memory_mb: memory_mb,
            });
        }
        adapters.sort_by(|left, right| {
            right
                .dedicated_memory_mb
                .cmp(&left.dedicated_memory_mb)
                .then_with(|| left.device_id.cmp(&right.device_id))
        });
        if adapters.is_empty() {
            let reason = if rejected.is_empty() {
                "没有枚举到硬件 DXGI Adapter".to_owned()
            } else {
                rejected.join("；")
            };
            Err(reason)
        } else {
            Ok(adapters)
        }
    }
}

#[cfg(not(windows))]
fn directml_adapters() -> Result<Vec<VoiceComputeDevice>, String> {
    Err("DirectML 仅在 Windows 上可用".into())
}

fn native_compatible_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.to_string_lossy();
        if let Some(network_path) = value.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{network_path}"));
        }
        if let Some(local_path) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(local_path);
        }
    }
    path.to_path_buf()
}

#[derive(Debug)]
struct CaptureState {
    samples: Vec<f32>,
    heard_speech: bool,
    last_voice: Instant,
    error: Option<String>,
}

fn capture_microphone_utterance() -> Result<(Vec<f32>, u32), VoiceError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| VoiceError::MicrophoneUnavailable("找不到默认输入设备".into()))?;
    let supported = device
        .default_input_config()
        .map_err(|error| VoiceError::MicrophoneUnavailable(error.to_string()))?;
    let sample_rate = supported.sample_rate();
    let channels = usize::from(supported.channels());
    let config = supported.config();
    let state = Arc::new(Mutex::new(CaptureState {
        samples: Vec::with_capacity(sample_rate as usize * 15),
        heard_speech: false,
        last_voice: Instant::now(),
        error: None,
    }));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build_capture_stream::<f32>(&device, &config, channels, &state),
        SampleFormat::I16 => build_capture_stream::<i16>(&device, &config, channels, &state),
        SampleFormat::U16 => build_capture_stream::<u16>(&device, &config, channels, &state),
        format => Err(VoiceError::MicrophoneUnavailable(format!(
            "不支持的麦克风采样格式：{format}"
        ))),
    }?;
    stream
        .play()
        .map_err(|error| VoiceError::MicrophoneUnavailable(error.to_string()))?;
    let started = Instant::now();
    loop {
        thread::sleep(Duration::from_millis(50));
        let capture = state
            .lock()
            .map_err(|_| VoiceError::SpeechRecognition("麦克风采集状态不可用".into()))?;
        if let Some(error) = &capture.error {
            return Err(VoiceError::MicrophoneUnavailable(error.clone()));
        }
        if !capture.heard_speech && started.elapsed() >= Duration::from_secs(5) {
            return Err(VoiceError::SpeechNotRecognized);
        }
        if capture.heard_speech && capture.last_voice.elapsed() >= Duration::from_millis(900) {
            break;
        }
        if started.elapsed() >= Duration::from_secs(15) {
            break;
        }
    }
    drop(stream);
    let capture = state
        .lock()
        .map_err(|_| VoiceError::SpeechRecognition("麦克风采集状态不可用".into()))?;
    Ok((trim_silence(&capture.samples, sample_rate), sample_rate))
}

fn build_capture_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    state: &Arc<Mutex<CaptureState>>,
) -> Result<cpal::Stream, VoiceError>
where
    T: Sample + SizedSample,
    f32: FromSample<T>,
{
    let data_state = Arc::clone(state);
    let error_state = Arc::clone(state);
    device
        .build_input_stream(
            config,
            move |data: &[T], _| append_capture_data(data, channels, &data_state),
            move |error| {
                if let Ok(mut capture) = error_state.lock() {
                    capture.error = Some(error.to_string());
                }
            },
            None,
        )
        .map_err(|error| VoiceError::MicrophoneUnavailable(error.to_string()))
}

fn append_capture_data<T>(data: &[T], channels: usize, state: &Arc<Mutex<CaptureState>>)
where
    T: Sample,
    f32: FromSample<T>,
{
    if channels == 0 {
        return;
    }
    let mono = data
        .chunks_exact(channels)
        .map(|frame| frame.iter().copied().map(f32::from_sample).sum::<f32>() / channels as f32)
        .collect::<Vec<_>>();
    if mono.is_empty() {
        return;
    }
    let rms = (mono.iter().map(|sample| sample * sample).sum::<f32>() / mono.len() as f32).sqrt();
    if let Ok(mut capture) = state.lock() {
        capture.samples.extend_from_slice(&mono);
        if rms >= 0.012 {
            capture.heard_speech = true;
            capture.last_voice = Instant::now();
        }
    }
}

fn trim_silence(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let Some(first) = samples.iter().position(|sample| sample.abs() >= 0.006) else {
        return Vec::new();
    };
    let last = samples
        .iter()
        .rposition(|sample| sample.abs() >= 0.006)
        .unwrap_or(first);
    let padding = (sample_rate as usize / 5).min(first);
    let end = (last + sample_rate as usize / 5).min(samples.len().saturating_sub(1));
    samples[first - padding..=end].to_vec()
}

fn resample_mono(samples: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if samples.is_empty() || source_rate == 0 || target_rate == 0 {
        return Vec::new();
    }
    if source_rate == target_rate {
        return samples.to_vec();
    }
    let output_len = samples.len() * target_rate as usize / source_rate as usize;
    (0..output_len)
        .map(|index| {
            let position = index as f64 * source_rate as f64 / target_rate as f64;
            let left = position.floor() as usize;
            let right = (left + 1).min(samples.len() - 1);
            let fraction = (position - left as f64) as f32;
            samples[left] * (1.0 - fraction) + samples[right] * fraction
        })
        .collect()
}

fn clean_sense_voice_text(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut remainder = value;
    while let Some(start) = remainder.find("<|") {
        result.push_str(&remainder[..start]);
        if let Some(end) = remainder[start + 2..].find("|>") {
            remainder = &remainder[start + 2 + end + 2..];
        } else {
            remainder = &remainder[start..];
            break;
        }
    }
    result.push_str(remainder);
    result.trim().to_owned()
}

#[must_use]
pub fn normalize_speech_text(value: &str) -> String {
    let mut result = String::new();
    let mut in_code_block = false;
    for line in value.lines() {
        if line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }
        for character in line.chars() {
            if !matches!(character, '*' | '_' | '`' | '#' | '>' | '[' | ']') {
                result.push(character);
            }
            if result.chars().count() >= 1_000 {
                break;
            }
        }
        if result.chars().count() >= 1_000 {
            break;
        }
        result.push(' ');
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_recognition_state_snapshot_does_not_relock_the_session() {
        let temporary = tempfile::tempdir().expect("temporary model directory");
        let runtime =
            SpeechRecognizerRuntime::new(temporary.path().to_path_buf(), VoiceComputeMode::Auto);
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(runtime.state());
        });

        let state = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("speech recognition state snapshot deadlocked");
        assert!(!state.installed);
        assert_eq!(state.compute_mode, VoiceComputeMode::Auto);
        assert!(state.backend.is_none());
        assert!(state.compute_device.is_none());
    }

    #[test]
    fn speech_normalization_removes_code_and_markdown() {
        let value = normalize_speech_text(
            "# 你好 **Hachimi**\n```rust\nprintln!(\"secret\");\n```\n[继续]说话",
        );
        assert_eq!(value, "你好 Hachimi 继续说话");
        assert!(!value.contains("secret"));
    }

    #[test]
    fn sense_voice_tags_are_removed() {
        assert_eq!(
            clean_sense_voice_text("<|zh|><|NEUTRAL|><|Speech|><|woitn|>你好，Hachimi。"),
            "你好，Hachimi。"
        );
    }

    #[test]
    fn mono_resampling_preserves_duration() {
        let source = vec![0.25_f32; 48_000];
        let output = resample_mono(&source, 48_000, 16_000);
        assert_eq!(output.len(), 16_000);
        assert!(
            output
                .iter()
                .all(|value| (*value - 0.25).abs() < f32::EPSILON)
        );
    }

    #[cfg(windows)]
    #[test]
    fn native_paths_strip_the_verbatim_prefix() {
        let path = native_compatible_path(Path::new(r"\\?\C:\Hachimi\voice-models"));
        assert_eq!(path, PathBuf::from(r"C:\Hachimi\voice-models"));
        let unc = native_compatible_path(Path::new(r"\\?\UNC\server\share\model"));
        assert_eq!(unc, PathBuf::from(r"\\server\share\model"));
    }

    #[cfg(windows)]
    #[test]
    fn bundled_sense_voice_transcribes_chinese_fixture() {
        let model_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../apps/desktop/src-tauri/resources/ai-models/speech-to-text/sensevoice-small",
        );
        if !model_dir.join("model.int8.onnx").is_file() {
            return;
        }
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(
                model_dir
                    .join("model.int8.onnx")
                    .to_string_lossy()
                    .into_owned(),
            ),
            language: Some("auto".into()),
            use_itn: true,
        };
        config.model_config.tokens =
            Some(model_dir.join("tokens.txt").to_string_lossy().into_owned());
        config.model_config.num_threads = 4;
        config.model_config.provider = Some("cpu".into());
        let recognizer = OfflineRecognizer::create(&config).expect("SenseVoice recognizer");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sensevoice-zh.wav")
            .to_string_lossy()
            .into_owned();
        let wave = sherpa_onnx::Wave::read(&fixture).expect("fixture WAV");
        let stream = recognizer.create_stream();
        stream.accept_waveform(wave.sample_rate(), wave.samples());
        recognizer.decode(&stream);
        let result = stream.get_result().expect("recognition result");
        let text = clean_sense_voice_text(&result.text);
        assert!(!text.is_empty(), "raw result: {}", result.text);
        assert!(!text.is_ascii());
    }

    #[cfg(windows)]
    #[test]
    fn bundled_sense_voice_warms_up_selected_backend_and_transcribes() {
        let model_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../apps/desktop/src-tauri/resources/ai-models/speech-to-text/sensevoice-small",
        );
        if !model_dir.join("model.int8.onnx").is_file() {
            return;
        }
        let directml_device_available = directml_adapters().is_ok();
        let (session, fallback) =
            load_sense_voice_with_fallback(&model_dir, VoiceComputeMode::Auto)
                .expect("SenseVoice backend warm-up");
        eprintln!("SenseVoice warm-up backend: {:?}", session.backend);
        if directml_device_available {
            assert_eq!(session.backend, VoiceComputeBackend::DirectMl);
            assert!(session.compute_device.is_some());
            assert!(fallback.is_none());
        } else {
            assert_eq!(session.backend, VoiceComputeBackend::Cpu);
            assert!(session.compute_device.is_none());
            assert!(fallback.is_some());
        }
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sensevoice-zh.wav")
            .to_string_lossy()
            .into_owned();
        let wave = sherpa_onnx::Wave::read(&fixture).expect("fixture WAV");
        let stream = session.recognizer.create_stream();
        stream.accept_waveform(wave.sample_rate(), wave.samples());
        session.recognizer.decode(&stream);
        let result = stream.get_result().expect("recognition result");
        assert!(!clean_sense_voice_text(&result.text).is_empty());
    }
}
