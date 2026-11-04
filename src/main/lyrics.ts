import type { LyricLine, LyricWord, MelodyNote } from '../shared/types';

interface LrcLine {
  start: number;
  text: string;
  words?: { text: string; start: number }[];
}

const TIME_TAG = /\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\]/g;
const WORD_TAG = /<(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?>/g;

function tagSeconds(min: string, sec: string, frac?: string): number {
  let ms = 0;
  if (frac) ms = frac.length === 1 ? Number(frac) * 100 : frac.length === 2 ? Number(frac) * 10 : Number(frac);
  return Number(min) * 60 + Number(sec) + ms / 1000;
}

function stripMetadata(text: string): string {
  return text
    .replace(/^\[[a-z]+:.*?\]\s*/i, '')
    .replace(/\s+/g, ' ')
    .trim();
}

export function parseLrc(raw: string): LrcLine[] {
  const lines: LrcLine[] = [];
  for (const original of raw.split(/\r?\n/)) {
    const stamps: number[] = [];
    TIME_TAG.lastIndex = 0;
    let match: RegExpExecArray | null;
    let lastIndex = 0;
    while ((match = TIME_TAG.exec(original))) {
      stamps.push(tagSeconds(match[1]!, match[2]!, match[3]));
      lastIndex = TIME_TAG.lastIndex;
    }
    if (!stamps.length) continue;
    let text = original.slice(lastIndex).trim();
    text = stripMetadata(text);
    if (!text || /^[\s♪♫\-–—]*$/.test(text)) continue;
    if (/^\[?(intro|outro|instrumental|verse|chorus|bridge|pre-chorus)/i.test(text) && text.length < 24) continue;

    let words: { text: string; start: number }[] | undefined;
    WORD_TAG.lastIndex = 0;
    if (WORD_TAG.test(text)) {
      WORD_TAG.lastIndex = 0;
      words = [];
      let cursor = 0;
      let wm: RegExpExecArray | null;
      let previousTime: number | null = null;
      const pieces: { text: string; time: number | null }[] = [];
      while ((wm = WORD_TAG.exec(text))) {
        const before = text.slice(cursor, wm.index);
        if (before.trim()) pieces.push({ text: before.trim(), time: previousTime });
        previousTime = tagSeconds(wm[1]!, wm[2]!, wm[3]);
        cursor = WORD_TAG.lastIndex;
      }
      const after = text.slice(cursor).trim();
      if (after) pieces.push({ text: after, time: previousTime });
      for (const piece of pieces) {
        for (const word of piece.text.split(/\s+/)) {
          if (word) words.push({ text: word, start: piece.time ?? stamps[0]! });
        }
      }
      // keep visible text without inline tags
      text = text.replace(WORD_TAG, '').replace(/\s+/g, ' ').trim();
    }

    for (const start of stamps) {
      lines.push({ start, text, words });
    }
  }
  lines.sort((a, b) => a.start - b.start);
  // de-duplicate timestamps
  const deduped: LrcLine[] = [];
  for (const line of lines) {
    if (deduped.length && Math.abs(deduped[deduped.length - 1]!.start - line.start) < 0.02) continue;
    deduped.push(line);
  }
  return deduped;
}

function distributeWords(text: string, start: number, end: number): LyricWord[] {
  const tokens = text.split(/\s+/).filter(Boolean);
  if (!tokens.length) return [];
  const weights = tokens.map((token) => Math.max(2, token.replace(/[^\p{L}\p{N}]/gu, '').length + 1));
  const total = weights.reduce((a, b) => a + b, 0);
  const span = Math.max(0.6, end - start);
  let cursor = start;
  return tokens.map((token, i) => {
    const duration = (weights[i]! / total) * span;
    const word = { text: token, start: cursor, end: cursor + duration };
    cursor += duration;
    return word;
  });
}

export function lrcToLines(raw: string, duration: number): LyricLine[] {
  const parsed = parseLrc(raw);
  if (!parsed.length) return [];
  const lines: LyricLine[] = [];
  for (let i = 0; i < parsed.length; i++) {
    const line = parsed[i]!;
    const next = parsed[i + 1];
    let end = next ? next.start : Math.min(duration, line.start + 6);
    if (end <= line.start + 0.2) end = line.start + 1.2;
    if (end - line.start > 14) end = line.start + 10;
    if (line.words && line.words.length) {
      const words: LyricWord[] = [];
      for (let w = 0; w < line.words.length; w++) {
        const word = line.words[w]!;
        const nextWord = line.words[w + 1];
        const start = Math.max(line.start, word.start || line.start);
        const wordEnd = nextWord && nextWord.start > start ? nextWord.start : end;
        words.push({ text: word.text, start, end: Math.max(start + 0.15, wordEnd) });
      }
      lines.push({ start: line.start, end, text: line.text, words });
    } else {
      lines.push({ start: line.start, end, text: line.text, words: distributeWords(line.text, line.start, end) });
    }
  }
  return lines;
}

export function layoutPlainLyrics(raw: string, notes: MelodyNote[], duration: number): LyricLine[] {
  const textLines = raw
    .split(/\r?\n/)
    .map((line) => line.replace(/\[[^\]]*\]/g, '').trim())
    .filter((line) => line && !/^(verse|chorus|bridge|intro|outro)/i.test(line));
  if (!textLines.length) return [];

  const starts = notes.map((n) => n.start);
  const ends = notes.map((n) => n.start + n.dur);
  if (!starts.length) return [];
  const first = Math.max(0, Math.min(...starts));
  const last = Math.min(duration, Math.max(...ends));
  const weights = textLines.map((line) => Math.max(4, line.replace(/\s+/g, '').length));
  const total = weights.reduce((a, b) => a + b, 0);
  const span = Math.max(8, last - first);
  let cursor = first;
  const lines: LyricLine[] = [];
  for (let i = 0; i < textLines.length; i++) {
    const length = (weights[i]! / total) * span;
    const start = cursor;
    const end = i === textLines.length - 1 ? last + 1.5 : cursor + length;
    lines.push({
      start,
      end,
      text: textLines[i]!,
      words: distributeWords(textLines[i]!, start, end)
    });
    cursor = end;
  }
  return lines;
}

function cleanForSearch(value: string): string {
  return value
    .replace(/\([^)]*(official|video|audio|lyrics?|visualizer|hd|4k|live|remaster)[^)]*\)/gi, '')
    .replace(/\[[^\]]*(official|video|audio|lyrics?|visualizer|hd|4k|live|remaster)[^\]]*\]/gi, '')
    .replace(/\b(official\s*(music\s*)?video|official\s*audio|lyric\s*video|visuali[sz]er|hd|4k)\b/gi, '')
    .replace(/\s*[-–—]\s*topic\s*$/i, '')
    .replace(/\s+/g, ' ')
    .trim();
}

interface LrclibHit {
  id: number;
  trackName: string;
  artistName: string;
  albumName?: string;
  duration?: number;
  instrumental?: boolean;
  plainLyrics?: string | null;
  syncedLyrics?: string | null;
}

async function fetchJson(url: string, timeoutMs = 12000): Promise<unknown> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, {
      signal: controller.signal,
      headers: {
        'user-agent': 'KaraokeMachine/2.0 (local karaoke practice app)',
        accept: 'application/json'
      }
    });
    if (!response.ok) return null;
    return await response.json();
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

export interface LyricsResult {
  lines: LyricLine[];
  provider: string;
  synced: boolean;
}

export async function fetchLyrics(
  title: string,
  artist: string,
  duration: number,
  notes: MelodyNote[]
): Promise<LyricsResult | null> {
  const track = cleanForSearch(title) || title;
  const performer = cleanForSearch(artist) || artist;
  const queries = [
    `https://lrclib.net/api/search?track_name=${encodeURIComponent(track)}&artist_name=${encodeURIComponent(performer)}`,
    `https://lrclib.net/api/search?q=${encodeURIComponent(`${track} ${performer}`)}`,
    `https://lrclib.net/api/search?q=${encodeURIComponent(track)}`
  ];

  let hits: LrclibHit[] = [];
  for (const url of queries) {
    const data = await fetchJson(url);
    if (Array.isArray(data) && data.length) {
      hits = data as LrclibHit[];
      break;
    }
  }

  const scored = hits
    .map((hit) => {
      const synced = hit.syncedLyrics ? 1 : 0;
      const titleMatch = normalize(hit.trackName).includes(normalize(track)) ? 2 : 0;
      const artistMatch = performer && normalize(hit.artistName).includes(normalize(performer)) ? 1 : 0;
      const durationDiff = hit.duration ? Math.abs(hit.duration - duration) : 10;
      return { hit, score: synced * 4 + titleMatch + artistMatch - Math.min(6, durationDiff / 10) };
    })
    .filter((entry) => entry.hit.syncedLyrics || entry.hit.plainLyrics)
    .sort((a, b) => b.score - a.score);

  const best = scored[0]?.hit;
  if (best) {
    if (best.syncedLyrics) {
      const lines = lrcToLines(best.syncedLyrics, duration);
      if (lines.length) return { lines, provider: 'LRCLIB (synced)', synced: true };
    }
    if (best.plainLyrics) {
      const lines = layoutPlainLyrics(best.plainLyrics, notes, duration);
      if (lines.length) return { lines, provider: 'LRCLIB (plain)', synced: false };
    }
  }

  // Fallback: lyrics.ovh plain-text API.
  try {
    const url = `https://api.lyrics.ovh/v1/${encodeURIComponent(performer)}/${encodeURIComponent(track)}`;
    const data = (await fetchJson(url, 10000)) as { lyrics?: string } | null;
    if (data?.lyrics) {
      const lines = layoutPlainLyrics(data.lyrics, notes, duration);
      if (lines.length) return { lines, provider: 'lyrics.ovh (plain)', synced: false };
    }
  } catch {
    // ignore
  }
  return null;
}

function normalize(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

/** Fit fetched lyric lines to the actual melody, correcting for video intros. */
export function alignLyrics(lines: LyricLine[], notes: MelodyNote[], duration: number): {
  lines: LyricLine[];
  notes: MelodyNote[];
  offset: number;
} {
  if (!lines.length || !notes.length) return { lines, notes, offset: 0 };

  const offsets: number[] = [];
  for (const line of lines) {
    let best: number | null = null;
    let bestDist = 3;
    for (const note of notes) {
      const dist = Math.abs(note.start - line.start);
      if (dist < bestDist) {
        bestDist = dist;
        best = note.start - line.start;
      }
    }
    if (best !== null) offsets.push(best);
  }
  offsets.sort((a, b) => a - b);
  const median = offsets.length ? offsets[Math.floor(offsets.length / 2)]! : 0;
  const shift = offsets.length >= 4 && Math.abs(median) > 0.18 && Math.abs(median) < 15 ? median : 0;

  const shifted = lines
    .map((line) => {
      const delta = shift;
      return {
        ...line,
        start: Math.max(0, line.start + delta),
        end: Math.max(0.2, line.end + delta),
        words: line.words.map((word) => ({
          text: word.text,
          start: Math.max(0, word.start + delta),
          end: Math.max(0.2, word.end + delta)
        }))
      };
    })
    .filter((line) => line.start < duration);

  const noteStarts = notes.map((note) => note.start);
  const noteEnds = notes.map((note) => note.start + note.dur);
  const firstNote = Math.min(...noteStarts);
  const lastNote = Math.max(...noteEnds);
  const fittedLines = notes.length
    ? shifted.filter((line) => {
        const hasNote = notes.some((note) => {
          const center = note.start + note.dur * 0.45;
          return center >= line.start - 0.45 && center <= line.end + 0.55;
        });
        if (hasNote) return true;
        // karaoke files often carry intro/outro ad-libs that are not present in
        // this particular recording. Don't show a line that has no sung note
        // anywhere near the intro or outro.
        if (line.start < firstNote - 1.5 || line.start > lastNote + 3) return false;
        if (line.end - line.start < 0.55) return false;
        return true;
      })
    : shifted;

  const fittedNotes = notes.map((note) => ({ ...note }));
  for (const note of fittedNotes) {
    const center = note.start + note.dur * 0.45;
    let lineIndex = -1;
    let lineDistance = 1.4;
    for (let i = 0; i < fittedLines.length; i++) {
      const line = fittedLines[i]!;
      if (center >= line.start - 0.12 && center <= line.end + 0.35) {
        lineIndex = i;
        break;
      }
      const dist = center < line.start ? line.start - center : center - line.end;
      if (dist < lineDistance) {
        lineDistance = dist;
        lineIndex = i;
      }
    }
    if (lineIndex < 0) continue;
    const line = fittedLines[lineIndex]!;
    note.line = lineIndex;
    let bestWord: LyricWord | null = null;
    let bestOverlap = 0;
    for (const word of line.words) {
      const overlap = Math.min(note.start + note.dur, word.end) - Math.max(note.start, word.start);
      if (overlap > bestOverlap) {
        bestOverlap = overlap;
        bestWord = word;
      }
    }
    if (!bestWord && line.words.length) {
      let bestDist = Infinity;
      for (const word of line.words) {
        const dist = Math.abs((word.start + word.end) / 2 - center);
        if (dist < bestDist) {
          bestDist = dist;
          bestWord = word;
        }
      }
    }
    if (bestWord) note.word = bestWord.text;
  }

  reassignWordsToNotes(fittedLines, fittedNotes);
  return { lines: fittedLines, notes: fittedNotes, offset: shift };
}

/**
 * Re-distribute the words of each lyric line across that line's contiguous
 * notes. This guarantees every word gets a group of notes even when the pitch
 * tracker missed a syllable, which is important when the words are drawn
 * directly over the melody.
 */
function reassignWordsToNotes(lines: LyricLine[], notes: MelodyNote[]): void {
  for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
    const line = lines[lineIndex]!;
    if (!line.words.length) continue;
    const indices: number[] = [];
    for (let i = 0; i < notes.length; i++) {
      if (notes[i]!.line === lineIndex) indices.push(i);
    }
    if (!indices.length) continue;
    const words = line.words;
    const n = indices.length;
    const w = words.length;
    if (n >= w) {
      const weights = words.map((word) => Math.max(2, word.text.replace(/\s+/g, '').length + 1));
      const total = weights.reduce((sum, value) => sum + value, 0);
      const counts = weights.map((weight) => Math.max(1, Math.floor((weight / total) * n)));
      let assigned = counts.reduce((sum, value) => sum + value, 0);
      const frac = weights.map((weight, i) => ({ i, value: (weight / total) * n - counts[i]! }));
      frac.sort((a, b) => b.value - a.value);
      let fi = 0;
      while (assigned < n) {
        counts[frac[fi % frac.length]!.i]!++;
        assigned++;
        fi++;
      }
      while (assigned > n) {
        const index = counts.findIndex((count) => count! > 1);
        if (index < 0) break;
        counts[index]!--;
        assigned--;
      }
      let cursor = 0;
      for (let wi = 0; wi < words.length; wi++) {
        for (let c = 0; c < counts[wi]!; c++) {
          const note = notes[indices[cursor]!]!;
          note.word = words[wi]!.text;
          note.wordIndex = wi;
          cursor++;
        }
      }
    } else {
      // More words than detected notes (fast lyric or missed syllables): let a
      // note carry a short phrase so every lyric word still appears on the ribbon.
      for (let j = 0; j < n; j++) {
        const start = Math.floor((j * w) / n);
        const end = Math.floor(((j + 1) * w) / n);
        const note = notes[indices[j]!]!;
        note.word = words
          .slice(start, Math.max(start + 1, end))
          .map((word) => word.text)
          .join(' ');
        note.wordIndex = start;
      }
    }
  }
}
