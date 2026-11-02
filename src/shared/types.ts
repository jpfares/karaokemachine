export type SongSource = 'scale' | 'import' | 'demo';

export interface MelodyNote {
  /** seconds from the start of the backing track */
  start: number;
  /** seconds */
  dur: number;
  /** MIDI note number, 69 = A4 */
  midi: number;
  /** syllable / word attached to this note */
  word?: string;
  /** index of the word within its lyric line */
  wordIndex?: number;
  /** index into Song.lines */
  line?: number;
  /** 0..1 */
  vel?: number;
  /** cents offset from equal temperament, for synthesized charts */
  cents?: number;
}

export interface LyricWord {
  text: string;
  start: number;
  end: number;
}

export interface LyricLine {
  start: number;
  end: number;
  text: string;
  words: LyricWord[];
}

export interface AudioFiles {
  /** relative file names inside the song folder */
  backing: string;
  vocals?: string;
  guide?: string;
}

export interface Song {
  id: string;
  title: string;
  artist?: string;
  source: SongSource;
  duration: number;
  bpm?: number;
  tags: string[];
  album?: string;
  cover?: string;
  notes: MelodyNote[];
  lines: LyricLine[];
  /** comfortable display range, MIDI */
  low: number;
  high: number;
  /** curated ordering in the library */
  order?: number;
  audio: AudioFiles;
  youtubeUrl?: string;
  lyricsProvider?: string;
  lyricsSynced?: boolean;
  createdAt?: string;
  /** false until a human submits a manual mapping */
  mapped?: boolean;
  /** the automatic chart, kept only as a translucent editor guide */
  suggestedNotes?: MelodyNote[];
  /** an in-progress manual mapping */
  draftNotes?: MelodyNote[];
  releaseDate?: string;
  artistBio?: string;
  albumBio?: string;
}

/** What the library screen needs. Audio files are resolved to media:// URLs. */
export interface SongSummary {
  id: string;
  title: string;
  artist?: string;
  source: SongSource;
  duration: number;
  bpm?: number;
  tags: string[];
  album?: string;
  cover?: string;
  noteCount: number;
  lyricLines: number;
  low: number;
  high: number;
  /** first ~80 notes for library artwork */
  preview: { start: number; dur: number; midi: number }[];
  order?: number;
  lyricsSynced?: boolean;
  lyricsProvider?: string;
  youtubeUrl?: string;
  mapped?: boolean;
}

/** Song returned to the renderer, with playable media URLs. */
export type PlayableSong = Omit<Song, 'audio'> & {
  audio: {
    backing: string;
    vocals?: string;
    guide?: string;
  };
};

export interface SongListItem {
  index: number;
  url: string;
  status: 'ready' | 'pending' | 'error';
  songId?: string;
  title?: string;
  artist?: string;
  error?: string;
  hasLyrics?: boolean;
}

export interface ImportProgress {
  url: string;
  index?: number;
  stage:
    | 'queued'
    | 'metadata'
    | 'download'
    | 'separate'
    | 'analyze'
    | 'lyrics'
    | 'write'
    | 'done'
    | 'error';
  percent: number;
  message: string;
}

export interface AppInfo {
  version: string;
  songListPath: string;
  assetsPath: string;
}
