import fs from 'node:fs/promises';
import path from 'node:path';
import { reassignWordsToNotes } from '../src/main/lyrics';
import type { Song } from '../src/shared/types';

async function main() {
  const dir = path.join(process.cwd(), 'assets/songs');
  const entries = await fs.readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const file = path.join(dir, entry.name, 'song.json');
    let song: Song;
    try {
      song = JSON.parse(await fs.readFile(file, 'utf8')) as Song;
    } catch {
      continue;
    }
    if (song.source !== 'import' || !song.lines.length) continue;
    reassignWordsToNotes(song.lines, song.notes);
    await fs.writeFile(file, JSON.stringify(song, null, 2));
    console.log(`${song.title}: ${song.lines.length} lines / ${song.notes.length} notes realigned`);
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
