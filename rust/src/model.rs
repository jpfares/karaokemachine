use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const GRADIENT: [(f32, [u8; 3]); 6] = [
    (0.0, [255, 77, 141]),
    (0.18, [255, 138, 92]),
    (0.36, [255, 209, 102]),
    (0.54, [124, 247, 196]),
    (0.72, [100, 200, 255]),
    (1.0, [167, 139, 250]),
];

pub fn pitch_color(t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let mut a = GRADIENT[0];
    let mut b = GRADIENT[GRADIENT.len() - 1];
    for i in 0..GRADIENT.len() - 1 {
        if t >= GRADIENT[i].0 && t <= GRADIENT[i + 1].0 {
            a = GRADIENT[i];
            b = GRADIENT[i + 1];
            break;
        }
    }
    let span = (b.0 - a.0).max(1e-6);
    let f = ((t - a.0) / span).clamp(0.0, 1.0);
    [
        (a.1[0] as f32 + (b.1[0] as f32 - a.1[0] as f32) * f) as u8,
        (a.1[1] as f32 + (b.1[1] as f32 - a.1[1] as f32) * f) as u8,
        (a.1[2] as f32 + (b.1[2] as f32 - a.1[2] as f32) * f) as u8,
    ]
}

pub fn note_name(midi: i32) -> String {
    const NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    let n = midi.rem_euclid(12);
    format!("{}{}", NAMES[n as usize], midi / 12 - 1)
}

pub fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    let m = (seconds / 60.0).floor() as i64;
    let s = (seconds % 60.0).floor() as i64;
    format!("{m}:{s:02}")
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RibbonSide {
    Left,
    Right,
}

/// A single connector between two notes. Links are independent edges, so
/// A–B and B–C can both exist without replacing one another.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RibbonLink {
    pub from: usize,
    pub to: usize,
    #[serde(rename = "fromSide", default = "default_from_side")]
    pub from_side: RibbonSide,
    #[serde(rename = "toSide", default = "default_to_side")]
    pub to_side: RibbonSide,
}

fn default_from_side() -> RibbonSide {
    RibbonSide::Right
}

fn default_to_side() -> RibbonSide {
    RibbonSide::Left
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Note {
    pub start: f64,
    pub dur: f64,
    pub midi: i32,
    #[serde(default)]
    pub word: Option<String>,
    #[serde(rename = "wordIndex", default)]
    pub word_index: Option<usize>,
    #[serde(default)]
    pub line: Option<usize>,
    #[serde(default)]
    pub vel: Option<f32>,
    /// Manual ribbon group. Notes with the same value are one visual ribbon.
    #[serde(default)]
    pub ribbon: Option<u32>,
}

impl Note {
    /// Whether two notes should be joined by a ribbon neck.
    ///
    /// Legacy charts only have word fields to describe a ribbon, so they fall
    /// back to word/line matching when no explicit ribbon group exists.
    pub fn shares_ribbon(&self, other: &Note) -> bool {
        match (self.ribbon, other.ribbon) {
            (Some(left), Some(right)) => left == right,
            (None, None) => {
                if self.word.is_none()
                    || other.word.is_none()
                    || self.word != other.word
                    || self.line != other.line
                {
                    return false;
                }
                match (self.word_index, other.word_index) {
                    (Some(left), Some(right)) => left == right,
                    _ => true,
                }
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlacedWord {
    pub text: String,
    /// Center time of the decorative word label.
    pub start: f64,
    /// 0.0 = top of the playfield, 1.0 = bottom. Decorative words are not
    /// attached to a note, so they can float anywhere in the level.
    #[serde(default = "default_word_y")]
    pub y: f32,
    #[serde(default)]
    pub line: Option<usize>,
    #[serde(rename = "wordIndex", default)]
    pub word_index: Option<usize>,
    #[serde(rename = "customId", default)]
    pub custom_id: Option<u32>,
}

fn default_word_y() -> f32 {
    0.5
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CustomWord {
    pub id: u32,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LyricWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LyricLine {
    pub start: f64,
    pub end: f64,
    pub text: String,
    #[serde(default)]
    pub words: Vec<LyricWord>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AudioFiles {
    pub backing: String,
    #[serde(default)]
    pub guide: Option<String>,
    #[serde(default)]
    pub vocals: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Song {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub bpm: Option<f64>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default)]
    pub lines: Vec<LyricLine>,
    #[serde(default)]
    pub low: i32,
    #[serde(default)]
    pub high: i32,
    #[serde(default)]
    pub order: Option<i64>,
    #[serde(default)]
    pub audio: AudioFiles,
    #[serde(rename = "youtubeUrl", default)]
    pub youtube_url: Option<String>,
    #[serde(rename = "lyricsProvider", default)]
    pub lyrics_provider: Option<String>,
    #[serde(rename = "lyricsSynced", default)]
    pub lyrics_synced: Option<bool>,
    #[serde(default)]
    pub mapped: bool,
    #[serde(rename = "releaseDate", default)]
    pub release_date: Option<String>,
    #[serde(rename = "artistBio", default)]
    pub artist_bio: Option<String>,
    #[serde(rename = "albumBio", default)]
    pub album_bio: Option<String>,
    /// The old automatic chart, kept only as a translucent guide in the editor.
    #[serde(
        rename = "suggestedNotes",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub suggested_notes: Vec<Note>,
    /// Work in progress for an unmapped song.
    #[serde(rename = "draftNotes", default, skip_serializing_if = "Vec::is_empty")]
    pub draft_notes: Vec<Note>,
    /// Explicit note-to-note ribbon connectors. Older charts use `Note.ribbon`
    /// groups; those are migrated when the editor opens the song.
    #[serde(rename = "ribbonLinks", default, skip_serializing_if = "Vec::is_empty")]
    pub ribbon_links: Vec<RibbonLink>,
    /// Decorative lyric labels, positioned independently of the notes.
    #[serde(rename = "placedWords", default, skip_serializing_if = "Vec::is_empty")]
    pub placed_words: Vec<PlacedWord>,
    /// Words the mapper added with the word-bank + button.
    #[serde(rename = "customWords", default, skip_serializing_if = "Vec::is_empty")]
    pub custom_words: Vec<CustomWord>,
    /// Map files found in the song folder. Populated by the library loader.
    #[serde(skip)]
    pub maps: Vec<MapInfo>,
}

impl Song {
    pub fn duration(&self) -> f64 {
        if self.duration > 0.0 {
            self.duration
        } else {
            self.notes
                .iter()
                .map(|n| n.start + n.dur)
                .fold(0.0, f64::max)
                + 2.0
        }
    }

    pub fn artist_display(&self) -> &str {
        self.artist.as_deref().unwrap_or("Unknown artist")
    }

    pub fn album_display(&self) -> &str {
        self.album.as_deref().unwrap_or(self.artist_display())
    }
}

pub const MAP_FILE_FORMAT: &str = "karaokemachine-level";
pub const MAP_FILE_VERSION: u32 = 1;

/// One selectable mapping file for a song. Both drafts and finished levels
/// use this format; `draft` decides whether the level can be played.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MapFile {
    #[serde(default = "default_map_format")]
    pub format: String,
    #[serde(default = "default_map_version")]
    pub version: u32,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default)]
    pub lines: Vec<LyricLine>,
    #[serde(rename = "placedWords", default, skip_serializing_if = "Vec::is_empty")]
    pub placed_words: Vec<PlacedWord>,
    #[serde(rename = "customWords", default, skip_serializing_if = "Vec::is_empty")]
    pub custom_words: Vec<CustomWord>,
    #[serde(rename = "ribbonLinks", default, skip_serializing_if = "Vec::is_empty")]
    pub ribbon_links: Vec<RibbonLink>,
    #[serde(default)]
    pub low: i32,
    #[serde(default)]
    pub high: i32,
}

fn default_map_format() -> String {
    MAP_FILE_FORMAT.into()
}

fn default_map_version() -> u32 {
    MAP_FILE_VERSION
}

impl MapFile {
    pub fn new_for_song(song: &Song) -> Self {
        Self {
            format: MAP_FILE_FORMAT.into(),
            version: MAP_FILE_VERSION,
            draft: true,
            notes: Vec::new(),
            lines: song.lines.clone(),
            placed_words: Vec::new(),
            custom_words: Vec::new(),
            ribbon_links: Vec::new(),
            low: song.low,
            high: song.high,
        }
    }

    pub fn from_legacy_song(song: &Song) -> Option<Self> {
        let notes = if !song.mapped && !song.draft_notes.is_empty() {
            song.draft_notes.clone()
        } else {
            song.notes.clone()
        };
        if notes.is_empty()
            && song.placed_words.is_empty()
            && song.custom_words.is_empty()
            && song.ribbon_links.is_empty()
        {
            return None;
        }
        Some(Self {
            format: MAP_FILE_FORMAT.into(),
            version: MAP_FILE_VERSION,
            draft: !song.mapped,
            notes,
            lines: song.lines.clone(),
            placed_words: song.placed_words.clone(),
            custom_words: song.custom_words.clone(),
            ribbon_links: song.ribbon_links.clone(),
            low: song.low,
            high: song.high,
        })
    }

    pub fn apply_to_song(&self, song: &mut Song) {
        song.notes = self.notes.clone();
        song.lines = self.lines.clone();
        song.placed_words = self.placed_words.clone();
        song.custom_words = self.custom_words.clone();
        song.ribbon_links = self.ribbon_links.clone();
        song.low = self.low;
        song.high = self.high;
        song.mapped = !self.draft;
        song.draft_notes.clear();
    }
}

/// A map file on disk, as shown in the editor and play selectors.
#[derive(Clone, Debug)]
pub struct MapInfo {
    pub path: PathBuf,
    pub name: String,
    pub draft: bool,
    pub notes: usize,
}

impl MapInfo {
    pub fn display_name(&self) -> String {
        format!(
            "{}  [{} · {} note{}]",
            self.name,
            if self.draft { "draft" } else { "level" },
            self.notes,
            if self.notes == 1 { "" } else { "s" }
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Settings {
    #[serde(default = "default_input_gain")]
    pub input_gain: f32,
    #[serde(default = "default_true")]
    pub count_in: bool,
    #[serde(default = "default_lead")]
    pub lead_time: f32,
    #[serde(default)]
    pub latency_ms: f32,
    #[serde(default = "default_sync")]
    pub sync_ms: f32,
    #[serde(default = "default_backing")]
    pub backing_volume: f32,
    #[serde(default = "default_true")]
    pub backing_enabled: bool,
    #[serde(default)]
    pub vocals_volume: f32,
    #[serde(default = "default_true")]
    pub show_lyrics: bool,
    #[serde(default)]
    pub mic_device: Option<String>,
    #[serde(default)]
    pub vocal_low: Option<i32>,
    #[serde(default)]
    pub vocal_high: Option<i32>,
    #[serde(default)]
    pub octave_shift: i32,
}

fn default_input_gain() -> f32 {
    1.4
}
fn default_true() -> bool {
    true
}
fn default_lead() -> f32 {
    6.0
}
fn default_sync() -> f32 {
    0.0
}
fn default_backing() -> f32 {
    0.9
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            input_gain: default_input_gain(),
            count_in: true,
            lead_time: default_lead(),
            latency_ms: 0.0,
            sync_ms: default_sync(),
            backing_volume: default_backing(),
            backing_enabled: true,
            vocals_volume: 0.0,
            show_lyrics: true,
            mic_device: None,
            vocal_low: None,
            vocal_high: None,
            octave_shift: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScoreRecord {
    pub score: u32,
    pub accuracy: f32,
    pub combo: u32,
}

pub type Scores = BTreeMap<String, ScoreRecord>;

#[derive(Clone, Debug, Default)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub cover_path: Option<std::path::PathBuf>,
    pub warmup: bool,
    pub songs: Vec<usize>,
}

/// The pitch lane the editor shows and the stage lays words out against: a
/// little around `midis`, at least two octaves tall and centred, `fallback`
/// when there are no notes. Word labels are placed as a fraction of this
/// lane, so everything that paints them must use the same formula.
pub fn lane_range(midis: &[i32], fallback: (i32, i32)) -> (i32, i32) {
    let mut low = fallback.0.max(0);
    let mut high = fallback.1.min(127);
    if let (Some(lo), Some(hi)) = (midis.iter().min(), midis.iter().max()) {
        low = *lo - 2;
        high = *hi + 3;
    }
    if high - low < 24 {
        let center = (high + low) as f32 / 2.0;
        low = (center - 12.0).floor() as i32;
        high = (center + 12.0).ceil() as i32;
    }
    let low = low.clamp(0, 108);
    (low, high.clamp(low + 18, 127))
}

/// The lane range a chart needs: a little below and above its notes, and at
/// least an octave tall.
pub fn pitch_range(notes: &[Note]) -> Option<(i32, i32)> {
    let midis = notes.iter().map(|note| note.midi);
    let (lo, hi) = (midis.clone().min()?, midis.max()?);
    let low = (lo - 2).clamp(0, 108);
    let high = (hi + 2).clamp(low + 12, 127);
    Some((low, high))
}

/// Split a lyric line into placeable words with timings: one per `LyricWord`
/// when the line has them, each multi-word entry divided by character weight,
/// otherwise the whole line as one word.
pub fn split_line_words(line: &LyricLine) -> Vec<(String, f64, f64)> {
    let mut source: Vec<(String, f64, f64)> = Vec::new();
    if line.words.is_empty() {
        source.push((
            line.text.clone(),
            line.start,
            line.end.max(line.start + 0.3),
        ));
    } else {
        for word in &line.words {
            source.push((
                word.text.clone(),
                word.start,
                word.end.max(word.start + 0.12),
            ));
        }
    }
    let mut result = Vec::new();
    for (text, start, end) in source {
        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.len() <= 1 {
            if let Some(part) = parts.first() {
                result.push((part.to_string(), start, end));
            }
            continue;
        }
        let weights: Vec<f64> = parts
            .iter()
            .map(|part| (part.chars().count() as f64 + 1.0).max(2.0))
            .collect();
        let total: f64 = weights.iter().sum();
        let span = (end - start).max(0.2);
        let mut cursor = start;
        for (part, weight) in parts.iter().zip(weights.iter()) {
            let duration = span * weight / total;
            result.push((part.to_string(), cursor, cursor + duration));
            cursor += duration;
        }
    }
    if result.is_empty() {
        result.push((
            line.text.clone(),
            line.start,
            line.end.max(line.start + 0.3),
        ));
    }
    result
}
