use anyhow::{anyhow, Context, Result};
use crate::layout::HighlightState;
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const PRELOAD_AHEAD: usize = 5;
const MIN_SPEED: f64 = 0.5;
const MAX_SPEED: f64 = 2.0;
pub const SPEED_STEP: f64 = 0.1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceStatus {
    Idle,
    Loading,
    Speaking,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct SpeakerInfo {
    pub id: u32,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct VoiceSettingsView {
    pub speaker_id: u32,
    pub speaker_label: String,
    pub speed: f64,
}

pub struct VoiceReader {
    base_url: String,
    speakers: Vec<SpeakerInfo>,
    speaker_index: usize,
    speed: f64,
    stop_flag: Arc<AtomicBool>,
    status: Arc<Mutex<VoiceStatus>>,
    highlight: Arc<Mutex<HighlightState>>,
    current_chunk: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
}

impl VoiceReader {
    pub fn new(
        base_url: impl Into<String>,
        speaker: u32,
        speed: f64,
        speakers: Vec<SpeakerInfo>,
    ) -> Self {
        let speakers = if speakers.is_empty() {
            vec![SpeakerInfo {
                id: speaker,
                label: format!("話者 {speaker}"),
            }]
        } else {
            speakers
        };

        let speaker_index = speakers
            .iter()
            .position(|info| info.id == speaker)
            .unwrap_or(0);

        Self {
            base_url: base_url.into(),
            speakers,
            speaker_index,
            speed: clamp_speed(speed),
            stop_flag: Arc::new(AtomicBool::new(false)),
            status: Arc::new(Mutex::new(VoiceStatus::Idle)),
            highlight: Arc::new(Mutex::new(HighlightState::default())),
            current_chunk: Arc::new(AtomicUsize::new(0)),
            worker: None,
        }
    }

    pub fn status(&self) -> VoiceStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn settings_view(&self) -> VoiceSettingsView {
        let speaker = &self.speakers[self.speaker_index];
        VoiceSettingsView {
            speaker_id: speaker.id,
            speaker_label: speaker.label.clone(),
            speed: self.speed,
        }
    }

    pub fn speaker_id(&self) -> u32 {
        self.speakers[self.speaker_index].id
    }

    pub fn prev_speaker(&mut self) {
        if self.speakers.is_empty() {
            return;
        }
        if self.speaker_index == 0 {
            self.speaker_index = self.speakers.len() - 1;
        } else {
            self.speaker_index -= 1;
        }
    }

    pub fn next_speaker(&mut self) {
        if self.speakers.is_empty() {
            return;
        }
        self.speaker_index = (self.speaker_index + 1) % self.speakers.len();
    }

    pub fn adjust_speed(&mut self, delta: f64) {
        self.speed = clamp_speed(self.speed + delta);
    }

    pub fn highlight_state(&self) -> HighlightState {
        self.highlight.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = VoiceStatus::Idle;
        self.highlight.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.current_chunk.store(0, Ordering::Relaxed);
    }

    pub fn speak_plan(&mut self, plan: crate::layout::SpeechPlan) {
        self.stop();
        self.stop_flag.store(false, Ordering::SeqCst);

        if plan.chunks.is_empty() || plan.normalized_text.is_empty() {
            *self.status.lock().unwrap_or_else(|e| e.into_inner()) =
                VoiceStatus::Error("読み上げるテキストがありません".to_string());
            return;
        }

        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = VoiceStatus::Loading;
        *self.highlight.lock().unwrap_or_else(|e| e.into_inner()) =
            HighlightState::from_plan(&plan);

        let base_url = self.base_url.clone();
        let speaker = self.speaker_id();
        let speed = self.speed;
        let stop_flag = Arc::clone(&self.stop_flag);
        let status = Arc::clone(&self.status);
        let highlight = Arc::clone(&self.highlight);
        let current_chunk = Arc::clone(&self.current_chunk);
        let chunks = plan.chunks;

        self.worker = Some(thread::spawn(move || {
            run_speak_session(SpeakJob {
                chunks,
                base_url,
                speaker,
                speed,
                runtime: SpeakRuntime {
                    stop_flag,
                    status,
                    highlight,
                    current_chunk,
                },
            });
        }));
    }
}

type WavBytes = Vec<u8>;
type WavResult = Result<WavBytes, String>;
type ChunkSlots = Mutex<Vec<Option<WavResult>>>;

#[derive(Clone)]
struct SpeakRuntime {
    stop_flag: Arc<AtomicBool>,
    status: Arc<Mutex<VoiceStatus>>,
    highlight: Arc<Mutex<HighlightState>>,
    current_chunk: Arc<AtomicUsize>,
}

struct SpeakJob {
    chunks: Vec<String>,
    base_url: String,
    speaker: u32,
    speed: f64,
    runtime: SpeakRuntime,
}

struct PreloadJob {
    chunks: Vec<String>,
    cache: Arc<ChunkCache>,
    inflight: Arc<Mutex<HashSet<usize>>>,
    playhead: Arc<AtomicUsize>,
    base_url: String,
    speaker: u32,
    speed: f64,
    stop_flag: Arc<AtomicBool>,
    prefetch_stop: Arc<AtomicBool>,
}

struct ChunkCache {
    slots: ChunkSlots,
    ready: Condvar,
}

impl ChunkCache {
    fn new(len: usize) -> Self {
        Self {
            slots: Mutex::new(vec![None; len]),
            ready: Condvar::new(),
        }
    }

    fn set(&self, index: usize, result: Result<Vec<u8>, String>) {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots[index] = Some(result);
        self.ready.notify_all();
    }

    fn take(&self, index: usize, stop: &AtomicBool) -> Result<Vec<u8>> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if stop.load(Ordering::SeqCst) {
                anyhow::bail!("読み上げを停止しました");
            }
            match &slots[index] {
                Some(Ok(wav)) => return Ok(wav.clone()),
                Some(Err(message)) => anyhow::bail!(message.clone()),
                None => {
                    let (guard, _) = self
                        .ready
                        .wait_timeout(slots, Duration::from_millis(100))
                        .unwrap_or_else(|e| e.into_inner());
                    slots = guard;
                }
            }
        }
    }

    fn is_pending(&self, index: usize) -> bool {
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())[index]
            .is_none()
    }
}

fn run_speak_session(job: SpeakJob) {
    let chunks = job.chunks;
    let base_url = job.base_url;
    let speaker = job.speaker;
    let speed = job.speed;
    let runtime = job.runtime;
    let stop_flag = Arc::clone(&runtime.stop_flag);
    let status = Arc::clone(&runtime.status);
    let highlight = Arc::clone(&runtime.highlight);
    let current_chunk = Arc::clone(&runtime.current_chunk);

    if chunks.is_empty() {
        *status.lock().unwrap_or_else(|e| e.into_inner()) = VoiceStatus::Idle;
        highlight.lock().unwrap_or_else(|e| e.into_inner()).clear();
        return;
    }

    let cache = Arc::new(ChunkCache::new(chunks.len()));
    let inflight = Arc::new(Mutex::new(HashSet::<usize>::new()));
    let playhead = Arc::new(AtomicUsize::new(0));
    let prefetch_stop = Arc::new(AtomicBool::new(false));

    let prefetch = {
        let prefetch_job = PreloadJob {
            chunks: chunks.clone(),
            cache: Arc::clone(&cache),
            inflight: Arc::clone(&inflight),
            playhead: Arc::clone(&playhead),
            base_url: base_url.clone(),
            speaker,
            speed,
            stop_flag: Arc::clone(&stop_flag),
            prefetch_stop: Arc::clone(&prefetch_stop),
        };
        thread::spawn(move || preload_chunks(prefetch_job))
    };

    let playback_result: Result<(), anyhow::Error> = (|| {
        let mut stream_handle = rodio::OutputStreamBuilder::open_default_stream()
            .context("音声出力デバイスを開けませんでした")?;
        stream_handle.log_on_drop(false);
        let sink = rodio::Sink::connect_new(stream_handle.mixer());

        for index in 0..chunks.len() {
            if stop_flag.load(Ordering::SeqCst) {
                break;
            }

            playhead.store(index, Ordering::Relaxed);
            current_chunk.store(index, Ordering::Relaxed);
            highlight
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .set_active_chunk(index);
            *status.lock().unwrap_or_else(|e| e.into_inner()) = VoiceStatus::Speaking;

            let wav = cache.take(index, &stop_flag)?;
            if stop_flag.load(Ordering::SeqCst) {
                break;
            }
            play_on_sink(&sink, &wav, &stop_flag)?;
            if stop_flag.load(Ordering::SeqCst) {
                break;
            }
        }

        sink.stop();
        wait_sink_empty(&sink);

        Ok(())
    })();

    prefetch_stop.store(true, Ordering::SeqCst);
    let _ = prefetch.join();

    highlight.lock().unwrap_or_else(|e| e.into_inner()).clear();
    current_chunk.store(0, Ordering::Relaxed);

    match playback_result {
        Ok(()) => {
            *status.lock().unwrap_or_else(|e| e.into_inner()) = VoiceStatus::Idle;
        }
        Err(_error) if stop_flag.load(Ordering::SeqCst) => {
            *status.lock().unwrap_or_else(|e| e.into_inner()) = VoiceStatus::Idle;
        }
        Err(error) => {
            *status.lock().unwrap_or_else(|e| e.into_inner()) =
                VoiceStatus::Error(error.to_string());
        }
    }
}

fn preload_chunks(job: PreloadJob) {
    let chunks = job.chunks;
    let cache = job.cache;
    let inflight = job.inflight;
    let playhead = job.playhead;
    let base_url = job.base_url;
    let speaker = job.speaker;
    let speed = job.speed;
    let stop_flag = job.stop_flag;
    let prefetch_stop = job.prefetch_stop;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("HTTP クライアントの初期化に失敗しました");

    while !prefetch_stop.load(Ordering::SeqCst) && !stop_flag.load(Ordering::SeqCst) {
        let play_idx = playhead.load(Ordering::Relaxed);
        let end = (play_idx + PRELOAD_AHEAD + 1).min(chunks.len());

        let mut to_load = Vec::new();
        {
            let inflight_guard = inflight.lock().unwrap_or_else(|e| e.into_inner());
            for index in 0..end {
                if cache.is_pending(index) && !inflight_guard.contains(&index) {
                    to_load.push(index);
                }
            }
        }

        for index in to_load {
            if prefetch_stop.load(Ordering::SeqCst) || stop_flag.load(Ordering::SeqCst) {
                return;
            }

            inflight.lock().unwrap_or_else(|e| e.into_inner()).insert(index);
            let result = synthesize_voicevox(
                &client,
                &base_url,
                &chunks[index],
                speaker,
                speed,
            )
            .map_err(|error| error.to_string());
            cache.set(index, result);
            inflight.lock().unwrap_or_else(|e| e.into_inner()).remove(&index);
        }

        if end >= chunks.len() && all_chunks_cached(&cache, chunks.len()) {
            break;
        }

        thread::sleep(Duration::from_millis(20));
    }
}

fn all_chunks_cached(cache: &ChunkCache, len: usize) -> bool {
    let slots = cache.slots.lock().unwrap_or_else(|e| e.into_inner());
    slots.iter().take(len).all(|slot| slot.is_some())
}

pub fn fetch_speakers(base_url: &str) -> Result<Vec<SpeakerInfo>> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("HTTP クライアントの初期化に失敗しました")?;

    let url = format!("{base_url}/speakers");
    let speakers: Vec<serde_json::Value> = client
        .get(&url)
        .send()
        .context("話者一覧の取得に失敗しました")?
        .json()
        .context("話者一覧の解析に失敗しました")?;

    let mut result = Vec::new();
    for speaker in speakers {
        let name = speaker
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("不明");
        if let Some(styles) = speaker.get("styles").and_then(|value| value.as_array()) {
            for style in styles {
                let style_name = style
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("ノーマル");
                let id = style.get("id").and_then(|value| value.as_u64()).unwrap_or(0) as u32;
                result.push(SpeakerInfo {
                    id,
                    label: format!("{name} ({style_name})"),
                });
            }
        }
    }

    result.sort_by_key(|info| info.id);
    Ok(result)
}

fn clamp_speed(speed: f64) -> f64 {
    speed.clamp(MIN_SPEED, MAX_SPEED)
}

fn synthesize_voicevox(
    client: &reqwest::blocking::Client,
    base_url: &str,
    text: &str,
    speaker: u32,
    speed: f64,
) -> Result<Vec<u8>> {
    let speaker_s = speaker.to_string();
    let query_url = format!("{base_url}/audio_query");
    let synthesis_url = format!("{base_url}/synthesis");

    let mut query: serde_json::Value = client
        .post(&query_url)
        .query(&[("text", text), ("speaker", speaker_s.as_str())])
        .send()
        .context("VOICEVOX への接続に失敗しました。engine が起動しているか確認してください")?
        .json()
        .context("audio_query の応答を解析できませんでした")?;

    if let Some(obj) = query.as_object_mut() {
        obj.insert("speedScale".to_string(), serde_json::json!(speed));
    }

    let wav = client
        .post(&synthesis_url)
        .query(&[("speaker", speaker_s.as_str())])
        .json(&query)
        .send()
        .context("音声合成リクエストに失敗しました")?
        .bytes()
        .context("合成音声の取得に失敗しました")?;

    Ok(wav.to_vec())
}

fn validate_wav_bytes(wav: &[u8]) -> Result<()> {
    if wav.len() < 12 {
        anyhow::bail!("音声データが空です");
    }
    if wav.starts_with(b"RIFF") {
        return Ok(());
    }

    let preview = String::from_utf8_lossy(&wav[..wav.len().min(120)]);
    anyhow::bail!("VOICEVOX が WAV ではなくエラーを返しました: {preview}");
}

fn play_on_sink(sink: &rodio::Sink, wav: &[u8], stop: &AtomicBool) -> Result<()> {
    validate_wav_bytes(wav)?;
    let cursor = Cursor::new(wav.to_vec());
    let source = rodio::Decoder::new_wav(cursor)
        .map_err(|error| anyhow!("WAV のデコードに失敗しました: {error}"))?;
    sink.append(source);

    while !sink.empty() {
        if stop.load(Ordering::SeqCst) {
            sink.stop();
            wait_sink_empty(sink);
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }

    Ok(())
}

fn wait_sink_empty(sink: &rodio::Sink) {
    for _ in 0..100 {
        if sink.empty() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_wav_response() {
        let error = validate_wav_bytes(b"{\"detail\":\"error\"}").unwrap_err();
        assert!(error.to_string().contains("WAV ではなく"));
    }

    #[test]
    fn clamp_speed_within_bounds() {
        assert_eq!(clamp_speed(0.1), MIN_SPEED);
        assert_eq!(clamp_speed(3.0), MAX_SPEED);
        assert_eq!(clamp_speed(1.2), 1.2);
    }

    #[test]
    fn chunk_cache_waits_until_ready() {
        let cache = Arc::new(ChunkCache::new(1));
        let cache_clone = Arc::clone(&cache);
        let stop = Arc::new(AtomicBool::new(false));

        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            cache_clone.set(0, Ok(vec![0, 1, 2]));
        });

        let wav = cache.take(0, &stop).expect("chunk should become ready");
        assert_eq!(wav, vec![0, 1, 2]);
        writer.join().unwrap();
    }
}
