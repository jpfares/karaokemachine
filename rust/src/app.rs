mod cover;
mod lobby;
mod maps;
mod settings;
mod stage;
mod upload;

use self::stage::Stage;
use self::upload::UploadState;
use crate::audio::{input_device_names, AudioEngine, LoadedAudio, Microphone};
use crate::editor::{
    spawn_raw_pitch_track, Editor, EditorEvent, RAW_PITCH_SAMPLE_RATE, VOCAL_BINS_PER_SECOND,
};
use crate::jobs::Job;
use crate::library::{
    build_albums, find_root, load_map_file, load_map_files, load_scores, load_settings, load_songs,
    maps_dir, read_songlist, read_songlist_path, save_map_file, song_audio_path,
};
use crate::map_package::{MapIndex, MapIndexEntry, MapPackage};
use crate::model::{pitch_color, Album, MapFile, MapInfo, Scores, Settings, Song};
use crate::pitch::{PitchReading, PitchTracker};
use crate::theme::{alpha, BG, INK, MINT, MUTED};
use eframe::egui::{self, Color32, Id, Pos2, RichText, Stroke, TextureHandle, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

const DESIGN_WIDTH: f32 = 1600.0;
const DESIGN_HEIGHT: f32 = 980.0;

fn pitch_col(t: f32) -> Color32 {
    let [r, g, b] = pitch_color(t);
    Color32::from_rgb(r, g, b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrawerView {
    Albums,
    Imports,
    Maps,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MapPickerMode {
    Edit,
    Play,
}

struct MapPicker {
    song_index: usize,
    mode: MapPickerMode,
    selected: Option<usize>,
    new_name: String,
    message: Option<String>,
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
    OpenUpload,
    ShowMaps,
    InstallMap,
    InstallRemoteMap(usize),
    EditSong(usize),
    /// Leave the stage for the home screen.
    Home,
}

/// Vocal-stem analysis the editor needs, computed by the decode job.
struct EditorAudio {
    peaks: Vec<f32>,
    mono: Vec<f32>,
}

struct Toast {
    message: String,
    until: f64,
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
    /// The song being decoded for the stage or the editor.
    audio_load: Option<Job<(LoadedAudio, Option<EditorAudio>)>>,
    mic: Option<Microphone>,
    mic_error: Option<String>,
    mic_devices: Vec<String>,
    tracker: PitchTracker,
    reading: Option<PitchReading>,
    drawer_open: bool,
    drawer_view: DrawerView,
    selected_album: Option<usize>,
    drawer_scroll: f32,
    settings_open: bool,
    range_test_open: bool,
    range_testing: bool,
    range_low_seen: Option<i32>,
    range_high_seen: Option<i32>,
    range_last_note: Option<i32>,
    range_note_frames: u32,
    /// The first click on "Clear high scores" arms a confirmation.
    clear_scores_armed: bool,
    upload: UploadState,
    map_index: Vec<MapIndexEntry>,
    map_index_job: Option<Job<MapIndex>>,
    map_index_loaded: bool,
    map_covers: HashMap<String, TextureHandle>,
    /// Cover downloads in flight, by map id.
    map_cover_jobs: Vec<(String, Job<egui::ColorImage>)>,
    /// The id of the map being downloaded, and the download itself.
    map_download: Option<(String, Job<MapPackage>)>,
    map_picker: Option<MapPicker>,
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
            songs.iter().position(|s| {
                s.id.to_lowercase() == needle || s.title.to_lowercase().contains(&needle)
            })
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
            audio_load: None,
            mic: None,
            mic_error: None,
            mic_devices: input_device_names(),
            tracker: PitchTracker::default(),
            reading: None,
            drawer_open: false,
            drawer_view: DrawerView::Albums,
            selected_album: None,
            drawer_scroll: 0.0,
            settings_open: false,
            range_test_open: false,
            range_testing: false,
            range_low_seen: None,
            range_high_seen: None,
            range_last_note: None,
            range_note_frames: 0,
            clear_scores_armed: false,
            upload: UploadState::default(),
            map_index: Vec::new(),
            map_index_job: None,
            map_index_loaded: false,
            map_covers: HashMap::new(),
            map_cover_jobs: Vec::new(),
            map_download: None,
            map_picker: None,
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
            shot_at: setting_env("SHOT_DELAY")
                .and_then(|v| v.parse().ok())
                .unwrap_or(2.2),
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
            app.drawer_view = DrawerView::Imports;
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
            app.upload.cover.popup_open = true;
        }
        match setting_env("DRAWER").as_deref() {
            Some("imports") => {
                app.drawer_view = DrawerView::Imports;
                app.drawer_open = true;
            }
            Some("maps") => {
                app.drawer_view = DrawerView::Maps;
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
        visuals.widgets.hovered.bg_stroke =
            Stroke::new(1.0f32, Color32::from_rgba_unmultiplied(124, 247, 196, 120));
        visuals.widgets.active.bg_stroke =
            Stroke::new(1.0f32, Color32::from_rgba_unmultiplied(124, 247, 196, 180));
        visuals.override_text_color = Some(INK);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(15, 21, 34);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(24, 32, 49);
        visuals.widgets.active.bg_fill = Color32::from_rgb(31, 42, 61);
        visuals.selection.bg_fill = alpha(MINT, 50);
        visuals.selection.stroke = Stroke::new(1.0f32, MINT);
        // Stock egui widgets (settings, upload form) take the painted
        // widgets' shape and palette so the two families read as one app.
        let corner = egui::CornerRadius::same(4);
        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = corner;
        }
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0f32, INK);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0f32, INK);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0f32, MINT);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0f32, MUTED);
        visuals.window_corner_radius = egui::CornerRadius::same(16);
        visuals.window_stroke = Stroke::new(1.0f32, Color32::from_white_alpha(30));
        visuals.window_shadow = egui::Shadow::NONE;
        ctx.set_visuals(visuals);
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(10.0, 9.0);
        style.spacing.button_padding = Vec2::new(14.0, 8.0);
        style.spacing.slider_width = 160.0;
        // One readable scale: nothing the user has to read is under 11 pt.
        use egui::{FontFamily, TextStyle};
        style.text_styles.insert(
            TextStyle::Small,
            egui::FontId::new(11.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Body,
            egui::FontId::new(13.5, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Button,
            egui::FontId::new(13.5, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            egui::FontId::new(12.0, FontFamily::Monospace),
        );
        ctx.set_style(style);
    }

    fn toast(&mut self, message: impl Into<String>, now: f64) {
        self.toast = Some(Toast {
            message: message.into(),
            until: now + 3.4,
        });
    }

    fn refresh_song_maps(&mut self, index: usize) {
        let Some(song) = self.songs.get_mut(index) else {
            return;
        };
        song.maps = load_map_files(&self.root, &song.id);
        song.mapped = song.maps.iter().any(|map| !map.draft);
    }

    fn open_map_picker(&mut self, index: usize, mode: MapPickerMode, now: f64) {
        if self.songs.get(index).is_none() {
            return;
        }
        self.refresh_song_maps(index);
        let song = &self.songs[index];
        if mode == MapPickerMode::Play {
            if song.maps.len() == 1 {
                let path = song.maps[0].path.clone();
                self.open_song_with_map(index, &path, now);
                return;
            }
            if song.maps.is_empty() {
                self.toast("No map files for this song. Choose one to edit.", now);
                self.open_map_picker(index, MapPickerMode::Edit, now);
                return;
            }
        }
        let selected = None;
        let new_name = if song.maps.iter().any(|map| map.name == "Level") {
            "New level".into()
        } else {
            "Level".into()
        };
        self.map_picker = Some(MapPicker {
            song_index: index,
            mode,
            selected,
            new_name,
            message: None,
        });
    }

    fn open_editor_for_map(&mut self, index: usize, path: &std::path::Path, now: f64) {
        let map = match load_map_file(path) {
            Ok(map) => map,
            Err(error) => {
                self.toast(error, now);
                return;
            }
        };
        let Some(mut song) = self.songs.get(index).cloned() else {
            return;
        };
        map.apply_to_song(&mut song);
        if let Some(audio) = &self.audio {
            audio.stop();
        }
        self.stage = None;
        self.drawer_open = false;
        self.settings_open = false;
        self.range_test_open = false;
        self.range_testing = false;
        let suggestions = song.suggested_notes.clone();
        let duration = song.duration().max(1.0);
        let backing = song_audio_path(&self.root, &song, "backing");
        let vocals = song_audio_path(&self.root, &song, "vocals");
        let editor = Editor::new(
            song,
            suggestions,
            map.notes.clone(),
            duration,
            path.to_path_buf(),
            map.draft,
        );
        if let Some(audio) = &self.audio {
            audio.set_volumes(0.9, 0.85);
            audio.set_rate(1.0);
        }
        self.editor = Some(editor);
        self.map_picker = None;
        let loading = self.start_audio_load(backing, Some(vocals), true);
        if let Some(editor) = &mut self.editor {
            editor.set_audio_loading(loading);
        }
        self.started_at = now;
        self.tracker.reset();
        self.reading = None;
    }

    fn create_map_file(&mut self, index: usize, name: &str, now: f64) -> Result<(), String> {
        let Some(song) = self.songs.get(index).cloned() else {
            return Err("Song not found.".into());
        };
        let mut stem = name.trim().trim_end_matches(".json").trim().to_string();
        if stem.is_empty() {
            return Err("Type a file name.".into());
        }
        stem = stem
            .chars()
            .map(|ch| {
                if ch.is_alphanumeric() || matches!(ch, '-' | '_' | ' ') {
                    ch
                } else {
                    '-'
                }
            })
            .collect();
        let stem = stem.trim();
        if stem.is_empty() || stem == "." || stem == ".." {
            return Err("That file name is not usable.".into());
        }
        let dir = maps_dir(&self.root, &song.id);
        let path = dir.join(format!("{stem}.json"));
        if path.exists() {
            return Err(format!("{} already exists.", path.display()));
        }
        let map = MapFile::new_for_song(&song);
        save_map_file(&path, &map)?;
        self.refresh_song_maps(index);
        self.open_editor_for_map(index, &path, now);
        Ok(())
    }

    /// Decode a song on a worker thread. `poll_audio_load` installs it into
    /// the engine when it is ready; a load started later replaces this one.
    /// Returns false when there is no audio engine to load into.
    fn start_audio_load(
        &mut self,
        backing: PathBuf,
        vocals: Option<PathBuf>,
        for_editor: bool,
    ) -> bool {
        let Some(audio) = &self.audio else {
            return false;
        };
        let rate = audio.sample_rate();
        self.audio_load = Some(Job::run("Audio loading", &self.ctx, move || {
            let audio = LoadedAudio::decode(&backing, vocals.as_deref(), rate)
                .map_err(|error| format!("{error:#}"))?;
            let analysis = for_editor.then(|| EditorAudio {
                peaks: audio.vocal_peaks(VOCAL_BINS_PER_SECOND),
                mono: audio.vocal_mono(RAW_PITCH_SAMPLE_RATE),
            });
            Ok((audio, analysis))
        }));
        true
    }

    fn poll_audio_load(&mut self, now: f64) {
        let Some(result) = self.audio_load.as_mut().and_then(Job::poll) else {
            return;
        };
        self.audio_load = None;
        let (loaded, analysis) = match result {
            Ok(loaded) => loaded,
            Err(error) => {
                if let Some(stage) = &mut self.stage {
                    stage.audio_loading = false;
                    stage.play_when_loaded = false;
                }
                if let Some(editor) = &mut self.editor {
                    editor.set_audio_loading(false);
                }
                self.toast(format!("Could not load audio: {error}"), now);
                return;
            }
        };
        let Some(audio) = &self.audio else {
            return;
        };
        let duration = audio.install(loaded);
        if let Some(editor) = &mut self.editor {
            editor.set_audio_ready(duration > 0.0);
            if duration > 0.0 {
                editor.sync_audio(audio);
            }
            if let Some(analysis) = analysis {
                editor.set_vocal_peaks(analysis.peaks);
                if !analysis.mono.is_empty() {
                    editor.set_pitch_job(spawn_raw_pitch_track(
                        &self.ctx,
                        analysis.mono,
                        VOCAL_BINS_PER_SECOND,
                    ));
                }
            }
        } else if let Some(stage) = &mut self.stage {
            stage.duration = duration.max(stage.duration);
            stage.audio_loaded = duration > 0.0;
            stage.audio_loading = false;
            audio.seek(stage.time);
            if stage.play_when_loaded {
                stage.play_when_loaded = false;
                if stage.audio_loaded {
                    audio.play();
                }
                stage.playing = true;
                stage.started = true;
            }
        }
    }

    /// Rebuild a stream that cpal reported dead (device unplugged or the
    /// default device changed), then reload whatever was open.
    fn recover_audio_devices(&mut self, now: f64) {
        if self.audio.as_ref().is_some_and(AudioEngine::failed) {
            self.audio = None;
            match AudioEngine::new(self.backing_level(), self.settings.vocals_volume) {
                Ok(audio) => {
                    self.audio = Some(audio);
                    self.toast("Audio output restarted after a device change.", now);
                    self.reload_current_audio();
                }
                Err(error) => self.toast(format!("Audio output stopped: {error:#}"), now),
            }
        }
        if self.mic.as_ref().is_some_and(Microphone::failed) {
            self.mic = None;
            self.toast("Microphone stopped; reconnecting.", now);
            self.start_mic();
        }
    }

    fn reload_current_audio(&mut self) {
        if let Some(editor) = &mut self.editor {
            let backing = song_audio_path(&self.root, &editor.song, "backing");
            let vocals = song_audio_path(&self.root, &editor.song, "vocals");
            editor.set_audio_ready(false);
            if let Some(audio) = &self.audio {
                audio.set_volumes(0.9, 0.85);
            }
            let loading = self.start_audio_load(backing, Some(vocals), true);
            if let Some(editor) = &mut self.editor {
                editor.set_audio_loading(loading);
            }
        } else if let Some(stage) = &mut self.stage {
            let backing = song_audio_path(&self.root, &stage.song, "backing");
            let vocals = song_audio_path(&self.root, &stage.song, "vocals");
            stage.play_when_loaded = stage.playing;
            stage.playing = false;
            stage.audio_loaded = false;
            stage.audio_loading = true;
            self.start_audio_load(backing, Some(vocals), false);
        }
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
        self.drawer_view = DrawerView::Albums;
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
                self.drawer_view = DrawerView::Imports;
                self.selected_album = None;
                self.drawer_scroll = 0.0;
            }
            Action::ShowAlbums => {
                self.drawer_view = DrawerView::Albums;
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
            Action::Home => {
                if let Some(audio) = &self.audio {
                    audio.stop();
                }
                self.stage = None;
                self.drawer_open = false;
                self.selected_album = None;
            }
            Action::OpenUpload => {
                self.upload.open = true;
                self.upload.message.clear();
                self.drawer_open = true;
                self.drawer_view = DrawerView::Imports;
                self.selected_album = None;
            }
            Action::ShowMaps => {
                self.drawer_view = DrawerView::Maps;
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
                self.open_map_picker(index, MapPickerMode::Edit, now);
            }
        }
    }

    fn draw_map_picker(&mut self, ctx: &egui::Context, now: f64) {
        let Some(mut picker) = self.map_picker.take() else {
            return;
        };
        let Some(song) = self.songs.get(picker.song_index).cloned() else {
            return;
        };
        let maps = song.maps.clone();
        let mut close = false;
        let mut window_open = true;
        let mut edit_path: Option<PathBuf> = None;
        let mut play_path: Option<PathBuf> = None;
        let mut create = false;
        egui::Window::new(match picker.mode {
            MapPickerMode::Edit => "Choose a map file",
            MapPickerMode::Play => "Choose a map",
        })
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_min_width(480.0);
            if let Some(message) = &picker.message {
                ui.colored_label(Color32::from_rgb(255, 93, 158), message);
            }
            match picker.mode {
                MapPickerMode::Edit => {
                    ui.label(format!(
                        "{} · {}.json files in the song's maps folder.",
                        song.title,
                        maps.len()
                    ));
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            if maps.is_empty() {
                                ui.label("No map files yet. Create one below.");
                            }
                            for (index, map) in maps.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    if ui
                                        .selectable_label(
                                            picker.selected == Some(index),
                                            map.display_name(),
                                        )
                                        .clicked()
                                    {
                                        picker.selected = Some(index);
                                    }
                                    if ui.button("EDIT").clicked() {
                                        edit_path = Some(map.path.clone());
                                    }
                                });
                            }
                        });
                    ui.separator();
                    ui.label("NEW MAP FILE");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut picker.new_name);
                        if ui.button("CREATE & EDIT").clicked() {
                            create = true;
                        }
                    });
                }
                MapPickerMode::Play => {
                    ui.label(format!("{} — choose a map.", song.title));
                    let selected_label = picker
                        .selected
                        .and_then(|index| maps.get(index))
                        .map(MapInfo::display_name)
                        .unwrap_or_else(|| "Select a map".into());
                    egui::ComboBox::from_id_salt("play_map_select")
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            for (index, map) in maps.iter().enumerate() {
                                ui.selectable_value(
                                    &mut picker.selected,
                                    Some(index),
                                    map.display_name(),
                                );
                            }
                        });
                    ui.add_space(10.0);
                    let can_play = picker.selected.and_then(|index| maps.get(index)).is_some();
                    if ui
                        .add_enabled(can_play, egui::Button::new("PLAY"))
                        .clicked()
                    {
                        play_path = picker
                            .selected
                            .and_then(|index| maps.get(index))
                            .map(|map| map.path.clone());
                    }
                }
            }
            ui.add_space(8.0);
            if ui.button("CANCEL").clicked() {
                close = true;
            }
        });
        if !window_open {
            close = true;
        }
        if let Some(path) = edit_path {
            self.open_editor_for_map(picker.song_index, &path, now);
            return;
        }
        if let Some(path) = play_path {
            self.open_song_with_map(picker.song_index, &path, now);
            return;
        }
        if create {
            match self.create_map_file(picker.song_index, &picker.new_name, now) {
                Ok(()) => return,
                Err(error) => picker.message = Some(error),
            }
        }
        if !close {
            self.map_picker = Some(picker);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context, now: f64) {
        if self.editor.is_some() {
            return;
        }
        if self.map_picker.is_some() {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.map_picker = None;
            }
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
                if let Some(stage) = &mut self.stage {
                    if stage.audio_loading {
                        stage.play_when_loaded = true;
                    } else {
                        if let Some(audio) = &self.audio {
                            audio.play();
                        }
                        stage.playing = true;
                        stage.started = true;
                    }
                }
            }
        }
        self.started_at = if self.started_at == 0.0 {
            now
        } else {
            self.started_at
        };
        self.handle_keys(ctx, now);
        self.recover_audio_devices(now);
        self.poll_audio_load(now);
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
            // AutoMap is handled inside the editor and never reaches here.
            EditorEvent::None | EditorEvent::AutoMap => {}
            EditorEvent::Back => {
                let mut leaving = true;
                if let Some(mut editor) = self.editor.take() {
                    if editor.is_draft() {
                        match editor.save_draft() {
                            Ok(()) => {
                                editor.mark_draft_saved();
                                self.toast("Draft saved.", now);
                            }
                            Err(error) => {
                                leaving = false;
                                self.toast(format!("Could not save the draft: {error}"), now);
                            }
                        }
                    } else if editor.is_dirty() {
                        // A finished level only persists through Submit. Keep
                        // the editor open if the edits cannot be submitted.
                        match editor.submit() {
                            Ok(()) => self.toast("Level updated.", now),
                            Err(error) => {
                                leaving = false;
                                self.toast(
                                    format!("{error} Fix that or undo your edits before leaving."),
                                    now,
                                );
                            }
                        }
                    }
                    if leaving {
                        editor.pause_audio(&self.audio);
                    } else {
                        self.editor = Some(editor);
                    }
                }
                if leaving {
                    if let Some(audio) = &self.audio {
                        audio.pause();
                    }
                    self.settings_open = false;
                    self.reload_library(ctx, now);
                    self.drawer_open = true;
                }
            }
            EditorEvent::ExportMap => self.export_editor_map(now),
            EditorEvent::SaveDraft => {
                let result = if let Some(editor) = &mut self.editor {
                    if !editor.is_draft() {
                        Err("This file is a finished level. Submit Mapping to save changes.".into())
                    } else {
                        editor.save_draft().map(|()| editor.mark_draft_saved())
                    }
                } else {
                    Ok(())
                };
                match result {
                    Ok(()) => self.toast("Draft saved.", now),
                    Err(error) => self.toast(error, now),
                }
            }
            EditorEvent::Submit => {
                if let Some(mut editor) = self.editor.take() {
                    editor.pause_audio(&self.audio);
                    match editor.submit() {
                        Ok(()) => {
                            self.reload_library(ctx, now);
                            self.drawer_open = true;
                            self.toast("Level submitted. This song is now playable.", now);
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
        self.draw_map_picker(ctx, now);
        if self.map_picker.is_some() {
            actions.clear();
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
                    match image::save_buffer(
                        &path,
                        &rgba,
                        w as u32,
                        h as u32,
                        image::ColorType::Rgba8,
                    ) {
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
            if stage.playing
                || stage.count_in_until.is_some()
                || self.mic.is_some()
                || self.range_test_open
            {
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

fn setting_env(name: &str) -> Option<String> {
    std::env::var(format!("KARAOKEMACHINE_{name}")).ok()
}

fn ctx_pointer(ui: &egui::Ui) -> Option<Pos2> {
    ui.input(|i| i.pointer.interact_pos())
}
