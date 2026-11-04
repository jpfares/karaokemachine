import { build } from 'esbuild';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.dirname(fileURLToPath(new URL('.', import.meta.url)));
const outdir = path.join(root, 'dist-tools');

await build({
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node22',
  sourcemap: true,
  logLevel: 'info',
  entryPoints: [
    path.join(root, 'scripts/generate-scales.ts'),
    path.join(root, 'scripts/prepare-songs.ts'),
    path.join(root, 'scripts/song-info.ts'),
    path.join(root, 'scripts/import-song.ts'),
    path.join(root, 'scripts/fetch-cover.ts')
  ],
  outdir
});
