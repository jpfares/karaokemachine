import fs from 'node:fs/promises';
import path from 'node:path';
import { enrichCover, fallbackCoverFor } from '../src/main/metadata';
import type { Song } from '../src/shared/types';

async function main() {
  const songsDir = path.join(process.cwd(), 'assets/songs');
  const entries = await fs.readdir(songsDir, { withFileTypes: true });
  let done = 0;
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const dir = path.join(songsDir, entry.name);
    const file = path.join(dir, 'song.json');
    let song: Song;
    try {
      song = JSON.parse(await fs.readFile(file, 'utf8')) as Song;
    } catch {
      continue;
    }
    if (song.source === 'scale') {
      await fs.writeFile(path.join(dir, 'cover.svg'), fallbackCoverFor(song.title, 'KaraokeMachine', song.notes));
      song.album = 'Warm-ups';
      song.cover = 'cover.svg';
    } else {
      const result = await enrichCover(song.title, song.artist ?? 'Unknown artist', song.notes, dir);
      song.album = result.album;
      song.cover = result.cover;
      console.log(`${song.title}: ${result.album} (${result.cover})`);
      done++;
    }
    await fs.writeFile(file, JSON.stringify(song, null, 2));
  }
  console.log(`enriched ${done} imported tracks and refreshed warm-up covers`);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
