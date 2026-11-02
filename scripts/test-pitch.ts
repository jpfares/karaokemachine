import { detectPitch } from '../src/renderer/pitch';

function testTone(hz: number, sampleRate = 48000, seconds = 0.08, noise = 0.005) {
  const n = Math.floor(sampleRate * seconds);
  const buffer = new Float32Array(n);
  let rnd = 12345;
  for (let i = 0; i < n; i++) {
    let s = Math.sin((2 * Math.PI * hz * i) / sampleRate);
    // add a few harmonics like a voice
    s += 0.4 * Math.sin((2 * Math.PI * hz * 2 * i) / sampleRate);
    s += 0.15 * Math.sin((2 * Math.PI * hz * 3 * i) / sampleRate);
    rnd = (rnd * 1664525 + 1013904223) >>> 0;
    s += ((rnd / 0xffffffff) * 2 - 1) * noise;
    buffer[i] = s * 0.3;
  }
  return buffer;
}

const cases = [
  { name: 'A2', hz: 110 },
  { name: 'C3', hz: 130.81 },
  { name: 'A4', hz: 440 },
  { name: 'C5', hz: 523.25 },
  { name: 'E5', hz: 659.25 },
  { name: 'A5', hz: 880 }
];

let failed = 0;
for (const sampleRate of [44100, 48000]) {
  for (const c of cases) {
    const buffer = testTone(c.hz, sampleRate);
    const result = detectPitch(buffer, sampleRate, { at: 0 });
    const cents = result ? (69 + 12 * Math.log2(result.hz / 440) - (69 + 12 * Math.log2(c.hz / 440))) * 100 : NaN;
    const ok = result && Math.abs(cents) < 12 && result.clarity > 0.8;
    if (!ok) failed++;
    console.log(
      `${ok ? 'ok  ' : 'FAIL'} ${c.name} @ ${sampleRate}: ${result ? result.hz.toFixed(2) : 'none'} Hz, ` +
        `${Number.isFinite(cents) ? cents.toFixed(1) : '?'} cents, clarity ${result?.clarity.toFixed(2) ?? '–'}`
    );
  }
}

// silence must not produce a pitch
const silence = new Float32Array(2048);
const silentResult = detectPitch(silence, 48000);
if (silentResult) {
  console.log('FAIL silence produced a pitch');
  failed++;
} else {
  console.log('ok   silence ignored');
}

if (failed) {
  console.error(`${failed} failures`);
  process.exit(1);
}
console.log('pitch detector: all good');
