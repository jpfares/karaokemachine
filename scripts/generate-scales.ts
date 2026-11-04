/**
 * Generates the built-in scale / warm-up songs:
 *   - a backing track (chords, bass, drums, keys)
 *   - an optional guide-vocal track (the melody, muted by default)
 *   - a note chart with solfège lyrics
 *
 * Everything is synthesized, so the app has ready-to-sing material with no
 * network, downloads or model weights required.
 */
import fs from 'node:fs/promises';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import type { LyricLine, LyricWord, MelodyNote, Song } from '../src/shared/types';
import { fallbackCoverFor } from '../src/main/metadata';

const ROOT = process.cwd();
const OUT = path.join(ROOT, 'assets/songs');
const SR = 44100;

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
const SOLFEGE = ['Do', 'Di', 'Re', 'Ri', 'Mi', 'Fa', 'Fi', 'Sol', 'Si', 'La', 'Li', 'Ti'];
const SOLFEGE_FLAT = ['Do', 'Ra', 'Re', 'Me', 'Mi', 'Fa', 'Se', 'Sol', 'Le', 'La', 'Te', 'Ti'];

function midiToName(midi: number): string {
  return `${NOTE_NAMES[((midi % 12) + 12) % 12]}${Math.floor(midi / 12) - 1}`;
}

function solfege(midi: number, flat = false): string {
  return (flat ? SOLFEGE_FLAT : SOLFEGE)[((midi % 12) + 12) % 12]!;
}

interface ScaleSpec {
  id: string;
  title: string;
  tags: string[];
  bpm: number;
  order: number;
  beatsPerNote: number;
  chords: number[][];
  phrases: { midis: number[]; flat?: boolean }[];
  cycle: number;
  swing?: boolean;
}

const CH = {
  Cmaj7: [48, 55, 59, 64, 67],
  Am7: [45, 52, 55, 60, 64],
  Fmaj7: [41, 48, 52, 57, 60],
  G6: [43, 50, 55, 59, 64],
  G7: [43, 50, 53, 59, 65],
  C7: [48, 55, 58, 64, 70],
  F7: [41, 48, 51, 57, 63],
  Dm7: [38, 45, 48, 53, 57],
  C: [48, 55, 60, 64, 67],
  Am: [45, 52, 57, 60, 64],
  F: [41, 48, 53, 57, 60],
  G: [43, 50, 55, 59, 62],
  Ab: [44, 51, 56, 60, 63]
};

const SCALES: ScaleSpec[] = [
  {
    id: 'scale-major',
    title: 'Major Scale',
    tags: ['Warm-up', 'C major', 'Do–Re–Mi'],
    bpm: 96,
    order: 1,
    beatsPerNote: 1,
    chords: [CH.Cmaj7, CH.Am7, CH.Fmaj7, CH.G6],
    phrases: [
      { midis: [60, 62, 64, 65, 67, 69, 71, 72] },
      { midis: [72, 71, 69, 67, 65, 64, 62, 60] },
      { midis: [60, 62, 64, 65, 67, 69, 71, 72] },
      { midis: [72, 71, 69, 67, 65, 64, 62, 60] }
    ],
    cycle: 2
  },
  {
    id: 'scale-pentatonic-major',
    title: 'Major Pentatonic',
    tags: ['Scale', 'C major', 'Five notes'],
    bpm: 100,
    order: 2,
    beatsPerNote: 0.9,
    chords: [CH.C, CH.G, CH.Am, CH.F],
    phrases: [
      { midis: [60, 62, 64, 67, 69, 72] },
      { midis: [72, 69, 67, 64, 62, 60] },
      { midis: [60, 62, 64, 67, 69, 72] },
      { midis: [72, 69, 67, 64, 62, 60] }
    ],
    cycle: 2
  },
  {
    id: 'scale-pentatonic-minor',
    title: 'Minor Pentatonic',
    tags: ['Scale', 'A minor', 'Bluesy'],
    bpm: 100,
    order: 3,
    beatsPerNote: 0.9,
    chords: [CH.Am7, CH.Fmaj7, CH.Cmaj7, CH.G6],
    phrases: [
      { midis: [57, 60, 62, 64, 67, 69] },
      { midis: [69, 67, 64, 62, 60, 57] },
      { midis: [57, 60, 62, 64, 67, 69] },
      { midis: [69, 67, 64, 62, 60, 57] }
    ],
    cycle: 2
  },
  {
    id: 'scale-blues',
    title: 'Blues Scale',
    tags: ['Scale', 'C blues', 'Blue notes'],
    bpm: 88,
    order: 4,
    beatsPerNote: 0.9,
    chords: [CH.C7, CH.F7, CH.C7, CH.G7],
    phrases: [
      { midis: [60, 63, 65, 66, 67, 70, 72], flat: true },
      { midis: [72, 70, 67, 66, 65, 63, 60], flat: true },
      { midis: [60, 63, 65, 66, 67, 70, 72], flat: true },
      { midis: [72, 70, 67, 66, 65, 63, 60], flat: true }
    ],
    cycle: 2,
    swing: true
  },
  {
    id: 'scale-chromatic',
    title: 'Chromatic Climb',
    tags: ['Warm-up', 'Chromatic', 'Agility'],
    bpm: 78,
    order: 5,
    beatsPerNote: 0.55,
    chords: [CH.Cmaj7, CH.Am7, CH.Dm7, CH.G7],
    phrases: [
      { midis: Array.from({ length: 13 }, (_, i) => 60 + i) },
      { midis: Array.from({ length: 13 }, (_, i) => 72 - i), flat: true },
      { midis: Array.from({ length: 13 }, (_, i) => 60 + i) },
      { midis: Array.from({ length: 13 }, (_, i) => 72 - i), flat: true }
    ],
    cycle: 3
  },
  {
    id: 'scale-arpeggio',
    title: 'Maj7 Arpeggio',
    tags: ['Leaps', 'C major 7', 'Intervals'],
    bpm: 92,
    order: 6,
    beatsPerNote: 0.8,
    chords: [CH.Cmaj7, CH.Am7, CH.Dm7, CH.G7],
    phrases: [
      { midis: [60, 64, 67, 71, 72, 71, 67, 64] },
      { midis: [60, 67, 64, 71, 72, 67, 64, 60] },
      { midis: [60, 64, 67, 71, 72, 71, 67, 64] },
      { midis: [60, 67, 64, 71, 72, 67, 64, 60] }
    ],
    cycle: 2
  }
];

interface Built {
  notes: MelodyNote[];
  lines: LyricLine[];
  totalBeats: number;
  beat: number;
}

function buildChart(spec: ScaleSpec): Built {
  const beat = 60 / spec.bpm;
  const introBeats = 4;
  const outroBeats = 4;
  let cursor = introBeats;
  const notes: MelodyNote[] = [];
  const lines: LyricLine[] = [];

  for (let repeat = 0; repeat < spec.cycle; repeat++) {
    spec.phrases.forEach((phrase, pi) => {
      const lineIndex = lines.length;
      const lineStart = cursor * beat;
      const words: LyricWord[] = [];
      const syllables: string[] = [];
      for (const midi of phrase.midis) {
        const start = cursor * beat;
        const dur = spec.beatsPerNote * beat * 0.88;
        const syl = solfege(midi, phrase.flat);
        notes.push({
          start: Number(start.toFixed(4)),
          dur: Number(dur.toFixed(4)),
          midi,
          word: syl,
          line: lineIndex,
          vel: 0.8
        });
        words.push({
          text: syl,
          start: Number(start.toFixed(4)),
          end: Number((start + dur).toFixed(4))
        });
        syllables.push(syl);
        cursor += spec.beatsPerNote;
      }
      lines.push({
        start: Number(lineStart.toFixed(4)),
        end: Number((cursor * beat + beat * 0.15).toFixed(4)),
        text: syllables.join(' '),
        words
      });
      // a small breath at the end of each line, not enough to sound like a gap
      cursor += 0.3;
      void pi;
    });
  }

  const totalBeats = Math.ceil(cursor / 4) * 4 + outroBeats;
  return { notes, lines, totalBeats, beat };
}

// ---------------------------------------------------------------------------
// synthesis

function seeded(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 0xffffffff;
  };
}

class Mix {
  left: Float32Array;
  right: Float32Array;
  constructor(public seconds: number) {
    const n = Math.ceil(seconds * SR);
    this.left = new Float32Array(n);
    this.right = new Float32Array(n);
  }

  add(
    startSec: number,
    durSec: number,
    freq: number,
    gain: number,
    opts: {
      pan?: number;
      attack?: number;
      release?: number;
      decay?: number;
      kind?: 'pad' | 'pluck' | 'bass' | 'bell';
      detune?: number;
      vibrato?: number;
    } = {}
  ) {
    const start = Math.floor(startSec * SR);
    const count = Math.floor(durSec * SR);
    const pan = opts.pan ?? 0;
    const attack = opts.attack ?? 0.01;
    const release = opts.release ?? 0.15;
    const decay = opts.decay ?? 0;
    const kind = opts.kind ?? 'pluck';
    const detune = opts.detune ?? 0;
    const vib = opts.vibrato ?? 0;
    const lg = Math.sqrt(0.5 * (1 - pan));
    const rg = Math.sqrt(0.5 * (1 + pan));
    for (let i = 0; i < count; i++) {
      const idx = start + i;
      if (idx < 0 || idx >= this.left.length) continue;
      const t = i / SR;
      const attackEnv = attack <= 0 ? 1 : Math.min(1, t / attack);
      const releaseEnv = release <= 0 ? 1 : Math.min(1, (durSec - t) / release);
      let env = attackEnv * releaseEnv;
      if (decay > 0) env *= Math.exp(-t * decay);
      if (env <= 0) continue;
      const vibPhase = vib ? Math.sin(2 * Math.PI * 5.2 * t) * vib : 0;
      const f = freq * Math.pow(2, (vibPhase + detune) / 1200);
      const phase = 2 * Math.PI * f * t;
      let s: number;
      switch (kind) {
        case 'pad':
          s = Math.sin(phase) * 0.5 + Math.sin(phase * 2.002) * 0.16 + Math.sin(phase * 3.001) * 0.06;
          break;
        case 'bass':
          s = Math.sin(phase) * 0.72 + Math.sin(phase * 2) * 0.14;
          break;
        case 'bell':
          s = Math.sin(phase) * 0.55 + Math.sin(phase * 2.01) * 0.2 + Math.sin(phase * 3.99) * 0.08;
          break;
        default:
          s = Math.sin(phase) * 0.55 + Math.sin(phase * 2.01) * 0.22 + Math.sin(phase * 3.02) * 0.07;
      }
      this.left[idx]! += s * env * gain * lg;
      this.right[idx]! += s * env * gain * rg;
    }
  }

  noise(startSec: number, durSec: number, gain: number, opts: { pan?: number; decay?: number; hp?: number } = {}) {
    const start = Math.floor(startSec * SR);
    const count = Math.floor(durSec * SR);
    const rand = seeded(Math.floor(startSec * 9973) + 13);
    const pan = opts.pan ?? 0;
    const decay = opts.decay ?? 18;
    const hp = opts.hp ?? 0.65;
    const lg = Math.sqrt(0.5 * (1 - pan));
    const rg = Math.sqrt(0.5 * (1 + pan));
    let last = 0;
    for (let i = 0; i < count; i++) {
      const idx = start + i;
      if (idx < 0 || idx >= this.left.length) continue;
      const t = i / SR;
      const white = rand() * 2 - 1;
      const filtered = white - last * hp;
      last = white;
      const env = Math.exp(-t * decay) * Math.min(1, t / 0.002);
      const s = filtered * env * gain;
      this.left[idx]! += s * lg;
      this.right[idx]! += s * rg;
    }
  }

  finish(): { left: Float32Array; right: Float32Array } {
    const n = this.left.length;
    let peak = 0;
    for (let i = 0; i < n; i++) {
      peak = Math.max(peak, Math.abs(this.left[i]!), Math.abs(this.right[i]!));
    }
    const scale = peak > 0.9 ? 0.9 / peak : 1;
    for (let i = 0; i < n; i++) {
      const fade = Math.min(1, i / (SR * 0.03), (n - i) / (SR * 0.08));
      this.left[i] = this.left[i]! * scale * fade;
      this.right[i] = this.right[i]! * scale * fade;
    }
    return { left: this.left, right: this.right };
  }
}

function chordAdd(mix: Mix, atSec: number, durSec: number, notes: number[], gain: number) {
  notes.forEach((midi, i) => {
    const freq = 440 * Math.pow(2, (midi - 69) / 12);
    const pan = (i / Math.max(1, notes.length - 1)) * 0.5 - 0.25;
    mix.add(atSec, durSec, freq, gain, {
      pan,
      attack: 0.35,
      release: 0.5,
      kind: 'pad',
      detune: i % 2 === 0 ? -4 : 4
    });
  });
}

function renderBacking(spec: ScaleSpec, built: Built): Mix {
  const { totalBeats, beat } = built;
  const seconds = totalBeats * beat + 2.4;
  const mix = new Mix(seconds);
  const rand = seeded(42);

  const bars = Math.ceil(totalBeats / 4);
  for (let bar = 0; bar < bars; bar++) {
    const at = bar * 4 * beat;
    const chord = spec.chords[bar % spec.chords.length]!;
    const outro = bar >= bars - 1;
    const intro = bar === 0;

    // chord pad + higher arpeggio
    chordAdd(mix, at, Math.min(4 * beat + 0.8, seconds - at), chord.slice(1), outro ? 0.16 : 0.1);
    if (!outro) {
      for (let eighth = 0; eighth < 8; eighth++) {
        const noteIdx = (eighth + bar) % chord.length;
        const midi = chord[noteIdx]! + 12;
        const freq = 440 * Math.pow(2, (midi - 69) / 12);
        mix.add(at + eighth * beat * 0.5, beat * 0.42, freq, intro ? 0.022 : 0.035, {
          pan: eighth % 2 ? 0.35 : -0.35,
          kind: 'pluck',
          decay: 5.5,
          release: 0.08
        });
      }
    }

    // bass
    if (!outro) {
      const root = chord[0]!;
      for (const b of intro ? [0] : [0, 2.5]) {
        const freq = 440 * Math.pow(2, (root - 69) / 12);
        mix.add(at + b * beat, beat * 0.8, freq, 0.16, {
          kind: 'bass',
          attack: 0.012,
          release: 0.12,
          decay: 1.8
        });
      }
    }

    // drums
    if (!intro && !outro) {
      for (let b = 0; b < 4; b++) {
        if (b === 0 || b === 2) {
          mix.noise(at + b * beat, 0.16, 0.14, { decay: 30, hp: 0.92 });
          mix.add(at + b * beat, 0.16, 52, 0.16, { kind: 'bass', attack: 0.002, release: 0.1, decay: 22 });
        } else {
          mix.noise(at + b * beat, 0.12, 0.06, { decay: 38, hp: 0.75 });
        }
        for (let half = 0; half < 2; half++) {
          mix.noise(at + b * beat + half * beat * 0.5, 0.045, half === 0 ? 0.023 : 0.032, {
            decay: 70,
            hp: 0.6,
            pan: 0.25
          });
        }
      }
    }

    // sparkle on the last chord
    if (outro) {
      for (let i = 0; i < chord.length; i++) {
        const midi = chord[i]! + 12;
        mix.add(at + i * 0.045, 2.0, 440 * Math.pow(2, (midi - 69) / 12), 0.06, {
          kind: 'bell',
          pan: (rand() * 2 - 1) * 0.4,
          attack: 0.005,
          release: 1.8,
          decay: 1.2
        });
      }
    }
  }

  return mix;
}

// The guide is rendered to the backing's exact length: the app pairs the two
// stems frame for frame, and a shorter guide used to be dropped entirely.
function renderGuide(built: Built, seconds: number): Mix {
  const { notes, beat } = built;
  const mix = new Mix(seconds);
  notes.forEach((note, i) => {
    const freq = 440 * Math.pow(2, (note.midi - 69) / 12);
    mix.add(note.start, note.dur * 1.02, freq, 0.24, {
      kind: 'bell',
      pan: (i % 3) * 0.2 - 0.2,
      attack: 0.012,
      release: 0.12,
      decay: 1.4,
      vibrato: 4
    });
  });
  return mix;
}

function wav(channels: Float32Array[]): Buffer {
  const frames = channels[0]!.length;
  const numCh = channels.length;
  const dataSize = frames * numCh * 2;
  const buf = Buffer.alloc(44 + dataSize);
  buf.write('RIFF', 0);
  buf.writeUInt32LE(36 + dataSize, 4);
  buf.write('WAVE', 8);
  buf.write('fmt ', 12);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20);
  buf.writeUInt16LE(numCh, 22);
  buf.writeUInt32LE(SR, 24);
  buf.writeUInt32LE(SR * numCh * 2, 28);
  buf.writeUInt16LE(numCh * 2, 32);
  buf.writeUInt16LE(16, 34);
  buf.write('data', 36);
  buf.writeUInt32LE(dataSize, 40);
  let off = 44;
  for (let i = 0; i < frames; i++) {
    for (let c = 0; c < numCh; c++) {
      const sample = Math.max(-1, Math.min(1, channels[c]![i]!));
      buf.writeInt16LE(Math.round(sample * 32767), off);
      off += 2;
    }
  }
  return buf;
}

function encodeOpus(wavPath: string, oggPath: string, bitrate: string) {
  const result = spawnSync(
    'ffmpeg',
    ['-y', '-v', 'error', '-i', wavPath, '-ar', '48000', '-ac', '2', '-c:a', 'libopus', '-b:a', bitrate, oggPath],
    { stdio: 'inherit' }
  );
  if (result.status !== 0) throw new Error(`ffmpeg could not encode ${oggPath}`);
}

async function main() {
  await fs.mkdir(OUT, { recursive: true });
  const createdAt = new Date().toISOString();

  for (const spec of SCALES) {
    const built = buildChart(spec);
    const dir = path.join(OUT, spec.id);
    await fs.mkdir(dir, { recursive: true });

    const backingMix = renderBacking(spec, built);
    const backing = backingMix.finish();
    await fs.writeFile(path.join(dir, 'backing.wav'), wav([backing.left, backing.right]));

    const guide = renderGuide(built, backingMix.seconds).finish();
    await fs.writeFile(path.join(dir, 'guide.wav'), wav([guide.left, guide.right]));

    encodeOpus(path.join(dir, 'backing.wav'), path.join(dir, 'backing.ogg'), '160k');
    encodeOpus(path.join(dir, 'guide.wav'), path.join(dir, 'guide.ogg'), '128k');
    await fs.rm(path.join(dir, 'backing.wav'), { force: true });
    await fs.rm(path.join(dir, 'guide.wav'), { force: true });

    const duration = built.totalBeats * built.beat + 2.4;
    const low = Math.min(...built.notes.map((n) => n.midi)) - 2;
    const high = Math.max(...built.notes.map((n) => n.midi)) + 2;
    await fs.mkdir(path.join(dir, 'maps'), { recursive: true });
    await fs.writeFile(
      path.join(dir, 'maps', 'Level.json'),
      JSON.stringify(
        {
          format: 'karaokemachine-level',
          version: 1,
          draft: false,
          notes: built.notes,
          lines: built.lines,
          placedWords: [],
          customWords: [],
          ribbonLinks: [],
          low,
          high
        },
        null,
        2
      )
    );
    const song: Song = {
      id: spec.id,
      title: spec.title,
      artist: 'KaraokeMachine warm-up',
      source: 'scale',
      duration: Number(duration.toFixed(2)),
      bpm: spec.bpm,
      tags: spec.tags,
      album: 'Warm-ups',
      cover: 'cover.svg',
      notes: [],
      lines: built.lines,
      low,
      high,
      order: spec.order,
      audio: { backing: 'backing.ogg', guide: 'guide.ogg' },
      lyricsProvider: 'Solfège (built-in)',
      lyricsSynced: true,
      createdAt
    };
    await fs.writeFile(path.join(dir, 'cover.svg'), fallbackCoverFor(spec.title, 'KaraokeMachine warm-up', built.notes));
    await fs.writeFile(path.join(dir, 'song.json'), JSON.stringify(song, null, 2));
    console.log(
      `generated ${spec.id}: ${built.notes.length} notes, ${built.lines.length} lines, ${duration.toFixed(1)}s`
    );
  }

  // write a small manifest for convenience
  await fs.writeFile(
    path.join(OUT, 'scales.json'),
    JSON.stringify(
      SCALES.map((s) => ({ id: s.id, title: s.title, tags: s.tags, order: s.order, bpm: s.bpm })),
      null,
      2
    )
  );
  console.log(`done — ${SCALES.length} warm-ups in ${OUT}`);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
