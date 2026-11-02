import fs from 'node:fs/promises';

function arg(name: string): string | undefined {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

async function main() {
  const url = arg('--url');
  const out = arg('--out');
  if (!url || !out) throw new Error('Missing --url or --out');
  const response = await fetch(url, {
    headers: { 'user-agent': 'KaraokeMachine/2.0' },
    signal: AbortSignal.timeout(20_000)
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  const data = Buffer.from(await response.arrayBuffer());
  if (data.length < 200) throw new Error('The image was empty');
  await fs.writeFile(out, data);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
