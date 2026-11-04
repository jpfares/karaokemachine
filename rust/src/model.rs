use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    const NAMES: [&str; 12] = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];
    let n = midi.rem_euclid(12);
    format!("{}{}", NAMES[n as usize], midi / 12 - 1)
}

pub fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    let m = (seconds / 60.0).floor() as i64;
    let s = (seconds % 60.0).floor() as i64;
    format!("{m}:{s:02}")
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
    #[serde(rename = "suggestedNotes", default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_notes: Vec<Note>,
    /// Work in progress for an unmapped song.
    #[serde(rename = "draftNotes", default, skip_serializing_if = "Vec::is_empty")]
    pub draft_notes: Vec<Note>,
    /// Decorative lyric labels, positioned independently of the notes.
    #[serde(rename = "placedWords", default, skip_serializing_if = "Vec::is_empty")]
    pub placed_words: Vec<PlacedWord>,
    /// Words the mapper added with the word-bank + button.
    #[serde(rename = "customWords", default, skip_serializing_if = "Vec::is_empty")]
    pub custom_words: Vec<CustomWord>,
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
    #[serde(default = "default_true")]
    pub show_note_words: bool,
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
            show_note_words: true,
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
