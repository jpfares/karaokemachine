use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::collections::VecDeque;
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Clone, Debug)]
struct Mixer {
    backing: Vec<f32>,
    vocals: Vec<f32>,
    frames: usize,
    pos: f64,
    rate: f32,
    playing: bool,
    backing_vol: f32,
    vocals_vol: f32,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            backing: Vec::new(),
            vocals: Vec::new(),
            frames: 0,
            pos: 0.0,
            rate: 1.0,
            playing: false,
            backing_vol: 0.9,
            vocals_vol: 0.0,
        }
    }
}

struct Shared {
    mixer: Mutex<Mixer>,
    /// Playhead in seconds, stored as f64 bits so a fractional playback
    /// position is not truncated every audio callback.
    position: AtomicU64,
    ended: AtomicUsize,
    sample_rate: f64,
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
            mixer: Mutex::new(Mixer {
                backing_vol,
                vocals_vol,
                ..Default::default()
            }),
            position: AtomicU64::new(0.0f64.to_bits()),
            ended: AtomicUsize::new(0),
            sample_rate: sample_rate as f64,
        });
        let cb_shared = shared.clone();
        let err_fn = |err| eprintln!("audio output error: {err}");
        let stream = match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| write_f32(data, channels, &cb_shared),
                err_fn,
                None,
            )?,
            SampleFormat::I16 => {
                let cb_shared = shared.clone();
                device.build_output_stream(
                    &config,
                    move |data: &mut [i16], _| write_i16(data, channels, &cb_shared),
                    err_fn,
                    None,
                )?
            }
            SampleFormat::U16 => {
                let cb_shared = shared.clone();
                device.build_output_stream(
                    &config,
                    move |data: &mut [u16], _| write_u16(data, channels, &cb_shared),
                    err_fn,
                    None,
                )?
            }
            other => return Err(anyhow!("unsupported output sample format: {other:?}")),
        };
        stream.play()?;
        Ok(Self {
            _stream: stream,
            shared,
            sample_rate,
        })
    }

    pub fn load(&self, backing_path: &Path, vocals_path: Option<&Path>) -> Result<f64> {
        let backing = decode_audio(backing_path, self.sample_rate as usize)?;
        let vocals = if let Some(path) = vocals_path {
            decode_audio(path, self.sample_rate as usize).unwrap_or_default()
        } else {
            Vec::new()
        };
        let frames = backing.len() / 2;
        let mut mixer = self.shared.mixer.lock().unwrap();
        mixer.backing = backing;
        mixer.vocals = if vocals.len() == frames * 2 { vocals } else { Vec::new() };
        mixer.frames = frames;
        mixer.pos = 0.0;
        mixer.playing = false;
        self.set_position(0.0);
        self.shared.ended.store(0, Ordering::Relaxed);
        Ok(frames as f64 / self.sample_rate as f64)
    }

    pub fn play(&self) {
        let mut mixer = self.shared.mixer.lock().unwrap();
        if mixer.frames > 0 {
            if mixer.pos >= mixer.frames.saturating_sub(2) as f64 {
                mixer.pos = 0.0;
            }
            mixer.playing = true;
        }
        self.shared.ended.store(0, Ordering::Relaxed);
    }

    pub fn pause(&self) {
        self.shared.mixer.lock().unwrap().playing = false;
    }

    pub fn stop(&self) {
        let mut mixer = self.shared.mixer.lock().unwrap();
        mixer.playing = false;
        mixer.pos = 0.0;
        self.set_position(0.0);
    }

    pub fn seek(&self, seconds: f64) {
        let mut mixer = self.shared.mixer.lock().unwrap();
        mixer.pos = (seconds.max(0.0) * self.sample_rate as f64).min(mixer.frames as f64);
        self.set_position(mixer.pos / self.sample_rate as f64);
    }

    pub fn position(&self) -> f64 {
        f64::from_bits(self.shared.position.load(Ordering::Relaxed))
    }

    fn set_position(&self, seconds: f64) {
        self.shared.position.store(seconds.to_bits(), Ordering::Relaxed);
    }

    pub fn set_rate(&self, rate: f32) {
        let mut mixer = self.shared.mixer.lock().unwrap();
        mixer.rate = rate.clamp(0.25, 2.0);
    }

    pub fn set_volumes(&self, backing: f32, vocals: f32) {
        let mut mixer = self.shared.mixer.lock().unwrap();
        mixer.backing_vol = backing.clamp(0.0, 1.5);
        mixer.vocals_vol = vocals.clamp(0.0, 1.5);
    }

}

fn sample_linear(data: &[f32], frame: usize, channel: usize, frames: usize, frac: f32) -> f32 {
    if frame >= frames {
        return 0.0;
    }
    let a = data.get(frame * 2 + channel).copied().unwrap_or(0.0);
    let next = (frame + 1).min(frames.saturating_sub(1));
    let b = data.get(next * 2 + channel).copied().unwrap_or(a);
    a + (b - a) * frac
}

fn next_frame(mixer: &mut Mixer) -> (f32, f32) {
    if !mixer.playing || mixer.pos >= mixer.frames as f64 {
        return (0.0, 0.0);
    }
    let frame = mixer.pos.floor() as usize;
    let frac = (mixer.pos - frame as f64) as f32;
    let l = sample_linear(&mixer.backing, frame, 0, mixer.frames, frac) * mixer.backing_vol
        + sample_linear(&mixer.vocals, frame, 0, mixer.frames, frac) * mixer.vocals_vol;
    let r = sample_linear(&mixer.backing, frame, 1, mixer.frames, frac) * mixer.backing_vol
        + sample_linear(&mixer.vocals, frame, 1, mixer.frames, frac) * mixer.vocals_vol;
    mixer.pos += mixer.rate.max(0.05) as f64;
    (l, r)
}

fn write_f32(data: &mut [f32], channels: usize, shared: &Arc<Shared>) {
    let mut mixer = shared.mixer.lock().unwrap();
    for frame in data.chunks_mut(channels) {
        let (l, r) = next_frame(&mut mixer);
        if channels == 1 {
            frame[0] = (l + r) * 0.5;
        } else {
            frame[0] = l;
            frame[1] = r;
            for sample in frame.iter_mut().skip(2) {
                *sample = 0.0;
            }
        }
    }
    shared.position.store((mixer.pos / shared.sample_rate).to_bits(), Ordering::Relaxed);
    if mixer.playing && mixer.pos >= mixer.frames as f64 {
        mixer.playing = false;
    }
}

fn write_i16(data: &mut [i16], channels: usize, shared: &Arc<Shared>) {
    let mut mixer = shared.mixer.lock().unwrap();
    for frame in data.chunks_mut(channels) {
        let (l, r) = next_frame(&mut mixer);
        if channels == 1 {
            frame[0] = (((l + r) * 0.5).clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        } else {
            frame[0] = (l.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            frame[1] = (r.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            for sample in frame.iter_mut().skip(2) {
                *sample = 0;
            }
        }
    }
    shared.position.store((mixer.pos / shared.sample_rate).to_bits(), Ordering::Relaxed);
    if mixer.playing && mixer.pos >= mixer.frames as f64 {
        mixer.playing = false;
    }
}

fn write_u16(data: &mut [u16], channels: usize, shared: &Arc<Shared>) {
    let mut mixer = shared.mixer.lock().unwrap();
    for frame in data.chunks_mut(channels) {
        let (l, r) = next_frame(&mut mixer);
        let to_u16 = |value: f32| -> u16 {
            ((value.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16
        };
        if channels == 1 {
            frame[0] = to_u16((l + r) * 0.5);
        } else {
            frame[0] = to_u16(l);
            frame[1] = to_u16(r);
            for sample in frame.iter_mut().skip(2) {
                *sample = u16::MAX / 2;
            }
        }
    }
    shared.position.store((mixer.pos / shared.sample_rate).to_bits(), Ordering::Relaxed);
    if mixer.playing && mixer.pos >= mixer.frames as f64 {
        mixer.playing = false;
    }
}

fn decode_audio(path: &Path, out_rate: usize) -> Result<Vec<f32>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
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
    loop {
        match format.next_packet() {
            Ok(packet) => {
                if packet.track_id() != track_id {
                    continue;
                }
                match decoder.decode(&packet) {
                    Ok(decoded) => {
                        let spec = *decoded.spec();
                        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                        buffer.copy_interleaved_ref(decoded);
                        interleaved.extend_from_slice(buffer.samples());
                    }
                    Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                    Err(_) => break,
                }
            }
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
        let right0 = if in_channels > 1 { interleaved[i0 * in_channels + 1] } else { left0 };
        let right1 = if in_channels > 1 { interleaved[i1 * in_channels + 1] } else { left1 };
        out[frame * 2] = left0 + (left1 - left0) * t;
        out[frame * 2 + 1] = right0 + (right1 - right0) * t;
    }
    Ok(out)
}

pub struct Microphone {
    _stream: cpal::Stream,
    ring: Arc<Mutex<VecDeque<f32>>>,
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
                for device in devices {
                    if device.name().map(|name| name == wanted).unwrap_or(false) {
                        chosen = Some(device);
                        break;
                    }
                }
            }
        }
        let device = chosen
            .or_else(|| host.default_input_device())
            .ok_or_else(|| anyhow!("no input microphone found"))?;
        let name = device.name().unwrap_or_else(|_| "microphone".into());
        let supported = device.default_input_config().context("microphone configuration")?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let sample_rate = config.sample_rate.0;
        let channels = config.channels as usize;
        let ring = Arc::new(Mutex::new(VecDeque::with_capacity(16384)));
        let cb_ring = ring.clone();
        let err_fn = |err| eprintln!("microphone error: {err}");
        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _| push_input(data, channels, &cb_ring),
                err_fn,
                None,
            )?,
            SampleFormat::I16 => {
                let cb_ring = ring.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _| {
                        let mono: Vec<f32> = data
                            .chunks(channels)
                            .map(|frame| frame.iter().map(|&s| s as f32 / i16::MAX as f32).sum::<f32>() / channels as f32)
                            .collect();
                        push_mono(&mono, &cb_ring);
                    },
                    err_fn,
                    None,
                )?
            }
            SampleFormat::U16 => {
                let cb_ring = ring.clone();
                device.build_input_stream(
                    &config,
                    move |data: &[u16], _| {
                        let mono: Vec<f32> = data
                            .chunks(channels)
                            .map(|frame| {
                                frame
                                    .iter()
                                    .map(|&s| (s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                                    .sum::<f32>()
                                    / channels as f32
                            })
                            .collect();
                        push_mono(&mono, &cb_ring);
                    },
                    err_fn,
                    None,
                )?
            }
            other => return Err(anyhow!("unsupported microphone format: {other:?}")),
        };
        stream.play()?;
        Ok(Self {
            _stream: stream,
            ring,
            sample_rate,
            device_name: name,
        })
    }

    pub fn latest(&self, count: usize) -> Vec<f32> {
        let ring = self.ring.lock().unwrap();
        let len = ring.len();
        let start = len.saturating_sub(count);
        ring.iter().skip(start).copied().collect()
    }

}

fn push_input(data: &[f32], channels: usize, ring: &Arc<Mutex<VecDeque<f32>>>) {
    let mono: Vec<f32> = data
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels.max(1) as f32)
        .collect();
    push_mono(&mono, ring);
}

fn push_mono(samples: &[f32], ring: &Arc<Mutex<VecDeque<f32>>>) {
    let mut ring = ring.lock().unwrap();
    for &sample in samples {
        if ring.len() >= 16384 {
            ring.pop_front();
        }
        ring.push_back(sample);
    }
}
