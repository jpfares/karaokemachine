import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs/promises';
import path from 'node:path';
import type { ImportProgress, LyricLine, MelodyNote, Song } from '../shared/types';
import { alignLyrics, fetchLyrics, layoutPlainLyrics } from './lyrics';
import { enrichCover } from './metadata';

export interface SongOverrides {
  audioUrl?: string;
  title?: string;
  artist?: string;
  album?: string;
  releaseDate?: string;
  cover?: string;
  artistBio?: string;
  albumBio?: string;
  lyricsText?: string;
}

export interface PipelineOptions {
  root?: string;
  assetsRoot?: string;
  force?: boolean;
  overrides?: SongOverrides;
  onProgress?: (progress: Omit<ImportProgress, 'url' | 'index'>) => void;
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const exists = async (file: string) => {
  try {
    await fs.access(file);
    return true;
  } catch {
    return false;
  }
};

function run(
  command: string,
  args: string[],
  options: { cwd?: string; onLine?: (line: string) => void; env?: NodeJS.ProcessEnv } = {}
): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe']
    });
    let stdout = '';
    let stderr = '';
    const handle = (chunk: Buffer, stream: 'out' | 'err') => {
      const text = chunk.toString();
      if (stream === 'out') stdout += text;
      else stderr += text;
      if (options.onLine) {
        for (const line of text.split(/[\r\n]+/)) {
          const trimmed = line.trim();
          if (trimmed) options.onLine(trimmed);
        }
      }
    };
    child.stdout.on('data', (chunk) => handle(chunk, 'out'));
    child.stderr.on('data', (chunk) => handle(chunk, 'err'));
    child.on('error', reject);
    child.on('close', (code) => {
      if (code === 0) resolve(stdout);
      else reject(new Error(`${path.basename(command)} exited with ${code}\n${stderr.slice(-1800)}`));
    });
  });
}

async function ffprobeDuration(file: string): Promise<number> {
  try {
    const out = await run('ffprobe', [
      '-v', 'error',
      '-show_entries', 'format=duration',
      '-of', 'default=nw=1:nk=1',
      file
    ]);
    const value = Number.parseFloat(out.trim());
    return Number.isFinite(value) ? value : 0;
  } catch {
    return 0;
  }
}

function videoId(url: string): string | null {
  try {
    const parsed = new URL(url);
    if (parsed.hostname.includes('youtu.be')) {
      const id = parsed.pathname.split('/').filter(Boolean)[0];
      return id && /^[\w-]{6,20}$/.test(id) ? id : null;
    }
    const v = parsed.searchParams.get('v');
    if (v && /^[\w-]{6,20}$/.test(v)) return v;
    const shorts = parsed.pathname.match(/\/shorts\/([\w-]{6,20})/);
    if (shorts) return shorts[1]!;
    const embed = parsed.pathname.match(/\/embed\/([\w-]{6,20})/);
    if (embed) return embed[1]!;
  } catch {
    // fall through
  }
  return null;
}

function cleanTitle(value: string): string {
  return value
    .replace(/\([^)]*(official|video|audio|lyrics?|visualizer|hd|4k|live|remaster|color coded)[^)]*\)/gi, '')
    .replace(/\[[^\]]*(official|video|audio|lyrics?|visualizer|hd|4k|live|remaster|color coded)[^\]]*\]/gi, '')
    .replace(/\b(official\s*(music\s*)?video|official\s*audio|lyric\s*video|visuali[sz]er|hd|4k)\b/gi, '')
    .replace(/\s*[-–—]\s*topic\s*$/i, '')
    .replace(/\s+/g, ' ')
    .trim();
}

function cleanArtist(value: string): string {
  return value
    .replace(/\s*[-–—]\s*topic\s*$/i, '')
    .replace(/VEVO$/i, '')
    .replace(/\s+/g, ' ')
    .trim();
}

interface VideoInfo {
  id?: string;
  title?: string;
  track?: string;
  artist?: string;
  creator?: string;
  uploader?: string;
  channel?: string;
  duration?: number;
}

async function readInfo(file: string): Promise<VideoInfo> {
  try {
    return JSON.parse(await fs.readFile(file, 'utf8')) as VideoInfo;
  } catch {
    return {};
  }
}

async function readNotes(file: string): Promise<{ notes: MelodyNote[]; stats?: Record<string, unknown> }> {
  try {
    const parsed = JSON.parse(await fs.readFile(file, 'utf8')) as {
      notes: (MelodyNote & { end: number; conf: number })[];
      stats?: Record<string, unknown>;
    };
    return {
      notes: parsed.notes.map((note) => ({
        start: note.start,
        dur: Math.max(0.09, note.dur || note.end - note.start),
        midi: note.midi,
        vel: note.vel,
        cents: note.cents
      })),
      stats: parsed.stats
    };
  } catch {
    return { notes: [] };
  }
}

async function storeCover(source: string, dir: string): Promise<string> {
  if (/^https?:\/\//i.test(source)) {
    const response = await fetch(source, {
      headers: { 'user-agent': 'KaraokeMachine/2.0' },
      signal: AbortSignal.timeout(20_000)
    });
    if (!response.ok) throw new Error(`cover download failed: ${response.status}`);
    const type = response.headers.get('content-type') ?? '';
    const data = Buffer.from(await response.arrayBuffer());
    if (data.length < 200) throw new Error('cover image was empty');
    const ext = type.includes('png')
      ? 'png'
      : type.includes('webp')
        ? 'webp'
        : type.includes('svg')
          ? 'svg'
          : 'jpg';
    const name = `cover.${ext}`;
    await fs.writeFile(path.join(dir, name), data);
    return name;
  }
  const local = path.resolve(source);
  const data = await fs.readFile(local);
  const ext = path.extname(local).replace('.', '').toLowerCase();
  const safe = ['jpg', 'jpeg', 'png', 'webp', 'svg'].includes(ext) ? ext : 'jpg';
  const name = `cover.${safe}`;
  await fs.writeFile(path.join(dir, name), data);
  return name;
}

async function fallbackSeparation(
  source: string,
  outDir: string,
  onProgress: (p: number) => void
): Promise<{ vocals: string; noVocals: string }> {
  onProgress(0.05);
  const vocals = path.join(outDir, 'vocals.wav');
  const noVocals = path.join(outDir, 'no_vocals.wav');
  // Rough center extraction / side cancellation. This is only used when the
  // neural separator is unavailable; Demucs is the preferred path.
  await run('ffmpeg', [
    '-y', '-v', 'error', '-i', source,
    '-af', 'pan=stereo|c0=0.5*c0+0.5*c1|c1=0.5*c0+0.5*c1',
    '-ar', '44100', vocals
  ]);
  onProgress(0.5);
  await run('ffmpeg', [
    '-y', '-v', 'error', '-i', source,
    '-af', 'pan=stereo|c0=0.5*c0-0.5*c1|c1=0.5*c1-0.5*c0',
    '-ar', '44100', noVocals
  ]);
  onProgress(1);
  return { vocals, noVocals };
}

export async function importUrl(url: string, options: PipelineOptions = {}): Promise<Song> {
  const root = options.root ?? process.cwd();
  const assetsRoot = options.assetsRoot ?? path.join(root, 'assets');
  const songsRoot = path.join(assetsRoot, 'songs');
  const emit = options.onProgress ?? (() => {});
  const overrides = options.overrides ?? {};
  const downloadUrl = overrides.audioUrl?.trim() || url;
  const id = `yt-${videoId(downloadUrl) ?? crypto.createHash('sha1').update(downloadUrl).digest('hex').slice(0, 11)}`;
  const finalDir = path.join(songsRoot, id);
  const songFile = path.join(finalDir, 'song.json');

  if (!options.force && (await exists(songFile))) {
    emit({ stage: 'done', percent: 100, message: 'Already in the library' });
    return JSON.parse(await fs.readFile(songFile, 'utf8')) as Song;
  }

  const work = path.join(root, '.work', id);
  await fs.mkdir(work, { recursive: true });
  await fs.mkdir(finalDir, { recursive: true });

  const demucs = path.join(root, '.venv/bin/demucs');
  const venvPython = path.join(root, '.venv/bin/python');
  const venvYtdlp = path.join(root, '.venv/bin/yt-dlp');
  const deno = path.join(process.env.HOME ?? '/home/deanp', '.deno/bin/deno');
  const analyzeScript = path.join(root, 'tools/analyze_vocals.py');
  const sourceWav = path.join(work, 'source.wav');
  const infoFile = path.join(work, 'source.info.json');

  let info = await readInfo(infoFile);

  // 1. Download -------------------------------------------------------------
  if (!(await exists(sourceWav))) {
    emit({ stage: 'download', percent: 3, message: 'Fetching audio…' });
    const ytdlp = (await exists(venvYtdlp)) ? venvYtdlp : 'yt-dlp';
    const jsArgs = (await exists(deno)) ? ['--js-runtimes', `deno:${deno}`] : [];
    await run(
      ytdlp,
      [
        ...jsArgs,
        '--no-update',
        '-f', 'bestaudio/best',
        '-x', '--audio-format', 'wav', '--audio-quality', '0',
        '--no-playlist', '--no-warnings', '--newline',
        '--write-info-json',
        '-o', path.join(work, 'source.%(ext)s'),
        downloadUrl
      ],
      {
        onLine: (line) => {
          const match = line.match(/\[download\]\s+([\d.]+)%/);
          if (match) {
            const pct = Number(match[1]);
            emit({
              stage: 'download',
              percent: 3 + Math.min(32, pct * 0.32),
              message: `Downloading audio… ${pct.toFixed(0)}%`
            });
          }
        }
      }
    );
    info = await readInfo(infoFile);
  }

  const title = overrides.title?.trim() || cleanTitle(info.track || info.title || '') || 'Untitled track';
  const artist =
    overrides.artist?.trim() ||
    cleanArtist(info.artist || info.creator || info.uploader || info.channel || '') ||
    'Unknown artist';
  let displayTitle = title;
  if (artist && displayTitle.toLowerCase().startsWith(`${artist.toLowerCase()} - `)) {
    displayTitle = displayTitle.slice(artist.length + 3).trim();
  }
  const durationHint = Number(info.duration) || 0;
  emit({ stage: 'metadata', percent: 36, message: `Downloaded “${displayTitle}”` });

  // 2. Separate vocals ------------------------------------------------------
  const separated = path.join(work, 'separated', 'htdemucs', 'source');
  let vocalsPath = path.join(separated, 'vocals.wav');
  let noVocalsPath = path.join(separated, 'no_vocals.wav');
  let stemProvider = 'demucs htdemucs';

  if (!(await exists(vocalsPath)) || !(await exists(noVocalsPath))) {
    emit({ stage: 'separate', percent: 40, message: 'Separating vocals from the backing…' });
    try {
      await fs.rm(path.join(work, 'separated'), { recursive: true, force: true });
      await run(
        demucs,
        [
          '--two-stems=vocals',
          '-n', 'htdemucs',
          '--device', 'cpu',
          '--jobs', '2',
          '-o', path.join(work, 'separated'),
          sourceWav
        ],
        {
          env: { ...process.env, PYTHONUNBUFFERED: '1' },
          onLine: (line) => {
            const match = line.match(/(\d{1,3})%/);
            if (match) {
              const pct = Math.min(100, Number(match[1]));
              emit({
                stage: 'separate',
                percent: 40 + pct * 0.28,
                message: `Separating vocals… ${pct.toFixed(0)}%`
              });
            }
          }
        }
      );
    } catch (error) {
      console.warn('[karaokemachine] demucs failed, using center-cancel fallback:', error);
      const fallback = await fallbackSeparation(sourceWav, work, (p) => {
        emit({
          stage: 'separate',
          percent: 40 + p * 28,
          message: `Splitting audio… ${Math.round(p * 100)}%`
        });
      });
      vocalsPath = fallback.vocals;
      noVocalsPath = fallback.noVocals;
      stemProvider = 'center-cancel (fallback)';
    }
  }
  if (!(await exists(vocalsPath)) || !(await exists(noVocalsPath))) {
    throw new Error('Vocal separation did not produce usable stems');
  }
  emit({ stage: 'separate', percent: 69, message: 'Building the two audio streams…' });

  // 3. Convert / normalise --------------------------------------------------
  const backingOut = path.join(finalDir, 'backing.mp3');
  const guideOut = path.join(finalDir, 'guide.mp3');
  const analysisWav = path.join(work, 'vocals-analysis.wav');
  await run('ffmpeg', [
    '-y', '-v', 'error', '-i', noVocalsPath,
    '-af', 'loudnorm=I=-14:TP=-1.5:LRA=11',
    '-ar', '48000', '-ac', '2', '-c:a', 'libmp3lame', '-b:a', '192k', backingOut
  ]);
  await run('ffmpeg', [
    '-y', '-v', 'error', '-i', vocalsPath,
    '-af', 'loudnorm=I=-17:TP=-1.5:LRA=11',
    '-ar', '48000', '-ac', '2', '-c:a', 'libmp3lame', '-b:a', '160k', guideOut
  ]);
  await run('ffmpeg', [
    '-y', '-v', 'error', '-i', vocalsPath,
    '-ac', '1', '-ar', '22050', '-c:a', 'pcm_s16le', analysisWav
  ]);
  const duration = (await ffprobeDuration(backingOut)) || durationHint || 0;

  // 4. Extract the melody ---------------------------------------------------
  emit({ stage: 'analyze', percent: 74, message: 'Tracing the vocal melody…' });
  const notesJson = path.join(work, 'notes.json');
  const python = (await exists(venvPython)) ? venvPython : 'python3';
  await run(python, [analyzeScript, analysisWav, notesJson], {
    onLine: (line) => {
      if (line.includes('analyzed')) {
        emit({ stage: 'analyze', percent: 82, message: line.replace(/^analyzed\s*/, 'Melody: ') });
      }
    }
  });
  const analyzed = await readNotes(notesJson);
  let notes = analyzed.notes
    .filter((note) => Number.isFinite(note.midi) && note.midi >= 36 && note.midi <= 92)
    .filter((note) => note.dur >= 0.09)
    .map((note) => ({
      ...note,
      start: Math.max(0, Math.min(duration, note.start)),
      dur: Math.max(0.09, Math.min(note.dur, duration - note.start)),
      midi: Math.round(note.midi)
    }))
    .filter((note) => note.start < duration - 0.05)
    .sort((a, b) => a.start - b.start);
  if (!notes.length) {
    // An instrumental is still singable: give the singer the guide pitches.
    emit({ stage: 'analyze', percent: 82, message: 'No clear vocal melody found — instrumental chart' });
  }

  // 5. Lyrics ---------------------------------------------------------------
  emit({ stage: 'lyrics', percent: 86, message: 'Looking up synced lyrics…' });
  let lines: LyricLine[] = [];
  let lyricsProvider = 'none';
  let lyricsSynced = false;
  if (overrides.lyricsText?.trim()) {
    lines = layoutPlainLyrics(overrides.lyricsText, notes, duration);
    lyricsProvider = 'User provided';
    emit({ stage: 'lyrics', percent: 86, message: 'Using the edited lyrics' });
  } else {
    try {
      const lyrics = await fetchLyrics(displayTitle, artist, duration, notes);
      if (lyrics) {
        lines = lyrics.lines;
        lyricsProvider = lyrics.provider;
        lyricsSynced = lyrics.synced;
      }
    } catch (error) {
      console.warn('[karaokemachine] lyrics lookup failed', error);
    }
  }

  if (lines.length && notes.length) {
    const aligned = alignLyrics(lines, notes, duration);
    lines = aligned.lines;
    notes = aligned.notes;
  }

  const midis = notes.map((note) => note.midi);
  const low = midis.length ? Math.max(28, Math.min(...midis) - 2) : 48;
  const high = midis.length ? Math.min(96, Math.max(...midis) + 2) : 76;

  let album: string | undefined = overrides.album?.trim() || undefined;
  let cover: string | undefined;
  if (overrides.cover?.trim()) {
    cover = await storeCover(overrides.cover.trim(), finalDir);
    emit({ stage: 'write', percent: 92, message: 'Saved the selected cover art' });
  } else {
    try {
      const art = await enrichCover(displayTitle, artist, notes, finalDir);
      album = album || art.album;
      cover = art.cover;
      emit({ stage: 'write', percent: 92, message: 'Found the album cover…' });
    } catch (error) {
      console.warn('[karaokemachine] album art lookup failed', error);
    }
  }

  const song: Song = {
    id,
    title: displayTitle,
    artist,
    source: 'import',
    duration: Number(duration.toFixed(2)),
    tags: ['Imported', stemProvider.includes('fallback') ? 'Quick split' : 'AI split'],
    album,
    cover,
    notes,
    lines,
    low,
    high,
    audio: {
      backing: 'backing.mp3',
      guide: 'guide.mp3'
    },
    youtubeUrl: url,
    lyricsProvider: lyricsProvider === 'none' ? undefined : lyricsProvider,
    lyricsSynced,
    createdAt: new Date().toISOString(),
    mapped: false,
    releaseDate: overrides.releaseDate?.trim() || undefined,
    artistBio: overrides.artistBio?.trim() || undefined,
    albumBio: overrides.albumBio?.trim() || undefined,
    suggestedNotes: notes,
    draftNotes: []
  };

  emit({ stage: 'write', percent: 94, message: 'Saving the song chart…' });
  await fs.writeFile(songFile, JSON.stringify(song, null, 2));

  // 7. Clean the chart with neural pitch tracking. This removes guitar-bleed
  //    octave spikes and flattens each word to its actual sung pitch.
  const cleanScript = path.join(root, 'tools/clean_charts.py');
  if (await exists(cleanScript)) {
    try {
      await run(venvPython, [cleanScript, finalDir], {});
    } catch (error) {
      console.warn('[karaokemachine] chart cleanup failed', error);
    }
  }

  // Move the cleaned automatic chart into suggestions. The new song is
  // deliberately UNMAPPED: the editor starts with an empty mapping and the old
  // chart is only a translucent reference.
  try {
    const saved = JSON.parse(await fs.readFile(songFile, 'utf8')) as Song;
    if (!saved.mapped) {
      saved.suggestedNotes = saved.notes ?? notes;
      saved.notes = [];
      saved.draftNotes = [];
      saved.mapped = false;
      await fs.writeFile(songFile, JSON.stringify(saved, null, 2));
    }
  } catch (error) {
    console.warn('[karaokemachine] could not stage suggestions', error);
  }

  emit({
    stage: 'done',
    percent: 100,
    message: `Ready — ${notes.length} suggested notes${lines.length ? `, ${lines.length} lyric lines` : ''}`
  });
  // Give the UI one frame to see 100%.
  await sleep(150);
  return { ...song, mapped: false, suggestedNotes: notes, notes: [] };
}
