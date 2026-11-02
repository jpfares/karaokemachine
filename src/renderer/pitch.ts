export interface PitchReading {
  hz: number;
  midi: number;
  cents: number;
  clarity: number;
  rms: number;
  at: number;
}

/**
 * YIN pitch detector (de Cheveigné & Kawahara).
 *
 * Tuned for a single human voice: 65 Hz – 1.2 kHz, with a cumulative-mean
 * normalized difference function and parabolic interpolation.
 */
export function detectPitch(
  input: Float32Array,
  sampleRate: number,
  options: { minHz?: number; maxHz?: number; threshold?: number; at?: number } = {}
): PitchReading | null {
  const minHz = options.minHz ?? 65;
  const maxHz = options.maxHz ?? 1200;
  const threshold = options.threshold ?? 0.24;
  const at = options.at ?? 0;

  const n = input.length;
  if (n < 256) return null;

  // DC removal + RMS gate in one pass
  let mean = 0;
  for (let i = 0; i < n; i++) mean += input[i]!;
  mean /= n;
  let energy = 0;
  for (let i = 0; i < n; i++) {
    const v = input[i]! - mean;
    energy += v * v;
  }
  const rms = Math.sqrt(energy / n);
  if (rms < 0.0015) return null;

  const minLag = Math.max(2, Math.floor(sampleRate / maxHz));
  const maxLag = Math.min(Math.floor(sampleRate / minHz), Math.floor(n / 2));
  if (maxLag <= minLag + 2) return null;

  const window = n - maxLag;
  if (window < 256) return null;

  // Difference function
  const diff = new Float32Array(maxLag + 1);
  for (let tau = 1; tau <= maxLag; tau++) {
    let sum = 0;
    for (let i = 0; i < window; i++) {
      const delta = input[i]! - input[i + tau]!;
      sum += delta * delta;
    }
    diff[tau] = sum;
  }

  // Cumulative mean normalized difference
  const cmnd = new Float32Array(maxLag + 1);
  cmnd[0] = 1;
  let running = 0;
  for (let tau = 1; tau <= maxLag; tau++) {
    running += diff[tau]!;
    cmnd[tau] = running === 0 ? 1 : (diff[tau]! * tau) / running;
  }

  // First dip below the threshold; else global minimum
  let tauEstimate = -1;
  for (let tau = minLag; tau <= maxLag; tau++) {
    if (cmnd[tau]! < threshold) {
      // walk to the local minimum of this dip
      let t = tau;
      while (t + 1 <= maxLag && cmnd[t + 1]! < cmnd[t]!) t++;
      tauEstimate = t;
      break;
    }
  }
  if (tauEstimate < 0) {
    let best = Infinity;
    for (let tau = minLag; tau <= maxLag; tau++) {
      if (cmnd[tau]! < best) {
        best = cmnd[tau]!;
        tauEstimate = tau;
      }
    }
    if (best > 0.58) return null;
  }

  // Guard against choosing a multiple of the period: if a strong earlier
  // dip exists at a harmonic (tau / 2, tau / 3), prefer it.
  for (const divisor of [2, 3]) {
    const t = Math.round(tauEstimate / divisor);
    if (t >= minLag && t < tauEstimate && cmnd[t]! < 0.36) {
      let local = t;
      while (local + 1 < tauEstimate && cmnd[local + 1]! < cmnd[local]!) local++;
      if (cmnd[local]! < cmnd[tauEstimate]! + 0.08) {
        tauEstimate = local;
        break;
      }
    }
  }

  // Parabolic interpolation around the chosen lag
  let betterTau = tauEstimate;
  if (tauEstimate > minLag && tauEstimate < maxLag) {
    const s0 = cmnd[tauEstimate - 1]!;
    const s1 = cmnd[tauEstimate]!;
    const s2 = cmnd[tauEstimate + 1]!;
    const denom = 2 * (2 * s1 - s2 - s0);
    if (Math.abs(denom) > 1e-9) {
      betterTau = tauEstimate + (s2 - s0) / denom;
    }
  }

  const hz = sampleRate / betterTau;
  if (!Number.isFinite(hz) || hz < minHz * 0.92 || hz > maxHz * 1.08) return null;

  const clarity = Math.max(0, Math.min(1, 1 - cmnd[tauEstimate]!));
  const midi = 69 + 12 * Math.log2(hz / 440);
  const rounded = Math.round(midi);
  const cents = (midi - rounded) * 100;

  return { hz, midi, cents, clarity, rms, at };
}

/** Median-of-5 log-frequency smoother with a short hold when unvoiced. */
export class PitchTracker {
  private history: { midi: number; hz: number; at: number }[] = [];
  private last: PitchReading | null = null;

  reset() {
    this.history = [];
    this.last = null;
  }

  push(reading: PitchReading | null, now: number): PitchReading | null {
    if (reading) {
      this.history.push({ midi: reading.midi, hz: reading.hz, at: now });
      if (this.history.length > 6) this.history.shift();
      const sorted = [...this.history].sort((a, b) => a.midi - b.midi);
      const median = sorted[Math.floor(sorted.length / 2)]!;
      const hz = 440 * Math.pow(2, (median.midi - 69) / 12);
      const midi = median.midi;
      const rounded = Math.round(midi);
      const smoothed: PitchReading = {
        ...reading,
        hz,
        midi,
        cents: (midi - rounded) * 100,
        at: now
      };
      this.last = smoothed;
      return smoothed;
    }
    // hold a note for 120 ms of missed frames so the dot doesn't flicker
    if (this.last && now - this.last.at < 0.12) return this.last;
    this.history = [];
    this.last = null;
    return null;
  }
}

export function midiToHz(midi: number): number {
  return 440 * Math.pow(2, (midi - 69) / 12);
}

export function hzToMidi(hz: number): number {
  return 69 + 12 * Math.log2(hz / 440);
}

export function centsBetween(a: number, b: number): number {
  return (a - b) * 100;
}
