import { fetchTrackInfo } from '../src/main/import-info';

function arg(name: string): string | undefined {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

async function main() {
  const url = arg('--url');
  if (!url) {
    console.log(JSON.stringify({ error: 'Missing --url' }));
    process.exit(1);
  }
  try {
    const info = await fetchTrackInfo(url);
    console.log(JSON.stringify(info));
  } catch (error) {
    console.log(
      JSON.stringify({ error: error instanceof Error ? error.message : String(error) })
    );
    process.exit(1);
  }
}

main().catch((error) => {
  console.log(JSON.stringify({ error: error instanceof Error ? error.message : String(error) }));
  process.exit(1);
});
