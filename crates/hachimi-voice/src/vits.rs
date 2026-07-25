use std::{
    collections::{BTreeMap, VecDeque},
    num::{NonZeroU16, NonZeroU32},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use hachimi_protocol::{
    SpeechPlaybackEvent, SpeechPlaybackPhase, SpeechPlaybackSource, SpeechTimeline,
    SpeechTimelineQuality, SpeechTurnEvent, SpeechTurnPhase, VoiceComputeBackend,
    VoiceComputeDevice, VoiceComputeMode, VoiceRuntimeState,
};
use rodio::{DeviceSinkBuilder, Player, buffer::SamplesBuffer};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsVitsModelConfig,
};

use crate::{VoiceAsset, VoiceError, native_compatible_path, normalize_speech_text};

const SPEECH_FRAME_MS: u16 = 20;
const MAX_SPOKEN_CHARS: usize = 1_000;

pub type VoiceEventSink = Arc<dyn Fn(SpeechPlaybackEvent) + Send + Sync + 'static>;
pub type VoiceTurnEventSink = Arc<dyn Fn(SpeechTurnEvent) + Send + Sync + 'static>;
pub type VoiceRuntimeStateSink = Arc<dyn Fn(VoiceRuntimeState) + Send + Sync + 'static>;

/// Extension point for TTS backends that can provide verified phoneme timing. Implementations
/// must return monotonically increasing frames within the PCM duration; invalid data is ignored
/// and the runtime falls back to the energy-locked jaw envelope.
pub trait LipSyncProvider: Send + Sync {
    fn timeline(
        &self,
        text: &str,
        samples: &[f32],
        sample_rate: u32,
        duration_ms: u32,
    ) -> Option<SpeechTimeline>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentenceSegment {
    pub index: u32,
    pub display_text: String,
    pub speech_text: String,
    pub text_start: u32,
    pub text_end: u32,
}

#[derive(Debug, Default)]
pub struct SentenceSegmenter {
    pending: String,
    emitted_chars: usize,
    spoken_chars: usize,
    next_index: u32,
}

impl SentenceSegmenter {
    #[must_use]
    pub fn push(&mut self, delta: &str) -> Vec<SentenceSegment> {
        self.pending.push_str(delta);
        let mut result = Vec::new();
        loop {
            let characters = self.pending.chars().collect::<Vec<_>>();
            if characters.is_empty() {
                break;
            }
            let punctuation = characters.iter().position(|character| {
                matches!(
                    character,
                    '。' | '！' | '？' | '!' | '?' | '；' | ';' | '\n'
                )
            });
            let split = punctuation.map(|index| index + 1).or_else(|| {
                if characters.len() >= 80 {
                    characters[..characters.len().min(100)]
                        .iter()
                        .rposition(|character| character.is_whitespace())
                        .filter(|index| *index >= 20)
                        .map(|index| index + 1)
                        .or_else(|| (characters.len() >= 100).then_some(100))
                } else {
                    None
                }
            });
            let Some(split) = split else { break };
            if let Some(segment) = self.take(split) {
                result.push(segment);
            }
        }
        result
    }

    #[must_use]
    pub fn finish(&mut self) -> Vec<SentenceSegment> {
        let count = self.pending.chars().count();
        self.take(count).into_iter().collect()
    }

    fn take(&mut self, count: usize) -> Option<SentenceSegment> {
        if count == 0 {
            return None;
        }
        let byte_end = self
            .pending
            .char_indices()
            .nth(count)
            .map_or(self.pending.len(), |(index, _)| index);
        let display_text = self.pending[..byte_end].to_owned();
        self.pending.drain(..byte_end);
        let start = self.emitted_chars;
        self.emitted_chars = self.emitted_chars.saturating_add(count);
        let remaining = MAX_SPOKEN_CHARS.saturating_sub(self.spoken_chars);
        let normalized = normalize_speech_text(&display_text);
        let speech_text = normalized.chars().take(remaining).collect::<String>();
        self.spoken_chars = self
            .spoken_chars
            .saturating_add(speech_text.chars().count());
        let segment = SentenceSegment {
            index: self.next_index,
            display_text,
            speech_text,
            text_start: u32::try_from(start).unwrap_or(u32::MAX),
            text_end: u32::try_from(self.emitted_chars).unwrap_or(u32::MAX),
        };
        self.next_index = self.next_index.saturating_add(1);
        Some(segment)
    }
}

enum VoiceCommand {
    Load {
        asset: Box<Option<VoiceAsset>>,
        mode: VoiceComputeMode,
        reply: Option<mpsc::SyncSender<Result<(), String>>>,
    },
    Segment {
        generation: u64,
        source: SpeechPlaybackSource,
        run_id: Option<String>,
        segment: SentenceSegment,
    },
    EndTurn {
        generation: u64,
        run_id: String,
    },
    Stop,
}

enum PlaybackCommand {
    Segment(SynthesizedSegment),
    Skipped {
        generation: u64,
        source: SpeechPlaybackSource,
        run_id: Option<String>,
        segment: SentenceSegment,
    },
    Failed {
        generation: u64,
        source: SpeechPlaybackSource,
        run_id: Option<String>,
        segment: SentenceSegment,
    },
    EndTurn {
        generation: u64,
        run_id: String,
    },
    Stop,
}

struct SynthesizedSegment {
    generation: u64,
    source: SpeechPlaybackSource,
    run_id: Option<String>,
    segment: SentenceSegment,
    samples: Vec<f32>,
    sample_rate: u32,
}

#[derive(Debug, Default)]
struct TurnPlaybackOutcome {
    skipped_language: bool,
    synthesis_failed: bool,
}

#[derive(Default)]
struct TurnInput {
    generation: u64,
    run_id: Option<String>,
    segmenter: SentenceSegmenter,
}

pub struct VoiceRuntime {
    synth_sender: mpsc::Sender<VoiceCommand>,
    playback_sender: mpsc::Sender<PlaybackCommand>,
    state: Arc<Mutex<VoiceRuntimeState>>,
    muted: Arc<AtomicBool>,
    speaking: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    speed_percent: Arc<AtomicU32>,
    turn: Mutex<TurnInput>,
    turn_sink: Option<VoiceTurnEventSink>,
    lip_sync_provider: Option<Arc<dyn LipSyncProvider>>,
}

#[derive(Default)]
pub struct VoiceRuntimeEventSinks {
    pub playback: Option<VoiceEventSink>,
    pub turn: Option<VoiceTurnEventSink>,
    pub state: Option<VoiceRuntimeStateSink>,
}

impl std::fmt::Debug for VoiceRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoiceRuntime")
            .field("state", &self.state())
            .finish()
    }
}

impl VoiceRuntime {
    #[must_use]
    pub fn start_with_event_sink(
        asset: Option<VoiceAsset>,
        muted: bool,
        speed_percent: u16,
        compute_mode: VoiceComputeMode,
        sinks: VoiceRuntimeEventSinks,
    ) -> Self {
        Self::start_with_event_sink_and_lip_sync_provider(
            asset,
            muted,
            speed_percent,
            compute_mode,
            sinks,
            None,
        )
    }

    #[must_use]
    pub fn start_with_event_sink_and_lip_sync_provider(
        asset: Option<VoiceAsset>,
        muted: bool,
        speed_percent: u16,
        compute_mode: VoiceComputeMode,
        sinks: VoiceRuntimeEventSinks,
        lip_sync_provider: Option<Arc<dyn LipSyncProvider>>,
    ) -> Self {
        let VoiceRuntimeEventSinks {
            playback: event_sink,
            turn: turn_sink,
            state: state_sink,
        } = sinks;
        let initial = asset.as_ref().map(|value| value.entry.clone());
        let speaker_count = initial
            .as_ref()
            .map_or(1, |entry| entry.speaker_count.max(1));
        let speaker_id = initial.as_ref().map_or(0, |entry| {
            entry.speaker_id.min(entry.speaker_count.saturating_sub(1))
        });
        let state = Arc::new(Mutex::new(VoiceRuntimeState {
            available: false,
            muted,
            model_id: initial.as_ref().map(|entry| entry.id.clone()),
            voice_name: initial
                .as_ref()
                .map_or_else(String::new, |entry| entry.name.clone()),
            speaking: false,
            speed_percent,
            provider: "sherpa_onnx_vits".into(),
            compute_mode,
            backend: None,
            compute_device: None,
            fallback_reason: None,
            loading: asset.is_some(),
            languages: initial.map_or_else(Vec::new, |entry| entry.languages),
            speaker_count,
            speaker_id,
        }));
        let muted_state = Arc::new(AtomicBool::new(muted));
        let speaking = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));
        let speed_state = Arc::new(AtomicU32::new(u32::from(speed_percent.clamp(50, 200))));
        let (synth_sender, synth_receiver) = mpsc::channel();
        let (playback_sender, playback_receiver) = mpsc::channel();

        let playback_state = Arc::clone(&state);
        let playback_muted = Arc::clone(&muted_state);
        let playback_speaking = Arc::clone(&speaking);
        let playback_generation = Arc::clone(&generation);
        let playback_turn_sink = turn_sink.clone();
        let playback_lip_sync_provider = lip_sync_provider.clone();
        thread::Builder::new()
            .name("hachimi-vits-playback".into())
            .spawn(move || {
                playback_worker(
                    playback_receiver,
                    PlaybackWorkerContext {
                        muted: &playback_muted,
                        speaking: &playback_speaking,
                        generation: &playback_generation,
                        state: &playback_state,
                        event_sink: event_sink.as_ref(),
                        turn_sink: playback_turn_sink.as_ref(),
                        lip_sync_provider: playback_lip_sync_provider.as_deref(),
                    },
                );
            })
            .expect("failed to start VITS playback worker");

        let worker_state = Arc::clone(&state);
        let worker_generation = Arc::clone(&generation);
        let worker_speed = Arc::clone(&speed_state);
        let worker_playback = playback_sender.clone();
        thread::Builder::new()
            .name("hachimi-vits-synthesis".into())
            .spawn(move || {
                synthesis_worker(
                    synth_receiver,
                    worker_playback,
                    &worker_generation,
                    &worker_speed,
                    &worker_state,
                    state_sink.as_ref(),
                );
            })
            .expect("failed to start VITS synthesis worker");

        let runtime = Self {
            synth_sender,
            playback_sender,
            state,
            muted: muted_state,
            speaking,
            generation,
            speed_percent: speed_state,
            turn: Mutex::new(TurnInput::default()),
            turn_sink,
            lip_sync_provider,
        };
        let _ = runtime.synth_sender.send(VoiceCommand::Load {
            asset: Box::new(asset),
            mode: compute_mode,
            reply: None,
        });
        runtime
    }

    #[must_use]
    pub fn state(&self) -> VoiceRuntimeState {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        state.muted = self.muted.load(Ordering::SeqCst);
        state.speaking = self.speaking.load(Ordering::SeqCst);
        state.speed_percent = self.speed_percent.load(Ordering::SeqCst) as u16;
        state
    }

    #[must_use]
    pub fn has_lip_sync_provider(&self) -> bool {
        self.lip_sync_provider.is_some()
    }

    pub fn load_model(
        &self,
        asset: Option<VoiceAsset>,
        mode: VoiceComputeMode,
    ) -> Result<(), VoiceError> {
        self.stop();
        let (sender, receiver) = mpsc::sync_channel(1);
        self.synth_sender
            .send(VoiceCommand::Load {
                asset: Box::new(asset),
                mode,
                reply: Some(sender),
            })
            .map_err(|_| VoiceError::VitsConfiguration("语音工作线程已停止".into()))?;
        receiver
            .recv_timeout(Duration::from_secs(120))
            .map_err(|_| VoiceError::VitsConfiguration("模型热身超时".into()))?
            .map_err(VoiceError::VitsConfiguration)
    }

    /// Loads and warms the candidate model before it becomes observable. If
    /// that fails, the previous model is synchronously restored so callers can
    /// keep the catalog selection unchanged without disabling speech.
    pub fn load_model_with_rollback(
        &self,
        candidate: Option<VoiceAsset>,
        previous: Option<VoiceAsset>,
        mode: VoiceComputeMode,
    ) -> Result<(), VoiceError> {
        let previous_mode = self.state().compute_mode;
        match self.load_model(candidate, mode) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = self.load_model(previous, previous_mode);
                Err(error)
            }
        }
    }

    pub fn update_settings(
        &self,
        speed_percent: u16,
        compute_mode: VoiceComputeMode,
        current_asset: Option<VoiceAsset>,
    ) -> Result<(), VoiceError> {
        if !(50..=200).contains(&speed_percent) {
            return Err(VoiceError::InvalidSpeed);
        }
        let previous_mode = self.state().compute_mode;
        if previous_mode != compute_mode {
            let previous_asset = current_asset.clone();
            if let Err(error) = self.load_model(current_asset, compute_mode) {
                let _ = self.load_model(previous_asset, previous_mode);
                return Err(error);
            }
        }
        self.speed_percent
            .store(u32::from(speed_percent), Ordering::SeqCst);
        if let Ok(mut state) = self.state.lock() {
            state.speed_percent = speed_percent;
            state.compute_mode = compute_mode;
        }
        Ok(())
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::SeqCst);
        if muted {
            self.stop();
        }
    }

    #[must_use]
    pub fn will_speak_pet(&self) -> bool {
        let state = self.state();
        state.available && !state.muted
    }

    pub fn begin_pet_turn(&self, run_id: String) -> bool {
        if !self.will_speak_pet() {
            return false;
        }
        self.stop();
        let generation = self.generation.load(Ordering::SeqCst);
        let mut turn = self.turn.lock().unwrap_or_else(|error| error.into_inner());
        *turn = TurnInput {
            generation,
            run_id: Some(run_id.clone()),
            segmenter: SentenceSegmenter::default(),
        };
        if let Some(sink) = &self.turn_sink {
            sink(SpeechTurnEvent {
                run_id,
                phase: SpeechTurnPhase::Started,
                message: None,
            });
        }
        true
    }

    pub fn push_pet_delta(&self, run_id: &str, delta: &str) {
        let mut turn = self.turn.lock().unwrap_or_else(|error| error.into_inner());
        if turn.run_id.as_deref() != Some(run_id) {
            return;
        }
        let generation = turn.generation;
        for segment in turn.segmenter.push(delta) {
            let _ = self.synth_sender.send(VoiceCommand::Segment {
                generation,
                source: SpeechPlaybackSource::PetTurn,
                run_id: Some(run_id.to_owned()),
                segment,
            });
        }
    }

    pub fn finish_pet_turn(&self, run_id: &str) -> bool {
        let mut turn = self.turn.lock().unwrap_or_else(|error| error.into_inner());
        if turn.run_id.as_deref() != Some(run_id) {
            return false;
        }
        let generation = turn.generation;
        for segment in turn.segmenter.finish() {
            let _ = self.synth_sender.send(VoiceCommand::Segment {
                generation,
                source: SpeechPlaybackSource::PetTurn,
                run_id: Some(run_id.to_owned()),
                segment,
            });
        }
        turn.run_id = None;
        let _ = self.synth_sender.send(VoiceCommand::EndTurn {
            generation,
            run_id: run_id.to_owned(),
        });
        true
    }

    pub fn speak(&self, text: &str) -> bool {
        if !self.will_speak_pet() {
            return false;
        }
        self.stop();
        let generation = self.generation.load(Ordering::SeqCst);
        let mut segmenter = SentenceSegmenter::default();
        let mut segments = segmenter.push(text);
        segments.extend(segmenter.finish());
        for segment in segments {
            let _ = self.synth_sender.send(VoiceCommand::Segment {
                generation,
                source: SpeechPlaybackSource::WorkbenchPreview,
                run_id: None,
                segment,
            });
        }
        true
    }

    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.speaking.store(false, Ordering::SeqCst);
        if let Ok(mut turn) = self.turn.lock() {
            if let (Some(sink), Some(run_id)) = (&self.turn_sink, turn.run_id.take()) {
                sink(SpeechTurnEvent {
                    run_id,
                    phase: SpeechTurnPhase::Stopped,
                    message: None,
                });
            }
            *turn = TurnInput::default();
        }
        let _ = self.synth_sender.send(VoiceCommand::Stop);
        let _ = self.playback_sender.send(PlaybackCommand::Stop);
    }
}

fn synthesis_worker(
    receiver: mpsc::Receiver<VoiceCommand>,
    playback: mpsc::Sender<PlaybackCommand>,
    generation: &AtomicU64,
    speed_percent: &AtomicU32,
    state: &Mutex<VoiceRuntimeState>,
    state_sink: Option<&VoiceRuntimeStateSink>,
) {
    let mut engine: Option<NativeVits> = None;
    let mut languages = Vec::new();
    while let Ok(command) = receiver.recv() {
        match command {
            VoiceCommand::Load { asset, mode, reply } => {
                let asset = *asset;
                if let Ok(mut value) = state.lock() {
                    value.loading = asset.is_some();
                    value.available = false;
                    value.compute_mode = mode;
                    value.backend = None;
                    value.compute_device = None;
                    value.fallback_reason = None;
                    value.model_id = asset.as_ref().map(|asset| asset.entry.id.clone());
                    value.voice_name = asset
                        .as_ref()
                        .map_or_else(String::new, |asset| asset.entry.name.clone());
                    value.languages = asset
                        .as_ref()
                        .map_or_else(Vec::new, |asset| asset.entry.languages.clone());
                    value.speaker_count = asset
                        .as_ref()
                        .map_or(1, |asset| asset.entry.speaker_count.max(1));
                    value.speaker_id = asset.as_ref().map_or(0, |asset| asset.entry.speaker_id);
                }
                let result = asset.as_ref().map_or_else(
                    || Err("未安装可用的 VITS 模型".into()),
                    |asset| load_with_fallback(asset, mode),
                );
                match result {
                    Ok((loaded, backend, compute_device, fallback_reason)) => {
                        languages = asset
                            .as_ref()
                            .map_or_else(Vec::new, |asset| asset.entry.languages.clone());
                        engine = Some(loaded);
                        if let Ok(mut value) = state.lock() {
                            value.available = true;
                            value.loading = false;
                            value.backend = Some(backend);
                            value.compute_device = compute_device;
                            value.fallback_reason = fallback_reason;
                        }
                        emit_runtime_state(state_sink, state);
                        if let Some(reply) = reply {
                            let _ = reply.send(Ok(()));
                        }
                    }
                    Err(error) => {
                        engine = None;
                        if let Ok(mut value) = state.lock() {
                            value.loading = false;
                            value.available = false;
                            value.fallback_reason = Some(error.clone());
                        }
                        emit_runtime_state(state_sink, state);
                        if let Some(reply) = reply {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
            }
            VoiceCommand::Segment {
                generation: requested,
                source,
                run_id,
                segment,
            } => {
                if generation.load(Ordering::SeqCst) != requested {
                    continue;
                }
                if source == SpeechPlaybackSource::PetTurn
                    && !supports_english(&languages)
                    && english_dominant(&segment.speech_text)
                {
                    let _ = playback.send(PlaybackCommand::Skipped {
                        generation: requested,
                        source,
                        run_id,
                        segment,
                    });
                    continue;
                }
                let Some(engine) = engine.as_mut() else {
                    let _ = playback.send(PlaybackCommand::Failed {
                        generation: requested,
                        source,
                        run_id,
                        segment,
                    });
                    continue;
                };
                if segment.speech_text.is_empty() {
                    let _ = playback.send(PlaybackCommand::Skipped {
                        generation: requested,
                        source,
                        run_id,
                        segment,
                    });
                    continue;
                }
                let speed = speed_percent.load(Ordering::SeqCst) as f32 / 100.0;
                match engine.generate(&segment.speech_text, speed) {
                    Ok((samples, sample_rate))
                        if generation.load(Ordering::SeqCst) == requested =>
                    {
                        let _ = playback.send(PlaybackCommand::Segment(SynthesizedSegment {
                            generation: requested,
                            source,
                            run_id,
                            segment,
                            samples,
                            sample_rate,
                        }));
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(%error, "native VITS segment synthesis failed");
                        let _ = playback.send(PlaybackCommand::Failed {
                            generation: requested,
                            source,
                            run_id,
                            segment,
                        });
                    }
                }
            }
            VoiceCommand::EndTurn { generation, run_id } => {
                let _ = playback.send(PlaybackCommand::EndTurn { generation, run_id });
            }
            VoiceCommand::Stop => {}
        }
    }
}

fn emit_runtime_state(sink: Option<&VoiceRuntimeStateSink>, state: &Mutex<VoiceRuntimeState>) {
    if let (Some(sink), Ok(state)) = (sink, state.lock()) {
        sink(state.clone());
    }
}

struct PlaybackWorkerContext<'a> {
    muted: &'a AtomicBool,
    speaking: &'a AtomicBool,
    generation: &'a AtomicU64,
    state: &'a Mutex<VoiceRuntimeState>,
    event_sink: Option<&'a VoiceEventSink>,
    turn_sink: Option<&'a VoiceTurnEventSink>,
    lip_sync_provider: Option<&'a dyn LipSyncProvider>,
}

#[derive(Debug, Default)]
struct PlaybackProgressTracker {
    last_media_position_ms: u32,
    playing_emitted: bool,
}

impl PlaybackProgressTracker {
    fn observe(&mut self, media_position_ms: u32) -> Option<SpeechPlaybackPhase> {
        if media_position_ms == 0 || media_position_ms <= self.last_media_position_ms {
            return None;
        }
        self.last_media_position_ms = media_position_ms;
        if self.playing_emitted {
            Some(SpeechPlaybackPhase::Progress)
        } else {
            self.playing_emitted = true;
            Some(SpeechPlaybackPhase::Playing)
        }
    }

    fn has_started(&self) -> bool {
        self.playing_emitted
    }
}

fn playback_worker(receiver: mpsc::Receiver<PlaybackCommand>, context: PlaybackWorkerContext<'_>) {
    let PlaybackWorkerContext {
        muted,
        speaking,
        generation,
        state,
        event_sink,
        turn_sink,
        lip_sync_provider,
    } = context;
    let mut device: Option<rodio::MixerDeviceSink> = None;
    let mut pending = VecDeque::new();
    let mut turn_outcomes = BTreeMap::<(u64, String), TurnPlaybackOutcome>::new();
    let mut event_sequence = 0_u32;
    loop {
        let command = if pending.is_empty() {
            match receiver.recv() {
                Ok(value) => value,
                Err(_) => break,
            }
        } else {
            pending.pop_front().expect("pending command")
        };
        match command {
            PlaybackCommand::Stop => {
                speaking.store(false, Ordering::SeqCst);
                if let Ok(mut value) = state.lock() {
                    value.speaking = false;
                }
                while receiver.try_recv().is_ok() {}
                turn_outcomes.clear();
            }
            PlaybackCommand::Skipped {
                generation: requested,
                source,
                run_id,
                segment,
            } => {
                if generation.load(Ordering::SeqCst) != requested {
                    continue;
                }
                if source == SpeechPlaybackSource::PetTurn
                    && let Some(run_id) = run_id.as_ref()
                {
                    turn_outcomes
                        .entry((requested, run_id.clone()))
                        .or_default()
                        .skipped_language = true;
                }
                emit_text_only(
                    event_sink,
                    &mut event_sequence,
                    requested,
                    source,
                    run_id,
                    &segment,
                    SpeechPlaybackPhase::Completed,
                );
            }
            PlaybackCommand::Failed {
                generation: requested,
                source,
                run_id,
                segment,
            } => {
                if generation.load(Ordering::SeqCst) != requested {
                    continue;
                }
                if source == SpeechPlaybackSource::PetTurn
                    && let Some(run_id) = run_id.as_ref()
                {
                    turn_outcomes
                        .entry((requested, run_id.clone()))
                        .or_default()
                        .synthesis_failed = true;
                }
                emit_text_only(
                    event_sink,
                    &mut event_sequence,
                    requested,
                    source,
                    run_id,
                    &segment,
                    SpeechPlaybackPhase::Failed,
                );
            }
            PlaybackCommand::EndTurn {
                generation: requested,
                run_id,
            } => {
                if generation.load(Ordering::SeqCst) == requested {
                    let outcome = turn_outcomes
                        .remove(&(requested, run_id.clone()))
                        .unwrap_or_default();
                    let (phase, message) = speech_turn_outcome(outcome);
                    if let Some(sink) = turn_sink {
                        sink(SpeechTurnEvent {
                            run_id,
                            phase,
                            message,
                        });
                    }
                }
            }
            PlaybackCommand::Segment(segment) => {
                if muted.load(Ordering::SeqCst)
                    || generation.load(Ordering::SeqCst) != segment.generation
                {
                    continue;
                }
                if device.is_none() {
                    match DeviceSinkBuilder::open_default_sink() {
                        Ok(output) => device = Some(output),
                        Err(error) => {
                            tracing::error!(%error, "failed to open audio output");
                            if segment.source == SpeechPlaybackSource::PetTurn
                                && let Some(run_id) = segment.run_id.as_ref()
                            {
                                turn_outcomes
                                    .entry((segment.generation, run_id.clone()))
                                    .or_default()
                                    .synthesis_failed = true;
                            }
                            emit_segment(
                                event_sink,
                                &mut event_sequence,
                                segment.generation,
                                segment.source,
                                segment.run_id.as_deref(),
                                &segment.segment,
                                SpeechPlaybackPhase::Failed,
                                0,
                                0,
                                None,
                            );
                            continue;
                        }
                    }
                }
                let duration_ms = u32::try_from(
                    (segment.samples.len() as u64).saturating_mul(1_000)
                        / u64::from(segment.sample_rate.max(1)),
                )
                .unwrap_or(u32::MAX);
                let timeline = resolve_lip_sync_timeline(
                    lip_sync_provider,
                    &segment.segment.speech_text,
                    &segment.samples,
                    segment.sample_rate,
                    duration_ms,
                );
                emit_segment(
                    event_sink,
                    &mut event_sequence,
                    segment.generation,
                    segment.source,
                    segment.run_id.as_deref(),
                    &segment.segment,
                    SpeechPlaybackPhase::Prepared,
                    0,
                    duration_ms,
                    Some(timeline),
                );
                let player = Player::connect_new(device.as_ref().expect("audio output").mixer());
                player.append(SamplesBuffer::new(
                    NonZeroU16::new(1).expect("one audio channel"),
                    NonZeroU32::new(segment.sample_rate).expect("validated sample rate"),
                    segment.samples,
                ));
                speaking.store(true, Ordering::SeqCst);
                if let Ok(mut value) = state.lock() {
                    value.speaking = true;
                }
                let mut progress = PlaybackProgressTracker::default();
                let mut stopped = false;
                while !player.empty() {
                    let media_position_ms = u32::try_from(player.get_pos().as_millis())
                        .unwrap_or(u32::MAX)
                        .min(duration_ms);
                    if let Some(phase) = progress.observe(media_position_ms) {
                        emit_segment(
                            event_sink,
                            &mut event_sequence,
                            segment.generation,
                            segment.source,
                            segment.run_id.as_deref(),
                            &segment.segment,
                            phase,
                            media_position_ms,
                            duration_ms,
                            None,
                        );
                    }
                    if muted.load(Ordering::SeqCst)
                        || generation.load(Ordering::SeqCst) != segment.generation
                    {
                        let media_position_ms = u32::try_from(player.get_pos().as_millis())
                            .unwrap_or(u32::MAX)
                            .min(duration_ms);
                        player.stop();
                        emit_segment(
                            event_sink,
                            &mut event_sequence,
                            segment.generation,
                            segment.source,
                            segment.run_id.as_deref(),
                            &segment.segment,
                            SpeechPlaybackPhase::Stopped,
                            media_position_ms,
                            duration_ms,
                            None,
                        );
                        stopped = true;
                        break;
                    }
                    match receiver.recv_timeout(Duration::from_millis(20)) {
                        Ok(PlaybackCommand::Stop) => {
                            let media_position_ms = u32::try_from(player.get_pos().as_millis())
                                .unwrap_or(u32::MAX)
                                .min(duration_ms);
                            player.stop();
                            emit_segment(
                                event_sink,
                                &mut event_sequence,
                                segment.generation,
                                segment.source,
                                segment.run_id.as_deref(),
                                &segment.segment,
                                SpeechPlaybackPhase::Stopped,
                                media_position_ms,
                                duration_ms,
                                None,
                            );
                            stopped = true;
                            break;
                        }
                        Ok(command) => pending.push_back(command),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                if generation.load(Ordering::SeqCst) == segment.generation
                    && !muted.load(Ordering::SeqCst)
                    && !stopped
                {
                    if !progress.has_started() && duration_ms > 0 {
                        emit_segment(
                            event_sink,
                            &mut event_sequence,
                            segment.generation,
                            segment.source,
                            segment.run_id.as_deref(),
                            &segment.segment,
                            SpeechPlaybackPhase::Playing,
                            duration_ms,
                            duration_ms,
                            None,
                        );
                    }
                    emit_segment(
                        event_sink,
                        &mut event_sequence,
                        segment.generation,
                        segment.source,
                        segment.run_id.as_deref(),
                        &segment.segment,
                        SpeechPlaybackPhase::Completed,
                        duration_ms,
                        duration_ms,
                        None,
                    );
                }
                speaking.store(false, Ordering::SeqCst);
                if let Ok(mut value) = state.lock() {
                    value.speaking = false;
                }
            }
        }
    }
}

fn speech_turn_outcome(outcome: TurnPlaybackOutcome) -> (SpeechTurnPhase, Option<String>) {
    if outcome.synthesis_failed {
        (SpeechTurnPhase::Failed, Some("voice_segment_failed".into()))
    } else if outcome.skipped_language {
        (
            SpeechTurnPhase::Skipped,
            Some("voice_language_unsupported".into()),
        )
    } else {
        (SpeechTurnPhase::Completed, None)
    }
}

fn emit_text_only(
    sink: Option<&VoiceEventSink>,
    sequence: &mut u32,
    generation: u64,
    source: SpeechPlaybackSource,
    run_id: Option<String>,
    segment: &SentenceSegment,
    terminal_phase: SpeechPlaybackPhase,
) {
    for phase in [SpeechPlaybackPhase::Prepared, terminal_phase] {
        emit_segment(
            sink,
            sequence,
            generation,
            source,
            run_id.as_deref(),
            segment,
            phase,
            0,
            0,
            None,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_segment(
    sink: Option<&VoiceEventSink>,
    sequence: &mut u32,
    generation: u64,
    source: SpeechPlaybackSource,
    run_id: Option<&str>,
    segment: &SentenceSegment,
    phase: SpeechPlaybackPhase,
    media_position_ms: u32,
    duration_ms: u32,
    timeline: Option<SpeechTimeline>,
) {
    if let Some(sink) = sink {
        *sequence = sequence.saturating_add(1);
        sink(SpeechPlaybackEvent {
            playback_id: format!("{generation}-{}", segment.index),
            run_id: run_id.map(ToOwned::to_owned),
            source,
            phase,
            media_position_ms,
            duration_ms,
            sequence: *sequence,
            timeline: matches!(phase, SpeechPlaybackPhase::Prepared)
                .then_some(timeline)
                .flatten(),
            segment_index: segment.index,
            text_start: segment.text_start,
            text_end: segment.text_end,
            display_text: matches!(phase, SpeechPlaybackPhase::Prepared)
                .then(|| segment.display_text.clone()),
        });
    }
}

fn load_with_fallback(
    asset: &VoiceAsset,
    mode: VoiceComputeMode,
) -> Result<
    (
        NativeVits,
        VoiceComputeBackend,
        Option<VoiceComputeDevice>,
        Option<String>,
    ),
    String,
> {
    if matches!(mode, VoiceComputeMode::Auto | VoiceComputeMode::DirectMl) {
        let directml_result = super::directml_adapters().and_then(|adapters| {
            let mut failures = Vec::new();
            for adapter in adapters {
                let provider = super::directml_provider_name(adapter.device_id);
                match NativeVits::new(asset, &provider).and_then(|mut engine| {
                    engine.generate("你好。", 1.0)?;
                    Ok(engine)
                }) {
                    Ok(engine) => return Ok((engine, adapter)),
                    Err(error) => failures.push(format!("{}：{error}", adapter.name)),
                }
            }
            Err(format!(
                "所有 DirectML 适配器热身失败：{}",
                failures.join("；")
            ))
        });
        match directml_result {
            Ok((engine, compute_device)) => {
                tracing::info!(
                    device_id = compute_device.device_id,
                    device_name = %compute_device.name,
                    "VITS Session warm-up completed with DirectML"
                );
                return Ok((
                    engine,
                    VoiceComputeBackend::DirectMl,
                    Some(compute_device),
                    None,
                ));
            }
            Err(reason) => {
                tracing::warn!(%reason, "VITS DirectML warm-up failed; rebuilding on CPU");
                let mut engine = NativeVits::new(asset, "cpu")?;
                engine.generate("你好。", 1.0)?;
                return Ok((
                    engine,
                    VoiceComputeBackend::Cpu,
                    None,
                    Some(format!("DirectML 不可用，已回退 CPU：{reason}")),
                ));
            }
        }
    }
    let mut engine = NativeVits::new(asset, "cpu")?;
    engine.generate("你好。", 1.0)?;
    tracing::info!("VITS Session warm-up completed with CPU");
    Ok((engine, VoiceComputeBackend::Cpu, None, None))
}

struct NativeVits {
    inner: OfflineTts,
    speaker_id: i32,
    strip_cjk_punctuation: bool,
}

impl NativeVits {
    fn new(asset: &VoiceAsset, provider_name: &str) -> Result<Self, String> {
        let path = |path: &std::path::Path| {
            let value = native_compatible_path(path).to_string_lossy().into_owned();
            (!value.contains('\0'))
                .then_some(value)
                .ok_or_else(|| "模型路径包含 NUL 字符".to_owned())
        };
        let model = path(&asset.model_path())?;
        let tokens = path(&asset.tokens_path())?;
        let lexicon = asset.lexicon_path().map(|value| path(&value)).transpose()?;
        let data_dir = asset.data_dir().map(|value| path(&value)).transpose()?;
        let dict_dir = asset.dict_dir().map(|value| path(&value)).transpose()?;
        let rule_fsts = asset
            .rule_fsts()
            .iter()
            .map(|value| path(value))
            .collect::<Result<Vec<_>, _>>()?
            .join(",");
        let threads = std::thread::available_parallelism()
            .map_or(2, usize::from)
            .div_ceil(2)
            .clamp(2, 4);
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                vits: OfflineTtsVitsModelConfig {
                    model: Some(model),
                    lexicon,
                    tokens: Some(tokens),
                    data_dir,
                    noise_scale: 0.667,
                    noise_scale_w: 0.8,
                    length_scale: 1.0,
                    dict_dir,
                },
                num_threads: i32::try_from(threads).unwrap_or(4),
                // sherpa debug logging prints full synthesis text, so it must stay disabled.
                debug: false,
                provider: Some(provider_name.into()),
                ..Default::default()
            },
            rule_fsts: (!rule_fsts.is_empty()).then_some(rule_fsts),
            max_num_sentences: 1,
            rule_fars: None,
            silence_scale: 0.2,
        };
        let inner = OfflineTts::create(&config)
            .ok_or_else(|| format!("{provider_name} Session 创建失败"))?;
        let sample_rate = inner.sample_rate();
        let speakers = inner.num_speakers();
        if sample_rate <= 0 || speakers <= 0 {
            return Err("模型采样率或说话人数无效".into());
        }
        let detected_speakers = u32::try_from(speakers).unwrap_or_default();
        if detected_speakers != asset.entry.speaker_count {
            return Err(format!(
                "模型实际包含 {detected_speakers} 个 Speaker，与导入记录 {} 不一致",
                asset.entry.speaker_count
            ));
        }
        if asset.entry.speaker_id >= detected_speakers {
            return Err(format!(
                "Speaker ID {} 超出有效范围 0–{}",
                asset.entry.speaker_id,
                detected_speakers.saturating_sub(1)
            ));
        }
        Ok(Self {
            inner,
            // The range check above is against sherpa's positive i32 count.
            speaker_id: asset.entry.speaker_id as i32,
            strip_cjk_punctuation: asset.entry.model_type.contains("piper")
                && asset
                    .entry
                    .languages
                    .iter()
                    .any(|language| language.starts_with("zh")),
        })
    }

    fn generate(&mut self, text: &str, speed: f32) -> Result<(Vec<f32>, u32), String> {
        let prepared;
        let text = if self.strip_cjk_punctuation {
            prepared = text
                .chars()
                .filter(|character| {
                    !matches!(
                        character,
                        '。' | '！' | '？' | '，' | '、' | '；' | '：' | '!' | '?' | ';' | ':'
                    )
                })
                .collect::<String>();
            prepared.trim()
        } else {
            text
        };
        if text.is_empty() {
            return Err("语音段没有可合成字符".into());
        }
        if text.contains('\0') {
            return Err("文本包含 NUL 字符".into());
        }
        let audio = self
            .inner
            .generate_with_config(
                text,
                &GenerationConfig {
                    sid: self.speaker_id,
                    speed,
                    ..Default::default()
                },
                None::<fn(&[f32], f32) -> bool>,
            )
            .ok_or_else(|| "原生运行时没有返回音频".to_owned())?;
        if audio.samples().is_empty() || audio.sample_rate() <= 0 {
            return Err("原生运行时返回了无效音频".into());
        }
        let samples = audio.samples().to_vec();
        let sample_rate = u32::try_from(audio.sample_rate())
            .map_err(|_| "原生运行时返回了无效采样率".to_owned())?;
        Ok((samples, sample_rate))
    }
}

fn supports_english(languages: &[String]) -> bool {
    languages
        .iter()
        .any(|language| language.to_ascii_lowercase().starts_with("en"))
}

fn english_dominant(text: &str) -> bool {
    let letters = text
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .count();
    let chinese = text
        .chars()
        .filter(|character| matches!(*character, '\u{3400}'..='\u{9fff}'))
        .count();
    letters >= 12 && letters > chinese.saturating_mul(2)
}

fn speech_envelope(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    if samples.is_empty() || sample_rate == 0 {
        return Vec::new();
    }
    let frame_size =
        (usize::try_from(sample_rate).unwrap_or_default() * usize::from(SPEECH_FRAME_MS) / 1_000)
            .max(1);
    let mut rms = samples
        .chunks(frame_size)
        .map(|frame| {
            (frame.iter().map(|sample| sample * sample).sum::<f32>() / frame.len().max(1) as f32)
                .sqrt()
        })
        .collect::<Vec<_>>();
    let mut sorted = rms.clone();
    sorted.sort_by(f32::total_cmp);
    let global_p95 = sorted
        .get(((sorted.len().saturating_sub(1)) as f32 * 0.95).round() as usize)
        .copied()
        .unwrap_or_default()
        .max(0.000_1);
    let floor = (global_p95 * 0.06).max(0.002);
    let segment_frames = (1_000 / usize::from(SPEECH_FRAME_MS)).max(1);
    let local_p95 = rms
        .chunks(segment_frames)
        .flat_map(|segment| {
            let mut sorted = segment.to_vec();
            sorted.sort_by(f32::total_cmp);
            let peak = sorted
                .get(((sorted.len().saturating_sub(1)) as f32 * 0.95).round() as usize)
                .copied()
                .unwrap_or(global_p95)
                .max(global_p95 * 0.35);
            std::iter::repeat_n(peak, segment.len())
        })
        .collect::<Vec<_>>();
    let mut smoothed = 0.0;
    for (value, peak) in rms.iter_mut().zip(local_p95) {
        let normalized = ((*value - floor) / (peak - floor).max(0.000_1)).clamp(0.0, 1.0);
        let time_constant_ms = if normalized > smoothed { 30.0 } else { 70.0 };
        let factor = 1.0 - (-f32::from(SPEECH_FRAME_MS) / time_constant_ms).exp();
        smoothed += (normalized - smoothed) * factor;
        *value = smoothed;
    }
    rms.into_iter()
        .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect()
}

fn energy_locked_timeline(samples: &[f32], sample_rate: u32) -> SpeechTimeline {
    SpeechTimeline {
        frame_duration_ms: SPEECH_FRAME_MS,
        jaw_open: speech_envelope(samples, sample_rate),
        visemes: None,
        quality: SpeechTimelineQuality::EnergyLocked,
    }
}

fn resolve_lip_sync_timeline(
    provider: Option<&dyn LipSyncProvider>,
    text: &str,
    samples: &[f32],
    sample_rate: u32,
    duration_ms: u32,
) -> SpeechTimeline {
    if let Some(timeline) =
        provider.and_then(|value| value.timeline(text, samples, sample_rate, duration_ms))
        && valid_phoneme_timeline(&timeline, duration_ms)
    {
        return timeline;
    }
    energy_locked_timeline(samples, sample_rate)
}

#[must_use]
pub fn valid_phoneme_timeline(timeline: &SpeechTimeline, duration_ms: u32) -> bool {
    if timeline.quality != SpeechTimelineQuality::PhonemeTimed
        || timeline.frame_duration_ms != SPEECH_FRAME_MS
        || timeline.jaw_open.len()
            != usize::try_from(duration_ms.div_ceil(u32::from(SPEECH_FRAME_MS)))
                .unwrap_or(usize::MAX)
        || timeline.visemes.as_ref().is_none_or(Vec::is_empty)
    {
        return false;
    }
    let mut previous = None;
    timeline.visemes.as_ref().is_some_and(|frames| {
        frames.iter().all(|frame| {
            let valid =
                frame.time_ms <= duration_ms && previous.is_none_or(|time| frame.time_ms > time);
            previous = Some(frame.time_ms);
            valid
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segmenter_handles_punctuation_hard_limits_and_tail() {
        let mut segmenter = SentenceSegmenter::default();
        assert!(segmenter.push("你好").is_empty());
        let first = segmenter.push("，世界！下一句");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].display_text, "你好，世界！");
        let tail = segmenter.finish();
        assert_eq!(tail[0].display_text, "下一句");
        assert_eq!(tail[0].text_start, first[0].text_end);
    }

    #[test]
    fn segmenter_caps_spoken_text_but_preserves_display_text() {
        let mut segmenter = SentenceSegmenter::default();
        let text = format!("{}。", "哈".repeat(1_100));
        let mut segments = segmenter.push(&text);
        segments.extend(segmenter.finish());
        assert_eq!(
            segments
                .iter()
                .map(|value| value.speech_text.chars().count())
                .sum::<usize>(),
            1_000
        );
        assert_eq!(
            segments
                .iter()
                .map(|value| value.display_text.chars().count())
                .sum::<usize>(),
            1_101
        );
    }

    #[test]
    fn english_detection_ignores_short_abbreviations() {
        assert!(!english_dominant("你好，AI 和 GPU 都可以。"));
        assert!(english_dominant(
            "This sentence should remain visible without Chinese speech."
        ));
    }

    #[test]
    fn envelope_tracks_noise_gate_attack_and_release() {
        let frame_samples = 16_000 * usize::from(SPEECH_FRAME_MS) / 1_000;
        let mut samples = vec![0.001; frame_samples * 4];
        samples.extend(std::iter::repeat_n(0.5, frame_samples * 5));
        samples.extend(std::iter::repeat_n(0.0, frame_samples * 6));
        let envelope = speech_envelope(&samples, 16_000);
        assert_eq!(envelope.len(), 15);
        assert!(envelope[..4].iter().all(|value| *value < 8));
        assert!(envelope[4] > envelope[3]);
        assert!(envelope[5] > envelope[4]);
        assert!(envelope[9] < envelope[8]);
        assert!(envelope[9] > envelope[14]);
        assert!(envelope[9] > 0, "release must not snap directly to zero");
    }

    #[test]
    fn energy_timeline_is_twenty_millisecond_jaw_only_data() {
        let samples = vec![0.25; 16_000 / 5];
        let timeline = energy_locked_timeline(&samples, 16_000);
        assert_eq!(timeline.frame_duration_ms, 20);
        assert_eq!(timeline.jaw_open.len(), 10);
        assert_eq!(timeline.quality, SpeechTimelineQuality::EnergyLocked);
        assert!(timeline.visemes.is_none());
    }

    #[test]
    fn phoneme_timeline_requires_monotonic_bounded_frames() {
        let frame = |time_ms| hachimi_protocol::SpeechVisemeFrame {
            time_ms,
            aa: 255,
            ih: 0,
            ou: 0,
            ee: 0,
            oh: 0,
        };
        let valid = SpeechTimeline {
            frame_duration_ms: 20,
            jaw_open: vec![0, 128, 255],
            visemes: Some(vec![frame(0), frame(20), frame(60)]),
            quality: SpeechTimelineQuality::PhonemeTimed,
        };
        assert!(valid_phoneme_timeline(&valid, 60));

        let mut duplicate = valid.clone();
        duplicate.visemes = Some(vec![frame(20), frame(20)]);
        assert!(!valid_phoneme_timeline(&duplicate, 60));

        let mut outside_pcm = valid.clone();
        outside_pcm.visemes = Some(vec![frame(61)]);
        assert!(!valid_phoneme_timeline(&outside_pcm, 60));

        let mut energy = valid;
        energy.quality = SpeechTimelineQuality::EnergyLocked;
        assert!(!valid_phoneme_timeline(&energy, 60));
    }

    #[derive(Clone)]
    struct FakeLipSyncProvider(SpeechTimeline);

    impl LipSyncProvider for FakeLipSyncProvider {
        fn timeline(
            &self,
            _text: &str,
            _samples: &[f32],
            _sample_rate: u32,
            _duration_ms: u32,
        ) -> Option<SpeechTimeline> {
            Some(self.0.clone())
        }
    }

    #[test]
    fn lip_sync_provider_is_used_only_for_a_verified_timeline() {
        let frame = |time_ms| hachimi_protocol::SpeechVisemeFrame {
            time_ms,
            aa: 255,
            ih: 0,
            ou: 0,
            ee: 0,
            oh: 0,
        };
        let valid = SpeechTimeline {
            frame_duration_ms: 20,
            jaw_open: vec![0, 128, 255],
            visemes: Some(vec![frame(0), frame(20), frame(40)]),
            quality: SpeechTimelineQuality::PhonemeTimed,
        };
        let samples = vec![0.25; 960];
        let selected = resolve_lip_sync_timeline(
            Some(&FakeLipSyncProvider(valid.clone())),
            "测试。",
            &samples,
            16_000,
            60,
        );
        assert_eq!(selected, valid);

        let mut invalid = valid;
        invalid.visemes = Some(vec![frame(20), frame(20)]);
        let fallback = resolve_lip_sync_timeline(
            Some(&FakeLipSyncProvider(invalid)),
            "测试。",
            &samples,
            16_000,
            60,
        );
        assert_eq!(fallback.quality, SpeechTimelineQuality::EnergyLocked);
        assert!(fallback.visemes.is_none());
    }

    #[test]
    fn playback_events_are_sequenced_and_timeline_only_appears_on_prepared() {
        let events = Arc::new(Mutex::new(Vec::<SpeechPlaybackEvent>::new()));
        let captured = Arc::clone(&events);
        let sink: VoiceEventSink = Arc::new(move |event| {
            captured.lock().expect("event lock").push(event);
        });
        let segment = SentenceSegment {
            index: 2,
            display_text: "测试。".into(),
            speech_text: "测试。".into(),
            text_start: 4,
            text_end: 7,
        };
        let timeline = energy_locked_timeline(&vec![0.25; 3_200], 16_000);
        let mut sequence = 0;
        emit_segment(
            Some(&sink),
            &mut sequence,
            9,
            SpeechPlaybackSource::PetTurn,
            Some("run-1"),
            &segment,
            SpeechPlaybackPhase::Prepared,
            0,
            200,
            Some(timeline.clone()),
        );
        emit_segment(
            Some(&sink),
            &mut sequence,
            9,
            SpeechPlaybackSource::PetTurn,
            Some("run-1"),
            &segment,
            SpeechPlaybackPhase::Playing,
            20,
            200,
            Some(timeline),
        );
        emit_segment(
            Some(&sink),
            &mut sequence,
            9,
            SpeechPlaybackSource::PetTurn,
            Some("run-1"),
            &segment,
            SpeechPlaybackPhase::Progress,
            40,
            200,
            None,
        );

        let events = events.lock().expect("event lock");
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(
            events
                .iter()
                .map(|event| event.media_position_ms)
                .collect::<Vec<_>>(),
            vec![0, 20, 40]
        );
        assert!(events[0].timeline.is_some());
        assert!(events[1..].iter().all(|event| event.timeline.is_none()));
        assert_eq!(events[0].display_text.as_deref(), Some("测试。"));
        assert!(events[1..].iter().all(|event| event.display_text.is_none()));
    }

    #[test]
    fn controlled_media_clock_stays_within_one_timeline_frame() {
        let mut tracker = PlaybackProgressTracker::default();
        let fake_media_positions = [0, 7, 21, 21, 42, 63, 82, 103];
        let observed = fake_media_positions
            .into_iter()
            .filter_map(|position| tracker.observe(position).map(|phase| (position, phase)))
            .collect::<Vec<_>>();
        assert_eq!(observed[0].1, SpeechPlaybackPhase::Playing);
        assert!(
            observed[1..]
                .iter()
                .all(|(_, phase)| *phase == SpeechPlaybackPhase::Progress)
        );
        assert!(observed.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert!(observed.iter().all(|(position, _)| {
            let timeline_frame_ms =
                position / u32::from(SPEECH_FRAME_MS) * u32::from(SPEECH_FRAME_MS);
            position - timeline_frame_ms < u32::from(SPEECH_FRAME_MS)
        }));
    }

    #[test]
    fn turn_outcome_reports_language_skips_and_synthesis_failures() {
        assert_eq!(
            speech_turn_outcome(TurnPlaybackOutcome {
                skipped_language: true,
                synthesis_failed: false,
            }),
            (
                SpeechTurnPhase::Skipped,
                Some("voice_language_unsupported".into())
            )
        );
        assert_eq!(
            speech_turn_outcome(TurnPlaybackOutcome {
                skipped_language: true,
                synthesis_failed: true,
            }),
            (SpeechTurnPhase::Failed, Some("voice_segment_failed".into()))
        );
    }

    #[cfg(windows)]
    #[test]
    fn bundled_melo_model_completes_bilingual_inference() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../apps/desktop/src-tauri/resources/ai-models/text-to-speech/vits-melo-zh-en",
        );
        if !root.is_dir() {
            return;
        }
        let temporary = tempfile::tempdir().expect("temporary catalog");
        let catalog = crate::VoiceCatalog::load(temporary.path(), root).expect("voice catalog");
        let asset = catalog.current_asset().expect("bundled voice asset");
        let (mut engine, backend, compute_device, _) =
            load_with_fallback(&asset, VoiceComputeMode::Cpu).expect("CPU VITS warm-up");
        assert_eq!(backend, VoiceComputeBackend::Cpu);
        assert!(compute_device.is_none());
        let (samples, sample_rate) = engine.generate("你好。", 1.0).expect("Chinese inference");
        assert_eq!(sample_rate, 44_100);
        assert!(!samples.is_empty());
        let (samples, sample_rate) = engine
            .generate("Hello from Hachimi.", 1.0)
            .expect("English inference");
        assert_eq!(sample_rate, 44_100);
        assert!(!samples.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn bundled_melo_model_warms_up_selected_backend() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../apps/desktop/src-tauri/resources/ai-models/text-to-speech/vits-melo-zh-en",
        );
        if !root.is_dir() {
            return;
        }
        let temporary = tempfile::tempdir().expect("temporary catalog");
        let catalog = crate::VoiceCatalog::load(temporary.path(), root).expect("voice catalog");
        let asset = catalog.current_asset().expect("bundled voice asset");
        let directml_device_available = super::super::directml_adapters().is_ok();
        let (mut engine, backend, compute_device, fallback) =
            load_with_fallback(&asset, VoiceComputeMode::Auto).expect("VITS backend warm-up");
        eprintln!("VITS warm-up backend: {backend:?}");
        if directml_device_available {
            assert_eq!(backend, VoiceComputeBackend::DirectMl);
            assert!(compute_device.is_some());
            assert!(fallback.is_none());
        } else {
            assert_eq!(backend, VoiceComputeBackend::Cpu);
            assert!(compute_device.is_none());
            assert!(fallback.is_some());
        }
        let (samples, sample_rate) = engine.generate("你好。", 1.0).expect("Chinese inference");
        assert_eq!(sample_rate, 44_100);
        assert!(!samples.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn failed_model_warmup_restores_the_previous_runtime() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../apps/desktop/src-tauri/resources/ai-models/text-to-speech/vits-melo-zh-en",
        );
        if !root.is_dir() {
            return;
        }
        let temporary = tempfile::tempdir().expect("temporary catalog");
        let catalog = crate::VoiceCatalog::load(temporary.path(), root).expect("voice catalog");
        let valid = catalog.current_asset().expect("bundled voice asset");
        let runtime = VoiceRuntime::start_with_event_sink(
            None,
            true,
            100,
            VoiceComputeMode::Cpu,
            VoiceRuntimeEventSinks::default(),
        );
        runtime
            .load_model(Some(valid.clone()), VoiceComputeMode::Cpu)
            .expect("initial model warm-up");

        let mut invalid = valid.clone();
        invalid.root = temporary.path().join("missing-model");
        assert!(
            runtime
                .load_model_with_rollback(
                    Some(invalid),
                    Some(valid.clone()),
                    VoiceComputeMode::Cpu,
                )
                .is_err()
        );
        let state = runtime.state();
        assert!(state.available);
        assert_eq!(state.model_id.as_deref(), Some(valid.entry.id.as_str()));
        assert_eq!(state.backend, Some(VoiceComputeBackend::Cpu));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires local archives listed in HACHIMI_TEST_VOICE_ARCHIVES"]
    fn imported_archives_complete_cpu_and_auto_warmup_with_selected_speaker() {
        let archives = std::env::var("HACHIMI_TEST_VOICE_ARCHIVES")
            .expect("set HACHIMI_TEST_VOICE_ARCHIVES to semicolon-separated archive paths");
        for (index, path) in archives
            .split(';')
            .filter(|value| !value.is_empty())
            .enumerate()
        {
            let source = std::path::Path::new(path);
            let inspection = crate::inspect_voice_archive(source).expect("inspect real archive");
            let temporary = tempfile::tempdir().expect("temporary catalog");
            let mut catalog = crate::VoiceCatalog::load(
                temporary.path().join("catalog"),
                temporary.path().join("missing-builtin"),
            )
            .expect("voice catalog");
            let snapshot = catalog
                .import_inspected(
                    &format!("Real voice {index}"),
                    source,
                    &inspection,
                    true,
                    inspection.suggested_speaker_id,
                )
                .expect("import real archive");
            let entry = snapshot.entries.last().expect("imported entry");
            let asset = catalog.asset(&entry.id).expect("imported asset");
            let (mut engine, backend, compute_device, _) =
                load_with_fallback(&asset, VoiceComputeMode::Cpu).expect("CPU VITS warm-up");
            assert_eq!(backend, VoiceComputeBackend::Cpu);
            assert!(compute_device.is_none());
            let sample_text = if supports_english(&inspection.languages) {
                "Hello. This is an English voice test."
            } else {
                "你好，Hachimi。这是中文语音测试。"
            };
            let (samples, sample_rate) = engine.generate(sample_text, 1.0).expect("inference");
            assert_eq!(sample_rate, inspection.sample_rate);
            assert!(!samples.is_empty());

            let (mut engine, auto_backend, auto_device, fallback) =
                load_with_fallback(&asset, VoiceComputeMode::Auto).expect("Auto VITS warm-up");
            eprintln!(
                "{}: Auto backend={auto_backend:?}, device={auto_device:?}, fallback={fallback:?}",
                source
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .unwrap_or("voice archive")
            );
            if super::super::directml_adapters().is_ok() {
                assert_eq!(auto_backend, VoiceComputeBackend::DirectMl);
                assert!(auto_device.is_some());
                assert!(fallback.is_none());
            } else {
                assert_eq!(auto_backend, VoiceComputeBackend::Cpu);
                assert!(auto_device.is_none());
                assert!(fallback.is_some());
            }
            let (samples, sample_rate) = engine
                .generate(sample_text, 1.0)
                .expect("Auto backend inference");
            assert_eq!(sample_rate, inspection.sample_rate);
            assert!(!samples.is_empty());
        }
    }
}
