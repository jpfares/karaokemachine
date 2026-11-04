//! Audio output and microphone input.
//!
//! Decoding is separated from playback so it can run on a worker thread:
//! [`LoadedAudio::decode`] produces interleaved stereo buffers at the output
//! rate, and [`AudioEngine::install`] swaps them in under the mixer lock,
//! which is the only time the real-time callback waits on the UI.

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use std::collections::VecDeque;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Length of the fade applied when playback starts, pauses or jumps, so a
/// transport change never produces a click.
const RAMP_SECONDS: f32 = 0.005;

/// Samples kept from the microphone: about a third of a second at 48 kHz.
const MIC_RING_SAMPLES: usize = 16384;

/// A song decoded to interleaved stereo at the engine's sample rate.
pub struct LoadedAudio {
    backing: Vec<f32>,
    vocals: Vec<f32>,
    frames: usize,
    sample_rate: u32,
}

impl LoadedAudio {
    /// Decode the backing track and, if present, the vocal stem. Safe to call
    /// from a worker thread. The vocal stem is padded or truncated to the
    /// backing's length; it used to be dropped on any mismatch, which
    /// silenced the guide vocal on every bundled warm-up.
    pub fn decode(
        backing_path: &Path,
        vocals_path: Option<&Path>,
        sample_rate: u32,
    ) -> Result<Self> {
        let backing = decode_audio(backing_path, sample_rate as usize)?;
        let vocals = match vocals_path {
            Some(path) => match decode_audio(path, sample_rate as usize) {
                Ok(vocals) => vocals,
                Err(error) => {
                    eprintln!("vocal stem unavailable ({}): {error:#}", path.display());
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        Ok(Self::from_samples(backing, vocals, sample_rate))
    }

    /// Build from already-decoded interleaved stereo buffers.
    pub fn from_samples(backing: Vec<f32>, mut vocals: Vec<f32>, sample_rate: u32) -> Self {
        let frames = backing.len() / 2;
        if !vocals.is_empty() && vocals.len() != frames * 2 {
            let difference = vocals.len() as f64 / 2.0 - frames as f64;
            eprintln!(
                "vocal stem is {:.2} s {} than the backing; fitting it",
                (difference.abs() / sample_rate.max(1) as f64),
                if difference > 0.0 {
                    "longer"
                } else {
                    "shorter"
                }
            );
            vocals.resize(frames * 2, 0.0);
        }
        Self {
            backing,
            vocals,
            frames,
            sample_rate,
        }
    }

    pub fn duration(&self) -> f64 {
        self.frames as f64 / self.sample_rate.max(1) as f64
    }

    pub fn has_vocals(&self) -> bool {
        !self.vocals.is_empty()
    }

    /// Peak amplitude of the vocal stem in short bins, for the editor's
    /// time-aligned waveform.
    pub fn vocal_peaks(&self, bins_per_second: f32) -> Vec<f32> {
        if self.frames == 0 || !self.has_vocals() {
            return Vec::new();
        }
        let bins = (self.duration() * bins_per_second.max(1.0) as f64)
            .ceil()
            .max(1.0) as usize;
        let frames_per_bin = (self.frames as f64 / bins as f64).max(1.0);
        let mut peaks = vec![0.0f32; bins];
        for (bin, peak) in peaks.iter_mut().enumerate() {
            let start = (bin as f64 * frames_per_bin).floor() as usize;
            let end = (((bin + 1) as f64 * frames_per_bin).ceil() as usize).min(self.frames);
            let (frames, _) = self.vocals[start * 2..end * 2].as_chunks::<2>();
            *peak = frames
                .iter()
                .map(|[left, right]| left.abs().max(right.abs()))
                .fold(0.0f32, f32::max);
        }
        peaks
    }

    /// Decimated mono copy of the vocal stem for offline pitch analysis.
    pub fn vocal_mono(&self, sample_rate: f32) -> Vec<f32> {
        if self.frames == 0 || !self.has_vocals() {
            return Vec::new();
        }
        let target_rate = sample_rate.max(1000.0) as f64;
        let source_rate = self.sample_rate.max(1) as f64;
        let out_frames = (self.frames as f64 * target_rate / source_rate).ceil() as usize;
        let last = self.frames - 1;
        let mut out = Vec::with_capacity(out_frames);
        for index in 0..out_frames {
            let source = index as f64 * source_rate / target_rate;
            let first = (source.floor() as usize).min(last);
            let second = (first + 1).min(last);
            let frac = (source - first as f64) as f32;
            let left = lerp(self.vocals[first * 2], self.vocals[second * 2], frac);
            let right = lerp(
                self.vocals[first * 2 + 1],
                self.vocals[second * 2 + 1],
                frac,
            );
            out.push((left + right) * 0.5);
        }
        out
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Everything the real-time callback reads. Guarded by one mutex that the
/// UI holds only for short field updates and the buffer swap in `install`.
#[derive(Clone, Debug)]
struct Mixer {
    backing: Vec<f32>,
    vocals: Vec<f32>,
    frames: usize,
    pos: f64,
    rate: f32,
    playing: bool,
    /// Current output gain, ramped toward 1.0 while playing and 0.0 otherwise.
    gain: f32,
    ramp_step: f32,
    backing_vol: f32,
    vocals_vol: f32,
    preview_phase: f32,
    preview_step: f32,
    preview_remaining: usize,
    preview_total: usize,
    preview_amp: f32,
    preview_rate: f32,
}

impl Mixer {
    fn new(sample_rate: u32, backing_vol: f32, vocals_vol: f32) -> Self {
        let rate = sample_rate.max(1) as f32;
        Self {
            backing: Vec::new(),
            vocals: Vec::new(),
            frames: 0,
            pos: 0.0,
            rate: 1.0,
            playing: false,
            gain: 0.0,
            ramp_step: 1.0 / (RAMP_SECONDS * rate),
            backing_vol,
            vocals_vol,
            preview_phase: 0.0,
            preview_step: 0.0,
            preview_remaining: 0,
            preview_total: 0,
            preview_amp: 0.0,
            preview_rate: rate,
        }
    }
}

struct Shared {
    mixer: Mutex<Mixer>,
    /// Playhead in seconds, stored as f64 bits so a fractional playback
    /// position is not truncated every audio callback.
    position: AtomicU64,
    sample_rate: f64,
    /// Set by cpal's error callback when the stream dies (device unplugged,
    /// default device changed). The UI polls it and rebuilds the engine.
    failed: AtomicBool,
}

pub struct AudioEngine {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
    sample_rate: u32,
}

impl AudioEngine {
    pub fn new(backing_vol: f32, vocals_vol: f32) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no default audio output device"))?;
        let supported = device
            .default_output_config()
            .context("could not query default output format")?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let sample_rate = config.sample_rate.0;
        let channels = config.channels as usize;
        let shared = Arc::new(Shared {
            mixer: Mutex::new(Mixer::new(sample_rate, backing_vol, vocals_vol)),
            position: AtomicU64::new(0.0f64.to_bits()),
            sample_rate: sample_rate as f64,
            failed: AtomicBool::new(false),
        });
        let stream = match sample_format {
            SampleFormat::F32 => output_stream::<f32>(&device, &config, channels, &shared)?,
            SampleFormat::F64 => output_stream::<f64>(&device, &config, channels, &shared)?,
            SampleFormat::I16 => output_stream::<i16>(&device, &config, channels, &shared)?,
            SampleFormat::I32 => output_stream::<i32>(&device, &config, channels, &shared)?,
            SampleFormat::I8 => output_stream::<i8>(&device, &config, channels, &shared)?,
            SampleFormat::U16 => output_stream::<u16>(&device, &config, channels, &shared)?,
            SampleFormat::U32 => output_stream::<u32>(&device, &config, channels, &shared)?,
            SampleFormat::U8 => output_stream::<u8>(&device, &config, channels, &shared)?,
            other => return Err(anyhow!("unsupported output sample format: {other:?}")),
        };
        stream.play()?;
        Ok(Self {
            _stream: stream,
            shared,
            sample_rate,
        })
    }

    /// The rate decoded audio must be resampled to before `install`.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Whether the output stream has died. Once true the engine is useless;
    /// drop it and build a new one.
    pub fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Relaxed)
    }

    /// Replace the current song. Playback stops at the start of the new one.
    /// Returns its duration in seconds.
    pub fn install(&self, audio: LoadedAudio) -> f64 {
        let duration = audio.duration();
        let mut mixer = self.lock();
        mixer.backing = audio.backing;
        mixer.vocals = audio.vocals;
        mixer.frames = audio.frames;
        mixer.pos = 0.0;
        mixer.playing = false;
        mixer.gain = 0.0;
        drop(mixer);
        self.set_position(0.0);
        duration
    }

    pub fn play(&self) {
        let mut mixer = self.lock();
        if mixer.frames > 0 {
            if mixer.pos >= mixer.frames.saturating_sub(2) as f64 {
                mixer.pos = 0.0;
            }
            mixer.playing = true;
        }
    }

    pub fn pause(&self) {
        self.lock().playing = false;
    }

    pub fn stop(&self) {
        let mut mixer = self.lock();
        mixer.playing = false;
        mixer.pos = 0.0;
        mixer.gain = 0.0;
        drop(mixer);
        self.set_position(0.0);
    }

    pub fn seek(&self, seconds: f64) {
        let mut mixer = self.lock();
        mixer.pos = (seconds.max(0.0) * self.sample_rate as f64).min(mixer.frames as f64);
        // Fade back in from the new position instead of jumping mid-wave.
        mixer.gain = 0.0;
        let position = mixer.pos / self.sample_rate as f64;
        drop(mixer);
        self.set_position(position);
    }

    pub fn position(&self) -> f64 {
        f64::from_bits(self.shared.position.load(Ordering::Relaxed))
    }

    fn set_position(&self, seconds: f64) {
        self.shared
            .position
            .store(seconds.to_bits(), Ordering::Relaxed);
    }

    /// Varispeed: the read position advances `rate` frames per output frame,
    /// so pitch changes with speed.
    pub fn set_rate(&self, rate: f32) {
        self.lock().rate = rate.clamp(0.25, 2.0);
    }

    /// Audition a note in the editor without changing the transport position.
    pub fn play_preview(&self, midi: i32, seconds: f32) {
        let mut mixer = self.lock();
        let frequency = 440.0 * 2.0f32.powf((midi as f32 - 69.0) / 12.0);
        let duration = seconds.clamp(0.08, 3.0);
        let rate = self.sample_rate.max(1) as f32;
        mixer.preview_phase = 0.0;
        mixer.preview_step = std::f32::consts::TAU * frequency / rate;
        mixer.preview_total = (duration * rate) as usize;
        mixer.preview_remaining = mixer.preview_total;
        mixer.preview_amp = 0.24;
        mixer.preview_rate = rate;
    }

    pub fn set_volumes(&self, backing: f32, vocals: f32) {
        let mut mixer = self.lock();
        mixer.backing_vol = backing.clamp(0.0, 1.5);
        mixer.vocals_vol = vocals.clamp(0.0, 1.5);
    }

    /// The callback never panics while holding the lock, so a poisoned mutex
    /// means a UI-side panic already happened; recover rather than cascade.
    fn lock(&self) -> std::sync::MutexGuard<'_, Mixer> {
        self.shared
            .mixer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn output_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let data_shared = shared.clone();
    let error_shared = shared.clone();
    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _| write_output(data, channels, &data_shared),
        move |error| {
            eprintln!("audio output error: {error}");
            error_shared.failed.store(true, Ordering::Relaxed);
        },
        None,
    )?)
}

fn sample_linear(data: &[f32], frame: usize, channel: usize, frames: usize, frac: f32) -> f32 {
    if frame >= frames {
        return 0.0;
    }
    let a = data.get(frame * 2 + channel).copied().unwrap_or(0.0);
    let next = (frame + 1).min(frames.saturating_sub(1));
    let b = data.get(next * 2 + channel).copied().unwrap_or(a);
    lerp(a, b, frac)
}

/// One output frame. The song is read while the gain is above zero so a
/// pause fades out on the last sample instead of cutting; the position only
/// advances while playing.
fn next_frame(mixer: &mut Mixer) -> (f32, f32) {
    let in_song = mixer.pos < mixer.frames as f64;
    let target = if mixer.playing && in_song { 1.0 } else { 0.0 };
    let step = mixer.ramp_step;
    mixer.gain += (target - mixer.gain).clamp(-step, step);
    let mut l = 0.0f32;
    let mut r = 0.0f32;
    if mixer.gain > 0.0 && in_song {
        let frame = mixer.pos.floor() as usize;
        let frac = (mixer.pos - frame as f64) as f32;
        l = (sample_linear(&mixer.backing, frame, 0, mixer.frames, frac) * mixer.backing_vol
            + sample_linear(&mixer.vocals, frame, 0, mixer.frames, frac) * mixer.vocals_vol)
            * mixer.gain;
        r = (sample_linear(&mixer.backing, frame, 1, mixer.frames, frac) * mixer.backing_vol
            + sample_linear(&mixer.vocals, frame, 1, mixer.frames, frac) * mixer.vocals_vol)
            * mixer.gain;
    }
    if mixer.playing && in_song {
        mixer.pos += mixer.rate.max(0.05) as f64;
    }

    // Editor note audition is independent of the transport, so it still
    // sounds while the backing is paused.
    if mixer.preview_remaining > 0 {
        let rate = mixer.preview_rate.max(1.0);
        let elapsed = (mixer.preview_total.saturating_sub(mixer.preview_remaining)) as f32 / rate;
        let remaining = mixer.preview_remaining as f32 / rate;
        let attack = (elapsed / 0.006).min(1.0);
        let release = (remaining / 0.035).min(1.0);
        let envelope = attack.min(release);
        let sample = (mixer.preview_phase.sin() * 0.75 + (mixer.preview_phase * 2.0).sin() * 0.18)
            * mixer.preview_amp
            * envelope;
        l += sample;
        r += sample;
        mixer.preview_phase = (mixer.preview_phase + mixer.preview_step) % std::f32::consts::TAU;
        mixer.preview_remaining -= 1;
    }
    (l, r)
}

/// The real-time output callback, generic over the device's sample type.
fn write_output<T>(data: &mut [T], channels: usize, shared: &Arc<Shared>)
where
    T: SizedSample + FromSample<f32>,
{
    let mut mixer = shared
        .mixer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for frame in data.chunks_mut(channels) {
        let (l, r) = next_frame(&mut mixer);
        if channels == 1 {
            frame[0] = T::from_sample(((l + r) * 0.5).clamp(-1.0, 1.0));
        } else {
            frame[0] = T::from_sample(l.clamp(-1.0, 1.0));
            frame[1] = T::from_sample(r.clamp(-1.0, 1.0));
            for sample in frame.iter_mut().skip(2) {
                *sample = T::EQUILIBRIUM;
            }
        }
    }
    shared.position.store(
        (mixer.pos / shared.sample_rate).to_bits(),
        Ordering::Relaxed,
    );
    if mixer.playing && mixer.pos >= mixer.frames as f64 {
        mixer.playing = false;
    }
}

/// Decode any symphonia-supported file to interleaved stereo at `out_rate`
/// with linear interpolation.
fn decode_audio(path: &Path, out_rate: usize) -> Result<Vec<f32>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .context("probing audio")?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| anyhow!("no default audio track in {}", path.display()))?;
    let track_id = track.id;
    let in_rate = track.codec_params.sample_rate.unwrap_or(48000) as usize;
    let in_channels = track
        .codec_params
        .channels
        .map(|channels| channels.count())
        .unwrap_or(2)
        .max(1);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("creating audio decoder")?;
    let mut interleaved: Vec<f32> = Vec::new();
    let mut buffer: Option<SampleBuffer<f32>> = None;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let capacity = decoded.capacity() as u64;
                let buffer = buffer.get_or_insert_with(|| SampleBuffer::<f32>::new(capacity, spec));
                if buffer.capacity() < decoded.capacity() * spec.channels.count() {
                    *buffer = SampleBuffer::<f32>::new(capacity, spec);
                }
                buffer.copy_interleaved_ref(decoded);
                interleaved.extend_from_slice(buffer.samples());
            }
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }
    if interleaved.is_empty() {
        return Ok(Vec::new());
    }
    let in_frames = interleaved.len() / in_channels;
    let out_frames = if in_rate == out_rate {
        in_frames
    } else {
        (in_frames as u128 * out_rate as u128 / in_rate as u128) as usize
    };
    let mut out = vec![0.0f32; out_frames * 2];
    for frame in 0..out_frames {
        let src = if in_rate == out_rate {
            frame as f64
        } else {
            frame as f64 * in_rate as f64 / out_rate as f64
        };
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(in_frames.saturating_sub(1));
        let t = (src - i0 as f64) as f32;
        let left0 = interleaved[i0 * in_channels];
        let left1 = interleaved[i1 * in_channels];
        let right0 = if in_channels > 1 {
            interleaved[i0 * in_channels + 1]
        } else {
            left0
        };
        let right1 = if in_channels > 1 {
            interleaved[i1 * in_channels + 1]
        } else {
            left1
        };
        out[frame * 2] = lerp(left0, left1, t);
        out[frame * 2 + 1] = lerp(right0, right1, t);
    }
    Ok(out)
}

pub struct Microphone {
    _stream: cpal::Stream,
    ring: Arc<Mutex<VecDeque<f32>>>,
    failed: Arc<AtomicBool>,
    pub sample_rate: u32,
    pub device_name: String,
}

pub fn input_device_names() -> Vec<String> {
    let host = cpal::default_host();
    let mut names = Vec::new();
    if let Ok(devices) = host.input_devices() {
        for device in devices {
            if let Ok(name) = device.name() {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    names
}

impl Microphone {
    pub fn new(selected: Option<&str>) -> Result<Self> {
        let host = cpal::default_host();
        let mut chosen = None;
        if let Some(wanted) = selected {
            if let Ok(devices) = host.input_devices() {
                chosen = devices
                    .into_iter()
                    .find(|device| device.name().map(|name| name == wanted).unwrap_or(false));
            }
        }
        let device = chosen
            .or_else(|| host.default_input_device())
            .ok_or_else(|| anyhow!("no input microphone found"))?;
        let name = device.name().unwrap_or_else(|_| "microphone".into());
        let supported = device
            .default_input_config()
            .context("microphone configuration")?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let sample_rate = config.sample_rate.0;
        let channels = config.channels as usize;
        let ring = Arc::new(Mutex::new(VecDeque::with_capacity(MIC_RING_SAMPLES)));
        let failed = Arc::new(AtomicBool::new(false));
        let stream = match sample_format {
            SampleFormat::F32 => input_stream::<f32>(&device, &config, channels, &ring, &failed)?,
            SampleFormat::F64 => input_stream::<f64>(&device, &config, channels, &ring, &failed)?,
            SampleFormat::I16 => input_stream::<i16>(&device, &config, channels, &ring, &failed)?,
            SampleFormat::I32 => input_stream::<i32>(&device, &config, channels, &ring, &failed)?,
            SampleFormat::I8 => input_stream::<i8>(&device, &config, channels, &ring, &failed)?,
            SampleFormat::U16 => input_stream::<u16>(&device, &config, channels, &ring, &failed)?,
            SampleFormat::U32 => input_stream::<u32>(&device, &config, channels, &ring, &failed)?,
            SampleFormat::U8 => input_stream::<u8>(&device, &config, channels, &ring, &failed)?,
            other => return Err(anyhow!("unsupported microphone format: {other:?}")),
        };
        stream.play()?;
        Ok(Self {
            _stream: stream,
            ring,
            failed,
            sample_rate,
            device_name: name,
        })
    }

    /// Whether the input stream has died. Drop and rebuild when true.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// The most recent `count` mono samples, oldest first.
    pub fn latest(&self, count: usize) -> Vec<f32> {
        let ring = self
            .ring
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let start = ring.len().saturating_sub(count);
        ring.iter().skip(start).copied().collect()
    }
}

fn input_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    channels: usize,
    ring: &Arc<Mutex<VecDeque<f32>>>,
    failed: &Arc<AtomicBool>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let ring = ring.clone();
    let failed = failed.clone();
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| push_input(data, channels, &ring),
        move |error| {
            eprintln!("microphone error: {error}");
            failed.store(true, Ordering::Relaxed);
        },
        None,
    )?)
}

/// Mix each input frame down to mono and append it to the ring, without
/// allocating on the capture thread.
fn push_input<T>(data: &[T], channels: usize, ring: &Arc<Mutex<VecDeque<f32>>>)
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = channels.max(1);
    let mut ring = ring.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for frame in data.chunks(channels) {
        let mono = frame
            .iter()
            .map(|&sample| f32::from_sample(sample))
            .sum::<f32>()
            / channels as f32;
        if ring.len() >= MIC_RING_SAMPLES {
            ring.pop_front();
        }
        ring.push_back(mono);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo(frames: usize, value: f32) -> Vec<f32> {
        vec![value; frames * 2]
    }

    #[test]
    fn vocal_stem_is_fitted_to_the_backing() {
        let shorter = LoadedAudio::from_samples(stereo(10, 0.5), stereo(6, 0.25), 48_000);
        assert_eq!(shorter.vocals.len(), 20);
        assert_eq!(&shorter.vocals[12..], &[0.0; 8]);
        let longer = LoadedAudio::from_samples(stereo(10, 0.5), stereo(14, 0.25), 48_000);
        assert_eq!(longer.vocals.len(), 20);
        let none = LoadedAudio::from_samples(stereo(10, 0.5), Vec::new(), 48_000);
        assert!(!none.has_vocals());
        assert!(none.vocal_peaks(10.0).is_empty());
    }

    #[test]
    fn vocal_peaks_and_mono_follow_the_stem() {
        let mut vocals = stereo(4, 0.0);
        vocals[2] = -0.8; // frame 1, left
        vocals[7] = 0.3; // frame 3, right
        let audio = LoadedAudio::from_samples(stereo(4, 0.0), vocals, 4);
        assert_eq!(audio.duration(), 1.0);
        assert_eq!(audio.vocal_peaks(2.0), vec![0.8, 0.3]);
        let mono = audio.vocal_mono(1000.0);
        assert_eq!(mono.len(), 1000);
        assert_eq!(mono[0], 0.0);
        assert_eq!(mono[250], -0.4);
    }

    fn mixer_with(frames: usize) -> Mixer {
        let mut mixer = Mixer::new(4, 1.0, 0.0);
        mixer.backing = stereo(frames, 1.0);
        mixer.frames = frames;
        mixer.ramp_step = 0.25;
        mixer
    }

    #[test]
    fn playback_ramps_in_from_silence() {
        let mut mixer = mixer_with(100);
        mixer.playing = true;
        let gains: Vec<f32> = (0..5).map(|_| next_frame(&mut mixer).0).collect();
        assert_eq!(gains, vec![0.25, 0.5, 0.75, 1.0, 1.0]);
        assert_eq!(mixer.pos, 5.0);
    }

    #[test]
    fn pause_fades_out_without_moving_the_playhead() {
        let mut mixer = mixer_with(100);
        mixer.playing = true;
        mixer.gain = 1.0;
        mixer.pos = 10.0;
        next_frame(&mut mixer);
        mixer.playing = false;
        let gains: Vec<f32> = (0..5).map(|_| next_frame(&mut mixer).0).collect();
        assert_eq!(gains, vec![0.75, 0.5, 0.25, 0.0, 0.0]);
        assert_eq!(mixer.pos, 11.0);
    }

    #[test]
    fn playback_stops_at_the_end() {
        let mut mixer = mixer_with(2);
        mixer.playing = true;
        mixer.gain = 1.0;
        for _ in 0..4 {
            next_frame(&mut mixer);
        }
        assert_eq!(next_frame(&mut mixer), (0.0, 0.0));
        assert_eq!(mixer.pos, 2.0);
    }
}
