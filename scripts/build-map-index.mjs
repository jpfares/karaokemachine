import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.dirname(fileURLToPath(new URL('.', import.meta.url)));
const mapsDir = path.join(root, 'site', 'maps');
const format = 'karaokemachine-map';

function fail(file, message) {
  throw new Error(`${path.basename(file)}: ${message}`);
}

async function main() {
  const entries = [];
  const allFiles = await fs.readdir(mapsDir).catch(() => []);
  const forbidden = allFiles.find((file) => /\.(mp3|wav|ogg|flac|m4a|opus|aac)$/i.test(file));
  if (forbidden) {
    throw new Error(`site/maps must contain JSON only; found audio file ${forbidden}`);
  }
  const files = allFiles
    .filter((file) => file.endsWith('.json') && file !== 'index.json')
    .sort();

  for (const file of files) {
    const full = path.join(mapsDir, file);
    let map;
    try {
      map = JSON.parse(await fs.readFile(full, 'utf8'));
    } catch (error) {
      fail(full, `invalid JSON: ${error.message}`);
    }
    if (map.format !== format) fail(full, `expected format "${format}"`);
    if (!map.id || !map.title) fail(full, 'missing id or title');
    if (!map.sourceUrl || !/^https?:\/\//i.test(map.sourceUrl)) {
      fail(full, 'missing a valid http(s) sourceUrl');
    }
    if (!Array.isArray(map.notes) || !map.notes.length) fail(full, 'no mapped notes');
    if (map.audio || map.audioFiles) fail(full, 'audio must not be embedded in a map package');
    entries.push({
      id: map.id,
      title: map.title,
      artist: map.artist || '',
      album: map.album || '',
      releaseDate: map.releaseDate || '',
      duration: Number(map.duration) || 0,
      sourceUrl: map.sourceUrl,
      file,
      cover: map.cover || '',
      notes: map.notes.length,
      lyrics: Array.isArray(map.lines) ? map.lines.length : 0,
      contributor: map.contributor || '',
      tags: Array.isArray(map.tags) ? map.tags : []
    });
  }

  entries.sort((a, b) => a.title.localeCompare(b.title));
  await fs.writeFile(
    path.join(mapsDir, 'index.json'),
    `${JSON.stringify({ generatedAt: new Date().toISOString(), maps: entries }, null, 2)}\n`
  );
  console.log(`Map index: ${entries.length} map${entries.length === 1 ? '' : 's'}`);
}

main().catch((error) => {
  console.error(error.message || error);
  process.exit(1);
});
