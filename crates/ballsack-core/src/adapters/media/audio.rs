//! Audio adapter: capture, Opus encode/decode, jitter buffer, and playout.
//!
//! - [`CpalAudioCapture`]: opens a microphone via cpal, converts the native
//!   format (any channel count / sample rate) to 48 kHz mono, Opus-encodes,
//!   and implements [`MediaCapture`].
//! - [`CpalAudioPlayback`]: receives Opus frames via [`MediaPlayback`], decodes
//!   them, mixes all peers, and plays through the default output device.
//! - [`SilenceCaptureSource`]: original test stub that emits silence frames.
//! - [`enumerate_input_devices`]: lists available input devices for a picker UI.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use async_trait::async_trait;
use audiopus::coder::{Decoder as OpusDecoder, Encoder as OpusEncoder};
use audiopus::{Application, Channels, SampleRate as OpusSampleRate};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::HeapRb;
use serde::Serialize;
use tokio::sync::Mutex;
use tracing::{debug, info, trace, warn};

use crate::application::ports::{CapturedFrame, MediaCapture, MediaPlayback};
use crate::domain::control::ControlMsg;
use crate::domain::identity::PeerId;

use super::clock::AUDIO_FRAME_TICKS;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Target sample rate for Opus (Hz).
const OPUS_SAMPLE_RATE: u32 = 48_000;
/// Opus frame size in samples at 48 kHz (20 ms).
const OPUS_FRAME_SAMPLES: usize = 960;
/// Opus encoding bitrate (bits/sec).  64 kbps gives clear voice quality
/// while staying well within typical LAN/internet bandwidth budgets.
const OPUS_BITRATE: i32 = 64_000;

// ---------------------------------------------------------------------------
// Device enumeration
// ---------------------------------------------------------------------------

/// Describes an available audio input device.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceInfo {
    /// Stable unique device identifier (used to select the device).
    pub id: String,
    /// Human-readable label for display.
    pub label: String,
    /// Whether this is the system's default input device.
    pub is_default: bool,
}

/// Build a human-readable label from the cpal device.
///
/// Tries `description()` first (cpal 0.17+) to get name + manufacturer,
/// falling back to the deprecated `name()`.
fn device_label(dev: &cpal::Device) -> String {
    if let Ok(desc) = dev.description() {
        let name = desc.name().to_string();
        if let Some(mfr) = desc.manufacturer() {
            format!("{name} ({mfr})")
        } else {
            name
        }
    } else {
        #[allow(deprecated)]
        dev.name().unwrap_or_else(|_| "Unknown".into())
    }
}

/// Get a stable string ID for a device (persists across reboots).
fn device_id_string(dev: &cpal::Device) -> Option<String> {
    dev.id().ok().map(|id| id.to_string())
}

/// List all available audio input devices.
pub fn enumerate_input_devices() -> Vec<AudioDeviceInfo> {
    let host = cpal::default_host();
    let default_id = host
        .default_input_device()
        .and_then(|d| device_id_string(&d));

    let mut seen_ids = std::collections::HashSet::new();
    let mut devices = Vec::new();
    if let Ok(iter) = host.input_devices() {
        for dev in iter {
            let Some(id) = device_id_string(&dev) else {
                continue;
            };
            // WASAPI exposes the same physical device as both a regular
            // endpoint and a "default" alias — skip duplicates by ID.
            if !seen_ids.insert(id.clone()) {
                continue;
            }
            let label = device_label(&dev);
            let is_default = default_id.as_deref() == Some(id.as_str());
            devices.push(AudioDeviceInfo {
                id,
                label,
                is_default,
            });
        }
    }
    devices
}

/// Find an input device by its stable ID string, or fall back to the default.
///
/// When `device_id` is `None` we resolve the *real* hardware device that
/// matches the system default rather than using the special WASAPI "default"
/// endpoint directly, because that virtual endpoint can apply unwanted audio
/// processing and report a different channel/sample-rate config.
fn find_input_device(device_id: Option<&str>) -> anyhow::Result<cpal::Device> {
    let host = cpal::default_host();

    if let Some(target) = device_id {
        // Explicit device selection — find by ID.
        if let Ok(iter) = host.input_devices() {
            for dev in iter {
                if device_id_string(&dev).as_deref() == Some(target) {
                    return Ok(dev);
                }
            }
        }
        anyhow::bail!("Audio input device with id '{}' not found", target);
    }

    // "System Default" — resolve the real hardware device that corresponds to
    // the system default.  We match by name because the WASAPI "default"
    // virtual endpoint can have a different ID than the actual hardware device.
    #[allow(deprecated)]
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok());

    if let Some(ref target_name) = default_name {
        if let Ok(iter) = host.input_devices() {
            for dev in iter {
                #[allow(deprecated)]
                if dev.name().ok().as_deref() == Some(target_name.as_str()) {
                    return Ok(dev);
                }
            }
        }
    }

    // Fallback: just use whatever default_input_device gives us.
    host.default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No default audio input device found"))
}

// ---------------------------------------------------------------------------
// Audio capture (cpal + Opus encoder)
// ---------------------------------------------------------------------------

/// Real audio capture: records from a microphone, encodes to Opus.
///
/// The cpal input stream runs on its own OS thread.  The callback converts
/// whatever the device produces (stereo, 44.1 kHz, etc.) to **mono** samples
/// at the **device's native sample rate** and pushes them into a ring buffer.
///
/// [`next_frame()`] pulls enough samples for 20 ms, resamples to 48 kHz
/// (960 samples) if necessary, Opus-encodes, and returns the bytes.
pub struct CpalAudioCapture {
    /// Consumer side of the capture ring buffer (mono f32 at device rate).
    consumer: ringbuf::HeapCons<f32>,
    /// Opus encoder (48 kHz, mono, VOIP).
    encoder: OpusEncoder,
    /// Timestamp counter (48 kHz ticks).
    timestamp: u32,
    /// Mute flag — when true, silence is sent instead of mic data.
    muted: Arc<AtomicBool>,
    /// The device's native sample rate.
    device_sample_rate: u32,
    /// Number of mono samples to pull per 20 ms frame (at device rate).
    device_frame_samples: usize,
    /// Keep the stream alive (dropped = stream stops).
    _stream: cpal::Stream,
    /// Shared target bitrate — written by the bitrate adapter, read each frame.
    target_bitrate: Option<Arc<AtomicI32>>,
    /// Last applied bitrate, to avoid redundant set_bitrate calls.
    current_bitrate: i32,
}

// SAFETY: CpalAudioCapture is only ever accessed via `&mut self`
// (MediaCapture::next_frame takes &mut self).  The non-Send/Sync inner types
// are never shared across threads — the struct is moved into exactly one
// tokio::spawn task and used exclusively there.
unsafe impl Send for CpalAudioCapture {}

impl CpalAudioCapture {
    /// Open the default input device and start recording.
    pub fn new() -> anyhow::Result<Self> {
        Self::with_device(None)
    }

    /// Open a specific input device by name (or default if `None`).
    pub fn with_device(device_name: Option<&str>) -> anyhow::Result<Self> {
        let device = find_input_device(device_name)?;

        #[allow(deprecated)]
        let dev_name = device.name().unwrap_or_default();

        // Query the device's preferred format so we don't force an unsupported config.
        let supported = device.default_input_config()?;
        let device_channels = supported.channels();
        let device_sample_rate = supported.sample_rate();

        info!(
            device = dev_name,
            channels = device_channels,
            sample_rate = device_sample_rate,
            "Opening audio input device (native format)"
        );

        // Open at the device's native channels + sample rate, but always f32.
        let config = cpal::StreamConfig {
            channels: device_channels,
            sample_rate: device_sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        // Ring buffer sized for ~100 ms of mono audio at the device rate.
        let ring_capacity = (device_sample_rate as usize) / 10;
        let ring = HeapRb::<f32>::new(ring_capacity.max(4800));
        let (mut producer, consumer) = ring.split();

        let ch = device_channels as usize;
        let stream = device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if ch == 1 {
                    // Mono — push directly.
                    let _written = producer.push_slice(data);
                } else {
                    // Multi-channel — downmix to mono by averaging channels.
                    for chunk in data.chunks_exact(ch) {
                        let mono: f32 = chunk.iter().sum::<f32>() / ch as f32;
                        let _ = producer.try_push(mono);
                    }
                }
            },
            |err| {
                warn!("Audio input stream error: {err}");
            },
            None,
        )?;
        stream.play()?;

        let mut encoder = OpusEncoder::new(
            OpusSampleRate::Hz48000,
            Channels::Mono,
            Application::Voip,
        )?;
        let _ = encoder.set_bitrate(audiopus::Bitrate::BitsPerSecond(OPUS_BITRATE));
        // Tell Opus the input is voice — enables built-in voice-specific optimisations.
        let _ = encoder.set_signal(audiopus::Signal::Voice);
        // Max encoder complexity (10) — better quality at slight CPU cost (negligible for mono voice).
        let _ = encoder.set_complexity(10);
        // Enable in-band Forward Error Correction so the decoder can partially
        // recover lost packets from subsequent ones.
        let _ = encoder.set_inband_fec(true);

        let device_frame_samples = (device_sample_rate as usize) / 50; // 20 ms
        let muted = Arc::new(AtomicBool::new(false));

        Ok(Self {
            consumer,
            encoder,
            timestamp: 0,
            muted,
            device_sample_rate,
            device_frame_samples,
            _stream: stream,
            target_bitrate: None,
            current_bitrate: OPUS_BITRATE,
        })
    }

    /// Get a handle to the mute flag (cheap clone).
    pub fn mute_handle(&self) -> Arc<AtomicBool> {
        self.muted.clone()
    }

    /// Attach a shared bitrate target for dynamic adaptation.
    pub fn set_bitrate_target(&mut self, target: Arc<AtomicI32>) {
        self.target_bitrate = Some(target);
    }
}

#[async_trait]
impl MediaCapture for CpalAudioCapture {
    async fn next_frame(&mut self) -> anyhow::Result<CapturedFrame> {
        // Dynamically adapt encoder bitrate if a shared target is set.
        if let Some(ref target) = self.target_bitrate {
            let desired = target.load(Ordering::Relaxed);
            if desired != self.current_bitrate {
                let _ = self
                    .encoder
                    .set_bitrate(audiopus::Bitrate::BitsPerSecond(desired));
                info!(
                    old = self.current_bitrate,
                    new = desired,
                    "Opus bitrate adapted"
                );
                self.current_bitrate = desired;
            }
        }

        // Wait until we have a full frame at the device's native rate.
        loop {
            if self.consumer.occupied_len() >= self.device_frame_samples {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }

        // Pull mono PCM at device rate.
        let mut device_pcm = vec![0.0f32; self.device_frame_samples];
        self.consumer.pop_slice(&mut device_pcm);

        // If muted, zero out so Opus encodes silence.
        if self.muted.load(Ordering::Relaxed) {
            device_pcm.fill(0.0);
        }

        // Resample to 48 kHz if the device rate differs.
        let pcm_48k = if self.device_sample_rate == OPUS_SAMPLE_RATE {
            device_pcm
        } else {
            resample(&device_pcm, self.device_sample_rate, OPUS_SAMPLE_RATE)
        };

        // Convert f32 [-1.0, 1.0] → i16 for Opus encoder.
        let pcm_i16: Vec<i16> = pcm_48k
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();

        // Encode.
        let mut opus_buf = vec![0u8; 4000];
        let encoded_len = self.encoder.encode(&pcm_i16, &mut opus_buf)?;
        opus_buf.truncate(encoded_len);

        let ts = self.timestamp;
        self.timestamp = self.timestamp.wrapping_add(AUDIO_FRAME_TICKS);

        trace!(len = opus_buf.len(), ts, "Captured & encoded audio frame");

        Ok(CapturedFrame {
            data: opus_buf,
            is_audio: true,
            timestamp: ts,
            video_chunk: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Simple linear resampler
// ---------------------------------------------------------------------------

/// Resample a buffer of mono f32 samples from `from_rate` to `to_rate` using
/// linear interpolation.  Good enough for voice at 20 ms chunks.
fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    let out_len = ((input.len() as u64 * to_rate as u64) / from_rate as u64) as usize;
    let ratio = from_rate as f64 / to_rate as f64;
    let mut output = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos as usize;
        let frac = src_pos - idx as f64;

        let a = input.get(idx).copied().unwrap_or(0.0);
        let b = input.get(idx + 1).copied().unwrap_or(a);
        output.push(a + (b - a) * frac as f32);
    }

    output
}

// ---------------------------------------------------------------------------
// Silence capture source (test / fallback stub)
// ---------------------------------------------------------------------------

/// Stub capture source that emits Opus silence frames every 20 ms.
pub struct SilenceCaptureSource {
    timestamp: u32,
    frame_ticks: u32,
}

impl SilenceCaptureSource {
    pub fn new() -> Self {
        Self {
            timestamp: 0,
            frame_ticks: AUDIO_FRAME_TICKS,
        }
    }
}

#[async_trait]
impl MediaCapture for SilenceCaptureSource {
    async fn next_frame(&mut self) -> anyhow::Result<CapturedFrame> {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let opus_frame = vec![0xF8, 0xFF, 0xFE];
        let ts = self.timestamp;
        self.timestamp = self.timestamp.wrapping_add(self.frame_ticks);
        Ok(CapturedFrame {
            data: opus_frame,
            is_audio: true,
            timestamp: ts,
            video_chunk: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Audio jitter buffer
// ---------------------------------------------------------------------------

/// Per-sender, per-ssrc audio jitter buffer.
///
/// Operates in two phases:
/// 1. **Buffering** — accumulate `min_depth` frames before starting playout
///    so we have a cushion to absorb network jitter.
/// 2. **Playout** — deliver frames in sequence order at a steady 20 ms pace.
///
/// Reverts to Buffering after the buffer drains completely.
struct AudioJitterBuffer {
    buffer: BTreeMap<u16, Vec<u8>>,
    next_playout_seq: Option<u16>,
    max_frames: usize,
    /// Minimum frames to accumulate before starting playout.
    min_depth: usize,
    /// `true` while we are still filling up to `min_depth`.
    buffering: bool,
    /// Ticks since the last successfully decoded frame.
    idle_ticks: u32,
}

impl AudioJitterBuffer {
    fn new(max_frames: usize, min_depth: usize) -> Self {
        Self {
            buffer: BTreeMap::new(),
            next_playout_seq: None,
            max_frames,
            min_depth: min_depth.max(1),
            buffering: true,
            idle_ticks: 0,
        }
    }

    fn insert(&mut self, seq: u16, opus_frame: Vec<u8>) {
        if self.buffer.len() >= self.max_frames {
            if let Some((&oldest_seq, _)) = self.buffer.iter().next() {
                self.buffer.remove(&oldest_seq);
            }
        }
        self.buffer.insert(seq, opus_frame);
    }

    fn pull(&mut self) -> Option<Vec<u8>> {
        // Nothing buffered → don't advance, stay in / return to buffering.
        if self.buffer.is_empty() {
            self.idle_ticks += 1;
            self.buffering = true;
            self.next_playout_seq = None;
            return None;
        }

        // Still filling up — wait until we have enough frames.
        if self.buffering {
            if self.buffer.len() < self.min_depth {
                return None; // don't advance, don't count idle
            }
            // Enough frames accumulated — switch to playout.
            self.buffering = false;
            self.next_playout_seq = None; // will be synced below
        }

        let earliest = *self.buffer.keys().next().unwrap();

        // Sync playout pointer to the buffer on first pull or after resync.
        let seq = match self.next_playout_seq {
            None => earliest,
            Some(nps) => {
                // If next_playout_seq fell behind the buffer, skip forward.
                let diff = earliest.wrapping_sub(nps);
                if diff > 0 && diff < 32768 {
                    earliest
                } else {
                    nps
                }
            }
        };

        if let Some(frame) = self.buffer.remove(&seq) {
            self.next_playout_seq = Some(seq.wrapping_add(1));
            self.idle_ticks = 0;
            Some(frame)
        } else {
            // Expected seq not buffered yet (late / lost) — advance past it.
            self.next_playout_seq = Some(seq.wrapping_add(1));
            self.idle_ticks += 1;
            None
        }
    }

    /// Returns true when this buffer has been idle long enough to discard.
    fn is_stale(&self) -> bool {
        // ~5 seconds of no decoded frames (250 ticks × 20 ms).
        self.idle_ticks > 250
    }
}

// ---------------------------------------------------------------------------
// Audio playback (cpal + Opus decoder + mixer)
// ---------------------------------------------------------------------------

/// Real audio playback: decodes Opus, mixes all peers, plays through speakers.
///
/// Uses the output device's native format (channels + sample rate) and converts
/// from the internal 48 kHz mono format as needed.
pub struct CpalAudioPlayback {
    /// (peer_id, ssrc) -> jitter buffer.
    buffers: Mutex<BTreeMap<(u64, u32), AudioJitterBuffer>>,
    /// Per-peer Opus decoders.
    decoders: Mutex<BTreeMap<u64, OpusDecoder>>,
    /// Producer side of the output ring buffer (mono f32 at device rate).
    producer: Mutex<ringbuf::HeapProd<f32>>,
    /// Max jitter buffer depth.
    max_buffer_frames: usize,
    /// Output device's native sample rate.
    device_sample_rate: u32,
    /// Output device's native channel count.
    #[allow(dead_code)]
    device_channels: u16,
    /// Keep the output stream alive.
    _stream: cpal::Stream,
    /// Cancellation token — cancelled when `stop()` is called.
    cancel: CancellationToken,
    /// Optional transport for sending ReceiverReport messages.
    transport: Mutex<Option<Arc<dyn crate::application::ports::Transport>>>,
    /// Our own peer id (needed as the "reporter" in receiver reports).
    our_peer_id: Mutex<Option<PeerId>>,
}

// SAFETY: All mutable state is behind tokio::sync::Mutex.
// cpal::Stream is Send on all desktop platforms.
unsafe impl Send for CpalAudioPlayback {}
unsafe impl Sync for CpalAudioPlayback {}

impl CpalAudioPlayback {
    /// Open the default output device and prepare for playback.
    pub fn new(max_buffer_frames: usize) -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("No default audio output device found"))?;

        #[allow(deprecated)]
        let dev_name = device.name().unwrap_or_default();

        let supported = device.default_output_config()?;
        let device_channels = supported.channels();
        let device_sample_rate = supported.sample_rate();

        info!(
            device = dev_name,
            channels = device_channels,
            sample_rate = device_sample_rate,
            "Opening audio output device (native format)"
        );

        let config = cpal::StreamConfig {
            channels: device_channels,
            sample_rate: device_sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        // Ring buffer: holds mono samples at device rate, ~200 ms headroom
        // to absorb Windows timer jitter (default resolution ~15.6 ms).
        let ring_capacity = (device_sample_rate as usize) / 5;
        let ring = HeapRb::<f32>::new(ring_capacity.max(9600));
        let (producer, mut consumer) = ring.split();

        let ch = device_channels as usize;
        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Zero the entire buffer first to avoid any trailing garbage
                // (e.g. when data.len() isn't a perfect multiple of ch).
                data.fill(0.0);

                if ch == 1 {
                    consumer.pop_slice(data);
                } else {
                    // Multi-channel output — duplicate mono sample to all channels.
                    for chunk in data.chunks_exact_mut(ch) {
                        let sample = consumer.try_pop().unwrap_or(0.0);
                        chunk.fill(sample);
                    }
                }
            },
            |err| {
                warn!("Audio output stream error: {err}");
            },
            None,
        )?;
        stream.play()?;

        Ok(Self {
            buffers: Mutex::new(BTreeMap::new()),
            decoders: Mutex::new(BTreeMap::new()),
            producer: Mutex::new(producer),
            max_buffer_frames,
            device_sample_rate,
            device_channels,
            _stream: stream,
            cancel: CancellationToken::new(),
            transport: Mutex::new(None),
            our_peer_id: Mutex::new(None),
        })
    }

    /// Spawn the 20 ms playout mixer loop.  Call once after construction.
    pub fn start(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            this.playout_loop().await;
        });
        debug!("Audio playout loop started");
    }

    /// Signal the playout loop to stop.  Safe to call multiple times.
    pub fn stop(&self) {
        self.cancel.cancel();
        debug!("Audio playout loop stop requested");
    }

    /// Attach a transport so the playout loop can send `ReceiverReport` messages.
    pub async fn set_transport(
        &self,
        transport: Arc<dyn crate::application::ports::Transport>,
        our_peer_id: PeerId,
    ) {
        *self.transport.lock().await = Some(transport);
        *self.our_peer_id.lock().await = Some(our_peer_id);
    }

    /// Internal mixer loop: pull from jitter buffers → decode → mix → resample → output ring.
    ///
    /// Uses wall-clock elapsed time to decide how many 20 ms frames to decode
    /// per wakeup.  On Windows the default timer resolution is ~15.6 ms so
    /// `tokio::time::interval(20ms)` actually fires every ~31 ms.  By decoding
    /// multiple frames per wakeup we compensate and keep the ring buffer fed
    /// at the correct rate.
    async fn playout_loop(&self) {
        let mut mix_buf = vec![0.0f32; OPUS_FRAME_SAMPLES];
        let mut decode_buf = vec![0i16; OPUS_FRAME_SAMPLES];

        // Diagnostic counters — logged every ~5 seconds (250 frames).
        let mut frames_decoded: u64 = 0;
        let mut frames_missed: u64 = 0;
        let mut frames_pushed: u64 = 0;
        let mut diag_decoded: u64 = 0;

        let start = tokio::time::Instant::now();
        // How many 20 ms frames should have been consumed by now.
        let mut frames_due: u64;
        // How many frames we have actually pushed to the ring.
        let mut frames_produced: u64 = 0;

        // Wake up every ~5 ms (Windows will round to ~15.6 ms, but that's OK —
        // we decode however many frames are due since the last wake).
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(5));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    info!("Audio playout loop stopped (cancelled)");
                    return;
                }
                _ = ticker.tick() => {}
            }

            // How many total 20 ms frames should have played out by now?
            let elapsed_ms = start.elapsed().as_millis() as u64;
            frames_due = elapsed_ms / 20;

            // Decode as many frames as needed to catch up.
            let frames_needed = frames_due.saturating_sub(frames_produced);
            if frames_needed == 0 {
                continue;
            }

            let mut buffers = self.buffers.lock().await;
            let mut decoders = self.decoders.lock().await;

            for _ in 0..frames_needed {
                mix_buf.fill(0.0);
                let mut peer_count = 0u32;

                for (&(peer_id, _ssrc), jitter) in buffers.iter_mut() {
                    let opus_data = jitter.pull();

                    let Some(ref data) = opus_data else {
                        frames_missed += 1;
                        continue;
                    };

                    let decoder = decoders.entry(peer_id).or_insert_with(|| {
                        OpusDecoder::new(OpusSampleRate::Hz48000, Channels::Mono)
                            .expect("Failed to create Opus decoder")
                    });

                    let decoded_samples =
                        match decoder.decode(Some(data.as_slice()), &mut decode_buf[..], false) {
                            Ok(n) => n,
                            Err(e) => {
                                warn!(?peer_id, "Opus decode error: {e}");
                                0
                            }
                        };

                    if decoded_samples > 0 {
                        peer_count += 1;
                        frames_decoded += 1;
                        let samples = decoded_samples.min(OPUS_FRAME_SAMPLES);
                        for i in 0..samples {
                            mix_buf[i] += decode_buf[i] as f32 / i16::MAX as f32;
                        }
                    }
                }

                if peer_count > 0 {
                    for s in mix_buf.iter_mut() {
                        *s = s.clamp(-1.0, 1.0);
                    }

                    // Resample 48 kHz → device rate if they differ.
                    let resampled;
                    let final_samples: &[f32] = if self.device_sample_rate == OPUS_SAMPLE_RATE {
                        &mix_buf[..]
                    } else {
                        resampled = resample(&mix_buf, OPUS_SAMPLE_RATE, self.device_sample_rate);
                        &resampled[..]
                    };

                    // Push mono samples; the cpal callback expands to device channels.
                    let mut producer = self.producer.lock().await;
                    let written = producer.push_slice(final_samples);
                    if written < final_samples.len() {
                        trace!(
                            "Output ring buffer full, dropped {} samples",
                            final_samples.len() - written
                        );
                    }
                    drop(producer);
                    frames_pushed += 1;
                }

                frames_produced += 1;
                diag_decoded += 1;
            }

            // Evict stale jitter buffers (disconnected peers with no new data).
            let stale_keys: Vec<_> = buffers
                .iter()
                .filter(|(_, jb)| jb.is_stale())
                .map(|(k, _)| *k)
                .collect();
            for key in &stale_keys {
                buffers.remove(key);
                decoders.remove(&key.0);
                debug!(peer_id = key.0, ssrc = key.1, "Evicted stale jitter buffer");
            }

            drop(decoders);
            drop(buffers);

            // Periodic diagnostic log + receiver reports (~every 250 frames = 5s of audio).
            if diag_decoded >= 250 {
                info!(
                    frames_decoded,
                    frames_missed,
                    frames_pushed,
                    produced = frames_produced,
                    due = frames_due,
                    "Audio playout stats (last 5s)"
                );

                // Send ReceiverReport for each active peer.
                let total = frames_decoded + frames_missed;
                if total > 0 {
                    let loss_fraction = frames_missed as f32 / total as f32;
                    let transport = self.transport.lock().await;
                    let our_id = *self.our_peer_id.lock().await;
                    if let (Some(ref tx), Some(_our_id)) = (&*transport, our_id) {
                        let buffers = self.buffers.lock().await;
                        let peer_ids: Vec<u64> = buffers
                            .keys()
                            .map(|(pid, _)| *pid)
                            .collect::<std::collections::BTreeSet<_>>()
                            .into_iter()
                            .collect();
                        let buf_depth = buffers
                            .values()
                            .next()
                            .map(|jb| jb.buffer.len() as u16)
                            .unwrap_or(0);
                        drop(buffers);

                        for pid in peer_ids {
                            let report = ControlMsg::ReceiverReport {
                                target_sender_id: PeerId(pid),
                                loss_fraction,
                                jitter_ms: 0.0,
                                buffer_depth: buf_depth,
                            };
                            if let Err(e) = tx.send_control(report).await {
                                trace!("Failed to send ReceiverReport: {e}");
                            }
                        }
                    }
                }

                frames_decoded = 0;
                frames_missed = 0;
                frames_pushed = 0;
                diag_decoded = 0;
            }
        }
    }

    fn get_or_create_buffer<'a>(
        buffers: &'a mut BTreeMap<(u64, u32), AudioJitterBuffer>,
        peer_id: PeerId,
        ssrc: u32,
        max_frames: usize,
    ) -> &'a mut AudioJitterBuffer {
        buffers
            .entry((peer_id.0, ssrc))
            .or_insert_with(|| {
                // min_depth=3 → buffer 60 ms before starting playout,
                // enough to absorb typical LAN / localhost jitter.
                AudioJitterBuffer::new(max_frames, 3)
            })
    }
}

#[async_trait]
impl MediaPlayback for CpalAudioPlayback {
    async fn push_audio(
        &self,
        peer_id: PeerId,
        ssrc: u32,
        seq: u16,
        _timestamp: u32,
        opus_frame: &[u8],
    ) -> anyhow::Result<()> {
        let mut buffers = self.buffers.lock().await;
        let is_new_peer = !buffers.contains_key(&(peer_id.0, ssrc));
        let buf =
            Self::get_or_create_buffer(&mut buffers, peer_id, ssrc, self.max_buffer_frames);
        buf.insert(seq, opus_frame.to_vec());
        if is_new_peer {
            info!(?peer_id, ssrc, "New audio stream started (first frame received)");
        }
        trace!(?peer_id, ssrc, seq, len = opus_frame.len(), "Buffered audio frame");
        Ok(())
    }

    async fn push_video(
        &self,
        _peer_id: PeerId,
        _ssrc: u32,
        _seq: u16,
        _timestamp: u32,
        _frame_id: u32,
        _chunk_index: u16,
        _chunk_count: u16,
        _is_keyframe: bool,
        _encoded_bytes: &[u8],
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Legacy aliases
// ---------------------------------------------------------------------------

/// Alias kept for backward compatibility with existing imports.
pub type AudioPlaybackAdapter = CpalAudioPlayback;

/// Alias kept for backward compatibility.
pub type AudioCaptureSource = SilenceCaptureSource;
