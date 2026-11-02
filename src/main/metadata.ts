import fs from 'node:fs/promises';
import path from 'node:path';
import type { MelodyNote } from '../shared/types';

export interface ItunesResult {
  trackName?: string;
  artistName?: string;
  collectionName?: string;
  artworkUrl100?: string;
  releaseDate?: string;
}

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];

function normalize(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

function noteName(midi: number): string {
  return `${NOTE_NAMES[((Math.round(midi) % 12) + 12) % 12]}${Math.floor(Math.round(midi) / 12) - 1}`;
}

function fallbackCover(title: string, artist: string, notes: MelodyNote[]): string {
  const width = 600;
  const height = 600;
  const source = notes.slice(0, 120);
  const low = source.length ? Math.min(...source.map((n) => n.midi)) - 2 : 48;
  const high = source.length ? Math.max(...source.map((n) => n.midi)) + 2 : 84;
  const span = Math.max(12, high - low);
  const start = source.length ? source[0]!.start : 0;
  const spanTime = Math.max(1, (source[source.length - 1]?.start ?? 1) - start);
  const colors = ['#ff5d9e', '#ff9b6e', '#ffd166', '#7cf7c4', '#64c8ff', '#a78bfa'];

  const bars: string[] = [];
  for (const note of source) {
    const x = 44 + ((note.start - start) / spanTime) * (width - 88);
    const y = 450 - ((note.midi - low) / span) * 300;
    const w = Math.max(3, (note.dur / spanTime) * (width - 88));
    const color = colors[Math.min(colors.length - 1, Math.floor(((note.midi - low) / span) * colors.length))]!;
    bars.push(`<rect x="${x.toFixed(1)}" y="${y.toFixed(1)}" width="${w.toFixed(1)}" height="8" rx="4" fill="${color}" opacity="0.88"/>`);
  }
  const safeTitle = title.replace(/[<>&"]/g, '').slice(0, 26);
  const safeArtist = artist.replace(/[<>&"]/g, '').slice(0, 26);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#10162a"/>
      <stop offset="0.55" stop-color="#070b14"/>
      <stop offset="1" stop-color="#1b0d18"/>
    </linearGradient>
    <radialGradient id="glow" cx="0.7" cy="0.18" r="0.8">
      <stop offset="0" stop-color="#ff5d9e" stop-opacity="0.28"/>
      <stop offset="1" stop-color="#ff5d9e" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <rect width="600" height="600" fill="url(#bg)"/>
  <rect width="600" height="600" fill="url(#glow)"/>
  <g opacity="0.15" stroke="#fff" stroke-width="1">
    ${Array.from({ length: 12 }, (_, i) => `<line x1="44" x2="556" y1="${150 + i * 26}" y2="${150 + i * 26}"/>`).join('')}
  </g>
  <line x1="172" y1="120" x2="172" y2="490" stroke="#7cf7c4" stroke-opacity="0.55" stroke-width="2"/>
  ${bars.join('')}
  <text x="44" y="540" fill="#f4f7fb" font-family="sans-serif" font-size="28" font-weight="700">${safeTitle}</text>
  <text x="44" y="566" fill="#8e96aa" font-family="sans-serif" font-size="14">${safeArtist}</text>
  <text x="556" y="566" text-anchor="end" fill="#7cf7c4" font-family="monospace" font-size="10" letter-spacing="3">KARAOKEMACHINE</text>
</svg>`;
}

export async function lookupItunes(
  title: string,
  artist: string
): Promise<{ album: string; coverUrl: string; releaseDate?: string } | null> {
  const term = `${title} ${artist}`.trim();
  const url = `https://itunes.apple.com/search?term=${encodeURIComponent(term)}&entity=song&limit=25&media=music`;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 10000);
  try {
    const response = await fetch(url, {
      signal: controller.signal,
      headers: { 'user-agent': 'KaraokeMachine/2.0 (local karaoke app)', accept: 'application/json' }
    });
    if (!response.ok) return null;
    const data = (await response.json()) as { results?: ItunesResult[] };
    const candidates = data.results ?? [];
    const wantedTitle = normalize(title);
    const wantedArtist = normalize(artist);
    let best: ItunesResult | null = null;
    let bestScore = -Infinity;
    for (const candidate of candidates) {
      if (!candidate.artworkUrl100) continue;
      const track = normalize(candidate.trackName ?? '');
      const performer = normalize(candidate.artistName ?? '');
      let score = 0;
      if (track === wantedTitle) score += 20;
      else if (track.startsWith(wantedTitle) || wantedTitle.startsWith(track)) score += 8;
      else score -= 16;
      if (performer === wantedArtist) score += 14;
      else if (performer.includes(wantedArtist) || wantedArtist.includes(performer)) score += 4;
      else score -= 20;
      if (candidate.collectionName) score += 1;
      const collection = candidate.collectionName ?? '';
      if (/greatest hits|best of|essential|anthology|collection|compilation|live|soundtrack|hits/i.test(collection)) {
        score -= 25;
      }
      if (/tribute|karaoke|lullaby|cover|instrumental|quartet|workout|made famous|in the style/i.test(`${collection} ${performer}`)) {
        score -= 30;
      }
      const year = Number.parseInt(candidate.releaseDate?.slice(0, 4) ?? '', 10);
      if (Number.isFinite(year)) score += Math.max(0, Math.min(2, (2010 - year) / 20));
      if (score > bestScore) {
        best = candidate;
        bestScore = score;
      }
    }
    if (!best || bestScore < 20 || !best.artworkUrl100) return null;
    return {
      album: best.collectionName ?? title,
      coverUrl: best.artworkUrl100.replace(/\/\d+x\d+bb\./, '/600x600bb.'),
      releaseDate: best.releaseDate?.slice(0, 10)
    };
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

export interface CoverResult {
  album: string;
  cover: string;
}

export async function enrichCover(
  title: string,
  artist: string,
  notes: MelodyNote[],
  dir: string
): Promise<CoverResult> {
  const hit = await lookupItunes(title, artist);
  if (hit) {
    try {
      const response = await fetch(hit.coverUrl, {
        headers: { 'user-agent': 'KaraokeMachine/2.0 (local karaoke app)' }
      });
      if (response.ok) {
        const data = Buffer.from(await response.arrayBuffer());
        if (data.length > 1000) {
          await fs.writeFile(path.join(dir, 'cover.jpg'), data);
          return { album: hit.album, cover: 'cover.jpg' };
        }
      }
    } catch {
      // fall through to the procedural cover
    }
  }
  await fs.writeFile(path.join(dir, 'cover.svg'), fallbackCover(title, artist, notes));
  return { album: hit?.album ?? title, cover: 'cover.svg' };
}

export function fallbackCoverFor(title: string, artist: string, notes: MelodyNote[]): string {
  return fallbackCover(title, artist, notes);
}

export { noteName };
