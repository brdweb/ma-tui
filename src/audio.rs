//! Embedded, authenticated Music Assistant Sendspin player.
//!
//! Compatibility source: music-assistant/server tag 2.10.2,
//! controllers/webserver/controller.py (`GET /sendspin`) and
//! controllers/webserver/sendspin_proxy.py (`auth` then `auth_ok`).
//!
//! App-owned handoffs are bounded and never wait for the output thread. The
//! pinned Sendspin 0.3.7 router itself uses unbounded receivers, and SyncedPlayer
//! takes internal queue locks in its callback: this is not a hard-real-time or
//! globally lock-free guarantee. Its split receivers also lack stream sequence
//! IDs, so boundary handling deliberately discards queued audio for safety.
//! Do not enable upstream Sendspin payload logging when installing a logger;
//! all errors/status emitted by this module are intentionally sanitized.
use anyhow::{bail, Result};
use base64::Engine;
use sendspin::audio::decode::{Decoder, FlacDecoder, OpusDecoder, PcmDecoder, PcmEndian};
use sendspin::audio::{AudioFormat, Codec};
use sendspin::protocol::messages::{AudioFormatSpec, StreamPlayerConfig};

#[path = "audio_health.rs"]
mod health;
use health::{CallbackStats, HealthDecision, HealthMonitor};

pub(crate) struct StreamDecoder {
    format: AudioFormat,
    decoder: Box<dyn Decoder>,
}
impl StreamDecoder {
    pub(crate) fn new(config: &StreamPlayerConfig, supported: &[AudioFormatSpec]) -> Result<Self> {
        if !supported.iter().any(|s| {
            s.codec == config.codec
                && s.channels == config.channels
                && s.sample_rate == config.sample_rate
                && s.bit_depth == config.bit_depth
        }) || !matches!(config.channels, 1 | 2)
            || !matches!(config.bit_depth, 16 | 24)
            || config.sample_rate == 0
        {
            bail!("Unsupported audio stream format");
        }
        let header = config
            .codec_header
            .as_ref()
            .map(|h| {
                base64::prelude::BASE64_STANDARD
                    .decode(h)
                    .map_err(|_| anyhow::anyhow!("Invalid audio codec header"))
            })
            .transpose()?;
        if config.codec == "flac" {
            if let Some(h) = &header {
                if h.len() < 42 || &h[..4] != b"fLaC" || h[4] & 0x7f != 0 || h[5..8] != [0, 0, 34] {
                    bail!("Invalid FLAC stream information");
                }
                let packed = u64::from_be_bytes(
                    h[18..26]
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("Invalid FLAC stream information"))?,
                );
                if (packed >> 44) as u32 != config.sample_rate
                    || ((packed >> 41) & 7) as u8 + 1 != config.channels
                    || ((packed >> 36) & 31) as u8 + 1 != config.bit_depth
                {
                    bail!("FLAC stream information disagrees with negotiation");
                }
            }
        }
        let (codec, decoder): (_, Box<dyn Decoder>) = match config.codec.as_str() {
            "pcm" => (
                Codec::Pcm,
                Box::new(PcmDecoder::with_endian(config.bit_depth, PcmEndian::Little)),
            ),
            "flac" => (
                Codec::Flac,
                Box::new(match &header {
                    Some(h) => FlacDecoder::with_header(h)
                        .map_err(|_| anyhow::anyhow!("Invalid FLAC header"))?,
                    None => FlacDecoder::new(),
                }),
            ),
            "opus" => (
                Codec::Opus,
                Box::new(
                    OpusDecoder::new(config.sample_rate, config.channels)
                        .map_err(|_| anyhow::anyhow!("Invalid Opus format"))?,
                ),
            ),
            _ => bail!("Unsupported audio codec"),
        };
        Ok(Self {
            format: AudioFormat {
                codec,
                channels: config.channels,
                sample_rate: config.sample_rate,
                bit_depth: config.bit_depth,
                codec_header: header,
            },
            decoder,
        })
    }
    pub(crate) fn decode(&mut self, data: &[u8]) -> Result<std::sync::Arc<[i32]>> {
        let frame = self.format.channels as usize * (self.format.bit_depth as usize / 8);
        if data.len() > 1024 * 1024
            || (self.format.codec == Codec::Pcm && !data.len().is_multiple_of(frame))
        {
            bail!("Invalid audio frame size");
        }
        let samples = self
            .decoder
            .decode(data)
            .map_err(|_| anyhow::anyhow!("Audio decoding failed"))?;
        if samples.len() % self.format.channels as usize != 0
            || samples.len() > self.format.sample_rate as usize * self.format.channels as usize * 2
        {
            bail!("Invalid decoded audio size");
        }
        Ok(samples)
    }
}

use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::{tungstenite::Message as WsMessage, MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Authentication errors deliberately contain neither credentials nor peer input.
pub(crate) async fn authenticate(
    url: &url::Url,
    token: &str,
    id: &str,
    deadline: Duration,
) -> Result<Socket> {
    tokio::time::timeout(deadline, async {
        let (mut socket, _) = tokio_tungstenite::connect_async_with_config(
            url.as_str(),
            Some(
                tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                    .max_message_size(Some(1024 * 1024))
                    .max_frame_size(Some(1024 * 1024)),
            ),
            false,
        )
        .await
        .map_err(|_| anyhow::anyhow!("Audio proxy connection failed"))?;
        socket
            .send(WsMessage::text(
                serde_json::json!({"type":"auth", "token":token, "client_id":id}).to_string(),
            ))
            .await
            .map_err(|_| anyhow::anyhow!("Audio proxy authentication send failed"))?;
        loop {
            match socket.next().await {
                Some(Ok(WsMessage::Text(text))) => {
                    let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
                        anyhow::anyhow!("Invalid audio proxy authentication response")
                    })?;
                    if value.get("type").and_then(|v| v.as_str()) == Some("auth_ok") {
                        return Ok(socket);
                    }
                    bail!("Audio proxy authentication rejected");
                }
                Some(Ok(WsMessage::Ping(data))) => socket
                    .send(WsMessage::Pong(data))
                    .await
                    .map_err(|_| anyhow::anyhow!("Audio proxy disconnected"))?,
                _ => bail!("Audio proxy authentication failed"),
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("Audio proxy authentication timed out"))?
}

use sendspin::audio::AudioBuffer;
use sendspin::protocol::messages::{
    ClientState, Message, PlayerCommandType, PlayerState, PlayerStateCommand, PlayerV1Support,
};
use sendspin::ProtocolClientBuilder;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc as thread_channel, Arc,
};
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone, Debug, Default)]
pub struct AudioStatus {
    pub state: String,
    pub detail: String,
}
// Deliberately no Debug implementation: this contains a credential.
pub struct AudioConfig {
    pub server: String,
    pub token: String,
    pub player_id: String,
    pub player_name: String,
    pub device_id: Option<String>,
    pub output_buffer_frames: Option<u32>,
    pub volume: u8,
    pub muted: bool,
}
pub struct AudioHandle {
    pub status: watch::Receiver<AudioStatus>,
    cancel: watch::Sender<bool>,
    stop: Arc<AtomicBool>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl AudioHandle {
    pub async fn shutdown(mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.cancel.send(true);
        if let Some(mut task) = self.task.take() {
            if tokio::time::timeout(Duration::from_secs(3), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}
impl Drop for AudioHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.cancel.send(true);
    }
}
pub(crate) type SharedClock = Arc<parking_lot::Mutex<sendspin::sync::ClockSync>>;
#[derive(Clone, Copy)]
pub(crate) struct Gain {
    pub volume: u8,
    pub muted: bool,
    pub delay: u16,
}
impl Gain {
    fn state(self) -> PlayerState {
        PlayerState {
            volume: Some(self.volume),
            muted: Some(self.muted),
            static_delay_ms: Some(self.delay),
            required_lead_time_ms: Some(500),
            min_buffer_ms: Some(500),
            supported_commands: Some(vec![PlayerStateCommand::SetStaticDelay]),
        }
    }
}
/// Receives decoded samples that are about to be played, with the local
/// instant the output is scheduled to emit them. Declared here so the audio
/// module does not depend on the interface; only real device output has one.
pub trait SampleSink: Send + Sync + 'static {
    fn push(&self, samples: &[i32], channels: u8, rate: u32, emitted: std::time::Instant);
    fn set_muted(&self, muted: bool);
    fn clear(&self);
}

// Constructed and destroyed on the audio thread; no Send requirement on CPAL.
pub(crate) trait Output: 'static {
    fn formats(&self) -> Result<Vec<AudioFormatSpec>>;
    fn begin(&mut self, format: AudioFormat, clock: SharedClock, gain: Gain) -> Result<()>;
    /// True only when this buffer was accepted by the local output queue.
    fn write(&mut self, buffer: AudioBuffer) -> bool;
    fn clear(&mut self);
    fn gain(&mut self, gain: Gain);
    fn failed(&self) -> bool;
    fn recovering(&self) -> bool {
        false
    }
    // Only driver failures may rebuild the output. Decode/buffer failures
    // remain fatal rather than being hidden by a reconnect loop.
    fn recoverable_failure(&self) -> bool {
        false
    }
    fn failure_detail(&self) -> String {
        "Audio output failed".to_string()
    }
    fn poll_failure(&mut self) -> Option<(bool, String)> {
        self.failed()
            .then(|| (self.recoverable_failure(), self.failure_detail()))
    }
    fn diagnostics(&self) -> Option<String> {
        None
    }
}
enum Work {
    Begin(StreamPlayerConfig, SharedClock, Gain, bool),
    Audio(sendspin::protocol::client::AudioChunk),
    End,
    Gain(Gain),
}
enum Feedback {
    Gain(Gain),
    Failed,
}

// Music Assistant may replay its advertised 2 MiB send-ahead immediately after
// a stream start. Keep the worker handoff large enough for that burst, while
// separately reserving encoded bytes so slot count cannot become unbounded RAM.
const MAX_WORK_ITEMS: usize = 1024;
const MAX_ENCODED_WORK_BYTES: usize = 2 * 1024 * 1024;
fn status(tx: &watch::Sender<AudioStatus>, state: &str, detail: &str) {
    tx.send_replace(AudioStatus {
        state: state.into(),
        detail: detail.into(),
    });
}

// A worker failure remains authoritative even if a protocol event arrives
// before the supervisor has finished shutting down that session.
fn session_status(tx: &watch::Sender<AudioStatus>, state: &str, detail: &str) {
    tx.send_if_modified(|current| {
        if current.state == "failed" || (current.state == state && current.detail == detail) {
            return false;
        }
        *current = AudioStatus {
            state: state.into(),
            detail: detail.into(),
        };
        true
    });
}

// Worker updates must not resurrect an old stream after its epoch was
// invalidated, or overwrite a reconnect/terminal failure from the supervisor.
fn stream_status(
    tx: &watch::Sender<AudioStatus>,
    epoch: &AtomicU64,
    generation: u64,
    state: &str,
    detail: &str,
) {
    tx.send_if_modified(|current| {
        if epoch.load(Ordering::Acquire) != generation
            || !matches!(
                current.state.as_str(),
                "connected" | "buffering" | "ready" | "recovering"
            )
            || (current.state == state && current.detail == detail)
        {
            return false;
        }
        *current = AudioStatus {
            state: state.into(),
            detail: detail.into(),
        };
        true
    });
}

// Only app-owned fixed messages may reach the TUI; decoder/driver errors can
// otherwise contain arbitrary peer data or local configuration.
fn safe_audio_error(error: &anyhow::Error) -> &'static str {
    let message = error.to_string();
    for safe in [
        "Unsupported audio stream format",
        "Invalid audio codec header",
        "Invalid FLAC stream information",
        "FLAC stream information disagrees with negotiation",
        "Invalid FLAC header",
        "Invalid Opus format",
        "Unsupported audio codec",
        "Invalid audio frame size",
        "Audio decoding failed",
        "Invalid decoded audio size",
        "Audio output stream creation failed",
        "Requested output buffer size is unsupported by this device",
        "Selected audio output device not found",
        "No default audio output device",
        "Audio output configuration unavailable",
        "Audio output formats unavailable",
        "No supported audio output formats",
        "Audio command queue overflow",
    ] {
        if message == safe {
            return safe;
        }
    }
    "Audio output or decoding failed"
}

fn worker_stopped(tx: &watch::Sender<AudioStatus>) {
    tx.send_if_modified(|current| {
        if current.state == "failed" {
            return false;
        }
        *current = AudioStatus {
            state: "failed".into(),
            detail: "Audio worker stopped".into(),
        };
        true
    });
}

// Each attempt owns fresh channels and a fresh device. A completed worker
// must release its output before the next one opens the configured device.
struct OutputWorker {
    work: thread_channel::SyncSender<(u64, Work)>,
    queued_audio: Arc<AtomicUsize>,
    feedback: mpsc::Receiver<(u64, Feedback)>,
    ready: oneshot::Receiver<std::result::Result<Vec<AudioFormatSpec>, &'static str>>,
    done: oneshot::Receiver<(bool, Option<String>)>,
    thread: std::thread::JoinHandle<()>,
}

fn spawn_output_worker<O, F>(
    factory: Arc<parking_lot::Mutex<F>>,
    mut gain: Gain,
    worker_stop: Arc<AtomicBool>,
    worker_epoch: Arc<AtomicU64>,
    worker_status: watch::Sender<AudioStatus>,
) -> Result<OutputWorker>
where
    O: Output,
    F: FnMut() -> Result<O> + Send + 'static,
{
    let (work_tx, work_rx) = thread_channel::sync_channel(MAX_WORK_ITEMS);
    let queued_audio = Arc::new(AtomicUsize::new(0));
    let queued_audio_worker = queued_audio.clone();
    let (feedback_tx, feedback_rx) = mpsc::channel(32);
    let (ready_tx, ready_rx) = oneshot::channel();
    let (done_tx, done_rx) = oneshot::channel();
    let worker = std::thread::Builder::new()
        .name("ma-tui-audio".into())
        .spawn(move || {
            let mut recoverable = false;
            let mut failure_detail = None;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut output = match (factory.lock())() {
                    Ok(o) => o,
                    Err(error) => {
                        recoverable = true;
                        let _ = ready_tx.send(Err(safe_audio_error(&error)));
                        return;
                    }
                };
                let formats = match output.formats() {
                    Ok(f) if !f.is_empty() => f,
                    _ => {
                        recoverable = true;
                        let _ = ready_tx.send(Err("No supported audio output formats"));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(formats.clone()));
                let mut current_epoch = 0;
                let mut decoder: Option<StreamDecoder> = None;
                let mut setup: Option<(StreamPlayerConfig, SharedClock)> = None;
                let mut active = false;
                let mut accepted_audio = false;
                let mut failed = None;
                let mut last_diagnostics = std::time::Instant::now();
                while !worker_stop.load(Ordering::Acquire) {
                    if let Some((retry, detail)) = output.poll_failure() {
                        recoverable = retry;
                        failed = Some(detail);
                        break;
                    }
                    if active {
                        if output.recovering() {
                            stream_status(
                                &worker_status,
                                &worker_epoch,
                                current_epoch,
                                "recovering",
                                "Audio output recovering",
                            );
                        } else {
                            let was_recovering = worker_status.borrow().state == "recovering";
                            if was_recovering {
                                stream_status(
                                    &worker_status,
                                    &worker_epoch,
                                    current_epoch,
                                    if accepted_audio { "ready" } else { "buffering" },
                                    if accepted_audio {
                                        "Audio buffer accepted by local output"
                                    } else {
                                        "Awaiting audio from stream"
                                    },
                                );
                            }
                        }
                    }
                    if active
                        && current_epoch == worker_epoch.load(Ordering::Acquire)
                        && last_diagnostics.elapsed() >= Duration::from_secs(1)
                    {
                        last_diagnostics = std::time::Instant::now();
                        if let Some(detail) = output.diagnostics() {
                            worker_status.send_if_modified(|current| {
                                if !matches!(
                                    current.state.as_str(),
                                    "buffering" | "ready" | "recovering"
                                ) || current.detail == detail
                                {
                                    return false;
                                }
                                current.detail = detail;
                                true
                            });
                        }
                    }
                    let new_epoch = worker_epoch.load(Ordering::Acquire);
                    if current_epoch != new_epoch {
                        output.clear();
                        decoder = None;
                        setup = None;
                        active = false;
                        accepted_audio = false;
                        current_epoch = new_epoch;
                    }
                    let (generation, event) = match work_rx.recv_timeout(Duration::from_millis(10))
                    {
                        Ok(w) => w,
                        Err(thread_channel::RecvTimeoutError::Timeout) => continue,
                        Err(_) => break,
                    };
                    if let Work::Audio(chunk) = &event {
                        queued_audio_worker.fetch_sub(chunk.data.len(), Ordering::AcqRel);
                    }
                    if generation != worker_epoch.load(Ordering::Acquire) {
                        continue;
                    }
                    // The generation may have changed while recv_timeout was waiting.
                    if generation != current_epoch {
                        output.clear();
                        decoder = None;
                        setup = None;
                        active = false;
                        accepted_audio = false;
                        current_epoch = generation;
                    }
                    let result: Result<()> = (|| {
                        match event {
                            Work::Begin(config, clock, next_gain, start_now) => {
                                output.clear();
                                decoder = None;
                                setup = None;
                                active = false;
                                accepted_audio = false;
                                let next = StreamDecoder::new(&config, &formats)?;
                                gain = next_gain;
                                if start_now {
                                    output.begin(next.format.clone(), clock.clone(), gain)?;
                                }
                                decoder = Some(next);
                                setup = Some((config, clock));
                                active = start_now;
                                stream_status(
                                    &worker_status,
                                    &worker_epoch,
                                    generation,
                                    "buffering",
                                    "Awaiting audio from stream",
                                );
                            }
                            Work::Audio(chunk) => {
                                if let Some(ref mut dec) = decoder {
                                    let samples = dec.decode(&chunk.data)?;
                                    if !active {
                                        if let Some((_, clock)) = &setup {
                                            output.begin(
                                                dec.format.clone(),
                                                clock.clone(),
                                                gain,
                                            )?;
                                            active = true;
                                        }
                                    }
                                    // A disconnect/clear can invalidate a decode in flight.
                                    if generation == worker_epoch.load(Ordering::Acquire)
                                        && !worker_stop.load(Ordering::Acquire)
                                    {
                                        let nonempty = !samples.is_empty();
                                        let accepted = output.write(AudioBuffer {
                                            timestamp: chunk.timestamp,
                                            samples,
                                            format: dec.format.clone(),
                                        });
                                        if !accepted {
                                            accepted_audio = false;
                                        } else if nonempty {
                                            accepted_audio = true;
                                        }
                                        if !accepted || nonempty {
                                            let recovering = output.recovering();
                                            stream_status(
                                                &worker_status,
                                                &worker_epoch,
                                                generation,
                                                if recovering {
                                                    "recovering"
                                                } else if accepted_audio {
                                                    "ready"
                                                } else {
                                                    "buffering"
                                                },
                                                if recovering {
                                                    "Audio output recovering"
                                                } else if accepted_audio {
                                                    "Audio buffer accepted by local output"
                                                } else {
                                                    "Awaiting audio from stream"
                                                },
                                            );
                                        }
                                    }
                                }
                            }
                            Work::End => {
                                output.clear();
                                decoder = None;
                                setup = None;
                                active = false;
                                accepted_audio = false;
                            }
                            Work::Gain(next) => {
                                gain = next;
                                output.gain(gain);
                                feedback_tx
                                    .try_send((generation, Feedback::Gain(gain)))
                                    .map_err(|_| anyhow::anyhow!("Audio command queue overflow"))?;
                            }
                        }
                        Ok(())
                    })();
                    if let Err(error) = result {
                        recoverable = error.to_string() == "Audio output stream creation failed";
                        failed = Some(safe_audio_error(&error).to_string());
                        break;
                    }
                }
                output.clear();
                if let Some(detail) = failed {
                    status(&worker_status, "failed", &detail);
                    failure_detail = Some(detail);
                    let _ = feedback_tx.try_send((current_epoch, Feedback::Failed));
                }
            }));
            if result.is_err() {
                recoverable = false;
                status(&worker_status, "failed", "Audio worker failed");
                failure_detail = Some("Audio worker failed".into());
            }
            let _ = done_tx.send((recoverable, failure_detail));
        })
        .map_err(|_| anyhow::anyhow!("Unable to start audio worker"))?;
    Ok(OutputWorker {
        work: work_tx,
        queued_audio,
        feedback: feedback_rx,
        ready: ready_rx,
        done: done_rx,
        thread: worker,
    })
}

pub(crate) fn start_with_output<O, F>(config: AudioConfig, factory: F) -> Result<AudioHandle>
where
    O: Output,
    F: FnMut() -> Result<O> + Send + 'static,
{
    let url = proxy_url(&config.server)?;
    if config.token.trim().is_empty()
        || config.player_id.trim().is_empty()
        || config.player_name.trim().is_empty()
        || config.volume > 100
    {
        bail!("Invalid audio configuration");
    }
    let runtime = tokio::runtime::Handle::try_current()
        .map_err(|_| anyhow::anyhow!("Audio requires a Tokio runtime"))?;
    let (status_tx, status_rx) = watch::channel(AudioStatus {
        state: "starting".into(),
        detail: "Initializing audio output".into(),
    });
    let (cancel, mut cancellation) = watch::channel(false);
    let stop = Arc::new(AtomicBool::new(false));
    let epoch = Arc::new(AtomicU64::new(0));
    let mut gain = Gain {
        volume: config.volume,
        muted: config.muted,
        delay: 0,
    };
    let factory = Arc::new(parking_lot::Mutex::new(factory));
    let task_stop = stop.clone();
    let task = runtime.spawn(async move {
        let mut recoveries = 0u32;
        loop {
            if *cancellation.borrow() || task_stop.load(Ordering::Acquire) {
                break;
            }
            let OutputWorker {
                work: work_tx,
                queued_audio,
                feedback: mut feedback_rx,
                ready: ready_rx,
                done: mut done_rx,
                thread: worker,
            } = match spawn_output_worker(
                factory.clone(),
                gain,
                task_stop.clone(),
                epoch.clone(),
                status_tx.clone(),
            ) {
                Ok(worker) => worker,
                Err(_) => {
                    status(&status_tx, "failed", "Unable to start audio worker");
                    break;
                }
            };
            let mut finished = None;
            let ready = tokio::select! {
                biased;
                _ = cancellation.changed() => None,
                r = ready_rx => r.ok(),
            };
            let initialized = matches!(&ready, Some(Ok(_)));
            let started_output = std::time::Instant::now();
            if let Some(Ok(formats)) = ready {
                let mut retry = Duration::from_millis(250);
                loop {
                    if *cancellation.borrow() || task_stop.load(Ordering::Acquire) {
                        break;
                    }
                    status(
                        &status_tx,
                        "connecting",
                        "Connecting to authenticated audio proxy",
                    );
                    let started = std::time::Instant::now();
                    let result = tokio::select! {
                        biased;
                        _ = cancellation.changed() => break,
                        r = &mut done_rx => {
                            finished = Some(r.unwrap_or((false, None)));
                            worker_stopped(&status_tx);
                            break;
                        },
                        r = session(&config, &url, &formats, &mut gain,
                            SessionIo {
                                work: &work_tx,
                                queued_audio: &queued_audio,
                                epoch: &epoch,
                                feedback: &mut feedback_rx,
                            },
                            &status_tx) => r,
                    };
                    epoch.fetch_add(1, Ordering::AcqRel);
                    if status_tx.borrow().state == "failed" {
                        break;
                    }
                    if feedback_rx.is_closed() {
                        worker_stopped(&status_tx);
                        break;
                    }
                    let detail = result
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "Audio connection closed".into());
                    if matches!(
                        detail.as_str(),
                        "Audio worker queue full or unavailable"
                            | "Audio worker encoded queue full"
                    ) {
                        status(&status_tx, "failed", "Audio worker queue overflow");
                        break;
                    }
                    status(&status_tx, "reconnecting", &detail);
                    if started.elapsed() > Duration::from_secs(30) {
                        retry = Duration::from_millis(250);
                    }
                    tokio::select! {
                        biased;
                        _ = cancellation.changed() => break,
                        r = &mut done_rx => {
                            finished = Some(r.unwrap_or((false, None)));
                            worker_stopped(&status_tx);
                            break;
                        },
                        _ = tokio::time::sleep(retry) => {},
                    }
                    retry = (retry * 2).min(Duration::from_secs(30));
                }
            } else if !*cancellation.borrow() {
                status(
                    &status_tx,
                    "failed",
                    ready
                        .and_then(Result::err)
                        .unwrap_or("Audio output initialization failed"),
                );
            }
            // Dropping the session closes its transport. Fresh channels and a
            // new negotiation ensure no samples from that session are replayed.
            epoch.fetch_add(1, Ordering::AcqRel);
            drop(work_tx);
            // Never block a Tokio executor on a stuck device thread, and never
            // reopen a device while the previous worker may still own it.
            let recovered = match finished {
                Some(recoverable) => Some(recoverable),
                None => tokio::time::timeout(Duration::from_secs(2), &mut done_rx)
                    .await
                    .ok()
                    .and_then(Result::ok),
            };
            if recovered.is_some() {
                let _ = worker.join();
            } else {
                status(&status_tx, "failed", "Audio worker did not stop");
            }
            // The exit reason is authoritative even if a connecting update
            // raced the worker's failure status.
            if let Some((_, Some(detail))) = &recovered {
                status(&status_tx, "failed", detail);
            }
            if *cancellation.borrow() || task_stop.load(Ordering::Acquire) {
                break;
            }
            if !matches!(recovered, Some((true, _))) || (!initialized && recoveries == 0) {
                break;
            }
            if initialized && started_output.elapsed() >= Duration::from_secs(30) {
                recoveries = 0;
            }
            if recoveries == 3 {
                break;
            }
            let detail = status_tx.borrow().detail.clone();
            status(
                &status_tx,
                "recovering",
                &format!("{detail}; retrying audio output ({}/3)", recoveries + 1),
            );
            let delay = Duration::from_millis(250 * (1 << recoveries));
            recoveries += 1;
            tokio::select! {
                biased;
                _ = cancellation.changed() => break,
                _ = tokio::time::sleep(delay) => {},
            }
        }
        task_stop.store(true, Ordering::Release);
        if *cancellation.borrow() {
            status(&status_tx, "stopped", "Audio stopped");
        }
    });
    Ok(AudioHandle {
        status: status_rx,
        cancel,
        stop,
        task: Some(task),
    })
}

struct SessionIo<'a> {
    work: &'a thread_channel::SyncSender<(u64, Work)>,
    queued_audio: &'a AtomicUsize,
    epoch: &'a Arc<AtomicU64>,
    feedback: &'a mut mpsc::Receiver<(u64, Feedback)>,
}
async fn session(
    config: &AudioConfig,
    url: &url::Url,
    formats: &[AudioFormatSpec],
    gain: &mut Gain,
    io: SessionIo<'_>,
    state: &watch::Sender<AudioStatus>,
) -> Result<()> {
    let SessionIo {
        work,
        queued_audio,
        epoch,
        feedback,
    } = io;
    let socket = authenticate(
        url,
        &config.token,
        &config.player_id,
        Duration::from_secs(10),
    )
    .await?;
    let builder = ProtocolClientBuilder::builder()
        .client_id(config.player_id.clone())
        .name(config.player_name.clone())
        .player_v1_support(PlayerV1Support {
            supported_formats: formats.to_vec(),
            buffer_capacity: 2 * 1024 * 1024,
            supported_commands: vec!["volume".into(), "mute".into()],
        })
        .initial_player_state(gain.state())
        .build();
    let client = tokio::time::timeout(Duration::from_secs(10), builder.accept(socket))
        .await
        .map_err(|_| anyhow::anyhow!("Sendspin handshake timed out"))?
        .map_err(|_| anyhow::anyhow!("Sendspin handshake failed"))?;
    let mut connection = client.split();
    // These roles were not requested. Closing their library receivers prevents
    // an unsolicited artwork/visualizer stream accumulating unread payloads.
    connection.artwork.close();
    connection.visualizer.close();
    while connection.artwork.try_recv().is_ok() {}
    while connection.visualizer.try_recv().is_ok() {}
    session_status(state, "connected", "Authenticated audio player connected");
    let mut stream_config: Option<StreamPlayerConfig> = None;
    loop {
        // Sendspin 0.3.7 exposes unbounded internal receivers. Drain promptly,
        // cap observed backlog and never await the bounded audio worker queue.
        if connection.audio.len() > MAX_WORK_ITEMS || connection.messages.len() > 64 {
            bail!("Audio receive queue overflow");
        }
        let generation = epoch.load(Ordering::Acquire);
        let send = |event| {
            work.try_send((epoch.load(Ordering::Acquire), event))
                .map_err(|_| anyhow::anyhow!("Audio worker queue full or unavailable"))
        };
        let send_audio = |chunk: sendspin::protocol::client::AudioChunk| {
            let bytes = chunk.data.len();
            queued_audio
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                    queued
                        .checked_add(bytes)
                        .filter(|next| *next <= MAX_ENCODED_WORK_BYTES)
                })
                .map_err(|_| anyhow::anyhow!("Audio worker encoded queue full"))?;
            if work
                .try_send((epoch.load(Ordering::Acquire), Work::Audio(chunk)))
                .is_err()
            {
                queued_audio.fetch_sub(bytes, Ordering::AcqRel);
                bail!("Audio worker queue full or unavailable");
            }
            Ok(())
        };
        tokio::select! {
                biased;
                event=feedback.recv()=>match event {
                    Some((g,Feedback::Gain(next))) if g==generation => {
                        connection.sender.send_message(Message::ClientState(ClientState {state:None,player:Some(next.state())})).await.map_err(|_|anyhow::anyhow!("Audio state confirmation failed"))?;
                    }
                    Some((_,Feedback::Failed))|None=>bail!("Audio output failed"),
                    _=>{}
                },
                message=connection.messages.recv()=>match message {
                    None=>bail!("Audio proxy disconnected"),
                    Some(Message::StreamStart(start))=>if let Some(player)=start.player {
                        // Split protocol receivers do not carry ordering metadata.
                        // Discard queued chunks at a stream boundary rather than
                        // accidentally decode old-format bytes under the new format.
                        while connection.audio.try_recv().is_ok() {}
                        epoch.fetch_add(1,Ordering::AcqRel);
                        session_status(state,"buffering","Awaiting audio from stream");
                        stream_config=Some(player.clone());
                        send(Work::Begin(player,connection.clock_sync.clone(),*gain,true))?;
                    },
                    Some(Message::StreamClear(clear))=>if clear.roles.as_ref().is_none_or(|roles|roles.iter().any(|r|r=="player")) {while connection.audio.try_recv().is_ok(){} epoch.fetch_add(1,Ordering::AcqRel);
                        session_status(state,if stream_config.is_some() {"buffering"} else {"connected"},if stream_config.is_some() {"Awaiting audio from stream"} else {"Authenticated audio player connected"});
                        if let Some(config)=&stream_config {send(Work::Begin(config.clone(),connection.clock_sync.clone(),*gain,false))?;} else {send(Work::End)?;}},
                    Some(Message::StreamEnd(end))=>if end.roles.as_ref().is_none_or(|roles|roles.iter().any(|r|r=="player")) {while connection.audio.try_recv().is_ok(){} epoch.fetch_add(1,Ordering::AcqRel); stream_config=None; session_status(state,"connected","Authenticated audio player connected"); send(Work::End)?;},
                    Some(Message::ServerCommand(command))=>if let Some(command)=command.player {
                        let mut next=*gain;
                        match command.command {
                            PlayerCommandType::Volume=>if let Some(volume)=command.volume {if volume>100 {bail!("Invalid audio volume command");}next.volume=volume;},
                            PlayerCommandType::Mute=>if let Some(muted)=command.mute {next.muted=muted;},
                            PlayerCommandType::SetStaticDelay=>if let Some(delay)=command.static_delay_ms {if delay>5000 {bail!("Invalid audio delay command");}next.delay=delay;},
                            _=>continue,
                        }
                        *gain=next; send(Work::Gain(next))?;
                    },
                    _=>{}
                },
                chunk=connection.audio.recv()=>match chunk {Some(c)=>send_audio(c)?,None=>bail!("Audio proxy disconnected")},
        }
    }
}

use cpal::traits::{DeviceTrait, HostTrait};
use sendspin::audio::{SyncedPlayer, SyncedPlayerConfig};

#[derive(Clone, Debug)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}
/// Enumerate local audio services/devices without creating a playback stream
/// or contacting Music Assistant.
pub fn devices() -> Result<Vec<AudioDevice>> {
    Ok(enumerate_devices()?
        .into_iter()
        .map(|(_, info)| info)
        .collect())
}
fn enumerate_devices() -> Result<Vec<(cpal::Device, AudioDevice)>> {
    let mut found = Vec::new();
    for id in cpal::available_hosts() {
        let host = cpal::host_from_id(id).map_err(|_| anyhow::anyhow!("Audio host unavailable"))?;
        for device in host
            .devices()
            .map_err(|_| anyhow::anyhow!("Audio device enumeration failed"))?
        {
            if !device
                .supported_output_configs()
                .is_ok_and(|mut c| c.next().is_some())
            {
                continue;
            }
            let id = device
                .id()
                .map_err(|_| anyhow::anyhow!("Audio device identifier unavailable"))?
                .to_string();
            let name = device
                .description()
                .map(|d| d.to_string())
                .unwrap_or_else(|_| id.clone());
            found.push((device, AudioDevice { id, name }));
        }
    }
    Ok(found)
}
/// `spectrum` receives what this endpoint plays, for the interface visualizer.
pub fn start(config: AudioConfig, spectrum: Option<Arc<dyn SampleSink>>) -> Result<AudioHandle> {
    if config
        .output_buffer_frames
        .is_some_and(|frames| !(256..=8192).contains(&frames))
    {
        bail!("Output buffer frames must be between 256 and 8192");
    }
    let device_id = config.device_id.clone();
    let buffer_frames = config.output_buffer_frames;
    start_with_output(config, move || {
        DeviceOutput::new(device_id.as_deref(), spectrum.clone(), buffer_frames)
    })
}

pub(crate) fn formats_for_ranges(ranges: &[(u16, u32, u32)]) -> Vec<AudioFormatSpec> {
    let mut result = Vec::new();
    // The device output sample representation is independent of wire depth.
    // PCM first avoids compressed-codec compatibility surprises on MA 2.10.2.
    for (codec, bit_depth) in [
        ("pcm", 16),
        ("pcm", 24),
        ("flac", 16),
        ("flac", 24),
        ("opus", 16),
    ] {
        for rate in [48000, 44100, 32000, 24000, 16000, 96000] {
            if codec == "opus" && rate != 48000 {
                continue;
            }
            for channels in [2, 1] {
                if ranges
                    .iter()
                    .any(|&(c, min, max)| c == channels && min <= rate && rate <= max)
                {
                    result.push(AudioFormatSpec {
                        codec: codec.into(),
                        channels: channels as u8,
                        sample_rate: rate,
                        bit_depth,
                    });
                }
            }
        }
    }
    result
}

#[derive(Default)]
pub(crate) struct QueueBudget {
    pending: std::collections::VecDeque<(std::time::Instant, usize)>,
    // Server microseconds, independent of mutable clock-sync estimates.
    last_end: Option<i128>,
    delay: u16,
    error: Option<&'static str>,
}

// MA 2.10.2's aiosendspin 9.1.1 accounts ENCODED bytes and permits a 30s
// buffered horizon. Our i32 queue needs up to 30 * 96000 * 2 * 4 bytes,
// independently of the 2 MiB encoded capacity advertised in client/hello.
const MAX_DECODED_QUEUE_BYTES: usize = 32 * 1024 * 1024;
const MAX_QUEUE_HORIZON: Duration = Duration::from_secs(35);
const MAX_QUEUED_CHUNKS: usize = 4096;
impl QueueBudget {
    pub(crate) fn set_delay(&mut self, delay: u16) -> bool {
        if self.delay == delay {
            return false;
        }
        *self = Self {
            delay,
            ..Default::default()
        };
        true
    }
    pub(crate) fn accept(
        &mut self,
        now: std::time::Instant,
        (server_timestamp, timestamp): (i64, std::time::Instant),
        duration: Duration,
        delay: u16,
        bytes: usize,
    ) -> bool {
        self.set_delay(delay);
        let Some(when) = timestamp.checked_sub(Duration::from_millis(delay.into())) else {
            return false;
        };
        let Some(end) = when.checked_add(duration) else {
            return false;
        };
        // Clock estimate updates can reorder local deadlines. Expire every
        // completed entry, not only a presumed time-ordered prefix.
        self.pending.retain(|(end, _)| *end > now);
        // A 2us tolerance accommodates integer timestamp rounding between chunks.
        self.error = if duration > Duration::from_secs(2) || bytes > 2 * 1024 * 1024 {
            Some("Audio chunk exceeds supported size")
        } else if when > now + MAX_QUEUE_HORIZON {
            Some("Audio timestamp exceeds supported buffer horizon")
        } else if self
            .last_end
            .is_some_and(|previous| i128::from(server_timestamp) + 2 < previous)
        {
            Some("Audio chunks have overlapping timestamps")
        } else if self.pending.iter().map(|(_, n)| *n).sum::<usize>() + bytes
            > MAX_DECODED_QUEUE_BYTES
            || self.pending.len() >= MAX_QUEUED_CHUNKS
        {
            Some("Decoded audio buffer capacity exceeded")
        } else {
            None
        };
        if self.error.is_some() {
            return false;
        }
        self.last_end = Some(i128::from(server_timestamp) + duration.as_micros() as i128);
        self.pending.push_back((end, bytes));
        true
    }
}

struct DeviceOutput {
    device: cpal::Device,
    // Decoded samples are copied here for the visualizer, tagged with the
    // instant the synchronized player is scheduled to emit them.
    spectrum: Option<Arc<dyn SampleSink>>,
    formats: Vec<AudioFormatSpec>,
    player: Option<SyncedPlayer>,
    clock: Option<SharedClock>,
    // Library's decoded queue is unbounded. Track scheduled buffers ourselves,
    // rejecting excessive or overlapping timestamps before enqueueing.
    queued: QueueBudget,
    failed: Option<&'static str>,
    buffer_frames: Option<u32>,
    backend: &'static str,
    allow_alsa_recovery: bool,
    callbacks: Arc<CallbackStats>,
    health: Option<HealthMonitor>,
    stream_format: Option<(u32, u8)>,
    driver_failure: Option<String>,
}
impl DeviceOutput {
    fn new(
        id: Option<&str>,
        spectrum: Option<Arc<dyn SampleSink>>,
        buffer_frames: Option<u32>,
    ) -> Result<Self> {
        let device = if let Some(id) = id {
            enumerate_devices()?
                .into_iter()
                .find(|(_, info)| info.id == id)
                .map(|(d, _)| d)
                .ok_or_else(|| anyhow::anyhow!("Selected audio output device not found"))?
        } else {
            cpal::default_host()
                .default_output_device()
                .ok_or_else(|| anyhow::anyhow!("No default audio output device"))?
        };
        // 0.3.7 builds using the default output sample type, overriding only
        // channels and rate. Advertise ranges supporting exactly that type.
        let sample_type = device
            .default_output_config()
            .map_err(|_| anyhow::anyhow!("Audio output configuration unavailable"))?
            .sample_format();
        let ranges = device
            .supported_output_configs()
            .map_err(|_| anyhow::anyhow!("Audio output formats unavailable"))?
            .filter(|r| r.sample_format() == sample_type)
            .filter(|r| {
                buffer_frames.is_none_or(|frames| match r.buffer_size() {
                    cpal::SupportedBufferSize::Range { min, max } => {
                        (*min..=*max).contains(&frames)
                    }
                    cpal::SupportedBufferSize::Unknown => true,
                })
            })
            .map(|r| (r.channels(), r.min_sample_rate(), r.max_sample_rate()))
            .collect::<Vec<_>>();
        let formats = formats_for_ranges(&ranges);
        if formats.is_empty() {
            if buffer_frames.is_some() {
                bail!("Requested output buffer size is unsupported by this device");
            }
            bail!("No supported audio output formats");
        }
        #[cfg(target_os = "linux")]
        let allow_alsa_recovery = device.id().is_ok_and(|id| id.host() == cpal::HostId::Alsa);
        #[cfg(not(target_os = "linux"))]
        let allow_alsa_recovery = false;
        let backend = device.id().map_or("unknown", |id| id.host().name());
        Ok(Self {
            device,
            spectrum,
            formats,
            player: None,
            clock: None,
            queued: Default::default(),
            failed: None,
            buffer_frames,
            backend,
            allow_alsa_recovery,
            callbacks: Arc::new(CallbackStats::new()),
            health: None,
            stream_format: None,
            driver_failure: None,
        })
    }
}
impl Output for DeviceOutput {
    fn formats(&self) -> Result<Vec<AudioFormatSpec>> {
        Ok(self.formats.clone())
    }
    fn begin(&mut self, format: AudioFormat, clock: SharedClock, gain: Gain) -> Result<()> {
        self.clear();
        self.callbacks = Arc::new(CallbackStats::new());
        let callbacks = self.callbacks.clone();
        let channels = usize::from(format.channels);
        let stream_format = (format.sample_rate, format.channels);
        let player = SyncedPlayer::with_process_callback(
            format,
            clock.clone(),
            SyncedPlayerConfig {
                device: Some(self.device.clone()),
                volume: gain.volume,
                muted: gain.muted,
                buffer_size: self.buffer_frames,
            },
            Box::new(move |samples| callbacks.observe(samples.len() / channels)),
        )
        .map_err(|_| anyhow::anyhow!("Audio output stream creation failed"))?;
        player.set_static_delay(gain.delay);
        self.player = Some(player);
        self.stream_format = Some(stream_format);
        self.health = Some(HealthMonitor::new(
            std::time::Instant::now(),
            self.allow_alsa_recovery,
        ));
        self.clock = Some(clock);
        if let Some(spectrum) = &self.spectrum {
            spectrum.set_muted(gain.muted);
        }
        Ok(())
    }
    fn write(&mut self, buffer: AudioBuffer) -> bool {
        let (Some(player), Some(clock)) = (&self.player, &self.clock) else {
            return false;
        };
        let sync = clock.lock();
        // No free-running output: wait for the library's monotonic clock sync.
        // Never add buffers to its unbounded queue while time is unknown/stale.
        if !sync.is_synchronized() || sync.is_stale() {
            drop(sync);
            player.clear();
            self.queued = QueueBudget::default();
            return false;
        }
        let Some(when) = sync.server_to_local_instant(buffer.timestamp) else {
            return false;
        };
        drop(sync);
        let now = std::time::Instant::now();
        let duration = Duration::from_micros(buffer.duration_us().max(0) as u64);
        let bytes = buffer.samples.len() * std::mem::size_of::<i32>();
        if !self.queued.accept(
            now,
            (buffer.timestamp, when),
            duration,
            player.static_delay_ms(),
            bytes,
        ) {
            self.failed = Some(self.queued.error.unwrap_or("Invalid audio timestamp"));
            player.clear();
            return false;
        }
        // Late chunks cannot contribute to audible output and need not queue.
        if when + duration < now {
            return false;
        }
        // Sendspin emits each sample `static_delay_ms` early so downstream
        // latency lands it on time, and the budget above measures the same
        // instant. Analyze against that scheduled emission; it is not a
        // measurement of device buffering or acoustic output.
        if let (Some(spectrum), Some(emitted)) = (
            &self.spectrum,
            when.checked_sub(Duration::from_millis(player.static_delay_ms().into())),
        ) {
            spectrum.push(
                &buffer.samples,
                buffer.format.channels,
                buffer.format.sample_rate,
                emitted,
            );
        }
        player.enqueue(buffer);
        true
    }
    fn clear(&mut self) {
        if let Some(player) = self.player.take() {
            player.clear();
            drop(player);
        }
        self.clock = None;
        self.health = None;
        self.stream_format = None;
        self.queued = QueueBudget::default();
        if let Some(spectrum) = &self.spectrum {
            spectrum.clear();
        }
    }
    fn gain(&mut self, gain: Gain) {
        if let Some(spectrum) = &self.spectrum {
            spectrum.set_muted(gain.muted);
        }
        if let Some(player) = &self.player {
            player.set_volume(gain.volume);
            player.set_mute(gain.muted);
            if player.static_delay_ms() != gain.delay {
                // The budget may be fresh after begin or a stale-clock reset;
                // only the actual player tells us whether its delay changed.
                player.clear();
                player.set_static_delay(gain.delay);
                self.queued = QueueBudget::default();
                // A delay change moves every scheduled emission instant.
                if let Some(spectrum) = &self.spectrum {
                    spectrum.clear();
                }
            }
            self.queued.set_delay(player.static_delay_ms());
        }
    }
    fn failed(&self) -> bool {
        self.failed.is_some() || self.driver_failure.is_some()
    }
    fn recovering(&self) -> bool {
        self.health
            .as_ref()
            .is_some_and(HealthMonitor::is_recovering)
    }
    fn recoverable_failure(&self) -> bool {
        self.failed.is_none() && self.driver_failure.is_some()
    }
    // sendspin's take_error() carries the real CPAL/driver message (a local
    // system diagnostic, not peer-supplied data — unlike the rest of this
    // module's sanitized-only messages, this one is safe to show verbatim,
    // still bounded and control-character-stripped as defense in depth).
    fn failure_detail(&self) -> String {
        if let Some(detail) = self.failed {
            return detail.to_string();
        }
        self.driver_failure
            .clone()
            .unwrap_or_else(|| "Audio output device reported a stream error".to_string())
    }
    fn poll_failure(&mut self) -> Option<(bool, String)> {
        if let (Some(player), Some(health)) = (&self.player, &mut self.health) {
            // Consume the library's error slot exactly once. CPAL's ALSA backend
            // prepares after XRUN, and the Pulse ALSA adapter can briefly lack
            // timing information. Only the narrowly matched reports get a
            // grace period, with new callbacks proving recovery completed.
            // Other errors still require rebuilding the worker.
            let error = player.take_error();
            let callbacks = self.callbacks.snapshot();
            if let Some((rate, _)) = self.stream_format {
                // CPAL may service the whole two-period ALSA ring in one
                // callback. Give both requested and observed large periods
                // headroom, without mistaking a scheduling gap for a period.
                let frames = callbacks
                    .max_frames
                    .max(self.buffer_frames.unwrap_or(0) as usize * 2);
                health.set_callback_timeout(Duration::from_secs_f64(
                    (4.0 * frames as f64 / f64::from(rate)).max(1.0),
                ));
            }
            if let HealthDecision::Failed(detail) =
                health.poll(std::time::Instant::now(), error.as_deref(), callbacks)
            {
                self.driver_failure = Some(stream_error_detail(&detail));
            }
        }
        self.failed()
            .then(|| (self.recoverable_failure(), self.failure_detail()))
    }
    fn diagnostics(&self) -> Option<String> {
        let (rate, channels) = self.stream_format?;
        let health = self.health.as_ref()?;
        let callbacks = self.callbacks.snapshot();
        let buffer = self
            .buffer_frames
            .map_or_else(|| "default".to_owned(), |n| n.to_string());
        Some(format!(
            "{} · {rate} Hz/{channels} ch · buffer {buffer} · callbacks {}–{} frames · max gap {:.1} ms · XRUN {}/{} recovered · timing {}/{} recovered{}",
            self.backend, callbacks.min_frames, callbacks.max_frames, callbacks.max_gap.as_secs_f64() * 1000.0,
            health.recovered_xruns(), health.observed_xruns(),
            health.recovered_timing_errors(), health.observed_timing_errors(),
            if health.is_recovering() { " · recovering" } else { "" },
        ))
    }
}

fn stream_error_detail(raw: &str) -> String {
    let error: String = raw.chars().filter(|c| !c.is_control()).take(512).collect();
    format!("Audio output device reported a stream error: {error}")
}

/// MA 2.10.2 mounts the authenticated receiver at /sendspin.
pub(crate) fn proxy_url(base: &str) -> Result<url::Url> {
    let mut url = url::Url::parse(base).map_err(|_| anyhow::anyhow!("Invalid audio server URL"))?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => bail!("Audio server requires HTTP or HTTPS"),
    };
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Audio server URL must not contain credentials, query, or fragment");
    }
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("Invalid audio server URL"))?;
    let path = format!("{}/sendspin", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}

#[cfg(test)]
mod stream_error_detail_tests {
    use super::stream_error_detail;

    #[test]
    fn carries_the_driver_message_through() {
        assert_eq!(
            stream_error_detail("device disconnected"),
            "Audio output device reported a stream error: device disconnected"
        );
    }

    #[test]
    fn strips_control_characters_and_caps_length() {
        let raw = format!("bad\x1b[31mtext\n{}", "x".repeat(600));
        let detail = stream_error_detail(&raw);
        assert!(!detail.contains('\x1b'));
        assert!(!detail.contains('\n'));
        // The static prefix plus at most 512 characters of sanitized message.
        let prefix = "Audio output device reported a stream error: ";
        assert!(detail.starts_with(prefix));
        assert_eq!(detail.len() - prefix.len(), 512);
    }
}

#[cfg(test)]
mod device_output_tests {
    use super::*;

    #[test]
    #[ignore = "requires Linux ALSA null output; run explicitly"]
    fn silent_callbacks_report_progress_and_requested_buffer_without_false_recovery() {
        for requested in [None, Some(1024)] {
            let mut output = DeviceOutput::new(Some("alsa:null"), None, requested).unwrap();
            output
                .begin(
                    AudioFormat {
                        codec: Codec::Pcm,
                        sample_rate: 48000,
                        channels: 2,
                        bit_depth: 16,
                        codec_header: None,
                    },
                    Arc::new(parking_lot::Mutex::new(sendspin::sync::ClockSync::default())),
                    Gain {
                        volume: 0,
                        muted: true,
                        delay: 0,
                    },
                )
                .unwrap();
            // No clock sync or enqueued media: a healthy silent device must
            // still report callbacks and must survive the watchdog interval.
            let start = std::time::Instant::now();
            while start.elapsed() < Duration::from_millis(1100) {
                assert!(
                    output.poll_failure().is_none(),
                    "{}",
                    output.failure_detail()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            let stats = output.callbacks.snapshot();
            assert!(stats.count > 1);
            assert!(stats.min_frames > 0);
            assert!(stats.max_frames >= stats.min_frames);
            if let Some(frames) = requested {
                // ALSA negotiates near the requested period, with a two-period
                // ring. The null sink accepts it exactly; one callback may
                // consume both periods when the entire ring is available.
                assert!(stats.min_frames >= frames as usize);
                assert!(stats.max_frames <= (frames * 2) as usize);
            }
            let detail = output.diagnostics().unwrap();
            assert!(detail.contains("48000 Hz/2 ch"), "{detail}");
            assert!(detail.contains("XRUN 0/0 recovered"), "{detail}");
            assert!(
                detail.contains(if requested.is_some() {
                    "buffer 1024"
                } else {
                    "buffer default"
                }),
                "{detail}"
            );
            output.clear();
            assert!(output.health.is_none());
            assert!(output.diagnostics().is_none());
            assert!(output.poll_failure().is_none());
        }
    }

    /// Records what the output hands to the visualizer, without depending on
    /// the analyzer itself.
    #[derive(Default)]
    struct RecordingSink {
        chunks: parking_lot::Mutex<Vec<(usize, u8, u32, std::time::Instant)>>,
        cleared: AtomicU64,
    }
    impl SampleSink for RecordingSink {
        fn push(&self, samples: &[i32], channels: u8, rate: u32, emitted: std::time::Instant) {
            self.chunks
                .lock()
                .push((samples.len(), channels, rate, emitted));
        }
        fn set_muted(&self, _muted: bool) {}
        fn clear(&self) {
            self.cleared.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[test]
    #[ignore = "requires Linux ALSA null output; run explicitly"]
    fn synchronized_output_accepts_full_advertised_pcm_buffer() {
        let sink = Arc::new(RecordingSink::default());
        let mut output = DeviceOutput::new(Some("alsa:null"), Some(sink.clone()), None).unwrap();
        let mut sync = sendspin::sync::ClockSync::default();
        std::thread::sleep(Duration::from_millis(2));
        let now = sync.clock().now_micros();
        sync.update(now - 500, now - 450, now - 450, now - 400);
        sync.update(now - 100, now - 50, now - 50, now);
        assert!(sync.is_synchronized());
        let start = sync.client_to_server_micros(now).unwrap() + 500_000;
        let clock = Arc::new(parking_lot::Mutex::new(sync));
        let format = AudioFormat {
            codec: Codec::Pcm,
            sample_rate: 48000,
            channels: 2,
            bit_depth: 16,
            codec_header: None,
        };
        output
            .begin(
                format.clone(),
                clock,
                Gain {
                    volume: 0,
                    muted: true,
                    delay: 0,
                },
            )
            .unwrap();
        for index in 0..540 {
            output.write(AudioBuffer {
                timestamp: start + index * 20_000,
                samples: vec![0; 1920].into(),
                format: format.clone(),
            });
            assert!(
                output.poll_failure().is_none(),
                "{} at chunk {index}",
                output.failure_detail()
            );
        }
        assert!(
            output.queued.pending.len() > 500,
            "buffers must pass through real synchronized output"
        );
        // Every buffer accepted for playback reaches the visualizer, tagged
        // with the instant the player is scheduled to emit it.
        let chunks = sink.chunks.lock().clone();
        assert_eq!(chunks.len(), 540);
        assert!(chunks
            .iter()
            .all(|(samples, channels, rate, _)| *samples == 1920
                && *channels == 2
                && *rate == 48000));
        for pair in chunks.windows(2) {
            let step = pair[1].3.duration_since(pair[0].3);
            assert!(
                step.abs_diff(Duration::from_millis(20)) < Duration::from_millis(1),
                "scheduled emission must advance with the audio: {step:?}"
            );
        }
        output.clear();
        assert!(sink.cleared.load(Ordering::Acquire) > 0);
    }

    #[test]
    #[ignore = "requires Linux ALSA null output; run explicitly"]
    fn begin_delay_can_reset_to_zero_before_audio_or_after_clock_reset() {
        let mut output = DeviceOutput::new(Some("alsa:null"), None, None).unwrap();
        let clock =
            std::sync::Arc::new(parking_lot::Mutex::new(sendspin::sync::ClockSync::default()));
        let format = AudioFormat {
            codec: Codec::Pcm,
            sample_rate: 48000,
            channels: 2,
            bit_depth: 16,
            codec_header: None,
        };
        let mut gain = Gain {
            volume: 30,
            muted: false,
            delay: 123,
        };
        output.begin(format.clone(), clock, gain).unwrap();
        assert_eq!(output.player.as_ref().unwrap().static_delay_ms(), 123);
        gain.delay = 0;
        output.gain(gain);
        assert_eq!(
            output.player.as_ref().unwrap().static_delay_ms(),
            0,
            "reset before any buffers must update the actual player"
        );

        gain.delay = 123;
        output.gain(gain);
        // The unsynchronized and stale-clock branches both clear the budget,
        // without clearing the player's configured delay.
        output.write(AudioBuffer {
            timestamp: 0,
            samples: vec![0; 1920].into(),
            format,
        });
        assert_eq!(output.queued.delay, 0);
        assert_eq!(output.player.as_ref().unwrap().static_delay_ms(), 123);
        gain.delay = 0;
        output.gain(gain);
        assert_eq!(
            output.player.as_ref().unwrap().static_delay_ms(),
            0,
            "budget reset must not suppress a player delay update"
        );
        output.clear();
    }
}
