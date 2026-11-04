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
  /** legacy ribbon group; new mappings use Song.ribbonLinks */
  ribbon?: number;
}

export type RibbonSide = 'left' | 'right';

export interface RibbonLink {
  from: number;
  to: number;
  fromSide?: RibbonSide;
  toSide?: RibbonSide;
}

export interface PlacedWord {
  text: string;
  /** center time of the decorative label */
  start: number;
  /** 0.0 = top of the playfield, 1.0 = bottom */
  y: number;
  line?: number;
  wordIndex?: number;
  customId?: number;
}

export interface CustomWord {
  id: number;
  text: string;
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
  /** explicit note-to-note ribbon connectors */
  ribbonLinks?: RibbonLink[];
  releaseDate?: string;
  artistBio?: string;
  albumBio?: string;
  /** decorative lyrics, positioned independently from notes */
  placedWords?: PlacedWord[];
  /** custom words added from the editor word bank */
  customWords?: CustomWord[];
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
