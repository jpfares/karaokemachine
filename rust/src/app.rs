use crate::audio::{input_device_names, AudioEngine, Microphone};
use crate::editor::{spawn_raw_pitch_track, Editor, EditorEvent, VOCAL_BINS_PER_SECOND, RAW_PITCH_SAMPLE_RATE};
use crate::jobs::{Job, StreamJob};
use crate::library::*;
use crate::map_package::{MapIndex, MapIndexEntry, MapPackage};
use base64::Engine;
use crate::model::*;
use crate::pitch::{detect_pitch, PitchReading, PitchTracker};
use crate::theme::{alpha, BG, GOLD, INK, LINE, MINT, MUTED, MUTED2, OCTAVE_BLUE, PINK};
use crate::widgets::{click, glyph_button, pill_button, pill_toggle, round_button};
use eframe::egui::{
    self, Align2, Color32, FontId, Id, Mesh, Pos2, Rect, RichText, Sense, Shape, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Vec2,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const DESIGN_WIDTH: f32 = 1600.0;
const DESIGN_HEIGHT: f32 = 980.0;
const MAPS_BASE_URL: &str = "https://rgbeans.github.io/karaokemachine/maps/";

const OCTAVE_TOLERANCE_CENTS: f32 = 50.0;
const IN_TUNE_TOLERANCE_CENTS: f32 = 85.0;

/// Fold a sung pitch into the octave of the target note.
///
/// Returns the cents error against that octave and the octave shift that was
/// removed. A singer an octave below the chart therefore lands on the right
/// note class with a small cents error, but the shift stays non-zero so the
/// note can be capped below "perfect".
fn fold_octave(midi: f32, target: i32) -> (f32, i32) {
    let shift = ((midi - target as f32) / 12.0).round() as i32;
    let folded = midi - 12.0 * shift as f32;
    ((folded - target as f32) * 100.0, shift)
}

fn same_lyric_word(a: &Note, b: &Note) -> bool {
    if a.word.is_none() || a.word != b.word || a.line != b.line {
        return false;
    }
    match (a.word_index, b.word_index) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

fn pitch_col(t: f32) -> Color32 {
    let [r, g, b] = pitch_color(t);
    Color32::from_rgb(r, g, b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrawerViewRust {
    Albums,
    Imports,
    Maps,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    TogglePlay,
    Restart,
    ToggleLyrics,
    ToggleVocals,
    ToggleBacking,
    OpenMenu,
    CloseMenu,
    OpenSettings,
    SelectAlbum(usize),
    BackToAlbums,
    PlaySong(usize),
    ShowImports,
    ShowAlbums,
    Reload,
    Seek(f64),
    RetrySong,
    OpenLibraryFromResults,
    OpenUpload,
    ShowMaps,
    InstallMap,
    InstallRemoteMap(usize),
    EditSong(usize),
}

#[derive(Clone, Copy, Debug)]
struct NoteStat {
    total: u32,
    hit: u32,
    cents_sum: f64,
    cents_count: u32,
    octave_frames: u32,
    judged: bool,
}

impl Default for NoteStat {
    fn default() -> Self {
        Self {
            total: 0,
            hit: 0,
            cents_sum: 0.0,
            cents_count: 0,
            octave_frames: 0,
            judged: false,
        }
    }
}

#[derive(Clone, Debug)]
struct Judgment {
    text: String,
    color: Color32,
    at: f64,
}

#[derive(Clone, Debug)]
struct ResultData {
    accuracy: f32,
    rank: char,
    perfect: u32,
    hit: u32,
    combo: u32,
    score: u32,
    new_best: bool,
}

struct Stage {
    song: Song,
    duration: f64,
    time: f64,
    playing: bool,
    started: bool,
    count_in_until: Option<f64>,
    score: u32,
    combo: u32,
    best_combo: u32,
    hit_notes: u32,
    perfects: u32,
    total_probe: u32,
    hit_probe: u32,
    finalize_ptr: usize,
    active: i32,
    stats: Vec<NoteStat>,
    judgment: Option<Judgment>,
    result: Option<ResultData>,
    audio_loaded: bool,
    octave_shift: i32,
}

impl Stage {
    fn new(mut song: Song, settings: &Settings) -> Self {
        let shift = settings.octave_shift.clamp(-2, 2);
        if shift != 0 {
            for note in &mut song.notes {
                note.midi += 12 * shift;
            }
        }
        let duration = song.duration().max(1.0);
        let stats = (0..song.notes.len()).map(|_| NoteStat::default()).collect();
        Self {
            song,
            duration,
            time: 0.0,
            playing: false,
            started: false,
            count_in_until: None,
            score: 0,
            combo: 0,
            best_combo: 0,
            hit_notes: 0,
            perfects: 0,
            total_probe: 0,
            hit_probe: 0,
            finalize_ptr: 0,
            active: -1,
            stats,
            judgment: None,
            result: None,
            audio_loaded: false,
            octave_shift: shift,
        }
    }

    fn set_octave_shift(&mut self, shift: i32) {
        let shift = shift.clamp(-2, 2);
        let delta = shift - self.octave_shift;
        if delta == 0 {
            return;
        }
        for note in &mut self.song.notes {
            note.midi += 12 * delta;
        }
        self.octave_shift = shift;
        self.reset_scoring(self.time);
    }

    /// Forget every judgment and score again from `from_time`.
    ///
    /// Notes that end before `from_time` count as already past and are never
    /// judged; everything later is judged fresh. This is the single place
    /// scoring state is rewound, so a restart, a seek and an octave change
    /// cannot drift apart.
    fn reset_scoring(&mut self, from_time: f64) {
        self.score = 0;
        self.combo = 0;
        self.best_combo = 0;
        self.hit_notes = 0;
        self.perfects = 0;
        self.total_probe = 0;
        self.hit_probe = 0;
        self.active = -1;
        self.judgment = None;
        self.result = None;
        for stat in &mut self.stats {
            *stat = NoteStat::default();
        }
        self.finalize_ptr = self
            .song
            .notes
            .iter()
            .position(|note| note.start + note.dur >= from_time)
            .unwrap_or(self.song.notes.len());
    }

    /// Move the playhead and score again from there. Callers seek the audio
    /// engine separately; the stage only owns chart time.
    ///
    /// Ignored while the results card is showing: a stray scrub must not
    /// dismiss a finished run. Sing It Again goes through `reset` instead.
    fn seek(&mut self, time: f64) {
        if self.result.is_some() {
            return;
        }
        self.time = time.clamp(0.0, self.duration);
        self.reset_scoring(self.time);
    }

    fn reset(&mut self) {
        self.time = 0.0;
        self.playing = false;
        self.started = false;
        self.count_in_until = None;
        self.reset_scoring(0.0);
    }

    fn active_note_at(&self, time: f64) -> i32 {
        let notes = &self.song.notes;
        if notes.is_empty() {
            return -1;
        }
        let mut lo = 0usize;
        let mut hi = notes.len() - 1;
        let mut found = None;
        while lo <= hi {
            let mid = (lo + hi) / 2;
            if notes[mid].start <= time + 0.02 {
                found = Some(mid);
                lo = mid + 1;
            } else {
                if mid == 0 {
                    break;
                }
                hi = mid - 1;
            }
        }
        if let Some(index) = found {
            for i in (index.saturating_sub(4)..=index).rev() {
                let note = &notes[i];
                if time >= note.start - 0.03 && time <= note.start + note.dur + 0.03 {
                    return i as i32;
                }
            }
        }
        -1
    }

    fn finish(&mut self, scores: &mut Scores) {
        if self.result.is_some() {
            return;
        }
        while self.finalize_ptr < self.song.notes.len() {
            self.finalize_note(self.finalize_ptr);
            self.finalize_ptr += 1;
        }
        let accuracy = if self.total_probe > 0 {
            self.hit_probe as f32 / self.total_probe as f32 * 100.0
        } else {
            0.0
        };
        let rank = if accuracy >= 92.0 {
            'S'
        } else if accuracy >= 82.0 {
            'A'
        } else if accuracy >= 70.0 {
            'B'
        } else if accuracy >= 55.0 {
            'C'
        } else {
            'D'
        };
        let new_best = save_score(
            scores,
            &self.song.id,
            ScoreRecord {
                score: self.score,
                accuracy,
                combo: self.best_combo,
            },
        );
        self.result = Some(ResultData {
            accuracy,
            rank,
            perfect: self.perfects,
            hit: self.hit_notes,
            combo: self.best_combo,
            score: self.score,
            new_best,
        });
        self.playing = false;
    }

    fn finalize_note(&mut self, index: usize) {
        let Some(stat) = self.stats.get_mut(index) else {
            return;
        };
        if stat.judged {
            return;
        }
        stat.judged = true;
        let ratio = if stat.total > 0 {
            stat.hit as f32 / stat.total as f32
        } else {
            0.0
        };
        let avg_cents = if stat.cents_count > 0 {
            stat.cents_sum / stat.cents_count as f64
        } else {
            99.0
        };
        // A few isolated octave glitches from the tracker should not break a
        // genuinely in-octave perfect note, but singing the whole note in
        // another octave must cap it below perfect.
        let octave_ratio = if stat.hit > 0 {
            stat.octave_frames as f32 / stat.hit as f32
        } else {
            0.0
        };
        if ratio >= 0.84 && avg_cents.abs() <= 38.0 && octave_ratio <= 0.25 {
            self.perfects += 1;
            self.hit_notes += 1;
            self.combo += 1;
            self.score += 160 + self.combo.min(40) * 5;
            self.judgment = Some(Judgment {
                text: "PERFECT".into(),
                color: MINT,
                at: 0.0,
            });
        } else if ratio >= 0.55 {
            self.hit_notes += 1;
            self.combo += 1;
            if octave_ratio > 0.25 {
                self.score += 80 + self.combo.min(30) * 3;
                self.judgment = Some(Judgment {
                    text: "OCTAVE".into(),
                    color: OCTAVE_BLUE,
                    at: 0.0,
                });
            } else {
                self.score += 90 + self.combo.min(30) * 3;
                self.judgment = Some(Judgment {
                    text: "NICE".into(),
                    color: GOLD,
                    at: 0.0,
                });
            }
        } else if ratio >= 0.28 {
            self.combo = 0;
            self.score += 35;
            self.judgment = Some(Judgment {
                text: "CLOSE".into(),
                color: Color32::from_rgb(255, 161, 92),
                at: 0.0,
            });
        } else {
            self.combo = 0;
            self.judgment = Some(Judgment {
                text: "MISS".into(),
                color: PINK,
                at: 0.0,
            });
        }
        self.best_combo = self.best_combo.max(self.combo);
    }

    fn score_tick(&mut self, chart_time: f64, reading: Option<PitchReading>, mic_ok: bool) {
        if !mic_ok {
            return;
        }
        while self.finalize_ptr < self.song.notes.len()
            && self.song.notes[self.finalize_ptr].start + self.song.notes[self.finalize_ptr].dur
                < chart_time - 0.04
        {
            self.finalize_note(self.finalize_ptr);
            self.finalize_ptr += 1;
        }
        let index = self.active_note_at(chart_time);
        self.active = index;
        if index < 0 || index as usize >= self.stats.len() {
            return;
        }
        let stat = &mut self.stats[index as usize];
        stat.total += 1;
        self.total_probe += 1;
        if let Some(reading) = reading {
            if reading.clarity >= 0.24 {
                let target = self.song.notes[index as usize].midi;
                let (cents, octave_shift) = fold_octave(reading.midi, target);
                let tolerance = if octave_shift != 0 {
                    OCTAVE_TOLERANCE_CENTS
                } else {
                    IN_TUNE_TOLERANCE_CENTS
                };
                if cents.abs() <= tolerance {
                    stat.hit += 1;
                    stat.cents_sum += cents as f64;
                    stat.cents_count += 1;
                    if octave_shift != 0 {
                        stat.octave_frames += 1;
                    }
                    self.hit_probe += 1;
                }
            }
        }
    }
}

struct Layout {
    header_h: f32,
    tray_h: f32,
    left_w: f32,
    hit_x: f32,
    lane_top: f32,
    lane_bottom: f32,
    lane_h: f32,
    low: i32,
    high: i32,
    pps: f32,
}

impl Layout {
    fn new(size: Vec2, song: &Song, settings: &Settings) -> Self {
        let width = size.x;
        let height = size.y;
        let header_h = 64.0;
        let tray_h = 104.0;
        let left_w = (width * 0.07).clamp(96.0, 132.0);
        let hit_x = left_w + 2.0;
        let lane_top = header_h + 20.0;
        let lane_bottom = height - tray_h - 18.0;
        // Fit the lanes to this song's actual melody, not a fixed window.
        // The extra lanes above the top note leave room for its lyric label.
        let mut midis: Vec<i32> = song.notes.iter().map(|n| n.midi).collect();
        midis.sort_unstable();
        let (lowest, highest) = if let (Some(lowest), Some(highest)) = (midis.first(), midis.last()) {
            (*lowest, *highest)
        } else {
            (song.low, song.high)
        };
        let mut low = lowest - 2;
        let mut high = highest + 3;
        if high - low > 58 && !midis.is_empty() {
            // Ignore a lone octave-jump artifact rather than shrinking the
            // entire melody to make room for it.
            let p = |q: f32| -> i32 {
                let idx = ((midis.len() as f32 * q) as usize).min(midis.len().saturating_sub(1));
                midis.get(idx).copied().unwrap_or(60)
            };
            low = p(0.01) - 2;
            high = p(0.99) + 3;
        }
        if high - low < 24 {
            let center = (high + low) as f32 / 2.0;
            low = (center - 12.0).floor() as i32;
            high = (center + 12.0).ceil() as i32;
        }
        low = low.clamp(0, 109);
        high = high.clamp(low + 18, 127);
        let lane_h = (lane_bottom - lane_top) / (high - low + 1) as f32;
        let pps = (width - hit_x - 26.0) / settings.lead_time.max(3.5);
        Self {
            header_h,
            tray_h,
            left_w,
            hit_x,
            lane_top,
            lane_bottom,
            lane_h,
            low,
            high,
            pps,
        }
    }

    fn lane_y(&self, midi: i32) -> f32 {
        self.lane_top + (self.high - midi) as f32 * self.lane_h + self.lane_h / 2.0
    }

    fn note_height(&self) -> f32 {
        (self.lane_h * 0.62).clamp(7.0, 22.0)
    }

    fn y_norm(&self, y: f32) -> f32 {
        ((y - self.lane_top) / (self.lane_bottom - self.lane_top).max(1.0)).clamp(0.0, 1.0)
    }
}

struct Toast {
    message: String,
    until: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct TrackInfo {
    #[serde(rename = "audioUrl", default)]
    audio_url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    album: Option<String>,
    #[serde(rename = "releaseDate", default)]
    release_date: Option<String>,
    #[serde(default)]
    cover: Option<String>,
    #[serde(rename = "artistBio", default)]
    artist_bio: Option<String>,
    #[serde(rename = "albumBio", default)]
    album_bio: Option<String>,
    #[serde(default)]
    lyrics: Option<String>,
    #[serde(rename = "lyricsProvider", default)]
    lyrics_provider: Option<String>,
    #[serde(rename = "lyricsSynced", default)]
    lyrics_synced: bool,
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    error: Option<String>,
}

enum ImportMessage {
    Progress(f32, String, String),
    Done(Result<String, String>),
}

#[derive(Default)]
struct UploadState {
    open: bool,
    url: String,
    title: String,
    artist: String,
    album: String,
    release_date: String,
    cover: String,
    artist_bio: String,
    album_bio: String,
    lyrics: String,
    audio_url: String,
    lyrics_provider: Option<String>,
    lyrics_synced: bool,
    duration: f64,
    info_loaded: bool,
    progress: f32,
    stage: String,
    message: String,
    artist_search: String,
    album_search: String,
    cover_popup_open: bool,
    cover_link: String,
    cover_preview_path: Option<PathBuf>,
    cover_preview_tex: Option<TextureHandle>,
    cover_preview_loaded: Option<PathBuf>,
    cover_fetch: Option<Job<PathBuf>>,
    cover_message: String,
    cover_error: bool,
    pending_map: Option<MapPackage>,
    fetch: Option<Job<TrackInfo>>,
    import: Option<StreamJob<ImportMessage>>,
}

impl UploadState {
    fn fetching(&self) -> bool {
        self.fetch.is_some()
    }

    fn importing(&self) -> bool {
        self.import.is_some()
    }

    fn cover_fetching(&self) -> bool {
        self.cover_fetch.is_some()
    }
}

pub struct KaraokeMachineApp {
    /// Cloned into every background job so a finished job can repaint.
    ctx: egui::Context,
    root: PathBuf,
    songs: Vec<Song>,
    albums: Vec<Album>,
    settings: Settings,
    scores: Scores,
    covers: HashMap<String, TextureHandle>,
    cover_paths: HashMap<String, PathBuf>,
    audio: Option<AudioEngine>,
    mic: Option<Microphone>,
    mic_error: Option<String>,
    mic_devices: Vec<String>,
    tracker: PitchTracker,
    reading: Option<PitchReading>,
    drawer_open: bool,
    drawer_view: DrawerViewRust,
    selected_album: Option<usize>,
    drawer_scroll: f32,
    settings_open: bool,
    range_test_open: bool,
    range_testing: bool,
    range_low_seen: Option<i32>,
    range_high_seen: Option<i32>,
    range_last_note: Option<i32>,
    range_note_frames: u32,
    upload: UploadState,
    map_index: Vec<MapIndexEntry>,
    map_index_job: Option<Job<MapIndex>>,
    map_index_loaded: bool,
    map_covers: HashMap<String, TextureHandle>,
    /// The id of the map being downloaded, and the download itself.
    map_download: Option<(String, Job<MapPackage>)>,
    stage: Option<Stage>,
    editor: Option<Editor>,
    songlist: Vec<String>,
    songlist_path: String,
    toast: Option<Toast>,
    started_at: f64,
    pending_song: Option<usize>,
    pending_time: f64,
    fake_pitch: Option<f32>,
    no_mic: bool,
    shot_path: Option<PathBuf>,
    shot_at: f64,
    shot_requested: bool,
    auto_play: bool,
    fps_log: bool,
    fps_frames: u32,
    fps_last: f64,
}

/// Keep the interface visually the same size at any window size. egui's
/// native DPI scaling is bypassed by the forced 1x X11 scale, so derive a
/// zoom from the native window size against the 1600x980 design canvas.
fn apply_ui_scale(ctx: &egui::Context) {
    // screen_rect and pixels_per_point are updated in the same pass, so their
    // product is a stable physical window size. Reading the point size and
    // multiplying by the zoom (or using viewport().inner_rect) lags one frame
    // behind a zoom change and makes the UI oscillate between two scales.
    let physical = ctx.screen_rect().size() * ctx.pixels_per_point();
    if physical.x < 1.0 || physical.y < 1.0 {
        return;
    }
    let native = ctx.native_pixels_per_point().unwrap_or(1.0).max(1.0);
    let current = ctx.zoom_factor();
    let target = (physical.x / (DESIGN_WIDTH * native))
        .min(physical.y / (DESIGN_HEIGHT * native))
        .clamp(1.0, 4.0);
    let target = (target * 100.0).round() / 100.0;
    if (target - current).abs() > 0.01 {
        ctx.set_zoom_factor(target);
    }
}

impl KaraokeMachineApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let root = find_root();
        let songs = load_songs(&root);
        let albums = build_albums(&songs);
        let mut settings = load_settings();
        if settings.backing_volume <= 0.01 {
            settings.backing_enabled = false;
        }
        let scores = load_scores();
        let songlist = read_songlist();
        let songlist_path = read_songlist_path();
        let pending_song = setting_env("SONG").and_then(|needle| {
            let needle = needle.to_lowercase();
            songs.iter().position(|s| s.id.to_lowercase() == needle || s.title.to_lowercase().contains(&needle))
        });
        let mut app = Self {
            ctx: cc.egui_ctx.clone(),
            root,
            songs,
            albums,
            settings,
            scores,
            covers: HashMap::new(),
            cover_paths: HashMap::new(),
            audio: match AudioEngine::new(0.9, 0.0) {
                Ok(audio) => Some(audio),
                Err(error) => {
                    eprintln!("audio engine unavailable: {error}");
                    None
                }
            },
            mic: None,
            mic_error: None,
            mic_devices: input_device_names(),
            tracker: PitchTracker::default(),
            reading: None,
            drawer_open: true,
            drawer_view: DrawerViewRust::Albums,
            selected_album: None,
            drawer_scroll: 0.0,
            settings_open: false,
            range_test_open: false,
            range_testing: false,
            range_low_seen: None,
            range_high_seen: None,
            range_last_note: None,
            range_note_frames: 0,
            upload: UploadState::default(),
            map_index: Vec::new(),
            map_index_job: None,
            map_index_loaded: false,
            map_covers: HashMap::new(),
            map_download: None,
            stage: None,
            editor: None,
            songlist,
            songlist_path,
            toast: None,
            started_at: 0.0,
            pending_song,
            pending_time: setting_env("T").and_then(|v| v.parse().ok()).unwrap_or(0.0),
            fake_pitch: setting_env("PITCH").and_then(|v| v.parse().ok()),
            no_mic: setting_env("NOMIC").is_some(),
            shot_path: setting_env("SHOT").map(PathBuf::from),
            shot_at: setting_env("SHOT_DELAY").and_then(|v| v.parse().ok()).unwrap_or(2.2),
            shot_requested: false,
            auto_play: setting_env("AUTOPLAY").is_some(),
            fps_log: setting_env("FPS").is_some(),
            fps_frames: 0,
            fps_last: 0.0,
        };
        app.apply_style(&cc.egui_ctx);
        app.preload_covers(&cc.egui_ctx);
        if setting_env("SETTINGS").is_some() {
            app.settings_open = true;
        }
        if setting_env("UPLOAD").is_some() {
            app.upload.open = true;
            app.drawer_open = true;
            app.drawer_view = DrawerViewRust::Imports;
            app.selected_album = None;
        }
        if let Some(url) = setting_env("UPLOAD_URL") {
            app.upload.url = url;
        }
        if setting_env("UPLOAD_FETCH").is_some() && !app.upload.url.trim().is_empty() {
            app.start_fetch(0.0);
        }
        if setting_env("COVER_POPUP").is_some() {
            app.upload.open = true;
            app.upload.cover_popup_open = true;
        }
        match setting_env("DRAWER").as_deref() {
            Some("imports") => {
                app.drawer_view = DrawerViewRust::Imports;
                app.drawer_open = true;
            }
            Some("maps") => {
                app.drawer_view = DrawerViewRust::Maps;
                app.drawer_open = true;
                app.start_map_index_fetch();
            }
            Some("album") => {
                if let Ok(index) = setting_env("ALBUM").unwrap_or_default().parse::<usize>() {
                    app.selected_album = Some(index);
                }
                app.drawer_open = true;
            }
            _ => {}
        }
        app
    }

    fn apply_style(&self, ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = Color32::from_rgb(9, 13, 23);
        visuals.extreme_bg_color = Color32::from_rgb(15, 21, 34);
        visuals.faint_bg_color = Color32::from_rgb(12, 17, 29);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0f32, Color32::from_white_alpha(38));
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0f32, Color32::from_rgba_unmultiplied(124, 247, 196, 120));
        visuals.widgets.active.bg_stroke = Stroke::new(1.0f32, Color32::from_rgba_unmultiplied(124, 247, 196, 180));
        visuals.override_text_color = Some(INK);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(15, 21, 34);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(24, 32, 49);
        visuals.widgets.active.bg_fill = Color32::from_rgb(31, 42, 61);
        visuals.selection.bg_fill = alpha(MINT, 50);
        ctx.set_visuals(visuals);
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(10.0, 9.0);
        style.spacing.button_padding = Vec2::new(14.0, 8.0);
        ctx.set_style(style);
    }

    fn preload_covers(&mut self, ctx: &egui::Context) {
        for song in &self.songs {
            if let Some(cover) = &song.cover {
                let path = self.root.join("assets/songs").join(&song.id).join(cover);
                self.cover_paths.insert(song.id.clone(), path);
            }
        }
        for (id, path) in self.cover_paths.clone() {
            if let Some(texture) = load_texture(ctx, &path) {
                self.covers.insert(id, texture);
            }
        }
    }

    fn toast(&mut self, message: impl Into<String>, now: f64) {
        self.toast = Some(Toast {
            message: message.into(),
            until: now + 3.4,
        });
    }

    fn reload_library(&mut self, ctx: &egui::Context, _now: f64) {
        self.songs = load_songs(&self.root);
        self.albums = build_albums(&self.songs);
        self.songlist = read_songlist();
        self.cover_paths.clear();
        self.covers.clear();
        self.preload_covers(ctx);
    }

    fn start_mic(&mut self) {
        match Microphone::new(self.settings.mic_device.as_deref()) {
            Ok(mic) => {
                self.mic = Some(mic);
                self.mic_error = None;
                self.tracker.reset();
            }
            Err(error) => {
                self.mic = None;
                self.mic_error = Some(error.to_string());
            }
        }
    }

    fn start_range_test(&mut self) {
        if self.mic.is_none() && !self.no_mic {
            self.start_mic();
        }
        self.range_test_open = true;
        self.range_testing = true;
        self.range_low_seen = None;
        self.range_high_seen = None;
        self.range_last_note = None;
        self.range_note_frames = 0;
    }

    fn update_range_test(&mut self) {
        if !self.range_test_open || !self.range_testing {
            return;
        }
        let Some(reading) = self.reading else {
            self.range_last_note = None;
            self.range_note_frames = 0;
            return;
        };
        if reading.clarity < 0.4 {
            return;
        }
        let note = reading.midi.round() as i32;
        if self.range_last_note == Some(note) {
            self.range_note_frames += 1;
        } else {
            self.range_last_note = Some(note);
            self.range_note_frames = 1;
        }
        if self.range_note_frames >= 4 {
            self.range_low_seen = Some(self.range_low_seen.map_or(note, |low| low.min(note)));
            self.range_high_seen = Some(self.range_high_seen.map_or(note, |high| high.max(note)));
        }
    }

    fn apply_octave_shift(&mut self) {
        self.settings.octave_shift = self.settings.octave_shift.clamp(-2, 2);
        if let Some(stage) = &mut self.stage {
            stage.set_octave_shift(self.settings.octave_shift);
        }
        save_settings(&self.settings);
    }

    fn fit_current_song_to_range(&mut self) {
        let (Some(range_low), Some(range_high), Some(stage)) = (
            self.settings.vocal_low,
            self.settings.vocal_high,
            self.stage.as_ref(),
        ) else {
            return;
        };
        if stage.song.notes.is_empty() {
            return;
        }
        let shift_now = 12 * stage.octave_shift;
        let low = stage.song.notes.iter().map(|note| note.midi - shift_now).min().unwrap_or(60);
        let high = stage.song.notes.iter().map(|note| note.midi - shift_now).max().unwrap_or(60);
        let mut best = 0i32;
        let mut best_penalty = i32::MAX;
        for shift in -2..=2 {
            let clipped_low = range_low - (low + 12 * shift);
            let clipped_high = (high + 12 * shift) - range_high;
            let penalty = clipped_low.max(0) + clipped_high.max(0);
            if penalty < best_penalty || (penalty == best_penalty && shift.abs() < best.abs()) {
                best_penalty = penalty;
                best = shift;
            }
        }
        self.settings.octave_shift = best;
        self.apply_octave_shift();
    }

    fn backing_level(&self) -> f32 {
        if self.settings.backing_enabled {
            self.settings.backing_volume
        } else {
            0.0
        }
    }

    fn apply_audio_volumes(&self) {
        if let Some(audio) = &self.audio {
            audio.set_volumes(self.backing_level(), self.settings.vocals_volume);
        }
    }

    fn open_editor(&mut self, index: usize, now: f64) {
        let Some(song) = self.songs.get(index).cloned() else {
            return;
        };
        self.audio.as_mut().map(|audio| audio.stop());
        self.stage = None;
        self.drawer_open = false;
        self.settings_open = false;
        self.range_test_open = false;
        self.range_testing = false;
        let suggestions = if !song.suggested_notes.is_empty() {
            song.suggested_notes.clone()
        } else if !song.mapped {
            song.notes.clone()
        } else {
            Vec::new()
        };
        let initial = if !song.mapped {
            if !song.draft_notes.is_empty() {
                song.draft_notes.clone()
            } else {
                Vec::new()
            }
        } else {
            song.notes.clone()
        };
        let duration = song.duration().max(1.0);
        let backing = song_audio_path(&self.root, &song, "backing");
        let vocals = song_audio_path(&self.root, &song, "vocals");
        let mut editor = Editor::new(song, suggestions, initial, duration);
        if let Some(audio) = &self.audio {
            audio.set_volumes(0.9, 0.85);
            audio.set_rate(1.0);
            match audio.load(&backing, Some(&vocals)) {
                Ok(duration) => {
                    editor.set_audio_ready(duration > 0.0);
                    editor.set_vocal_peaks(audio.vocal_peaks(VOCAL_BINS_PER_SECOND));
                    let mono = audio.vocal_mono(RAW_PITCH_SAMPLE_RATE);
                    if !mono.is_empty() {
                        editor.set_pitch_job(spawn_raw_pitch_track(
                            &self.ctx,
                            mono,
                            VOCAL_BINS_PER_SECOND,
                        ));
                    }
                }
                Err(error) => {
                    self.toast(format!("Audio preview unavailable: {error}"), now);
                }
            }
        }
        self.editor = Some(editor);
        self.started_at = now;
        self.tracker.reset();
        self.reading = None;
    }

    fn open_song(&mut self, index: usize, now: f64) {
        let Some(song) = self.songs.get(index).cloned() else {
            return;
        };
        if !song.mapped {
            self.open_editor(index, now);
            return;
        }
        self.editor = None;
        self.audio.as_mut().map(|audio| audio.stop());
        if let Some(audio) = &self.audio {
            audio.set_rate(1.0);
        }
        if self.mic.is_none() && !self.no_mic {
            self.start_mic();
        }
        let backing = song_audio_path(&self.root, &song, "backing");
        let vocals = song_audio_path(&self.root, &song, "vocals");
        let mut stage = Stage::new(song, &self.settings);
        if let Some(audio) = &self.audio {
            audio.set_volumes(self.backing_level(), self.settings.vocals_volume);
            match audio.load(&backing, Some(&vocals)) {
                Ok(duration) => {
                    stage.duration = duration.max(stage.duration);
                    stage.audio_loaded = true;
                }
                Err(error) => {
                    self.toast(format!("Could not load audio: {error}"), now);
                }
            }
        }
        self.stage = Some(stage);
        self.drawer_open = false;
        self.settings_open = false;
        self.started_at = now;
        self.tracker.reset();
        self.reading = None;
    }

    fn toggle_play(&mut self, now: f64) {
        let Some(stage) = &mut self.stage else {
            return;
        };
        if stage.result.is_some() {
            return;
        }
        if stage.count_in_until.is_some() {
            stage.count_in_until = None;
            return;
        }
        if stage.playing {
            if let Some(audio) = &self.audio {
                audio.pause();
            }
            stage.playing = false;
            return;
        }
        if stage.time >= stage.duration - 0.1 {
            self.restart_song(now);
            return;
        }
        if !stage.started && self.settings.count_in {
            stage.count_in_until = Some(now + 3.2);
            stage.playing = false;
            return;
        }
        if stage.audio_loaded {
            if let Some(audio) = &self.audio {
                audio.play();
            }
            stage.playing = true;
            stage.started = true;
        } else {
            stage.playing = true;
            stage.started = true;
            self.toast("No audio output available; chart only.", now);
        }
    }

    fn restart_song(&mut self, now: f64) {
        if let Some(stage) = &mut self.stage {
            stage.reset();
            stage.time = 0.0;
            if let Some(audio) = &self.audio {
                audio.stop();
            }
            self.tracker.reset();
            self.reading = None;
            self.started_at = now;
            if self.settings.count_in {
                stage.count_in_until = Some(now + 3.2);
            } else {
                self.toggle_play(now);
            }
        }
    }

    fn toggle_lyrics(&mut self) {
        self.settings.show_lyrics = !self.settings.show_lyrics;
        save_settings(&self.settings);
    }

    fn toggle_vocals(&mut self) {
        self.settings.vocals_volume = if self.settings.vocals_volume > 0.01 {
            0.0
        } else {
            0.8
        };
        self.apply_audio_volumes();
        save_settings(&self.settings);
    }

    fn toggle_backing(&mut self) {
        self.settings.backing_enabled = !self.settings.backing_enabled;
        if self.settings.backing_enabled && self.settings.backing_volume <= 0.01 {
            self.settings.backing_volume = 0.9;
        }
        self.apply_audio_volumes();
        save_settings(&self.settings);
    }

    fn open_menu(&mut self) {
        if let Some(stage) = &mut self.stage {
            if stage.playing {
                if let Some(audio) = &self.audio {
                    audio.pause();
                }
                stage.playing = false;
            }
        }
        self.drawer_view = DrawerViewRust::Albums;
        self.selected_album = None;
        self.drawer_open = true;
    }

    fn handle_action(&mut self, action: Action, now: f64, ctx: &egui::Context) {
        match action {
            Action::TogglePlay => self.toggle_play(now),
            Action::Restart => self.restart_song(now),
            Action::ToggleLyrics => self.toggle_lyrics(),
            Action::ToggleVocals => self.toggle_vocals(),
            Action::ToggleBacking => self.toggle_backing(),
            Action::OpenMenu => self.open_menu(),
            Action::CloseMenu => {
                self.drawer_open = false;
                self.selected_album = None;
            }
            Action::OpenSettings => {
                self.mic_devices = input_device_names();
                self.settings_open = true;
            }
            Action::SelectAlbum(index) => {
                self.selected_album = Some(index);
                self.drawer_scroll = 0.0;
            }
            Action::BackToAlbums => {
                self.selected_album = None;
                self.drawer_scroll = 0.0;
            }
            Action::PlaySong(index) => self.open_song(index, now),
            Action::ShowImports => {
                self.drawer_view = DrawerViewRust::Imports;
                self.selected_album = None;
                self.drawer_scroll = 0.0;
            }
            Action::ShowAlbums => {
                self.drawer_view = DrawerViewRust::Albums;
                self.selected_album = None;
                self.drawer_scroll = 0.0;
            }
            Action::Reload => {
                self.reload_library(ctx, now);
                self.toast("Library reloaded.", now);
            }
            Action::Seek(time) => {
                if let Some(stage) = &mut self.stage {
                    stage.seek(time);
                    if let Some(audio) = &self.audio {
                        audio.seek(stage.time);
                    }
                }
            }
            Action::RetrySong => self.restart_song(now),
            Action::OpenLibraryFromResults => {
                self.open_menu();
            }
            Action::OpenUpload => {
                self.upload.open = true;
                self.upload.message.clear();
                self.drawer_open = true;
                self.drawer_view = DrawerViewRust::Imports;
                self.selected_album = None;
            }
            Action::ShowMaps => {
                self.drawer_view = DrawerViewRust::Maps;
                self.selected_album = None;
                self.drawer_scroll = 0.0;
                self.start_map_index_fetch();
            }
            Action::InstallMap => self.install_map_file(now, ctx),
            Action::InstallRemoteMap(index) => {
                if let Some(entry) = self.map_index.get(index).cloned() {
                    self.start_remote_map_install(entry);
                }
            }
            Action::EditSong(index) => {
                self.drawer_open = false;
                self.open_editor(index, now);
            }
        }
    }

    fn tick_stage(&mut self, ctx: &egui::Context, now: f64) {
        if self.editor.is_some() {
            return;
        }
        let Some(stage) = &mut self.stage else {
            return;
        };
        if let Some(until) = stage.count_in_until {
            if now >= until {
                stage.count_in_until = None;
                if let Some(audio) = &self.audio {
                    if stage.audio_loaded {
                        audio.play();
                    }
                }
                stage.playing = true;
                stage.started = true;
                stage.time = 0.0;
            }
        }
        if stage.playing {
            if let Some(audio) = &self.audio {
                if stage.audio_loaded {
                    stage.time = audio.position();
                } else {
                    stage.time += 1.0 / 60.0;
                }
            } else {
                stage.time += 1.0 / 60.0;
            }
            if stage.time >= stage.duration - 0.04 {
                stage.finish(&mut self.scores);
                if let Some(audio) = &self.audio {
                    audio.pause();
                }
            }
        }
        let chart_time = stage.time - self.settings.sync_ms as f64 / 1000.0;
        if stage.playing {
            let mic_ok = self.mic.is_some() || self.fake_pitch.is_some();
            stage.score_tick(chart_time, self.reading, mic_ok);
        } else {
            stage.active = stage.active_note_at(chart_time);
        }
        if let Some(judgment) = &mut stage.judgment {
            if judgment.at == 0.0 {
                judgment.at = now;
            }
        }
        if stage.playing || stage.count_in_until.is_some() || self.mic.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn update_pitch(&mut self) {
        if let Some(midi) = self.fake_pitch {
            self.reading = Some(PitchReading {
                midi,
                cents: (midi - midi.round()) * 100.0,
                clarity: 1.0,
                rms: 0.2,
            });
            return;
        }
        if let Some(mic) = &self.mic {
            let samples = mic.latest(2048);
            if samples.len() >= 1024 {
                let raw = detect_pitch(&samples, mic.sample_rate as f32);
                if let Some(reading) = raw {
                    let capturing = self.range_test_open || stage_needs_input(&self.stage);
                    if reading.clarity >= 0.18 && capturing {
                        self.reading = self.tracker.push(Some(reading));
                    } else {
                        self.reading = None;
                    }
                } else {
                    self.reading = None;
                }
            } else {
                self.reading = None;
            }
        } else {
            self.reading = None;
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context, now: f64) {
        if self.editor.is_some() {
            return;
        }
        if self.upload.open {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !self.upload.importing() {
                self.upload.open = false;
            }
            return;
        }
        if self.settings_open {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.settings_open = false;
            }
            return;
        }
        if self.drawer_open {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.drawer_open = false;
                self.selected_album = None;
            }
            return;
        }
        if self.stage.is_none() {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.drawer_open = true;
            }
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_play(now);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::R)) {
            self.restart_song(now);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::L)) {
            self.toggle_lyrics();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::V)) {
            self.toggle_vocals();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::B)) {
            self.toggle_backing();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.open_menu();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            if let Some(stage) = &mut self.stage {
                stage.seek(stage.time - 5.0);
                if let Some(audio) = &self.audio {
                    audio.seek(stage.time);
                }
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            if let Some(stage) = &mut self.stage {
                stage.seek(stage.time + 5.0);
                if let Some(audio) = &self.audio {
                    audio.seek(stage.time);
                }
            }
        }
    }

    fn draw_lobby(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let rect = ui.max_rect();
        let p = ui.painter().clone();
        p.rect_filled(rect, 0.0, BG);
        let left_w = 98.0;
        let top = 74.0;
        let low = 52;
        let high = 80;
        let lane_h = (rect.height() - top - 30.0) / (high - low + 1) as f32;
        let lane_y = |midi: i32| top + (high - midi) as f32 * lane_h + lane_h / 2.0;
        for i in 0..=(high - low) {
            let y = top + i as f32 * lane_h;
            let strong = (high - i) % 12 == 0;
            p.line_segment(
                [Pos2::new(0.0, y), Pos2::new(rect.width(), y)],
                Stroke::new(if strong { 1.0f32 } else { 0.6f32 }, Color32::from_white_alpha(if strong { 22 } else { 9 })),
            );
        }
        p.rect_filled(Rect::from_min_size(Pos2::ZERO, Vec2::new(left_w, rect.height())), 0.0, Color32::from_rgba_unmultiplied(5, 8, 15, 235));
        p.line_segment(
            [Pos2::new(left_w, top), Pos2::new(left_w, rect.height() - 30.0)],
            Stroke::new(1.0f32, alpha(MINT, 80)),
        );
        for midi in [55, 60, 64, 67, 72, 76] {
            let y = lane_y(midi);
            p.text(
                Pos2::new(left_w - 9.0, y),
                Align2::RIGHT_CENTER,
                note_name(midi),
                FontId::monospace(9.0),
                alpha(INK, 60),
            );
        }
        let points: Vec<Pos2> = (0..=180)
            .map(|i| {
                let t = i as f32 / 180.0;
                let midi = 64.0
                    + (t * std::f32::consts::TAU * 2.2).sin() * 7.0
                    + (t * std::f32::consts::TAU * 5.0 + 0.7).sin() * 2.3;
                let y = top + (high as f32 - midi + 0.5) * lane_h;
                Pos2::new(left_w + 26.0 + t * (rect.width() - left_w - 56.0), y)
            })
            .collect();
        let mut mesh = Mesh::default();
        add_ribbon_mesh(&mut mesh, &points, 24.0, low, high, top, rect.height() - 30.0, 1.0);
        p.add(Shape::mesh(mesh));
        p.text(Pos2::new(28.0, 30.0), Align2::LEFT_CENTER, "KARAOKEMACHINE", FontId::proportional(18.0), alpha(INK, 200));
        p.text(Pos2::new(28.0, 50.0), Align2::LEFT_CENTER, "MUSIC LIBRARY", FontId::monospace(8.0), MUTED2);
        let brand = Rect::from_min_size(Pos2::new(rect.width() - 170.0, 22.0), Vec2::new(140.0, 34.0));
        if pill_button(ui, brand, "LIBRARY", true) {
            actions.push(Action::OpenMenu);
        }
        if !self.drawer_open {
            let center = Rect::from_center_size(rect.center() + Vec2::new(-100.0, 0.0), Vec2::new(220.0, 46.0));
            if pill_button(ui, center, "OPEN LIBRARY", true) {
                actions.push(Action::OpenMenu);
            }
        }
    }

    fn draw_stage(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>, now: f64) {
        let Some(stage) = &self.stage else {
            return;
        };
        let rect = ui.max_rect();
        let layout = Layout::new(rect.size(), &stage.song, &self.settings);
        let chart_time = stage.time - self.settings.sync_ms as f64 / 1000.0;
        let p = ui.painter().clone();
        p.rect_filled(rect, 0.0, BG);
        // header
        let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), layout.header_h));
        p.rect_filled(header, 0.0, Color32::from_rgba_unmultiplied(5, 8, 15, 242));
        p.line_segment([header.left_bottom(), header.right_bottom()], Stroke::new(1.0f32, LINE));
        let back = Rect::from_min_size(Pos2::new(16.0, 15.0), Vec2::new(36.0, 34.0));
        if glyph_button(ui, back, "BACK") {
            actions.push(Action::OpenMenu);
        }
        p.text(Pos2::new(66.0, 17.0), Align2::LEFT_TOP, "NOW SINGING", FontId::monospace(8.0), MUTED2);
        p.text(Pos2::new(66.0, 30.0), Align2::LEFT_TOP, &stage.song.title, FontId::proportional(20.0), INK);
        p.text(
            Pos2::new(66.0, 50.0),
            Align2::LEFT_TOP,
            format!("{}   ·   {}", stage.song.artist_display(), stage.song.tags.join(" · ")),
            FontId::proportional(10.0),
            MUTED,
        );
        let accuracy = if stage.total_probe > 0 {
            (stage.hit_probe as f32 / stage.total_probe as f32 * 100.0).round() as i32
        } else {
            0
        };
        let stats_x = rect.width() - 360.0;
        for (i, (label, value, color)) in [
            ("SCORE", stage.score.to_string(), INK),
            ("COMBO", format!("{}×", stage.combo), GOLD),
            ("ACCURACY", if stage.total_probe > 0 { format!("{accuracy}%") } else { "—".into() }, MINT),
        ]
        .iter()
        .enumerate()
        {
            let x = stats_x + i as f32 * 78.0;
            p.text(Pos2::new(x, 15.0), Align2::LEFT_TOP, *label, FontId::monospace(7.0), MUTED2);
            p.text(Pos2::new(x, 28.0), Align2::LEFT_TOP, value, FontId::monospace(16.0), *color);
        }
        let settings_btn = Rect::from_min_size(Pos2::new(rect.width() - 50.0, 14.0), Vec2::new(34.0, 34.0));
        if glyph_button(ui, settings_btn, "SET") {
            actions.push(Action::OpenSettings);
        }

        // lanes
        let clip = Rect::from_min_max(
            Pos2::new(layout.hit_x, layout.lane_top),
            Pos2::new(rect.width(), layout.lane_bottom),
        );
        let painter = ui.painter().with_clip_rect(clip);
        for midi in layout.low..=layout.high {
            let top = layout.lane_top + (layout.high - midi) as f32 * layout.lane_h;
            let y = top + layout.lane_h;
            let black = [1, 3, 6, 8, 10].contains(&(((midi % 12) + 12) % 12));
            if black {
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(layout.hit_x, top), Vec2::new(rect.width() - layout.hit_x, layout.lane_h)),
                    0.0,
                    Color32::from_white_alpha(9),
                );
            }
            let is_c = midi % 12 == 0;
            painter.line_segment(
                [Pos2::new(layout.hit_x, y), Pos2::new(rect.width(), y)],
                Stroke::new(if is_c { 1.2f32 } else { 0.7f32 }, Color32::from_white_alpha(if is_c { 44 } else { 17 })),
            );
        }
        // grid
        let interval = stage.song.bpm.map(|bpm| 60.0 / bpm).unwrap_or(1.0);
        let first = (chart_time / interval).floor() as i64 - 1;
        let last = ((chart_time + self.settings.lead_time as f64 + 2.0) / interval).ceil() as i64 + 1;
        for tick in first..=last {
            let t = tick as f64 * interval;
            let x = layout.hit_x + (t - chart_time) as f32 * layout.pps;
            if x < layout.hit_x || x > rect.width() {
                continue;
            }
            let bar = stage.song.bpm.map(|_| tick % 4 == 0).unwrap_or((t.round() as i64) % 5 == 0);
            if stage.song.bpm.is_none() && !bar {
                continue;
            }
            painter.line_segment(
                [Pos2::new(x, layout.lane_top), Pos2::new(x, layout.lane_bottom)],
                Stroke::new(if bar { 1.2f32 } else { 0.7f32 }, Color32::from_white_alpha(if bar { 30 } else { 13 })),
            );
        }
        let active = stage.active_note_at(chart_time);
        if active >= 0 {
            let note = &stage.song.notes[active as usize];
            let y = layout.lane_y(note.midi);
            let color = pitch_col(layout.y_norm(y));
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(layout.hit_x, y - layout.lane_h / 2.0),
                    Vec2::new((rect.width() - layout.hit_x).min(430.0), layout.lane_h),
                ),
                0.0,
                alpha(color, 28),
            );
        }

        // melody
        draw_melody(&painter, stage, &layout, active, chart_time, rect.width());
        // Decorative lyrics. Older mapped charts only have words attached to
        // notes, so fall back to the legacy renderer for those.
        if self.settings.show_lyrics {
            if stage.song.placed_words.is_empty() {
                draw_words(&painter, stage, &layout, chart_time, rect.width());
            } else {
                draw_placed_words(&painter, stage, &layout, chart_time, rect.width());
            }
        }

        // left rail labels
        let rail = Rect::from_min_size(Pos2::new(0.0, layout.header_h), Vec2::new(layout.left_w, rect.height() - layout.tray_h - layout.header_h));
        p.rect_filled(rail, 0.0, Color32::from_rgba_unmultiplied(5, 8, 15, 242));
        p.line_segment(
            [Pos2::new(layout.left_w, layout.lane_top), Pos2::new(layout.left_w, layout.lane_bottom)],
            Stroke::new(1.0f32, alpha(MINT, 70)),
        );
        for midi in layout.low..=layout.high {
            let y = layout.lane_y(midi);
            let is_target = active >= 0 && stage.song.notes[active as usize].midi == midi;
            let color = if is_target { INK } else if midi % 12 == 0 { alpha(INK, 120) } else { alpha(INK, 55) };
            if is_target {
                p.rect_filled(
                    Rect::from_min_size(Pos2::new(0.0, y - layout.lane_h / 2.0), Vec2::new(layout.left_w, layout.lane_h)),
                    0.0,
                    Color32::from_white_alpha(17),
                );
            }
            p.text(
                Pos2::new(layout.left_w - 10.0, y),
                Align2::RIGHT_CENTER,
                note_name(midi),
                FontId::monospace((layout.lane_h * 0.42).clamp(7.0, 10.0)),
                color,
            );
        }
        // target bracket + voice bubble
        if active >= 0 {
            let note = &stage.song.notes[active as usize];
            let y = layout.lane_y(note.midi);
            let h = layout.note_height() + 14.0;
            let bracket = Rect::from_center_size(Pos2::new(layout.hit_x, y), Vec2::new(28.0, h));
            p.rect_stroke(bracket, 6.0, Stroke::new(1.0f32, Color32::from_white_alpha(60)), StrokeKind::Inside);
        }
        if let Some(reading) = self.reading {
            let target = if active >= 0 {
                Some(stage.song.notes[active as usize].midi)
            } else {
                None
            };
            let (folded_cents, octave_shift) = match target {
                Some(target) => fold_octave(reading.midi, target),
                None => (reading.cents, 0),
            };
            let octave_match = target.is_some()
                && octave_shift != 0
                && folded_cents.abs() <= OCTAVE_TOLERANCE_CENTS;
            let bubble_midi = if octave_match {
                target.unwrap() as f32
            } else {
                reading.midi
            };
            let y = layout
                .lane_y(bubble_midi.round() as i32)
                .clamp(layout.lane_top + 14.0, layout.lane_bottom - 14.0);
            let color = if let Some(target) = target {
                if octave_match {
                    OCTAVE_BLUE
                } else {
                    let cents = (reading.midi - target as f32) * 100.0;
                    if cents.abs() <= 30.0 {
                        MINT
                    } else if cents.abs() <= 65.0 {
                        GOLD
                    } else if cents.abs() <= 120.0 {
                        Color32::from_rgb(255, 161, 92)
                    } else {
                        PINK
                    }
                }
            } else {
                pitch_col(layout.y_norm(y))
            };
            p.circle_filled(Pos2::new(layout.hit_x, y), 38.0, alpha(color, 25));
            p.circle_filled(Pos2::new(layout.hit_x, y), 23.0, alpha(color, 190));
            p.circle_filled(Pos2::new(layout.hit_x, y), 16.0, color);
            p.circle_filled(Pos2::new(layout.hit_x - 4.0, y - 4.0), 5.0, Color32::WHITE);
            let label = if let Some(target) = target {
                if octave_match {
                    format!(
                        "{}   OCT {:+}",
                        note_name(reading.midi.round() as i32),
                        octave_shift
                    )
                } else {
                    format!(
                        "{}   {:+.0}¢",
                        note_name(reading.midi.round() as i32),
                        (reading.midi - target as f32) * 100.0
                    )
                }
            } else {
                format!("{}   {:+.0}¢", note_name(reading.midi.round() as i32), reading.cents)
            };
            let label_rect = Rect::from_min_size(Pos2::new(layout.hit_x + 34.0, y - 12.0), Vec2::new(98.0, 24.0));
            p.rect_filled(label_rect, 6.0, Color32::from_rgba_unmultiplied(5, 8, 15, 220));
            p.rect_stroke(label_rect, 6.0, Stroke::new(1.0f32, alpha(color, 70)), StrokeKind::Inside);
            p.text(label_rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(9.0), color);
        } else if self.mic.is_some() {
            p.text(
                Pos2::new(layout.hit_x + 18.0, layout.lane_y((layout.low + layout.high) / 2)),
                Align2::LEFT_CENTER,
                "LISTENING",
                FontId::monospace(8.0),
                alpha(INK, 45),
            );
        } else if let Some(error) = &self.mic_error {
            p.text(
                Pos2::new(layout.hit_x + 18.0, layout.lane_y((layout.low + layout.high) / 2)),
                Align2::LEFT_CENTER,
                format!("MIC OFF · {error}"),
                FontId::proportional(11.0),
                PINK,
            );
        }

        // count-in
        if let Some(until) = stage.count_in_until {
            let left = (until - now).max(0.0).ceil();
            let text = if left <= 0.3 { "SING".to_string() } else { format!("{left:.0}") };
            p.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(150.0), alpha(MINT, 220));
            p.text(rect.center() + Vec2::new(0.0, 110.0), Align2::CENTER_CENTER, "GET READY", FontId::monospace(10.0), MUTED);
        }
        if let Some(judgment) = &stage.judgment {
            if now - judgment.at < 0.65 {
                p.text(
                    Pos2::new(rect.width() * 0.5, layout.lane_top + 80.0),
                    Align2::CENTER_CENTER,
                    &judgment.text,
                    FontId::proportional(30.0),
                    judgment.color,
                );
            }
        }

        // timeline
        let tray_top = rect.height() - layout.tray_h;
        let timeline = Rect::from_min_size(Pos2::new(layout.hit_x + 12.0, tray_top - 28.0), Vec2::new(rect.width() - layout.hit_x - 36.0, 5.0));
        p.rect_filled(timeline, 3.0, Color32::from_white_alpha(18));
        let ratio = (stage.time / stage.duration.max(0.01)).clamp(0.0, 1.0) as f32;
        p.rect_filled(
            Rect::from_min_size(timeline.min, Vec2::new(timeline.width() * ratio, timeline.height())),
            3.0,
            MINT,
        );
        p.circle_filled(Pos2::new(timeline.left() + timeline.width() * ratio, timeline.center().y), 5.0, Color32::WHITE);
        p.text(Pos2::new(layout.hit_x + 12.0, tray_top - 19.0), Align2::LEFT_TOP, format_time(stage.time), FontId::monospace(8.0), MUTED2);
        p.text(Pos2::new(rect.width() - 24.0, tray_top - 19.0), Align2::RIGHT_TOP, format_time(stage.duration), FontId::monospace(8.0), MUTED2);
        if click(ui, timeline, "timeline_click") {
            if let Some(pos) = ctx_pointer(ui) {
                let t = ((pos.x - timeline.left()) / timeline.width()).clamp(0.0, 1.0) as f64 * stage.duration;
                actions.push(Action::Seek(t));
            }
        }

        // bottom transport
        let tray = Rect::from_min_size(Pos2::new(24.0, rect.height() - layout.tray_h + 18.0), Vec2::new(rect.width() - 48.0, layout.tray_h - 28.0));
        p.rect_filled(tray, 24.0, Color32::from_rgba_unmultiplied(9, 13, 23, 235));
        p.rect_stroke(tray, 24.0, Stroke::new(1.0f32, Color32::from_white_alpha(26)), StrokeKind::Inside);
        let play_rect = Rect::from_center_size(Pos2::new(rect.width() * 0.5, tray.center().y), Vec2::new(58.0, 58.0));
        if round_button(ui, play_rect, if stage.playing { "PAUSE" } else { "PLAY" }, true) {
            actions.push(Action::TogglePlay);
        }
        let restart = Rect::from_center_size(Pos2::new(play_rect.left() - 46.0, tray.center().y), Vec2::new(38.0, 38.0));
        if round_button(ui, restart, "R", false) {
            actions.push(Action::Restart);
        }
        let lyrics = Rect::from_min_size(Pos2::new(play_rect.right() + 36.0, tray.center().y - 17.0), Vec2::new(96.0, 34.0));
        if pill_toggle(ui, lyrics, if self.settings.show_lyrics { "LYRICS ON" } else { "LYRICS OFF" }, self.settings.show_lyrics) {
            actions.push(Action::ToggleLyrics);
        }
        let vocals = Rect::from_min_size(lyrics.right_top() + Vec2::new(8.0, 0.0), lyrics.size());
        if pill_toggle(ui, vocals, if self.settings.vocals_volume > 0.01 { "VOCALS ON" } else { "VOCALS OFF" }, self.settings.vocals_volume > 0.01) {
            actions.push(Action::ToggleVocals);
        }
        let backing = Rect::from_min_size(vocals.right_top() + Vec2::new(8.0, 0.0), lyrics.size());
        if pill_toggle(ui, backing, if self.settings.backing_enabled { "BACKING ON" } else { "BACKING OFF" }, self.settings.backing_enabled) {
            actions.push(Action::ToggleBacking);
        }
        let menu_btn = Rect::from_min_size(Pos2::new(tray.left() + 16.0, tray.center().y - 17.0), Vec2::new(100.0, 34.0));
        if pill_toggle(ui, menu_btn, "LIBRARY", false) {
            actions.push(Action::OpenMenu);
        }
        let settings_tray = Rect::from_min_size(Pos2::new(tray.right() - 120.0, tray.center().y - 17.0), Vec2::new(100.0, 34.0));
        if pill_toggle(ui, settings_tray, "SETTINGS", false) {
            actions.push(Action::OpenSettings);
        }

        // results
        if let Some(result) = &stage.result {
            p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(3, 6, 12, 190));
            let card = Rect::from_center_size(rect.center(), Vec2::new(620.0, 470.0));
            p.rect_filled(card, 24.0, Color32::from_rgb(9, 14, 24));
            p.rect_stroke(card, 24.0, Stroke::new(1.0f32, Color32::from_white_alpha(30)), StrokeKind::Inside);
            p.text(card.center_top() + Vec2::new(0.0, 28.0), Align2::CENTER_CENTER, "SESSION COMPLETE", FontId::monospace(9.0), MUTED2);
            if result.new_best {
                let badge = Rect::from_center_size(card.center_top() + Vec2::new(0.0, 54.0), Vec2::new(84.0, 20.0));
                p.rect_filled(badge, 10.0, Color32::from_rgba_unmultiplied(255, 209, 102, 24));
                p.text(badge.center(), Align2::CENTER_CENTER, "NEW BEST", FontId::monospace(7.0), GOLD);
            }
            p.text(card.center_top() + Vec2::new(0.0, 92.0), Align2::CENTER_CENTER, result.rank.to_string(), FontId::proportional(64.0), MINT);
            let copy = match result.rank {
                'S' => "Perfect pitch energy.",
                'A' => "Delightfully in tune.",
                'B' => "Solid pipes.",
                'C' => "Finding the key.",
                _ => "Warm-up first.",
            };
            p.text(card.center_top() + Vec2::new(0.0, 160.0), Align2::CENTER_CENTER, copy, FontId::proportional(30.0), INK);
            let stats = [
                ("SCORE", result.score.to_string()),
                ("PITCH ACCURACY", format!("{:.0}%", result.accuracy)),
                ("NOTES HIT", format!("{}/{}", result.hit, stage.song.notes.len())),
                ("PERFECT", result.perfect.to_string()),
                ("BEST COMBO", format!("{}×", result.combo)),
            ];
            for (i, (label, value)) in stats.iter().enumerate() {
                let x = card.left() + 44.0 + i as f32 * 108.0;
                p.text(Pos2::new(x, card.center().y + 60.0), Align2::LEFT_TOP, *label, FontId::monospace(7.0), MUTED2);
                p.text(Pos2::new(x, card.center().y + 76.0), Align2::LEFT_TOP, value, FontId::monospace(17.0), INK);
            }
            let retry = Rect::from_center_size(Pos2::new(card.center().x - 90.0, card.bottom() - 70.0), Vec2::new(160.0, 44.0));
            if pill_button(ui, retry, "SING IT AGAIN", true) {
                actions.push(Action::RetrySong);
            }
            let library = Rect::from_center_size(Pos2::new(card.center().x + 110.0, card.bottom() - 70.0), Vec2::new(170.0, 44.0));
            if pill_button(ui, library, "BACK TO LIBRARY", false) {
                actions.push(Action::OpenLibraryFromResults);
            }
        }
        let _ = now;
    }

    fn draw_drawer(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>, now: f64) {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        let anim = ctx.animate_bool(Id::new("drawer_anim"), self.drawer_open);
        if anim <= 0.001 {
            return;
        }
        let width = 430.0_f32.min(rect.width() * 0.94);
        let panel = Rect::from_min_max(
            Pos2::new(rect.width() - width * anim, 0.0),
            Pos2::new(rect.width(), rect.height()),
        );
        let scrim = Rect::from_min_max(rect.min, Pos2::new((rect.width() - width * anim).max(0.0), rect.height()));
        ui.painter().rect_filled(scrim, 0.0, Color32::from_rgba_unmultiplied(3, 6, 12, (110.0 * anim) as u8));
        if click(ui, scrim, "drawer_scrim") {
            actions.push(Action::CloseMenu);
        }
        let p = ui.painter().with_clip_rect(panel);
        p.rect_filled(panel, 0.0, Color32::from_rgb(8, 12, 21));
        p.line_segment([panel.left_top(), panel.left_bottom()], Stroke::new(1.0f32, Color32::from_white_alpha(28)));
        // header
        p.text(panel.left_top() + Vec2::new(20.0, 22.0), Align2::LEFT_TOP, "KARAOKEMACHINE · LIBRARY", FontId::monospace(8.0), MUTED2);
        let drawer_title = if self.drawer_view == DrawerViewRust::Imports {
            "Imports"
        } else if self.drawer_view == DrawerViewRust::Maps {
            "Map Browser"
        } else if self.selected_album.is_some() {
            "Album"
        } else {
            "Records & warm-ups"
        };
        p.text(panel.left_top() + Vec2::new(20.0, 38.0), Align2::LEFT_TOP, drawer_title, FontId::proportional(24.0), INK);
        let close = Rect::from_min_size(panel.right_top() + Vec2::new(-48.0, 16.0), Vec2::new(30.0, 30.0));
        if glyph_button(ui, close, "X") {
            actions.push(Action::CloseMenu);
        }
        let body = Rect::from_min_max(
            Pos2::new(panel.left() + 18.0, panel.top() + 80.0),
            Pos2::new(panel.right() - 18.0, panel.bottom() - 72.0),
        );
        p.line_segment([Pos2::new(panel.left(), body.top() - 12.0), Pos2::new(panel.right(), body.top() - 12.0)], Stroke::new(1.0f32, Color32::from_white_alpha(18)));
        // wheel scroll
        if let Some(pos) = ctx_pointer(ui) {
            if body.contains(pos) {
                let delta = ui.input(|i| i.raw_scroll_delta.y);
                if delta.abs() > 0.0 {
                    self.drawer_scroll = (self.drawer_scroll - delta).max(0.0);
                }
            }
        }
        p.text(body.left_top(), Align2::LEFT_TOP, " ", FontId::proportional(1.0), Color32::TRANSPARENT);
        if self.drawer_view == DrawerViewRust::Imports {
            self.draw_drawer_imports(ui, &p, body, actions);
        } else if self.drawer_view == DrawerViewRust::Maps {
            self.draw_drawer_maps(ui, &p, body, actions);
        } else if let Some(index) = self.selected_album {
            self.draw_album_detail(ui, &p, body, index, actions);
        } else {
            self.draw_album_grid(ui, &p, body, actions);
        }
        // footer
        let foot = Rect::from_min_max(Pos2::new(panel.left(), panel.bottom() - 60.0), panel.right_bottom());
        p.rect_filled(foot, 0.0, Color32::from_rgb(8, 12, 21));
        p.line_segment([foot.left_top(), foot.right_top()], Stroke::new(1.0f32, Color32::from_white_alpha(22)));
        let btn_w = foot.width() / 4.0 - 8.0;
        for (i, (label, view)) in [
            ("ALBUMS", 0usize),
            ("IMPORTS", 1),
            ("MAPS", 2),
            ("SETTINGS", 3),
        ]
        .iter()
        .enumerate()
        {
            let br = Rect::from_min_size(Pos2::new(foot.left() + 6.0 + i as f32 * (btn_w + 4.0), foot.top() + 11.0), Vec2::new(btn_w, 34.0));
            if pill_toggle(
                ui,
                br,
                label,
                (*view == 0
                    && self.drawer_view == DrawerViewRust::Albums
                    && self.selected_album.is_none())
                    || (*view == 1 && self.drawer_view == DrawerViewRust::Imports)
                    || (*view == 2 && self.drawer_view == DrawerViewRust::Maps),
            ) {
                match view {
                    0 => actions.push(Action::ShowAlbums),
                    1 => actions.push(Action::ShowImports),
                    2 => actions.push(Action::ShowMaps),
                    _ => actions.push(Action::OpenSettings),
                }
            }
        }
        let _ = now;
    }

    fn draw_album_grid(&self, ui: &mut egui::Ui, p: &egui::Painter, body: Rect, actions: &mut Vec<Action>) {
        let intro_h = 48.0;
        p.text(body.left_top(), Align2::LEFT_TOP, "SELECT A RECORD", FontId::monospace(8.0), MUTED2);
        p.text(body.left_top() + Vec2::new(0.0, 16.0), Align2::LEFT_TOP, "Every track needs a manual mapping. Pick an album, then a song.", FontId::proportional(11.0), MUTED);
        let cols = 2;
        let gap = 14.0;
        let cell_w = (body.width() - gap) / cols as f32;
        let art = cell_w.min(170.0);
        let cell_h = art + 47.0;
        let top = body.top() + intro_h;
        let mut max_y = 0.0f32;
        for (i, album) in self.albums.iter().enumerate() {
            let row = i / cols;
            let col = i % cols;
            let x = body.left() + col as f32 * (cell_w + gap);
            let y = top + row as f32 * (cell_h + 14.0) - self.drawer_scroll;
            if y + cell_h < body.top() || y > body.bottom() {
                continue;
            }
            max_y = max_y.max(y + cell_h);
            let art_rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(art, art));
            p.rect_filled(art_rect, 12.0, Color32::from_rgb(18, 25, 42));
            if let Some(tex) = self.covers.get(&self.songs[album.songs[0]].id) {
                p.image(tex.id(), art_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            } else {
                paint_cover_fallback(p, art_rect, &album.title);
            }
            p.rect_stroke(art_rect, 12.0, Stroke::new(1.0f32, Color32::from_white_alpha(30)), StrokeKind::Inside);
            if album.warmup {
                let badge = Rect::from_min_size(art_rect.left_top() + Vec2::new(8.0, 8.0), Vec2::new(65.0, 20.0));
                p.rect_filled(badge, 10.0, Color32::from_rgba_unmultiplied(8, 12, 21, 180));
                p.text(badge.center(), Align2::CENTER_CENTER, "WARM-UPS", FontId::monospace(6.0), GOLD);
            }
            let unmapped = album
                .songs
                .iter()
                .filter(|index| !self.songs[**index].mapped)
                .count();
            if unmapped > 0 {
                let badge = Rect::from_min_size(
                    Pos2::new(art_rect.right() - 94.0, art_rect.top() + 8.0),
                    Vec2::new(86.0, 20.0),
                );
                p.rect_filled(badge, 10.0, Color32::from_rgba_unmultiplied(8, 12, 21, 215));
                p.text(
                    badge.center(),
                    Align2::CENTER_CENTER,
                    format!("{unmapped} UNMAPPED"),
                    FontId::monospace(6.0),
                    PINK,
                );
            }
            p.text(art_rect.left_bottom() + Vec2::new(2.0, 9.0), Align2::LEFT_TOP, &album.title, FontId::proportional(13.0), INK);
            p.text(art_rect.left_bottom() + Vec2::new(2.0, 26.0), Align2::LEFT_TOP, format!("{} · {} tracks", album.artist, album.songs.len()), FontId::monospace(8.0), MUTED2);
            let hit = Rect::from_min_size(Pos2::new(x, y), Vec2::new(art, cell_h));
            if click(ui, hit, ("album", i)) {
                actions.push(Action::SelectAlbum(i));
            }
        }
    }

    fn draw_album_detail(&self, ui: &mut egui::Ui, p: &egui::Painter, body: Rect, index: usize, actions: &mut Vec<Action>) {
        let Some(album) = self.albums.get(index) else {
            return;
        };
        let back = Rect::from_min_size(body.left_top(), Vec2::new(90.0, 28.0));
        if pill_toggle(ui, back, "← ALBUMS", false) {
            actions.push(Action::BackToAlbums);
        }
        let art = Rect::from_min_size(body.left_top() + Vec2::new(0.0, 42.0), Vec2::new(126.0, 126.0));
        p.rect_filled(art, 12.0, Color32::from_rgb(18, 25, 42));
        if let Some(tex) = self.covers.get(&self.songs[album.songs[0]].id) {
            p.image(tex.id(), art, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        } else {
            paint_cover_fallback(p, art, &album.title);
        }
        p.rect_stroke(art, 12.0, Stroke::new(1.0f32, Color32::from_white_alpha(30)), StrokeKind::Inside);
        p.text(art.right_top() + Vec2::new(16.0, 2.0), Align2::LEFT_TOP, if album.warmup { "KARAOKEMACHINE SESSIONS" } else { "ALBUM" }, FontId::monospace(8.0), MUTED2);
        p.text(art.right_top() + Vec2::new(16.0, 20.0), Align2::LEFT_TOP, &album.title, FontId::proportional(26.0), INK);
        let release = album
            .songs
            .iter()
            .find_map(|index| self.songs[*index].release_date.clone());
        let meta = match release {
            Some(date) => format!("{} · {} tracks · {}", album.artist, album.songs.len(), date),
            None => format!("{} · {} tracks", album.artist, album.songs.len()),
        };
        p.text(art.right_top() + Vec2::new(16.0, 54.0), Align2::LEFT_TOP, meta, FontId::monospace(9.0), MUTED);
        let unmapped = album
            .songs
            .iter()
            .filter(|index| !self.songs[**index].mapped)
            .count();
        p.text(
            art.right_top() + Vec2::new(16.0, 76.0),
            Align2::LEFT_TOP,
            if unmapped > 0 {
                format!(
                    "{} of {} tracks need a manual mapping.",
                    unmapped,
                    album.songs.len()
                )
            } else {
                "All tracks mapped and playable.".into()
            },
            FontId::proportional(10.0),
            if unmapped > 0 { PINK } else { MINT },
        );
        let list_top = art.bottom() + 26.0 - self.drawer_scroll;
        for (i, &song_index) in album.songs.iter().enumerate() {
            let song = &self.songs[song_index];
            let row = Rect::from_min_size(Pos2::new(body.left(), list_top + i as f32 * 46.0), Vec2::new(body.width(), 42.0));
            if row.bottom() < body.top() || row.top() > body.bottom() {
                continue;
            }
            p.text(row.left_center() + Vec2::new(2.0, -8.0), Align2::LEFT_CENTER, format!("{:02}", i + 1), FontId::monospace(8.0), MUTED2);
            p.text(row.left_center() + Vec2::new(30.0, -7.0), Align2::LEFT_CENTER, &song.title, FontId::proportional(12.5), INK);
            let sub = if !song.mapped {
                "UNMAPPED · manual word and note mapping required"
            } else if song.lyrics_synced.unwrap_or(false) {
                "synced lyrics"
            } else if !song.lines.is_empty() {
                "lyrics"
            } else {
                "instrumental"
            };
            p.text(row.left_center() + Vec2::new(30.0, 9.0), Align2::LEFT_CENTER, sub, FontId::monospace(7.0), if song.mapped { MUTED2 } else { PINK });
            let meta = format!("{} · {}", song.bpm.map(|b| format!("{b:.0} BPM · ")).unwrap_or_default(), format_time(song.duration()));
            p.text(Pos2::new(row.right() - 104.0, row.center().y), Align2::RIGHT_CENTER, meta, FontId::monospace(7.0), MUTED2);
            p.text(Pos2::new(row.right() - 66.0, row.center().y), Align2::RIGHT_CENTER, if song.mapped { "PLAY" } else { "MAP" }, FontId::monospace(7.0), if song.mapped { MINT } else { PINK });
            let edit = Rect::from_min_size(Pos2::new(row.right() - 36.0, row.top() + 9.0), Vec2::new(30.0, 24.0));
            if pill_toggle(ui, edit, "EDIT", !song.mapped) {
                actions.push(Action::EditSong(song_index));
            }
            let main = Rect::from_min_max(row.min, Pos2::new(row.right() - 42.0, row.bottom()));
            if click(ui, main, ("track", song_index)) {
                actions.push(Action::PlaySong(song_index));
            }
        }
    }

    fn draw_drawer_maps(&self, ui: &mut egui::Ui, p: &egui::Painter, body: Rect, actions: &mut Vec<Action>) {
        let import = Rect::from_min_size(body.left_top(), Vec2::new(body.width(), 38.0));
        if pill_button(ui, import, "IMPORT MAP FILE", false) {
            actions.push(Action::InstallMap);
        }
        let list_top = body.top() + 50.0 - self.drawer_scroll;
        for (i, entry) in self.map_index.iter().enumerate() {
            let row = Rect::from_min_size(
                Pos2::new(body.left(), list_top + i as f32 * 68.0),
                Vec2::new(body.width(), 62.0),
            );
            if row.bottom() < body.top() || row.top() > body.bottom() {
                continue;
            }
            let cover = Rect::from_min_size(row.left_top(), Vec2::new(54.0, 54.0));
            p.rect_filled(cover, 8.0, Color32::from_rgb(18, 25, 42));
            if let Some(texture) = self.map_covers.get(&entry.id) {
                p.image(
                    texture.id(),
                    cover,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                paint_cover_fallback(p, cover, &entry.title);
            }
            p.text(
                Pos2::new(row.left() + 66.0, row.top() + 9.0),
                Align2::LEFT_TOP,
                &entry.title,
                FontId::proportional(12.0),
                INK,
            );
            p.text(
                Pos2::new(row.left() + 66.0, row.top() + 27.0),
                Align2::LEFT_TOP,
                &entry.artist,
                FontId::monospace(7.0),
                MUTED,
            );
            let install = Rect::from_min_size(
                Pos2::new(row.right() - 82.0, row.top() + 17.0),
                Vec2::new(76.0, 28.0),
            );
            if self.map_download.as_ref().map(|(id, _)| id.as_str()) == Some(entry.id.as_str()) {
                let _ = pill_toggle(ui, install, "...", false);
            } else {
                if pill_button(ui, install, "INSTALL", true) {
                    actions.push(Action::InstallRemoteMap(i));
                }
            }
            p.line_segment(
                [row.left_bottom(), row.right_bottom()],
                Stroke::new(0.6f32, Color32::from_white_alpha(14)),
            );
        }

        let mut y = list_top + self.map_index.len() as f32 * 68.0;
        for (song_index, song) in self
            .songs
            .iter()
            .enumerate()
            .filter(|(_, song)| song.source == "scale")
        {
            let row = Rect::from_min_size(
                Pos2::new(body.left(), y),
                Vec2::new(body.width(), 62.0),
            );
            y += 68.0;
            if row.bottom() < body.top() || row.top() > body.bottom() {
                continue;
            }
            let cover = Rect::from_min_size(row.left_top(), Vec2::new(54.0, 54.0));
            p.rect_filled(cover, 8.0, Color32::from_rgb(18, 25, 42));
            if let Some(texture) = self.covers.get(&song.id) {
                p.image(
                    texture.id(),
                    cover,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                paint_cover_fallback(p, cover, &song.title);
            }
            p.text(
                Pos2::new(row.left() + 66.0, row.top() + 9.0),
                Align2::LEFT_TOP,
                &song.title,
                FontId::proportional(12.0),
                INK,
            );
            p.text(
                Pos2::new(row.left() + 66.0, row.top() + 27.0),
                Align2::LEFT_TOP,
                format!("Bundled warm-up · {}", format_time(song.duration())),
                FontId::monospace(7.0),
                MUTED,
            );
            let sing = Rect::from_min_size(
                Pos2::new(row.right() - 82.0, row.top() + 17.0),
                Vec2::new(76.0, 28.0),
            );
            if pill_button(ui, sing, "SING", true) {
                actions.push(Action::PlaySong(song_index));
            }
            let main = Rect::from_min_max(row.min, Pos2::new(row.right() - 90.0, row.bottom()));
            if click(ui, main, ("map_warmup_row", song_index)) {
                actions.push(Action::PlaySong(song_index));
            }
            p.line_segment(
                [row.left_bottom(), row.right_bottom()],
                Stroke::new(0.6f32, Color32::from_white_alpha(14)),
            );
        }
    }

    fn draw_drawer_imports(&self, ui: &mut egui::Ui, p: &egui::Painter, body: Rect, actions: &mut Vec<Action>) {
        p.text(body.left_top(), Align2::LEFT_TOP, "IMPORT QUEUE", FontId::monospace(8.0), MUTED2);
        p.text(body.left_top() + Vec2::new(0.0, 18.0), Align2::LEFT_TOP, "From your list", FontId::proportional(26.0), INK);
        p.text(body.left_top() + Vec2::new(0.0, 52.0), Align2::LEFT_TOP, &self.songlist_path, FontId::monospace(8.0), MUTED2);
        let ready = self.songlist.iter().filter(|url| self.songs.iter().any(|s| s.youtube_url.as_deref() == Some(url.as_str()))).count();
        p.text(body.left_top() + Vec2::new(0.0, 76.0), Align2::LEFT_TOP, format!("{ready}/{} ready", self.songlist.len()), FontId::monospace(8.0), MUTED);
        let reload = Rect::from_min_size(Pos2::new(body.right() - 100.0, body.top() + 70.0), Vec2::new(100.0, 30.0));
        if pill_toggle(ui, reload, "RELOAD", false) {
            actions.push(Action::Reload);
        }
        let upload = Rect::from_min_size(Pos2::new(body.right() - 250.0, body.top() + 70.0), Vec2::new(138.0, 30.0));
        if pill_toggle(ui, upload, "UPLOAD SONG", true) {
            actions.push(Action::OpenUpload);
        }
        let list_top = body.top() + 104.0 - self.drawer_scroll;
        for (i, url) in self.songlist.iter().enumerate() {
            let y = list_top + i as f32 * 38.0;
            if y + 30.0 < body.top() || y > body.bottom() {
                continue;
            }
            let row = Rect::from_min_size(Pos2::new(body.left(), y), Vec2::new(body.width(), 34.0));
            let song = self.songs.iter().find(|s| s.youtube_url.as_deref() == Some(url.as_str()));
            p.line_segment([Pos2::new(row.left(), row.bottom()), Pos2::new(row.right(), row.bottom())], Stroke::new(0.6f32, Color32::from_white_alpha(12)));
            p.text(Pos2::new(row.left() + 2.0, row.center().y), Align2::LEFT_CENTER, format!("{:02}", i + 1), FontId::monospace(7.0), MUTED2);
            let title = song.map(|s| s.title.clone()).unwrap_or_else(|| format!("Track {:02}", i + 1));
            p.text(Pos2::new(row.left() + 28.0, row.center().y - 7.0), Align2::LEFT_CENTER, title, FontId::proportional(11.0), if song.is_some() { INK } else { MUTED });
            p.text(Pos2::new(row.left() + 28.0, row.center().y + 8.0), Align2::LEFT_CENTER, url.split("v=").nth(1).unwrap_or(url), FontId::monospace(7.0), MUTED2);
            let status = if let Some(song) = song {
                if song.mapped { "PLAY" } else { "UNMAPPED" }
            } else {
                "QUEUED"
            };
            p.text(
                Pos2::new(row.right() - 10.0, row.center().y),
                Align2::RIGHT_CENTER,
                status,
                FontId::monospace(7.0),
                match song {
                    Some(song) if song.mapped => MINT,
                    Some(_) => PINK,
                    None => MUTED2,
                },
            );
            if let Some(song_index) = self.songs.iter().position(|s| s.youtube_url.as_deref() == Some(url.as_str())) {
                if click(ui, row, ("import_track", i)) {
                    actions.push(Action::PlaySong(song_index));
                }
            }
        }
        p.text(body.left_bottom() + Vec2::new(0.0, -24.0), Align2::LEFT_BOTTOM, "Audio stays local. Vocals are optional.", FontId::monospace(8.0), MUTED2);
    }




    fn start_map_index_fetch(&mut self) {
        if self.map_index_job.is_some() {
            return;
        }
        self.map_index_job = Some(Job::run("The map browser", &self.ctx, || {
            let base = std::env::var("KARAOKEMACHINE_MAPS_BASE_URL")
                .unwrap_or_else(|_| MAPS_BASE_URL.to_string());
            http_get_text(&format!("{base}index.json"))
                .and_then(|raw| serde_json::from_str::<MapIndex>(&raw).map_err(|error| format!("Could not parse map index: {error}")))
        }));
    }

    fn start_remote_map_install(&mut self, entry: MapIndexEntry) {
        if self.map_download.is_some() || entry.file.is_empty() {
            return;
        }
        let base = std::env::var("KARAOKEMACHINE_MAPS_BASE_URL")
            .unwrap_or_else(|_| MAPS_BASE_URL.to_string());
        let url = format!("{base}{}", encode_path_segment(&entry.file));
        let job = Job::run("The map download", &self.ctx, move || {
            http_get_text(&url).and_then(|raw| MapPackage::from_json(&raw))
        });
        self.map_download = Some((entry.id.clone(), job));
    }

    fn poll_maps(&mut self, ctx: &egui::Context, now: f64) {
        let index_result = self.map_index_job.as_mut().and_then(Job::poll);
        if let Some(result) = index_result {
            self.map_index_job = None;
            self.map_index_loaded = true;
            match result {
                Ok(index) => {
                    self.map_index = index.maps;
                    for entry in &self.map_index {
                        if !entry.cover.trim().is_empty()
                            && !self.map_covers.contains_key(&entry.id)
                        {
                            if let Some(texture) =
                                load_data_uri_texture(ctx, &entry.id, &entry.cover)
                            {
                                self.map_covers.insert(entry.id.clone(), texture);
                            }
                        }
                    }
                }
                Err(error) => {
                    self.map_index.clear();
                    eprintln!("map browser: {error}");
                }
            }
        }

        let download_result = self.map_download.as_mut().and_then(|(_, job)| job.poll());
        if let Some(result) = download_result {
            self.map_download = None;
            match result {
                Ok(map) => self.install_map(map, now, ctx),
                Err(error) => self.toast(error, now),
            }
        }
    }

    fn install_map_file(&mut self, now: f64, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("KaraokeMachine map", &["json"])
            .pick_file()
        else {
            return;
        };
        match MapPackage::load(&path) {
            Ok(map) => self.install_map(map, now, ctx),
            Err(error) => self.toast(error, now),
        }
    }

    fn install_map(&mut self, map: MapPackage, now: f64, ctx: &egui::Context) {
        if map.source_url.trim().is_empty() {
            self.toast("This map has no audio source link.", now);
            return;
        }
        if let Some(existing) = self
            .songs
            .iter()
            .find(|song| {
                song.id == map.id
                    || song.youtube_url.as_deref() == Some(map.source_url.as_str())
                    || (song.title.eq_ignore_ascii_case(&map.title)
                        && song.artist.as_deref() == map.artist.as_deref())
            })
            .cloned()
        {
            let dir = self.root.join("assets/songs").join(&existing.id);
            let backing = song_audio_path(&self.root, &existing, "backing");
            if backing.exists() {
                let mut song = existing.clone();
                match map.save_cover(&dir) {
                    Ok(cover) => {
                        if cover.is_some() {
                            song.cover = cover;
                        }
                    }
                    Err(error) => {
                        self.toast(error, now);
                        return;
                    }
                }
                if let Err(error) = map.apply_to_song(&mut song) {
                    self.toast(error, now);
                    return;
                }
                if let Err(error) = write_song_file(&dir, &song) {
                    self.toast(error, now);
                    return;
                }
                self.reload_library(ctx, now);
                if let Some(index) = self.songs.iter().position(|song| song.id == existing.id) {
                    self.drawer_open = false;
                    self.open_song(index, now);
                    self.toast("Map installed on the existing local audio.", now);
                } else {
                    self.toast("Map saved. Reload the library to see it.", now);
                }
                return;
            }
        }

        let work = self.root.join(".work/map-cover");
        let cover = if let Err(error) = std::fs::create_dir_all(&work) {
            self.toast(format!("Could not prepare the map installer: {error}"), now);
            return;
        } else {
            match map.save_cover(&work) {
                Ok(Some(name)) => Some(work.join(name)),
                Ok(None) => None,
                Err(error) => {
                    self.toast(error, now);
                    return;
                }
            }
        };
        let lyrics = map
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        self.upload.url = map.source_url.clone();
        self.upload.audio_url = map.source_url.clone();
        self.upload.title = map.title.clone();
        self.upload.artist = map.artist.clone().unwrap_or_default();
        self.upload.album = map.album.clone().unwrap_or_default();
        self.upload.release_date = map.release_date.clone().unwrap_or_default();
        self.upload.lyrics = lyrics;
        self.upload.cover = cover
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        self.upload.cover_preview_path = cover;
        self.upload.cover_preview_loaded = None;
        self.upload.cover_preview_tex = None;
        self.upload.cover_error = false;
        self.upload.info_loaded = true;
        self.upload.message = "Map loaded. Downloading audio from the source link…".into();
        self.upload.open = true;
        self.upload.pending_map = Some(map);
        self.start_import(now);
    }

    fn export_editor_map(&mut self, now: f64) {
        let result: Result<Option<PathBuf>, String> = if let Some(editor) = &self.editor {
            if !editor.song.mapped {
                Err("Submit the mapping before exporting it.".into())
            } else {
                MapPackage::from_song(&self.root, &editor.song, None).and_then(|map| {
                    let default_name = map.file_name();
                    let Some(path) = rfd::FileDialog::new()
                        .set_file_name(default_name)
                        .add_filter("KaraokeMachine map", &["json"])
                        .save_file()
                    else {
                        return Ok(None);
                    };
                    map.save(&path)?;
                    Ok(Some(path))
                })
            }
        } else {
            Ok(None)
        };
        match result {
            Ok(Some(path)) => self.toast(
                format!("Map exported to {}", path.display()),
                now,
            ),
            Ok(None) => {}
            Err(error) => self.toast(error, now),
        }
    }

    fn poll_upload(&mut self, ctx: &egui::Context, now: f64) {
        let cover_result = self.upload.cover_fetch.as_mut().and_then(Job::poll);
        if let Some(result) = cover_result {
            self.upload.cover_fetch = None;
            match result {
                Ok(path) => {
                    self.upload.cover = path.to_string_lossy().to_string();
                    self.upload.cover_preview_path = Some(path);
                    self.upload.cover_preview_loaded = None;
                    self.upload.cover_message = "Cover art ready.".into();
                    self.upload.cover_error = false;
                }
                Err(error) => {
                    self.upload.cover_message = error;
                    self.upload.cover_error = true;
                }
            }
        }

        let fetched = self.upload.fetch.as_mut().and_then(Job::poll);
        let mut start_cover: Option<String> = None;
        if let Some(result) = fetched {
            self.upload.fetch = None;
            match result {
                Ok(info) => {
                    if let Some(error) = info.error {
                        self.upload.message = error;
                    } else {
                        let cover_url = info
                            .cover
                            .clone()
                            .filter(|value| !value.trim().is_empty());
                        self.upload.title = info.title;
                        self.upload.artist = info.artist;
                        self.upload.album = info.album.unwrap_or_default();
                        self.upload.release_date = info.release_date.unwrap_or_default();
                        self.upload.cover = info.cover.unwrap_or_default();
                        self.upload.artist_bio = info.artist_bio.unwrap_or_default();
                        self.upload.album_bio = info.album_bio.unwrap_or_default();
                        self.upload.lyrics = info.lyrics.unwrap_or_default();
                        self.upload.lyrics_provider = info.lyrics_provider;
                        self.upload.lyrics_synced = info.lyrics_synced;
                        self.upload.audio_url = info.audio_url;
                        self.upload.duration = info.duration;
                        self.upload.info_loaded = true;
                        self.upload.artist_search.clear();
                        self.upload.album_search.clear();
                        if let Some(url) = cover_url {
                            start_cover = Some(url);
                        } else {
                            self.upload.cover_preview_path = None;
                            self.upload.cover_preview_tex = None;
                            self.upload.cover_preview_loaded = None;
                            self.upload.cover_message.clear();
                            self.upload.cover_error = false;
                        }
                        self.upload.message = if self.upload.lyrics.trim().is_empty() {
                            "Details fetched. No free lyrics were found — paste or edit them below.".into()
                        } else {
                            "Details and lyrics fetched. Edit anything before importing.".into()
                        };
                    }
                }
                Err(error) => {
                    self.upload.message = error;
                }
            }
        }

        if let Some(url) = start_cover {
            self.start_cover_fetch(url);
        }

        let (messages, ended) = match self.upload.import.as_mut().map(StreamJob::drain) {
            Some(drained) => (drained.messages, drained.ended),
            None => (Vec::new(), false),
        };
        let mut finished = false;
        let mut result: Option<Result<String, String>> = None;
        for message in messages {
            match message {
                ImportMessage::Progress(percent, stage, message) => {
                    self.upload.progress = percent;
                    self.upload.stage = stage;
                    self.upload.message = message;
                }
                ImportMessage::Done(done) => {
                    finished = true;
                    result = Some(done);
                }
            }
        }
        if ended && !finished {
            // The worker exited without reporting. Treat it as a failed import
            // rather than leaving the dialog stuck on "importing".
            finished = true;
            result = Some(Err(self
                .upload
                .import
                .as_ref()
                .map(StreamJob::death_notice)
                .unwrap_or_default()));
        }
        if finished {
            self.upload.import = None;
            match result {
                Some(Ok(id)) => {
                    self.upload.open = false;
                    let pending_map = self.upload.pending_map.take();
                    if let Some(map) = pending_map {
                        let dir = self.root.join("assets/songs").join(&id);
                        let result: Result<(), String> = (|| {
                            let path = dir.join("song.json");
                            let raw = std::fs::read_to_string(&path)
                                .map_err(|error| format!("Could not read the imported song: {error}"))?;
                            let mut song: Song = serde_json::from_str(&raw)
                                .map_err(|error| format!("Could not parse the imported song: {error}"))?;
                            if let Some(cover) = map.save_cover(&dir)? {
                                song.cover = Some(cover);
                            }
                            map.apply_to_song(&mut song)?;
                            write_song_file(&dir, &song)
                        })();
                        match result {
                            Ok(()) => {
                                self.reload_library(ctx, now);
                                if let Some(index) = self.songs.iter().position(|song| song.id == id) {
                                    self.open_song(index, now);
                                    self.toast(
                                        "Map installed. Audio was downloaded from its source link.",
                                        now,
                                    );
                                } else {
                                    self.toast("Map installed, but the song was not found after reload.", now);
                                }
                            }
                            Err(error) => self.toast(error, now),
                        }
                    } else {
                        self.reload_library(ctx, now);
                        if let Some(index) = self.songs.iter().position(|song| song.id == id) {
                            self.open_editor(index, now);
                            self.toast("Song prepared. The manual editor is ready.", now);
                        } else {
                            self.toast("Song imported, but it was not found after reload.", now);
                        }
                    }
                }
                Some(Err(error)) => {
                    self.upload.message = error;
                }
                None => {
                    self.upload.message = "The import ended without reporting a result.".into();
                }
            }
        }
    }


    fn update_cover_preview(&mut self, ctx: &egui::Context) {
        if self.upload.cover_preview_path != self.upload.cover_preview_loaded {
            self.upload.cover_preview_loaded = self.upload.cover_preview_path.clone();
            self.upload.cover_preview_tex = self
                .upload
                .cover_preview_path
                .as_ref()
                .and_then(|path| load_texture(ctx, path));
        }
    }

    fn set_cover_from_path(&mut self, path: PathBuf, message: impl Into<String>) {
        if !path.exists() {
            self.upload.cover_error = true;
            self.upload.cover_message = format!("Cover image not found: {}", path.display());
            return;
        }
        self.upload.cover = path.to_string_lossy().to_string();
        self.upload.cover_preview_path = Some(path);
        self.upload.cover_preview_loaded = None;
        self.upload.cover_preview_tex = None;
        self.upload.cover_fetch = None;
        self.upload.cover_error = false;
        self.upload.cover_message = message.into();
    }

    fn start_cover_fetch(&mut self, url: String) {
        let url = url.trim().to_string();
        if url.is_empty() {
            return;
        }
        self.upload.cover_link = url.clone();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            self.set_cover_from_path(PathBuf::from(&url), "Cover image selected.");
            return;
        }
        if self.upload.cover_fetching() {
            return;
        }
        let script = self.root.join("dist-tools/fetch-cover.js");
        if !script.exists() {
            self.upload.cover_error = true;
            self.upload.cover_message =
                "Cover preview tools are not built. Run `npm install && npm run assets`.".into();
            return;
        }
        let work = self.root.join(".work");
        if let Err(error) = std::fs::create_dir_all(&work) {
            self.upload.cover_error = true;
            self.upload.cover_message = format!("Could not create the work directory: {error}");
            return;
        }
        let out = work.join("upload-cover-preview.bin");
        let root = self.root.clone();
        self.upload.cover_error = false;
        self.upload.cover_message = "Downloading cover preview…".into();
        self.upload.cover_fetch = Some(Job::run("The cover preview helper", &self.ctx, move || {
            run_cover_download(&root, &url, &out)
        }));
    }

    fn pick_cover_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Image files", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
        {
            self.set_cover_from_path(path, "Cover image selected. Click DONE to continue.");
        }
    }

    fn paste_cover_image(&mut self) {
        let image = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_image());
        let image = match image {
            Ok(image) => image,
            Err(error) => {
                self.upload.cover_error = true;
                self.upload.cover_message = format!("Clipboard image unavailable: {error}");
                return;
            }
        };
        let work = self.root.join(".work");
        if let Err(error) = std::fs::create_dir_all(&work) {
            self.upload.cover_error = true;
            self.upload.cover_message = format!("Could not create the work directory: {error}");
            return;
        }
        let path = work.join("upload-cover-paste.png");
        let Some(buffer) = image::RgbaImage::from_raw(
            image.width as u32,
            image.height as u32,
            image.bytes.into_owned(),
        ) else {
            self.upload.cover_error = true;
            self.upload.cover_message = "The clipboard image was not readable.".into();
            return;
        };
        if let Err(error) = buffer.save(&path) {
            self.upload.cover_error = true;
            self.upload.cover_message = format!("Could not save the clipboard image: {error}");
            return;
        }
        self.set_cover_from_path(path, "Cover pasted from the clipboard. Click DONE to continue.");
    }

    fn draw_cover_popup(&mut self, ctx: &egui::Context) {
        if !self.upload.cover_popup_open {
            return;
        }
        let mut open = self.upload.cover_popup_open;
        let mut load_link = false;
        let mut pick_file = false;
        let mut paste_image = false;
        let mut remove_cover = false;
        let mut done = false;
        egui::Window::new("Cover art")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size(Vec2::new(560.0, 780.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(520.0);
                egui::ScrollArea::vertical().max_height(720.0).show(ui, |ui| {
                    let (preview, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), 270.0),
                        Sense::hover(),
                    );
                    paint_cover_preview(
                        ui,
                        preview,
                        self.upload.cover_preview_tex.as_ref(),
                        !self.upload.cover.trim().is_empty(),
                        self.upload.cover_fetching(),
                    );
                    if !self.upload.cover_message.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(&self.upload.cover_message)
                                .size(10.0)
                                .color(if self.upload.cover_error { PINK } else { MUTED }),
                        );
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("PASTE AN IMAGE LINK")
                            .monospace()
                            .size(8.0)
                            .color(MUTED2),
                    );
                    ui.horizontal(|ui| {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.upload.cover_link)
                                .desired_width(380.0),
                        );
                        if self.upload.cover_link.is_empty() {
                            paint_placeholder(
                                ui,
                                response.rect,
                                "https://…/cover.jpg",
                                false,
                            );
                        }
                        if ui
                            .add_enabled(
                                !self.upload.cover_fetching()
                                    && !self.upload.cover_link.trim().is_empty(),
                                egui::Button::new("LOAD LINK"),
                            )
                            .clicked()
                        {
                            load_link = true;
                        }
                    });
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("UPLOAD A FILE")
                            .monospace()
                            .size(8.0)
                            .color(MUTED2),
                    );
                    if ui.button("SELECT AN IMAGE FILE").clicked() {
                        pick_file = true;
                    }
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("PASTE FROM CLIPBOARD")
                            .monospace()
                            .size(8.0)
                            .color(MUTED2),
                    );
                    if ui.button("PASTE IMAGE").clicked() {
                        paste_image = true;
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.upload.cover_preview_tex.is_some(),
                                egui::Button::new("USE THIS COVER"),
                            )
                            .clicked()
                        {
                            done = true;
                        }
                        if ui.button("REMOVE COVER").clicked() {
                            remove_cover = true;
                        }
                    });
                });
            });
        if !open {
            self.upload.cover_popup_open = false;
        }
        if done {
            self.upload.cover_popup_open = false;
        }
        if load_link {
            self.start_cover_fetch(self.upload.cover_link.clone());
        }
        if pick_file {
            self.pick_cover_file();
        }
        if paste_image {
            self.paste_cover_image();
        }
        if remove_cover {
            self.upload.cover.clear();
            self.upload.cover_preview_path = None;
            self.upload.cover_preview_tex = None;
            self.upload.cover_preview_loaded = None;
            self.upload.cover_link.clear();
            self.upload.cover_fetch = None;
            self.upload.cover_message = "Cover removed.".into();
            self.upload.cover_error = false;
        }
    }

    fn start_fetch(&mut self, now: f64) {
        if self.upload.fetching() || self.upload.importing() {
            return;
        }
        self.upload.cover_fetch = None;
        let url = self.upload.url.trim().to_string();
        if url.is_empty() {
            self.upload.message = "Paste a YouTube, Spotify or SoundCloud link first.".into();
            return;
        }
        let script = self.root.join("dist-tools/song-info.js");
        if !script.exists() {
            self.upload.message = "Import tools are not built. Run `npm install && npm run assets` first.".into();
            return;
        }
        let root = self.root.clone();
        self.upload.info_loaded = false;
        self.upload.message = "Reading the link and looking up lyrics…".into();
        let _ = now;
        self.upload.fetch = Some(Job::run("The metadata helper", &self.ctx, move || {
            run_song_info(&root, &url)
        }));
    }

    fn start_import(&mut self, now: f64) {
        if self.upload.fetching() || self.upload.importing() {
            return;
        }
        if self.upload.url.trim().is_empty() {
            self.upload.message = "Paste a link first.".into();
            return;
        }
        if self.upload.title.trim().is_empty() || self.upload.artist.trim().is_empty() {
            self.upload.message = "Title and artist are required.".into();
            return;
        }
        if self.upload.audio_url.trim().is_empty()
            && self.upload.url.to_lowercase().contains("spotify.com")
        {
            self.upload.message =
                "Spotify needs Fetch Details first so KaraokeMachine can match a public audio source."
                    .into();
            return;
        }
        let script = self.root.join("dist-tools/import-song.js");
        if !script.exists() {
            self.upload.message = "Import tools are not built. Run `npm install && npm run assets` first.".into();
            return;
        }
        let meta = serde_json::json!({
            "audioUrl": self.upload.audio_url,
            "title": self.upload.title,
            "artist": self.upload.artist,
            "album": self.upload.album,
            "releaseDate": self.upload.release_date,
            "cover": self.upload.cover,
            "artistBio": self.upload.artist_bio,
            "albumBio": self.upload.album_bio,
            "lyricsText": self.upload.lyrics,
        });
        let work = self.root.join(".work");
        if let Err(error) = std::fs::create_dir_all(&work) {
            self.upload.message = format!("Could not create the work directory: {error}");
            return;
        }
        let meta_path = work.join("editor-import.json");
        if let Err(error) = std::fs::write(&meta_path, serde_json::to_string_pretty(&meta).unwrap_or_default()) {
            self.upload.message = format!("Could not write import metadata: {error}");
            return;
        }
        let log_path = work.join("editor-import.log");
        let log = match std::fs::File::create(&log_path) {
            Ok(file) => file,
            Err(error) => {
                self.upload.message = format!("Could not open the import log: {error}");
                return;
            }
        };
        let root = self.root.clone();
        let url = self.upload.url.trim().to_string();
        self.upload.progress = 0.0;
        self.upload.stage = "Queued".into();
        self.upload.message = "Downloading and separating the audio. This can take several minutes.".into();
        let _ = now;
        self.upload.import = Some(StreamJob::run("The importer", &self.ctx, move |sender| {
            let child = Command::new("node")
                .arg(&script)
                .arg("--url")
                .arg(&url)
                .arg("--meta")
                .arg(&meta_path)
                .current_dir(&root)
                .stdout(Stdio::piped())
                .stderr(Stdio::from(log))
                .spawn();
            let mut child = match child {
                Ok(child) => child,
                Err(error) => {
                    sender.send(ImportMessage::Done(Err(format!("Could not start Node: {error}"))));
                    return;
                }
            };
            let mut result_id: Option<String> = None;
            let mut error_message: Option<String> = None;
            if let Some(stdout) = child.stdout.take() {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Some(rest) = line.strip_prefix("PROGRESS\t") {
                        let parts: Vec<&str> = rest.splitn(3, '\t').collect();
                        if let Some(percent) = parts.first().and_then(|value| value.parse::<f32>().ok()) {
                            let stage = parts.get(1).copied().unwrap_or("Working");
                            let message = parts.get(2).copied().unwrap_or(stage);
                            sender.send(ImportMessage::Progress(
                                percent,
                                stage.to_string(),
                                message.to_string(),
                            ));
                        }
                    } else if let Some(id) = line.strip_prefix("RESULT\t") {
                        result_id = Some(id.trim().to_string());
                    } else if let Some(error) = line.strip_prefix("ERROR\t") {
                        error_message = Some(error.trim().to_string());
                    }
                }
            }
            let status = child.wait();
            match status {
                Ok(status) if status.success() => {
                    if let Some(id) = result_id {
                        sender.send(ImportMessage::Done(Ok(id)));
                    } else {
                        sender.send(ImportMessage::Done(Err(
                            "Import finished but did not report a song id.".into()
                        )));
                    }
                }
                _ => {
                    let log = std::fs::read_to_string(root.join(".work/editor-import.log")).unwrap_or_default();
                    let tail = log.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                    let message = error_message
                        .or_else(|| if tail.trim().is_empty() { None } else { Some(tail) })
                        .unwrap_or_else(|| "The import failed. Check yt-dlp, ffmpeg and Demucs.".into());
                    sender.send(ImportMessage::Done(Err(message)));
                }
            }
        }));
    }

    fn draw_upload(&mut self, ctx: &egui::Context, now: f64) {
        let mut open = self.upload.open;
        let mut artists: Vec<String> = self.songs.iter().filter_map(|song| song.artist.clone()).collect();
        artists.sort_by_key(|name| name.to_lowercase());
        artists.dedup();
        let mut albums: Vec<(String, String)> = self
            .songs
            .iter()
            .filter_map(|song| {
                let album = song.album.clone()?;
                let artist = song.artist.clone().unwrap_or_default();
                Some((album, artist))
            })
            .collect();
        albums.sort_by_key(|(album, _)| album.to_lowercase());
        albums.dedup();
        let mut fetch_clicked = false;
        let mut import_clicked = false;
        let mut pick_artist: Option<String> = None;
        let mut pick_album: Option<(String, String)> = None;

        self.update_cover_preview(ctx);
        {
            let upload = &mut self.upload;
            egui::Window::new("Upload a song")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .fixed_size(Vec2::new(930.0, 850.0))
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_min_width(890.0);
                    ui.spacing_mut().interact_size.y = 28.0;
                    egui::ScrollArea::vertical().max_height(700.0).show(ui, |ui| {
                        ui.label(
                            RichText::new("Paste a YouTube, Spotify or SoundCloud link. Fetch fills in whatever it can; every field remains editable.")
                                .size(11.0)
                                .color(MUTED),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("LINK").monospace().size(8.0).color(MUTED2));
                            let link = ui.add(
                                egui::TextEdit::singleline(&mut upload.url)
                                    .desired_width(640.0),
                            );
                            if upload.url.is_empty() {
                                paint_placeholder(
                                    ui,
                                    link.rect,
                                    "https://open.spotify.com/track/…  ·  https://youtu.be/…",
                                    false,
                                );
                            }
                            let can_fetch = !upload.fetching()
                                && !upload.importing()
                                && !upload.url.trim().is_empty();
                            if can_fetch
                                && (link.has_focus() || link.lost_focus())
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                fetch_clicked = true;
                            }
                            let fetch_label = if upload.fetching() {
                                "FETCHING…"
                            } else if upload.info_loaded {
                                "REFETCH"
                            } else {
                                "FETCH DETAILS"
                            };
                            if ui
                                .add_enabled(
                                    !upload.fetching() && !upload.importing(),
                                    egui::Button::new(fetch_label),
                                )
                                .clicked()
                            {
                                fetch_clicked = true;
                            }
                        });
                        if upload.fetching() {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(
                                    RichText::new("Reading the link and resolving the audio source…")
                                        .size(10.0)
                                        .color(MUTED),
                                );
                            });
                        }
                        if upload.info_loaded {
                            let audio_ok = !upload.audio_url.trim().is_empty();
                            let accent = if audio_ok { MINT } else { GOLD };
                            egui::Frame::NONE
                                .fill(alpha(accent, 18))
                                .corner_radius(8.0)
                                .inner_margin(egui::Margin::symmetric(10, 8))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(if audio_ok {
                                                "AUDIO READY"
                                            } else {
                                                "AUDIO NOT RESOLVED"
                                            })
                                            .size(12.0)
                                            .strong()
                                            .color(accent),
                                        );
                                        if upload.duration > 0.0 {
                                            ui.label(
                                                RichText::new(format!(
                                                    "· {}",
                                                    format_time(upload.duration)
                                                ))
                                                .size(10.0)
                                                .color(MUTED),
                                            );
                                        }
                                        let lyric_lines = upload
                                            .lyrics
                                            .lines()
                                            .filter(|line| !line.trim().is_empty())
                                            .count();
                                        ui.label(
                                            RichText::new(if lyric_lines > 0 {
                                                format!("· {lyric_lines} lyric lines")
                                            } else {
                                                "· no lyrics found — add them below".into()
                                            })
                                            .size(10.0)
                                            .color(MUTED),
                                        );
                                    });
                                    if audio_ok {
                                        ui.label(
                                            RichText::new("The audio source is resolved. Edit anything before importing.")
                                                .size(10.0)
                                                .color(MUTED),
                                        );
                                    }
                                });
                        }
                        ui.add_space(10.0);
                        ui.label(RichText::new("EXISTING METADATA").monospace().size(8.0).color(MUTED2));
                        ui.horizontal(|ui| {
                            let combo_width = 400.0;
                            let artist_label = if upload.artist.trim().is_empty() {
                                "Choose an existing artist".to_string()
                            } else {
                                format!("Artist: {}", upload.artist)
                            };
                            egui::ComboBox::from_id_salt("upload_existing_artist")
                                .selected_text(artist_label)
                                .width(combo_width)
                                .close_behavior(egui::popup::PopupCloseBehavior::CloseOnClickOutside)
                                .show_ui(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut upload.artist_search)
                                            .hint_text("Search artists…")
                                            .desired_width(f32::INFINITY),
                                    );
                                    let query = upload.artist_search.trim().to_lowercase();
                                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                                        let mut matches = 0usize;
                                        for artist in &artists {
                                            if !query.is_empty()
                                                && !artist.to_lowercase().contains(&query)
                                            {
                                                continue;
                                            }
                                            matches += 1;
                                            if ui.selectable_label(false, artist).clicked() {
                                                pick_artist = Some(artist.clone());
                                                ui.close_menu();
                                                ui.memory_mut(|memory| memory.close_popup());
                                            }
                                        }
                                        if matches == 0 {
                                            ui.label(
                                                RichText::new("No matching artists")
                                                    .size(10.0)
                                                    .color(MUTED2),
                                            );
                                        }
                                    });
                                });
                            ui.add_space(12.0);
                            let album_label = if upload.album.trim().is_empty() {
                                "Choose an existing album".to_string()
                            } else if upload.artist.trim().is_empty() {
                                format!("Album: {}", upload.album)
                            } else {
                                format!("{} · {}", upload.album, upload.artist)
                            };
                            egui::ComboBox::from_id_salt("upload_existing_album")
                                .selected_text(album_label)
                                .width(combo_width)
                                .close_behavior(egui::popup::PopupCloseBehavior::CloseOnClickOutside)
                                .show_ui(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut upload.album_search)
                                            .hint_text("Search albums…")
                                            .desired_width(f32::INFINITY),
                                    );
                                    let query = upload.album_search.trim().to_lowercase();
                                    let filtered: Vec<&(String, String)> = albums
                                        .iter()
                                        .filter(|(album, artist)| {
                                            (upload.artist.trim().is_empty()
                                                || artist.eq_ignore_ascii_case(upload.artist.trim()))
                                                && (query.is_empty()
                                                    || album.to_lowercase().contains(&query)
                                                    || artist.to_lowercase().contains(&query))
                                        })
                                        .collect();
                                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                                        if filtered.is_empty() {
                                            ui.label(
                                                RichText::new("No matching albums — create a new one below.")
                                                    .size(10.0)
                                                    .color(MUTED2),
                                            );
                                        }
                                        for (album, artist) in filtered {
                                            if ui
                                                .selectable_label(false, format!("{album} · {artist}"))
                                                .clicked()
                                            {
                                                pick_album = Some((album.clone(), artist.clone()));
                                                ui.close_menu();
                                                ui.memory_mut(|memory| memory.close_popup());
                                            }
                                        }
                                    });
                                });
                        });
                        ui.add_space(10.0);
                        egui::Grid::new("upload_metadata_grid")
                            .num_columns(2)
                            .spacing([12.0, 6.0])
                            .show(ui, |ui| {
                                ui.label("Title");
                                ui.add(egui::TextEdit::singleline(&mut upload.title).desired_width(680.0));
                                ui.end_row();
                                ui.label("Artist");
                                ui.add(egui::TextEdit::singleline(&mut upload.artist).desired_width(680.0));
                                ui.end_row();
                                ui.label("Album");
                                ui.add(egui::TextEdit::singleline(&mut upload.album).desired_width(680.0));
                                ui.end_row();
                                ui.label("Release date");
                                let release = ui.add(
                                    egui::TextEdit::singleline(&mut upload.release_date)
                                        .desired_width(680.0),
                                );
                                if upload.release_date.is_empty() {
                                    paint_placeholder(ui, release.rect, "YYYY-MM-DD", false);
                                }
                                ui.end_row();
                                ui.label("Artist bio");
                                ui.add(egui::TextEdit::multiline(&mut upload.artist_bio).desired_rows(2).desired_width(680.0));
                                ui.end_row();
                                ui.label("Album bio");
                                ui.add(egui::TextEdit::multiline(&mut upload.album_bio).desired_rows(2).desired_width(680.0));
                                ui.end_row();
                            });
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("COVER ART").monospace().size(8.0).color(MUTED2));
                            if upload.cover_fetching() {
                                ui.spinner();
                                ui.label(RichText::new("Loading preview…").size(10.0).color(GOLD));
                            } else {
                                ui.label(
                                    RichText::new("Click the image area to choose, link, or paste artwork.")
                                        .size(10.0)
                                        .color(MUTED2),
                                );
                            }
                        });
                        let (cover_rect, cover_response) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 150.0),
                            Sense::click(),
                        );
                        paint_cover_preview(
                            ui,
                            cover_rect,
                            upload.cover_preview_tex.as_ref(),
                            !upload.cover.trim().is_empty(),
                            upload.cover_fetching(),
                        );
                        if cover_response.clicked() {
                            upload.cover_popup_open = true;
                        }
                        if !upload.cover_message.is_empty() {
                            ui.label(
                                RichText::new(&upload.cover_message)
                                    .size(10.0)
                                    .color(if upload.cover_error { PINK } else { MUTED }),
                            );
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("LYRICS").monospace().size(8.0).color(MUTED2));
                            if let Some(provider) = &upload.lyrics_provider {
                                ui.label(
                                    RichText::new(format!(
                                        "{}{}",
                                        provider,
                                        if upload.lyrics_synced { " · synced" } else { "" }
                                    ))
                                    .size(10.0)
                                    .color(MINT),
                                );
                            }
                            if upload.duration > 0.0 {
                                ui.label(RichText::new(format!("· {}", format_time(upload.duration))).size(10.0).color(MUTED2));
                            }
                        });
                        ui.label(
                            RichText::new("One sung line per row. Words are split automatically into the editor's word bank.")
                                .size(10.0)
                                .color(MUTED2),
                        );
                        let lyrics = ui.add(
                            egui::TextEdit::multiline(&mut upload.lyrics)
                                .desired_rows(6)
                                .desired_width(f32::INFINITY),
                        );
                        if upload.lyrics.is_empty() {
                            paint_placeholder(
                                ui,
                                lyrics.rect,
                                "Paste lyrics here if the lookup did not find them.",
                                true,
                            );
                        }
                    });
                    ui.add_space(6.0);
                    ui.separator();
                    if upload.importing() {
                        ui.add(
                            egui::ProgressBar::new(upload.progress / 100.0)
                                .show_percentage()
                                .text(format!("{} · {}", upload.stage, upload.message)),
                        );
                        ui.label(
                            RichText::new("The editor opens automatically after the backing and vocal stems are ready.")
                                .size(10.0)
                                .color(MUTED),
                        );
                    } else {
                        let button = egui::Button::new("DOWNLOAD, SPLIT & OPEN EDITOR")
                            .min_size(Vec2::new(ui.available_width(), 36.0));
                        if ui
                            .add_enabled(!upload.url.trim().is_empty(), button)
                            .clicked()
                        {
                            import_clicked = true;
                        }
                        ui.label(
                            RichText::new("Requires Node, yt-dlp, ffmpeg and Demucs. The repo .venv is detected automatically.")
                                .size(9.0)
                                .color(MUTED2),
                        );
                    }
                    if !upload.message.is_empty() {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(&upload.message)
                                .size(11.0)
                                .color(if upload.info_loaded { MUTED } else { PINK }),
                        );
                    }
                });
            if !open && !upload.importing() {
                upload.open = false;
            }
        }
        if let Some(artist) = pick_artist {
            self.upload.artist = artist.clone();
            self.upload.artist_search.clear();
            self.upload.album_search.clear();
            if self.upload.artist_bio.trim().is_empty() {
                if let Some(bio) = self
                    .songs
                    .iter()
                    .find(|song| song.artist.as_deref() == Some(artist.as_str()))
                    .and_then(|song| song.artist_bio.clone())
                {
                    self.upload.artist_bio = bio;
                }
            }
        }
        if let Some((album, artist)) = pick_album {
            let from_song = self
                .songs
                .iter()
                .find(|song| {
                    song.album.as_deref() == Some(album.as_str())
                        && song.artist.as_deref() == Some(artist.as_str())
                })
                .cloned();
            self.upload.album = album;
            self.upload.album_search.clear();
            if self.upload.artist.trim().is_empty() {
                self.upload.artist = artist;
            }
            if let Some(song) = from_song {
                if self.upload.release_date.trim().is_empty() {
                    self.upload.release_date = song.release_date.unwrap_or_default();
                }
                if self.upload.album_bio.trim().is_empty() {
                    self.upload.album_bio = song.album_bio.unwrap_or_default();
                }
                if self.upload.artist_bio.trim().is_empty() {
                    self.upload.artist_bio = song.artist_bio.unwrap_or_default();
                }
                if self.upload.cover.trim().is_empty() {
                    if let Some(cover) = &song.cover {
                        let path = self
                            .root
                            .join("assets/songs")
                            .join(&song.id)
                            .join(cover);
                        self.set_cover_from_path(path, "Cover copied from the existing album.");
                    }
                }
            }
        }
        if fetch_clicked {
            self.start_fetch(now);
        }
        if import_clicked {
            self.start_import(now);
        }
        self.draw_cover_popup(ctx);
    }

    fn draw_range_test(&mut self, ctx: &egui::Context) {
        let mut open = self.range_test_open;
        egui::Window::new("Vocal range")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(430.0);
                ui.label(
                    RichText::new(
                        "Sing a comfortable low note, hold it, then slide up to a comfortable high note.                          The picker records only stable pitches.",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
                ui.add_space(8.0);
                let mic_ok = self.mic.is_some();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(
                            self.mic
                                .as_ref()
                                .map(|mic| mic.device_name.clone())
                                .or_else(|| self.mic_error.clone())
                                .unwrap_or_else(|| "No microphone".into()),
                        )
                        .size(11.0)
                        .color(if mic_ok { MINT } else { PINK }),
                    );
                    if ui.small_button("Retry").clicked() {
                        self.start_mic();
                    }
                });
                let current = self
                    .reading
                    .map(|reading| format!("{}   {:+.0}¢", note_name(reading.midi.round() as i32), reading.cents))
                    .unwrap_or_else(|| "—".into());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Now").size(12.0).color(MUTED));
                    ui.label(RichText::new(current).size(26.0).color(INK).strong());
                });
                let low = self.range_low_seen.or(self.settings.vocal_low);
                let high = self.range_high_seen.or(self.settings.vocal_high);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Lowest").size(11.0).color(MUTED));
                    ui.label(
                        RichText::new(low.map(note_name).unwrap_or_else(|| "—".into()))
                            .size(18.0)
                            .color(MINT),
                    );
                    ui.add_space(20.0);
                    ui.label(RichText::new("Highest").size(11.0).color(MUTED));
                    ui.label(
                        RichText::new(high.map(note_name).unwrap_or_else(|| "—".into()))
                            .size(18.0)
                            .color(GOLD),
                    );
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let listen = if self.range_testing { "PAUSE" } else { "LISTEN" };
                    if ui.button(listen).clicked() {
                        self.range_testing = !self.range_testing;
                    }
                    if ui.button("Reset").clicked() {
                        self.range_low_seen = None;
                        self.range_high_seen = None;
                        self.range_last_note = None;
                        self.range_note_frames = 0;
                    }
                    let can_save = self.range_low_seen.is_some() && self.range_high_seen.is_some();
                    if ui.add_enabled(can_save, egui::Button::new("Save range")).clicked() {
                        self.settings.vocal_low = self.range_low_seen;
                        self.settings.vocal_high = self.range_high_seen;
                        save_settings(&self.settings);
                        self.toast("Vocal range saved.", ctx.input(|i| i.time));
                    }
                });
                if let (Some(low), Some(high)) = (self.settings.vocal_low, self.settings.vocal_high) {
                    ui.label(
                        RichText::new(format!(
                            "Saved range: {} – {} · {} semitones",
                            note_name(low),
                            note_name(high),
                            high - low
                        ))
                        .size(10.0)
                        .color(MUTED2),
                    );
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Use the octave shift in Settings, or Fit current song to my range, to move a chart into this range.")
                        .size(10.0)
                        .color(MUTED2),
                );
            });
        if open != self.range_test_open {
            self.range_test_open = open;
            if !open {
                self.range_testing = false;
            }
        }
    }

    fn draw_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        let mut changed = false;
        let octave_before = self.settings.octave_shift;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(480.0);
                ui.add_space(4.0);
                ui.label(RichText::new("MICROPHONE").monospace().size(9.0).color(MUTED2));
                let current_device = self.settings.mic_device.clone();
                let mut pick = current_device.clone();
                ui.horizontal(|ui| {
                    ui.label("Input");
                    egui::ComboBox::from_id_salt("mic_device")
                        .selected_text(pick.clone().unwrap_or_else(|| "System default".into()))
                        .width(230.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut pick, None, "System default");
                            for name in &self.mic_devices {
                                ui.selectable_value(&mut pick, Some(name.clone()), name);
                            }
                        });
                    if ui.small_button("Refresh").clicked() {
                        self.mic_devices = input_device_names();
                    }
                });
                if pick != current_device {
                    self.settings.mic_device = pick;
                    save_settings(&self.settings);
                    self.start_mic();
                }
                let mic_name = self
                    .mic
                    .as_ref()
                    .map(|m| m.device_name.clone())
                    .or_else(|| self.mic_error.clone())
                    .unwrap_or_else(|| "No microphone".into());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(mic_name).size(11.0).color(if self.mic.is_some() { MINT } else { PINK }));
                    if ui.small_button("Retry").clicked() {
                        self.start_mic();
                    }
                });
                changed |= ui.add(egui::Slider::new(&mut self.settings.input_gain, 0.5..=3.0).text("Input gain")).changed();
                ui.add_space(10.0);
                ui.label(RichText::new("PLAYBACK").monospace().size(9.0).color(MUTED2));
                if ui.checkbox(&mut self.settings.backing_enabled, "Play backing track").changed() {
                    if self.settings.backing_enabled && self.settings.backing_volume <= 0.01 {
                        self.settings.backing_volume = 0.9;
                    }
                    changed = true;
                }
                changed |= ui.add(egui::Slider::new(&mut self.settings.backing_volume, 0.0..=1.0).text("Backing level")).changed();
                let mut vocals_on = self.settings.vocals_volume > 0.01;
                if ui.checkbox(&mut vocals_on, "Play original vocals").changed() {
                    self.settings.vocals_volume = if vocals_on { 0.8 } else { 0.0 };
                    changed = true;
                }
                changed |= ui.add(egui::Slider::new(&mut self.settings.vocals_volume, 0.0..=1.0).text("Vocal level")).changed();
                if ui.checkbox(&mut self.settings.show_lyrics, "Show lyric words on notes").changed() {
                    changed = true;
                }
                ui.add_space(10.0);
                ui.label(RichText::new("VOCAL RANGE").monospace().size(9.0).color(MUTED2));
                let range_text = match (self.settings.vocal_low, self.settings.vocal_high) {
                    (Some(low), Some(high)) => format!("{} – {} · {} semitones", note_name(low), note_name(high), high - low),
                    _ => "Not measured".into(),
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(range_text).size(12.0).color(INK));
                    if ui.button("Test range").clicked() {
                        self.start_range_test();
                    }
                });
                changed |= ui
                    .add(egui::Slider::new(&mut self.settings.octave_shift, -2..=2).text("Octave shift"))
                    .changed();
                let can_fit = self.settings.vocal_low.is_some()
                    && self.settings.vocal_high.is_some()
                    && self.stage.is_some();
                if ui.add_enabled(can_fit, egui::Button::new("Fit current song to my range")).clicked() {
                    self.fit_current_song_to_range();
                }
                ui.label(
                    RichText::new("Octave shift moves the chart only; the backing stays in the original key (octaves are equivalent).")
                        .size(9.0)
                        .color(MUTED2),
                );
                ui.add_space(10.0);
                ui.label(RichText::new("STAGE").monospace().size(9.0).color(MUTED2));
                changed |= ui.add(egui::Slider::new(&mut self.settings.lead_time, 4.0..=10.0).text("Notes arrive (s)")).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.sync_ms, -150.0..=150.0).text("Sync trim (ms)")).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.latency_ms, -150.0..=300.0).text("Mic latency (ms)")).changed();
                if ui.checkbox(&mut self.settings.count_in, "Count-in before a take").changed() {
                    changed = true;
                }
                ui.add_space(12.0);
                if ui.button("Clear high scores").clicked() {
                    self.scores.clear();
                    save_scores(&self.scores);
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Headphones recommended — speaker bleed can register as pitch.").size(10.0).color(MUTED2));
            });
        if open != self.settings_open {
            self.settings_open = open;
        }
        if self.settings.octave_shift != octave_before {
            self.apply_octave_shift();
        }
        if changed {
            self.apply_audio_volumes();
            save_settings(&self.settings);
        }
    }
}

impl eframe::App for KaraokeMachineApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_ui_scale(ctx);
        let now = ctx.input(|i| i.time);
        if setting_env("INPUT_DEBUG").is_some() {
            let (pos, any_click, any_down) = ctx.input(|i| {
                (
                    i.pointer.interact_pos(),
                    i.pointer.any_click(),
                    i.pointer.any_down(),
                )
            });
            if any_click || any_down || pos.is_some() {
                eprintln!("input pos={pos:?} click={any_click} down={any_down}");
            }
        }
        if let Some(index) = self.pending_song.take() {
            self.open_song(index, now);
            if self.pending_time > 0.0 {
                if let Some(stage) = &mut self.stage {
                    stage.time = self.pending_time;
                    if let Some(audio) = &self.audio {
                        audio.seek(self.pending_time);
                    }
                    stage.active = stage.active_note_at(self.pending_time);
                }
            }
            if self.auto_play {
                if let Some(audio) = &self.audio {
                    audio.play();
                }
                if let Some(stage) = &mut self.stage {
                    stage.playing = true;
                    stage.started = true;
                }
            }
        }
        self.started_at = if self.started_at == 0.0 { now } else { self.started_at };
        self.handle_keys(ctx, now);
        self.tick_stage(ctx, now);
        self.update_pitch();
        self.update_range_test();
        self.poll_upload(ctx, now);
        self.poll_maps(ctx, now);
        let mut actions = Vec::new();
        let mut editor_event = EditorEvent::None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                ui.painter().rect_filled(rect, 0.0, BG);
                if self.editor.is_some() {
                    if let Some(editor) = &mut self.editor {
                        editor_event = editor.draw(ui, &self.audio);
                    }
                } else if self.stage.is_some() {
                    self.draw_stage(ui, &mut actions, now);
                } else {
                    self.draw_lobby(ui, &mut actions);
                }
                if self.drawer_open && self.editor.is_none() {
                    self.draw_drawer(ui, &mut actions, now);
                }
            });
        match editor_event {
            EditorEvent::None => {}
            EditorEvent::Back => {
                if let Some(mut editor) = self.editor.take() {
                    editor.pause_audio(&self.audio);
                    if let Err(error) = editor.save_draft(&self.root) {
                        self.toast(error, now);
                    }
                }
                self.audio.as_mut().map(|audio| audio.pause());
                self.settings_open = false;
                self.reload_library(ctx, now);
                self.drawer_open = true;
                self.toast("Draft saved. The song remains UNMAPPED.", now);
            }
            EditorEvent::ExportMap => self.export_editor_map(now),
            EditorEvent::SaveDraft => {
                let result = if let Some(editor) = &mut self.editor {
                    if editor.song.mapped {
                        Err("This song is already mapped. Use Submit Mapping to save edits.".into())
                    } else {
                        editor
                            .save_draft(&self.root)
                            .map(|()| editor.mark_draft_saved())
                    }
                } else {
                    Ok(())
                };
                match result {
                    Ok(()) => self.toast("Draft saved. The song remains UNMAPPED.", now),
                    Err(error) => self.toast(error, now),
                }
            }
            EditorEvent::Submit => {
                if let Some(mut editor) = self.editor.take() {
                    editor.pause_audio(&self.audio);
                    match editor.submit(&self.root) {
                        Ok(()) => {
                            self.reload_library(ctx, now);
                            self.drawer_open = true;
                            self.toast("Mapping submitted. This song is now playable.", now);
                        }
                        Err(error) => {
                            self.editor = Some(editor);
                            self.toast(error, now);
                        }
                    }
                }
            }
        }
        if self.settings_open {
            self.draw_settings(ctx);
        }
        if self.range_test_open {
            self.draw_range_test(ctx);
        }
        if self.upload.open {
            self.draw_upload(ctx, now);
        }
        if let Some(toast) = &self.toast {
            let message = toast.message.clone();
            let until = toast.until;
            if now < until {
                egui::Area::new(Id::new("toast"))
                    .fixed_pos(Pos2::new(ctx.screen_rect().center().x - 180.0, 82.0))
                    .show(ctx, |ui| {
                        egui::Frame::NONE
                            .fill(Color32::from_rgba_unmultiplied(9, 13, 23, 240))
                            .corner_radius(16.0)
                            .inner_margin(egui::Margin::symmetric(16, 9))
                            .show(ui, |ui| {
                                ui.label(RichText::new(message).size(11.0).color(INK));
                            });
                    });
            } else {
                self.toast = None;
            }
        }
        for action in actions {
            self.handle_action(action, now, ctx);
        }
        if let Some(path) = self.shot_path.clone() {
            if !self.shot_requested && now - self.started_at >= self.shot_at {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                self.shot_requested = true;
                ctx.request_repaint();
            }
            let events = ctx.input(|i| i.events.clone());
            for event in events {
                if let egui::Event::Screenshot { image, .. } = event {
                    let [w, h] = image.size;
                    let mut rgba = Vec::with_capacity(w * h * 4);
                    for pixel in &image.pixels {
                        rgba.push(pixel.r());
                        rgba.push(pixel.g());
                        rgba.push(pixel.b());
                        rgba.push(pixel.a());
                    }
                    eprintln!("saving screenshot {}x{} to {}", w, h, path.display());
                    match image::save_buffer(&path, &rgba, w as u32, h as u32, image::ColorType::Rgba8) {
                        Ok(()) => eprintln!("screenshot saved"),
                        Err(error) => eprintln!("screenshot save failed: {error}"),
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    break;
                }
            }
        }
        if self.editor.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if let Some(stage) = &self.stage {
            if stage.playing || stage.count_in_until.is_some() || self.mic.is_some() || self.range_test_open {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        } else if self.range_test_open {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        if self.shot_path.is_some() && !self.shot_requested {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if self.fps_log {
            self.fps_frames += 1;
            if self.fps_last == 0.0 {
                self.fps_last = now;
            } else if now - self.fps_last >= 2.0 {
                eprintln!("fps: {:.1}", self.fps_frames as f64 / (now - self.fps_last));
                self.fps_frames = 0;
                self.fps_last = now;
            }
            ctx.request_repaint();
        }
    }
}

fn run_cover_download(root: &Path, url: &str, out: &Path) -> Result<PathBuf, String> {
    let script = root.join("dist-tools/fetch-cover.js");
    if !script.exists() {
        return Err("Cover preview tools are not built. Run `npm install && npm run assets`.".into());
    }
    let output = Command::new("node")
        .arg(&script)
        .arg("--url")
        .arg(url)
        .arg("--out")
        .arg(out)
        .current_dir(root)
        .output()
        .map_err(|error| format!("Could not start the cover downloader: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Cover download failed: {}",
            stderr
                .lines()
                .next_back()
                .unwrap_or("unknown error")
        ));
    }
    let bytes = std::fs::read(out)
        .map_err(|error| format!("Could not read the downloaded cover: {error}"))?;
    let image = image::load_from_memory(&bytes)
        .map_err(|error| format!("The downloaded cover was not an image: {error}"))?;
    let png = out.with_extension("png");
    image
        .save(&png)
        .map_err(|error| format!("Could not save the cover preview: {error}"))?;
    let _ = std::fs::remove_file(out);
    Ok(png)
}

fn paint_placeholder(ui: &egui::Ui, rect: Rect, text: &str, multiline: bool) {
    let painter = ui.painter();
    let color = Color32::from_rgba_unmultiplied(94, 102, 121, 55);
    if multiline {
        painter.text(
            rect.left_top() + Vec2::new(8.0, 7.0),
            Align2::LEFT_TOP,
            text,
            FontId::proportional(12.0),
            color,
        );
    } else {
        painter.text(
            rect.left_center() + Vec2::new(7.0, 0.0),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(12.0),
            color,
        );
    }
}

fn paint_cover_preview(
    ui: &egui::Ui,
    rect: Rect,
    texture: Option<&TextureHandle>,
    has_cover: bool,
    loading: bool,
) {
    let p = ui.painter();
    p.rect_filled(rect, 12.0, Color32::from_rgb(8, 12, 21));
    if let Some(texture) = texture {
        let size = texture.size_vec2();
        let scale = (rect.width() / size.x.max(1.0))
            .min(rect.height() / size.y.max(1.0));
        let target = Rect::from_center_size(rect.center(), size * scale);
        p.image(
            texture.id(),
            target,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        let label = Rect::from_min_size(
            Pos2::new(rect.right() - 126.0, rect.bottom() - 36.0),
            Vec2::new(114.0, 24.0),
        );
        p.rect_filled(label, 7.0, Color32::from_rgba_unmultiplied(5, 8, 15, 210));
        p.text(
            label.center(),
            Align2::CENTER_CENTER,
            "CHANGE COVER",
            FontId::monospace(7.0),
            MINT,
        );
    } else {
        let center = rect.center() + Vec2::new(0.0, -20.0);
        let tray = Rect::from_center_size(center + Vec2::new(0.0, 12.0), Vec2::new(42.0, 10.0));
        p.rect_stroke(
            tray,
            3.0,
            Stroke::new(1.6f32, alpha(MUTED, 180)),
            StrokeKind::Inside,
        );
        p.line_segment(
            [
                Pos2::new(center.x, center.y - 16.0),
                Pos2::new(center.x, center.y + 10.0),
            ],
            Stroke::new(1.7f32, MUTED),
        );
        p.line_segment(
            [
                Pos2::new(center.x - 9.0, center.y - 7.0),
                Pos2::new(center.x, center.y - 16.0),
            ],
            Stroke::new(1.7f32, MUTED),
        );
        p.line_segment(
            [
                Pos2::new(center.x + 9.0, center.y - 7.0),
                Pos2::new(center.x, center.y - 16.0),
            ],
            Stroke::new(1.7f32, MUTED),
        );
        let title = if loading {
            "Loading cover art…"
        } else if has_cover {
            "Cover linked — click to preview or replace"
        } else {
            "Click to upload cover art"
        };
        p.text(
            center + Vec2::new(0.0, 48.0),
            Align2::CENTER_CENTER,
            title,
            FontId::proportional(14.0),
            if loading { GOLD } else { MUTED },
        );
        p.text(
            center + Vec2::new(0.0, 68.0),
            Align2::CENTER_CENTER,
            "PNG, JPG, image link, or clipboard paste",
            FontId::monospace(8.0),
            MUTED2,
        );
    }
    p.rect_stroke(
        rect,
        12.0,
        Stroke::new(
            1.0f32,
            if has_cover {
                alpha(MINT, 80)
            } else {
                Color32::from_white_alpha(28)
            },
        ),
        StrokeKind::Inside,
    );
}

fn http_get_text(url: &str) -> Result<String, String> {
    let script = "fetch(process.argv[1],{headers:{'user-agent':'KaraokeMachine/2.0'}}).then(async r=>{if(!r.ok)throw new Error('HTTP '+r.status);process.stdout.write(await r.text())}).catch(e=>{console.error(e.message);process.exit(1)})";
    let node_error = match Command::new("node")
        .arg("-e")
        .arg(script)
        .arg(url)
        .output()
    {
        Ok(output) if output.status.success() => {
            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }
        Ok(output) => String::from_utf8_lossy(&output.stderr).trim().to_string(),
        Err(error) => error.to_string(),
    };

    let output = Command::new("curl")
        .args(["-fsSL", url])
        .output()
        .map_err(|error| format!("Could not download {url}: {error}"))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if message.is_empty() { node_error } else { message });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn encode_path_segment(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn load_data_uri_texture(
    ctx: &egui::Context,
    key: &str,
    data_uri: &str,
) -> Option<TextureHandle> {
    let (_, payload) = data_uri.split_once(";base64,")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    let color = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    Some(ctx.load_texture(
        format!("map-cover-{key}"),
        color,
        TextureOptions::LINEAR,
    ))
}

fn write_song_file(dir: &Path, song: &Song) -> Result<(), String> {
    let path = dir.join("song.json");
    let raw = serde_json::to_string_pretty(song).map_err(|error| error.to_string())?;
    std::fs::write(&path, raw)
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

fn run_song_info(root: &Path, url: &str) -> Result<TrackInfo, String> {
    let script = root.join("dist-tools/song-info.js");
    let output = Command::new("node")
        .arg(&script)
        .arg("--url")
        .arg(url)
        .current_dir(root)
        .output()
        .map_err(|error| format!("Could not run Node: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stdout.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("");
    let parsed: TrackInfo = serde_json::from_str(line.trim()).map_err(|error| {
        format!(
            "Could not read the track info: {error}{}",
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!("\n{}", stderr.lines().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"))
            }
        )
    })?;
    if let Some(error) = &parsed.error {
        return Err(error.clone());
    }
    if !output.status.success() {
        return Err(format!("Track lookup failed: {}", parsed.error.unwrap_or_else(|| "unknown error".into())));
    }
    Ok(parsed)
}

fn setting_env(name: &str) -> Option<String> {
    std::env::var(format!("KARAOKEMACHINE_{name}"))
        .or_else(|_| std::env::var(format!("KEYLINE_{name}")))
        .ok()
}

fn stage_needs_input(stage: &Option<Stage>) -> bool {
    stage.as_ref().map(|s| s.playing || s.result.is_none()).unwrap_or(false)
}

fn ctx_pointer(ui: &egui::Ui) -> Option<Pos2> {
    ui.input(|i| i.pointer.interact_pos())
}

fn paint_cover_fallback(p: &egui::Painter, rect: Rect, title: &str) {
    let c = pitch_col(((title.len() * 37) % 100) as f32 / 100.0);
    p.rect_filled(rect, 12.0, alpha(c, 80));
    p.circle_filled(rect.center(), rect.width() * 0.22, alpha(c, 100));
    p.text(rect.center(), Align2::CENTER_CENTER, title.chars().next().unwrap_or('K').to_string(), FontId::proportional(rect.width() * 0.25), INK);
}

fn load_texture(ctx: &egui::Context, path: &Path) -> Option<TextureHandle> {
    let image = image::open(path).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    let color = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    Some(ctx.load_texture(path.to_string_lossy(), color, TextureOptions::LINEAR))
}

fn draw_melody(
    p: &egui::Painter,
    stage: &Stage,
    layout: &Layout,
    active: i32,
    chart_time: f64,
    width: f32,
) {
    let notes = &stage.song.notes;
    if notes.is_empty() {
        return;
    }
    let t = chart_time;
    let h = layout.note_height();
    let mut mesh = Mesh::default();
    let mut previous: Option<(f32, f32)> = None;

    for (i, note) in notes.iter().enumerate() {
        let x0 = layout.hit_x + (note.start - t) as f32 * layout.pps;
        let x1 = layout.hit_x + (note.start + note.dur - t) as f32 * layout.pps;
        if x1 < layout.hit_x - 2.0 {
            previous = None;
            continue;
        }
        if x0 > width + 60.0 {
            break;
        }
        let y = layout.lane_y(note.midi);
        let group_prev = i > 0 && {
            let prev = &notes[i - 1];
            let gap = note.start - (prev.start + prev.dur);
            let limit = if note.ribbon.is_some() { 3.0 } else { 0.85 };
            gap <= limit && prev.shares_ribbon(note)
        };
        let group_next = notes.get(i + 1).map(|next| {
            let gap = next.start - (note.start + note.dur);
            let limit = if note.ribbon.is_some() { 3.0 } else { 0.85 };
            gap <= limit && note.shares_ribbon(next)
        }).unwrap_or(false);
        // Each note keeps its own rounded body. A thin neck connects it to the
        // next note of the same word/phrase, so the word remains one ribbon but
        // every note is visible.
        let sx = x0.max(layout.hit_x) + if group_prev { 0.6 } else { 2.0 };
        let ex = (x1 - if group_next { 0.6 } else { 2.0 }).max(sx + 1.0);

        if let Some((prev_end, prev_y)) = previous {
            if group_prev && sx > prev_end + 0.5 {
                let steps = 8usize;
                let mut bridge = Vec::with_capacity(steps + 1);
                for step in 0..=steps {
                    let f = step as f32 / steps as f32;
                    let smooth = f * f * (3.0 - 2.0 * f);
                    bridge.push(Pos2::new(
                        prev_end + (sx - prev_end) * f,
                        prev_y + (y - prev_y) * smooth,
                    ));
                }
                add_ribbon_mesh(
                    &mut mesh,
                    &bridge,
                    (h * 0.38).max(3.0),
                    layout.low,
                    layout.high,
                    layout.lane_top,
                    layout.lane_bottom,
                    0.95,
                );
            }
        }

        add_ribbon_mesh(
            &mut mesh,
            &[Pos2::new(sx, y), Pos2::new(ex, y)],
            h,
            layout.low,
            layout.high,
            layout.lane_top,
            layout.lane_bottom,
            0.95,
        );
        previous = Some((ex, y));
    }
    p.add(Shape::mesh(mesh));
    if active >= 0 {
        if let Some(note) = notes.get(active as usize) {
            let x0 = layout.hit_x + (note.start - t) as f32 * layout.pps;
            let x1 = layout.hit_x + (note.start + note.dur - t) as f32 * layout.pps;
            let y = layout.lane_y(note.midi);
            let r = Rect::from_min_max(
                Pos2::new(x0.max(layout.hit_x), y - h / 2.0),
                Pos2::new(x1.max(layout.hit_x + 8.0), y + h / 2.0),
            );
            p.rect_stroke(r, h / 2.0, Stroke::new(1.4f32, Color32::WHITE), StrokeKind::Inside);
        }
    }
}

fn add_ribbon_mesh(
    mesh: &mut Mesh,
    points: &[Pos2],
    width: f32,
    low: i32,
    high: i32,
    lane_top: f32,
    lane_bottom: f32,
    opacity: f32,
) {
    if points.len() < 2 {
        return;
    }
    let mut clean: Vec<Pos2> = Vec::with_capacity(points.len());
    for &point in points {
        if clean.last().map(|last| (point.x - last.x).abs() > 0.01 || (point.y - last.y).abs() > 0.01).unwrap_or(true) {
            clean.push(point);
        }
    }
    if clean.len() < 2 {
        return;
    }
    let smooth = clean;
    let base = mesh.vertices.len() as u32;
    for (i, point) in smooth.iter().enumerate() {
        let prev = smooth[i.saturating_sub(1)];
        let next = smooth[(i + 1).min(smooth.len() - 1)];
        let tangent = (next - prev).normalized();
        let normal = Vec2::new(-tangent.y, tangent.x) * (width / 2.0);
        let top = *point + normal;
        let bottom = *point - normal;
        let t = ((point.y - lane_top) / (lane_bottom - lane_top).max(1.0)).clamp(0.0, 1.0);
        let c = alpha(pitch_col(t), (opacity * 255.0) as u8);
        mesh.colored_vertex(top, c);
        mesh.colored_vertex(bottom, c);
        if i > 0 {
            let i0 = base + (i as u32 - 1) * 2;
            let i1 = base + i as u32 * 2;
            mesh.add_triangle(i0, i0 + 1, i1);
            mesh.add_triangle(i0 + 1, i1 + 1, i1);
        }
    }
    // rounded caps
    let first = smooth[0];
    let last = smooth[smooth.len() - 1];
    let radius = width / 2.0;
    let add_cap = |mesh: &mut Mesh, center: Pos2| {
        let t = ((center.y - lane_top) / (lane_bottom - lane_top).max(1.0)).clamp(0.0, 1.0);
        let c = alpha(pitch_col(t), (opacity * 255.0) as u8);
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(center, c);
        const SEGMENTS: usize = 12;
        for i in 0..=SEGMENTS {
            let angle = std::f32::consts::TAU * i as f32 / SEGMENTS as f32;
            mesh.colored_vertex(center + Vec2::new(angle.cos(), angle.sin()) * radius, c);
        }
        for i in 0..SEGMENTS {
            mesh.add_triangle(base, base + 1 + i as u32, base + 2 + i as u32);
        }
    };
    add_cap(mesh, first);
    add_cap(mesh, last);
    let _ = (low, high);
}

fn draw_placed_words(p: &egui::Painter, stage: &Stage, layout: &Layout, time: f64, width: f32) {
    for word in &stage.song.placed_words {
        if word.start < time - 1.5 || word.start > time + 11.0 {
            continue;
        }
        let x = layout.hit_x + (word.start - time) as f32 * layout.pps;
        if x < layout.hit_x - 80.0 || x > width + 100.0 {
            continue;
        }
        let y = (layout.lane_top
            + word.y.clamp(0.02, 0.98) * (layout.lane_bottom - layout.lane_top))
            .clamp(layout.lane_top + 12.0, layout.lane_bottom - 12.0);
        let active = (time - word.start).abs() <= 0.45;
        let font = FontId::proportional(if active { 15.0 } else { 13.0 });
        let text_w = word.text.chars().count() as f32 * 7.4 + 14.0;
        let pill_w = text_w.min(190.0);
        let half = pill_w * 0.5 + 2.0;
        let cx = x.max(layout.hit_x + half).min(width - half);
        let pill = Rect::from_center_size(Pos2::new(cx, y), Vec2::new(pill_w, 24.0));
        p.rect_filled(
            pill,
            5.0,
            Color32::from_rgba_unmultiplied(5, 8, 15, if active { 205 } else { 120 }),
        );
        p.rect_stroke(
            pill,
            5.0,
            Stroke::new(
                1.0f32,
                if active {
                    alpha(GOLD, 110)
                } else {
                    Color32::from_white_alpha(22)
                },
            ),
            StrokeKind::Inside,
        );
        if active {
            p.line_segment(
                [
                    Pos2::new(pill.left() + 5.0, pill.bottom() + 2.0),
                    Pos2::new(pill.right() - 5.0, pill.bottom() + 2.0),
                ],
                Stroke::new(2.0f32, GOLD),
            );
        }
        let display = if word.text.chars().count() > 24 {
            format!("{}…", word.text.chars().take(23).collect::<String>())
        } else {
            word.text.clone()
        };
        p.text(
            pill.center(),
            Align2::CENTER_CENTER,
            display,
            font,
            if active { Color32::WHITE } else { alpha(INK, 175) },
        );
    }
}

fn draw_words(p: &egui::Painter, stage: &Stage, layout: &Layout, time: f64, width: f32) {
    let h = layout.note_height();
    let notes = &stage.song.notes;
    let mut index = 0usize;
    while index < notes.len() {
        let Some(text) = notes[index].word.as_deref() else {
            index += 1;
            continue;
        };

        let mut end_index = index + 1;
        let mut has_rest = false;
        while end_index < notes.len() {
            let previous = &notes[end_index - 1];
            let next = &notes[end_index];
            let same_word = same_lyric_word(&notes[index], next);
            let gap = next.start - (previous.start + previous.dur);
            // An explicit wordIndex identifies one word occurrence, so it stays
            // one label even if the singer takes a breath inside the word.
            let joined = if next.word_index.is_some() && notes[index].word_index.is_some() {
                gap <= 3.0
            } else {
                gap <= 0.85
            };
            if !same_word || !joined {
                break;
            }
            if gap > 0.85 {
                has_rest = true;
            }
            end_index += 1;
        }
        let group = &notes[index..end_index];
        index = end_index;

        let start = group[0].start;
        let end = group
            .iter()
            .map(|note| note.start + note.dur)
            .fold(start, f64::max);
        if end < time - 0.35 || start > time + 10.0 {
            continue;
        }
        let anchor = group
            .iter()
            .max_by(|a, b| a.dur.partial_cmp(&b.dur).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(&group[0]);
        let centre = if has_rest {
            group[0].start + group[0].dur * 0.5
        } else {
            (start + end) * 0.5
        };
        let x = layout.hit_x + (centre - time) as f32 * layout.pps;
        if x < layout.hit_x + 4.0 || x > width + 80.0 {
            continue;
        }
        let y = layout.lane_y(anchor.midi) - h / 2.0 - 17.0;
        if y < layout.lane_top + 8.0 || y > layout.lane_bottom - 2.0 {
            continue;
        }
        let active = time >= start - 0.02 && time <= end + 0.02;
        let fg = if active { Color32::WHITE } else { alpha(INK, 170) };
        let font = FontId::proportional(if active { 15.0 } else { 13.5 });
        let width_est = text.chars().count() as f32 * 7.4 + 12.0;
        let pill_w = width_est.min(170.0);
        // Keep the whole word readable while it passes the playhead.
        let half = pill_w * 0.5 + 2.0;
        let cx = x.max(layout.hit_x + half).min(width - half);
        let pill = Rect::from_center_size(Pos2::new(cx, y), Vec2::new(pill_w, 24.0));
        p.rect_filled(
            pill,
            5.0,
            Color32::from_rgba_unmultiplied(5, 8, 15, if active { 200 } else { 115 }),
        );
        if active {
            p.rect_stroke(pill, 5.0, Stroke::new(1.0f32, alpha(GOLD, 90)), StrokeKind::Inside);
            p.line_segment(
                [
                    Pos2::new(pill.left() + 5.0, pill.bottom() + 2.0),
                    Pos2::new(pill.right() - 5.0, pill.bottom() + 2.0),
                ],
                Stroke::new(2.0f32, GOLD),
            );
        }
        let display = if text.chars().count() > 22 {
            format!("{}…", text.chars().take(21).collect::<String>())
        } else {
            text.to_string()
        };
        p.text(pill.center(), Align2::CENTER_CENTER, display, font, fg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A song whose notes are `(start, dur, midi)` triples.
    fn song(notes: &[(f64, f64, i32)]) -> Song {
        let notes: Vec<_> = notes
            .iter()
            .map(|(start, dur, midi)| json!({ "start": start, "dur": dur, "midi": midi }))
            .collect();
        serde_json::from_value(json!({
            "id": "test",
            "title": "Test",
            "duration": 10.0,
            "mapped": true,
            "notes": notes,
        }))
        .expect("test song")
    }

    fn stage(notes: &[(f64, f64, i32)]) -> Stage {
        Stage::new(song(notes), &Settings::default())
    }

    fn reading(midi: f32) -> Option<PitchReading> {
        Some(PitchReading {
            midi,
            cents: (midi - midi.round()) * 100.0,
            clarity: 1.0,
            rms: 0.2,
        })
    }

    /// Feed the stage one reading every 20 ms from `from` to `to` seconds.
    /// Running past a note's end lets `score_tick` finalize it, so tests do
    /// not need `finish`, which would also write the user's scores file.
    fn sing(stage: &mut Stage, from: f64, to: f64, midi: f32) {
        let mut t = from;
        while t <= to {
            stage.score_tick(t, reading(midi), true);
            t += 0.02;
        }
    }

    fn judgment(stage: &Stage) -> &str {
        stage.judgment.as_ref().map(|j| j.text.as_str()).unwrap_or("")
    }

    #[test]
    fn fold_octave_keeps_cents_and_reports_the_shift() {
        assert_eq!(fold_octave(60.0, 60), (0.0, 0));
        assert_eq!(fold_octave(72.0, 60), (0.0, 1));
        assert_eq!(fold_octave(47.5, 60), (-50.0, -1));
    }

    #[test]
    fn in_tune_notes_are_perfect() {
        let mut stage = stage(&[(0.0, 1.0, 60), (2.0, 1.0, 64)]);
        // Everything is sung at 60: the first note is perfect, the second
        // (target 64) is a miss.
        sing(&mut stage, 0.0, 3.5, 60.0);
        assert_eq!(stage.hit_notes, 1);
        assert_eq!(stage.perfects, 1);
        assert_eq!(judgment(&stage), "MISS");
    }

    #[test]
    fn singing_an_octave_down_counts_as_octave_not_perfect() {
        let mut stage = stage(&[(0.0, 1.0, 60)]);
        sing(&mut stage, 0.0, 1.5, 48.0);
        assert_eq!(stage.hit_notes, 1);
        assert_eq!(stage.perfects, 0);
        assert_eq!(judgment(&stage), "OCTAVE");
    }

    #[test]
    fn seeking_back_and_singing_again_does_not_double_count() {
        let mut stage = stage(&[(0.0, 1.0, 60), (2.0, 1.0, 60)]);
        sing(&mut stage, 0.0, 3.5, 60.0);
        assert_eq!(stage.hit_notes, 2);
        let first_score = stage.score;

        stage.seek(0.0);
        assert_eq!(stage.hit_notes, 0);
        assert_eq!(stage.score, 0);
        assert_eq!(stage.finalize_ptr, 0);

        sing(&mut stage, 0.0, 3.5, 60.0);
        assert_eq!(stage.hit_notes, 2);
        assert_eq!(stage.perfects, 2);
        assert_eq!(stage.score, first_score);
    }

    #[test]
    fn seeking_forward_skips_notes_without_judging_them() {
        let mut stage = stage(&[(0.0, 1.0, 60), (2.0, 1.0, 60)]);
        stage.seek(1.5);
        assert_eq!(stage.finalize_ptr, 1);
        sing(&mut stage, 1.5, 3.5, 60.0);
        assert_eq!(stage.hit_notes, 1);
        assert_eq!(stage.combo, 1);
        assert_eq!(judgment(&stage), "PERFECT");
    }

    #[test]
    fn seek_clamps_to_the_song() {
        let mut stage = stage(&[(0.0, 1.0, 60)]);
        stage.seek(-3.0);
        assert_eq!(stage.time, 0.0);
        stage.seek(1e9);
        assert_eq!(stage.time, stage.duration);
        assert_eq!(stage.finalize_ptr, 1);
    }

    #[test]
    fn seek_is_ignored_while_a_result_is_showing() {
        let mut stage = stage(&[(0.0, 1.0, 60)]);
        sing(&mut stage, 0.0, 1.5, 60.0);
        stage.time = 1.5;
        stage.result = Some(ResultData {
            accuracy: 100.0,
            rank: 'S',
            perfect: 1,
            hit: 1,
            combo: 1,
            score: stage.score,
            new_best: false,
        });
        stage.seek(0.0);
        assert!(stage.result.is_some());
        assert_eq!(stage.time, 1.5);
        assert_eq!(stage.hit_notes, 1);
    }

    #[test]
    fn octave_shift_transposes_notes_and_rescans_from_the_playhead() {
        let mut stage = stage(&[(0.0, 1.0, 60), (2.0, 1.0, 60)]);
        sing(&mut stage, 0.0, 1.5, 60.0);
        stage.time = 1.5;
        stage.set_octave_shift(1);
        assert_eq!(stage.song.notes[0].midi, 72);
        assert_eq!(stage.hit_notes, 0);
        assert_eq!(stage.finalize_ptr, 1);
        sing(&mut stage, 1.5, 3.5, 72.0);
        assert_eq!(stage.perfects, 1);
    }
}
