import fs from 'node:fs/promises';
import path from 'node:path';
import { alignLyrics, lrcToLines } from '../src/main/lyrics';
import type { Song } from '../src/shared/types';

async function main() {
  const dir = process.argv[2];
  const lrcPath = process.argv[3];
  if (!dir || !lrcPath) throw new Error('usage: apply-lyrics <song-dir> <file.lrc>');
  const songPath = path.join(dir, 'song.json');
  const song = JSON.parse(await fs.readFile(songPath, 'utf8')) as Song;
  const raw = await fs.readFile(lrcPath, 'utf8');
  const lines = lrcToLines(raw, song.duration || 0);
  if (!lines.length) throw new Error('no lyric lines');
  const fitted = alignLyrics(lines, song.notes, song.duration || 0);
  song.lines = fitted.lines;
  song.notes = fitted.notes;
  song.lyricsProvider = 'LRCLIB (synced)';
  song.lyricsSynced = true;
  await fs.writeFile(songPath, JSON.stringify(song, null, 2));
  console.log(`${song.title}: ${fitted.lines.length} lines / ${fitted.notes.length} notes, offset ${fitted.offset.toFixed(3)}s`);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
