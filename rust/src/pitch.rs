#[derive(Clone, Copy, Debug)]
pub struct PitchReading {
    pub midi: f32,
    pub cents: f32,
    pub clarity: f32,
    pub rms: f32,
}

pub fn detect_pitch(samples: &[f32], sample_rate: f32) -> Option<PitchReading> {
    let n = samples.len();
    if n < 512 {
        return None;
    }
    let mean = samples.iter().sum::<f32>() / n as f32;
    let mut rms = 0.0;
    for sample in samples {
        let value = sample - mean;
        rms += value * value;
    }
    let rms = (rms / n as f32).sqrt();
    if rms < 0.0015 {
        return None;
    }

    let min_lag = (sample_rate / 1200.0).floor().max(2.0) as usize;
    let max_lag = (sample_rate / 65.0).floor().min((n / 2) as f32) as usize;
    if max_lag <= min_lag + 2 {
        return None;
    }
    let window = n - max_lag;
    if window < 256 {
        return None;
    }

    let mut diff = vec![0.0f32; max_lag + 1];
    for tau in 1..=max_lag {
        let mut sum = 0.0f32;
        for i in 0..window {
            let d = samples[i] - samples[i + tau];
            sum += d * d;
        }
        diff[tau] = sum;
    }
    let mut cmnd = vec![1.0f32; max_lag + 1];
    let mut running = 0.0f32;
    for tau in 1..=max_lag {
        running += diff[tau];
        cmnd[tau] = if running <= 1e-12 {
            1.0
        } else {
            diff[tau] * tau as f32 / running
        };
    }

    let threshold = 0.24;
    let mut estimate = 0usize;
    let mut found = false;
    for tau in min_lag..=max_lag {
        if cmnd[tau] < threshold {
            let mut t = tau;
            while t < max_lag && cmnd[t + 1] < cmnd[t] {
                t += 1;
            }
            estimate = t;
            found = true;
            break;
        }
    }
    if !found {
        let mut best = f32::INFINITY;
        for (tau, &value) in cmnd.iter().enumerate().take(max_lag + 1).skip(min_lag) {
            if value < best {
                best = value;
                estimate = tau;
            }
        }
        if best > 0.58 {
            return None;
        }
    }

    // avoid choosing a multiple of the period
    for divisor in [2usize, 3] {
        let candidate = (estimate + divisor / 2) / divisor;
        if candidate >= min_lag && candidate < estimate && cmnd[candidate] < 0.36 {
            let mut local = candidate;
            while local + 1 < estimate && cmnd[local + 1] < cmnd[local] {
                local += 1;
            }
            if cmnd[local] < cmnd[estimate] + 0.08 {
                estimate = local;
                break;
            }
        }
    }

    let mut tau = estimate as f32;
    if estimate > min_lag && estimate < max_lag {
        let s0 = cmnd[estimate - 1];
        let s1 = cmnd[estimate];
        let s2 = cmnd[estimate + 1];
        let denom = 2.0 * (2.0 * s1 - s2 - s0);
        if denom.abs() > 1e-9 {
            tau += (s2 - s0) / denom;
        }
    }
    let hz = sample_rate / tau;
    if !hz.is_finite() || hz < 60.0 || hz > 1300.0 {
        return None;
    }
    let clarity = (1.0 - cmnd[estimate]).clamp(0.0, 1.0);
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let rounded = midi.round();
    Some(PitchReading {
        midi,
        cents: (midi - rounded) * 100.0,
        clarity,
        rms,
    })
}

#[derive(Default)]
pub struct PitchTracker {
    history: Vec<PitchReading>,
}

impl PitchTracker {
    pub fn reset(&mut self) {
        self.history.clear();
    }

    pub fn push(&mut self, reading: Option<PitchReading>) -> Option<PitchReading> {
        let reading = reading?;
        self.history.push(reading);
        if self.history.len() > 6 {
            self.history.remove(0);
        }
        let mut midis: Vec<f32> = self.history.iter().map(|r| r.midi).collect();
        midis.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = midis[midis.len() / 2];
        Some(PitchReading {
            midi: median,
            cents: (median - median.round()) * 100.0,
            clarity: reading.clarity,
            rms: reading.rms,
        })
    }
}
