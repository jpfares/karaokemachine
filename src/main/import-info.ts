import { spawn } from 'node:child_process';
import path from 'node:path';
import type { MelodyNote } from '../shared/types';
import { fetchLyrics } from './lyrics';
import { lookupItunes } from './metadata';
import { ytDlpCommand } from './tools';

export interface TrackInfo {
  sourceUrl: string;
  audioUrl: string;
  title: string;
  artist: string;
  album?: string;
  releaseDate?: string;
  cover?: string;
  artistBio?: string;
  albumBio?: string;
  lyrics?: string;
  lyricsProvider?: string;
  lyricsSynced?: boolean;
  duration: number;
}

interface YtDlpInfo {
  id?: string;
  title?: string;
  track?: string;
  artist?: string;
  creator?: string;
  uploader?: string;
  channel?: string;
  album?: string;
  release_date?: string;
  upload_date?: string;
  duration?: number;
  thumbnail?: string;
  webpage_url?: string;
}

function run(command: string, args: string[], timeoutMs = 120_000): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      reject(new Error(`${path.basename(command)} timed out`));
    }, timeoutMs);
    child.stdout.on('data', (chunk) => (stdout += chunk.toString()));
    child.stderr.on('data', (chunk) => (stderr += chunk.toString()));
    child.on('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      if (code === 0) resolve(stdout);
      else reject(new Error(`${path.basename(command)} exited with ${code}\n${stderr.slice(-1600)}`));
    });
  });
}

function ytDlpPrefix(): Promise<string[]> {
  return ytDlpCommand(process.cwd());
}

export function cleanTitle(value: string): string {
  return value
    .replace(/\([^)]*(official|video|audio|lyrics?|visualizer|hd|4k|live|remaster|color coded)[^)]*\)/gi, '')
    .replace(/\[[^\]]*(official|video|audio|lyrics?|visualizer|hd|4k|live|remaster|color coded)[^\]]*\]/gi, '')
    .replace(/\b(official\s*(music\s*)?video|official\s*audio|lyric\s*video|visuali[sz]er|hd|4k)\b/gi, '')
    .replace(/\s*[-–—]\s*topic\s*$/i, '')
    .replace(/\s+/g, ' ')
    .trim();
}

export function cleanArtist(value: string): string {
  return value
    .replace(/\s*[-–—]\s*topic\s*$/i, '')
    .replace(/VEVO$/i, '')
    .replace(/\s+/g, ' ')
    .trim();
}

function isSpotify(url: URL): boolean {
  return url.hostname.endsWith('spotify.com');
}

function isSoundCloud(url: URL): boolean {
  return url.hostname.endsWith('soundcloud.com') || url.hostname.endsWith('snd.sc');
}

function spotifyParts(rawTitle: string): { title: string; artist: string } {
  const clean = rawTitle.replace(/\s*\|\s*Spotify\s*$/i, '').trim();
  const match = clean.match(/^(.*?)\s*[-–—]\s*song(?:\s+and\s+lyrics)?\s+by\s+(.+)$/i);
  if (match) {
    return { title: match[1]!.trim(), artist: match[2]!.trim() };
  }
  return { title: clean, artist: '' };
}

async function resolveSpotify(url: string): Promise<{ audioUrl: string; title: string; artist: string; cover?: string }> {
  let title = '';
  let artist = '';
  let cover: string | undefined;
  try {
    const response = await fetch(`https://open.spotify.com/oembed?url=${encodeURIComponent(url)}`, {
      headers: { 'user-agent': 'KaraokeMachine/2.0', accept: 'application/json' },
      signal: AbortSignal.timeout(12_000)
    });
    if (response.ok) {
      const data = (await response.json()) as { title?: string; thumbnail_url?: string };
      const parsed = spotifyParts(data.title ?? '');
      title = parsed.title;
      artist = parsed.artist;
      cover = data.thumbnail_url;
    }
  } catch {
    // fall through to a YouTube search with the URL itself
  }

  const query = title && artist ? `${artist} ${title}` : title || url;
  const [command, ...prefix] = await ytDlpPrefix();
  const found = await run(command, [
    ...prefix,
    '--no-warnings',
    '--flat-playlist',
    '--skip-download',
    '--print',
    '%(id)s',
    `ytsearch1:${query} official audio`
  ]);
  const videoId = found.split(/\r?\n/).map((line) => line.trim()).find(Boolean);
  if (!videoId) throw new Error('Could not find a YouTube match for that Spotify track');
  return {
    audioUrl: `https://www.youtube.com/watch?v=${videoId}`,
    title,
    artist,
    cover
  };
}

interface ResolvedSource {
  audioUrl: string;
  spotify?: { title: string; artist: string; cover?: string };
}

async function resolveSource(url: string): Promise<ResolvedSource> {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw new Error('Enter a valid YouTube, Spotify or SoundCloud link');
  }
  if (!/^https?:$/.test(parsed.protocol)) {
    throw new Error('Only http and https links are supported');
  }
  if (isSpotify(parsed)) {
    const spotify = await resolveSpotify(url);
    return { audioUrl: spotify.audioUrl, spotify };
  }
  if (
    parsed.hostname.includes('youtube.com') ||
    parsed.hostname.includes('youtu.be') ||
    isSoundCloud(parsed)
  ) {
    return { audioUrl: parsed.toString() };
  }
  throw new Error('Paste a YouTube, Spotify or SoundCloud link');
}

export async function resolveAudioUrl(url: string): Promise<string> {
  return (await resolveSource(url)).audioUrl;
}

async function readYtDlp(url: string): Promise<YtDlpInfo> {
  const [command, ...prefix] = await ytDlpPrefix();
  const raw = await run(command, [
    ...prefix,
    '--no-warnings',
    '--no-playlist',
    '--skip-download',
    '--dump-single-json',
    url
  ]);
  const start = raw.indexOf('{');
  if (start < 0) throw new Error('The link did not return track information');
  return JSON.parse(raw.slice(start)) as YtDlpInfo;
}

function formatCompactDate(value: string): string {
  if (/^\d{8}$/.test(value)) {
    return `${value.slice(0, 4)}-${value.slice(4, 6)}-${value.slice(6, 8)}`;
  }
  return value;
}

function releaseFromInfo(info: YtDlpInfo): string | undefined {
  if (info.release_date) return formatCompactDate(info.release_date);
  if (info.upload_date && /^\d{8}$/.test(info.upload_date)) {
    return formatCompactDate(info.upload_date);
  }
  return undefined;
}

function decodeEntities(value: string): string {
  return value
    .replace(/&#x([0-9a-f]+);/gi, (_, hex) => String.fromCodePoint(Number.parseInt(hex, 16)))
    .replace(/&#(\d+);/g, (_, number) => String.fromCodePoint(Number.parseInt(number, 10)))
    .replace(/&nbsp;/gi, ' ')
    .replace(/&amp;/gi, '&')
    .replace(/&quot;/gi, '"')
    .replace(/&#39;|&apos;/gi, "'")
    .replace(/&lt;/gi, '<')
    .replace(/&gt;/gi, '>');
}

async function geniusLyrics(title: string, artist: string): Promise<string | null> {
  try {
    const response = await fetch(
      `https://genius.com/api/search/multi?q=${encodeURIComponent(`${title} ${artist}`)}`,
      {
        headers: {
          'user-agent': 'Mozilla/5.0 KaraokeMachine/2.0',
          accept: 'application/json'
        },
        signal: AbortSignal.timeout(12_000)
      }
    );
    if (!response.ok) return null;
    const data = (await response.json()) as {
      response?: {
        sections?: {
          hits?: {
            result?: {
              url?: string;
              title?: string;
              primary_artist?: { name?: string };
              artist_names?: string;
            };
          }[];
        }[];
      };
    };
    const hits = (data.response?.sections ?? []).flatMap((section) => section.hits ?? []);
    const wantedTitle = title.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
    const wantedArtist = artist.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
    const song = hits
      .map((hit) => hit.result)
      .filter((result): result is NonNullable<typeof result> => Boolean(result?.url))
      .map((result) => {
        const candidateTitle = (result.title ?? '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
        const candidateArtist = (result.primary_artist?.name ?? result.artist_names ?? '')
          .toLowerCase()
          .replace(/[^\p{L}\p{N}]+/gu, ' ')
          .trim();
        let score = 0;
        if (candidateTitle === wantedTitle) score += 8;
        else if (candidateTitle.includes(wantedTitle) || wantedTitle.includes(candidateTitle)) score += 4;
        if (wantedArtist && candidateArtist.includes(wantedArtist)) score += 4;
        return { result, score };
      })
      .sort((a, b) => b.score - a.score)[0]?.result;
    if (!song?.url) return null;
    const page = await fetch(song.url, {
      headers: {
        'user-agent':
          'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36',
        accept: 'text/html'
      },
      signal: AbortSignal.timeout(15_000)
    });
    if (!page.ok) return null;
    const html = await page.text();
    const blocks = [...html.matchAll(/<div[^>]*data-lyrics-container="true"[^>]*>([\s\S]*?)<\/div>/gi)].map(
      (match) => match[1] ?? ''
    );
    if (!blocks.length) return null;
    const text = blocks
      .map((block) =>
        decodeEntities(
          block
            .replace(/<br\s*\/?>/gi, '\n')
            .replace(/<[^>]+>/g, '')
        )
      )
      .join('\n')
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line && !/^\[[^\]]+\]$/.test(line))
      .join('\n')
      .trim();
    return text || null;
  } catch {
    return null;
  }
}

export async function fetchTrackInfo(url: string): Promise<TrackInfo> {
  const source = await resolveSource(url);
  const audioUrl = source.audioUrl;
  const info = await readYtDlp(audioUrl);
  const spotify = source.spotify ?? null;
  const rawTitle = spotify?.title || info.track || info.title || '';
  const rawArtist =
    spotify?.artist || info.artist || info.creator || info.uploader || info.channel || '';
  let title = cleanTitle(rawTitle) || 'Untitled track';
  let artist = cleanArtist(rawArtist) || 'Unknown artist';
  if (artist && title.toLowerCase().startsWith(`${artist.toLowerCase()} - `)) {
    title = title.slice(artist.length + 3).trim();
  }
  const duration = Number(info.duration) || 0;
  const itunes = await lookupItunes(title, artist).catch(() => null);
  const album = info.album || itunes?.album;
  const cover = itunes?.coverUrl || spotify?.cover || info.thumbnail;
  const releaseDate = itunes?.releaseDate || releaseFromInfo(info);

  let lyrics: string | undefined;
  let lyricsProvider: string | undefined;
  let lyricsSynced = false;
  try {
    const synthetic: MelodyNote[] = [{ start: 0, dur: Math.max(1, duration), midi: 60 }];
    const found = await fetchLyrics(title, artist, duration || 240, synthetic);
    if (found) {
      lyrics = found.lines.map((line) => line.text).join('\n');
      lyricsProvider = found.provider;
      lyricsSynced = found.synced;
    }
  } catch {
    // ignored; Genius below
  }
  if (!lyrics) {
    lyrics = (await geniusLyrics(title, artist)) ?? undefined;
    if (lyrics) lyricsProvider = 'Genius';
  }

  return {
    sourceUrl: url,
    audioUrl,
    title,
    artist,
    album,
    releaseDate,
    cover,
    lyrics,
    lyricsProvider,
    lyricsSynced,
    duration
  };
}
