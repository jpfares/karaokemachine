import fs from 'node:fs/promises';
import path from 'node:path';
import { importUrl } from '../src/main/pipeline';
import type { ImportProgress } from '../src/shared/types';

function arg(name: string): string | undefined {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

async function main() {
  const root = process.cwd();
  const file = arg('--file') ?? path.join(process.env.HOME ?? '/home/deanp', 'Downloads/karaoke-songs.txt');
  const limit = Number(arg('--limit') ?? 0);
  const force = process.argv.includes('--force');
  const only = arg('--url');

  let urls: string[] = [];
  if (only) {
    urls = [only];
  } else {
    try {
      const raw = await fs.readFile(file, 'utf8');
      urls = raw
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter((line) => line && !line.startsWith('#'));
    } catch (error) {
      console.error(`Cannot read ${file}`, error);
      process.exit(1);
    }
  }
  if (limit > 0) urls = urls.slice(0, limit);
  console.log(`Preparing ${urls.length} track(s) from ${file}`);

  for (let i = 0; i < urls.length; i++) {
    const url = urls[i]!;
    console.log(`\n[${i + 1}/${urls.length}] ${url}`);
    try {
      await importUrl(url, {
        root,
        assetsRoot: path.join(root, 'assets'),
        force,
        onProgress: (progress: Omit<ImportProgress, 'url'>) => {
          const pct = Math.round(progress.percent).toString().padStart(3);
          console.log(`  ${pct}%  ${progress.stage.padEnd(10)} ${progress.message}`);
        }
      });
    } catch (error) {
      console.error(`  failed: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
