import fs from 'node:fs/promises';
import { importUrl, type SongOverrides } from '../src/main/pipeline';

function arg(name: string): string | undefined {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

async function main() {
  const url = arg('--url');
  const metaFile = arg('--meta');
  if (!url) throw new Error('Missing --url');

  let overrides: SongOverrides = {};
  if (metaFile) {
    overrides = JSON.parse(await fs.readFile(metaFile, 'utf8')) as SongOverrides;
  }

  const song = await importUrl(url, {
    root: process.cwd(),
    assetsRoot: process.cwd() + '/assets',
    force: process.argv.includes('--force'),
    overrides,
    onProgress: (progress) => {
      console.log(
        `PROGRESS\t${progress.percent.toFixed(1)}\t${progress.stage}\t${progress.message.replace(/[\r\n]+/g, ' ')}`
      );
    }
  });
  console.log(`RESULT\t${song.id}`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : String(error));
  console.log(`ERROR\t${error instanceof Error ? error.message.replace(/[\r\n]+/g, ' ') : String(error)}`);
  process.exit(1);
});
