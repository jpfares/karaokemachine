//! The upload dialog: metadata fetch, cover picker and the import job.

use super::cover::CoverPicker;
use super::maps::install_package_as_map;
use super::KaraokeMachineApp;
use crate::automap;
use crate::jobs::{Job, StreamJob};
use crate::library::{maps_dir, save_map_file};
use crate::map_package::MapPackage;
use crate::model::{format_time, Song};
use crate::theme::{alpha, GOLD, MINT, MUTED, MUTED2, PINK};
use crate::tools::node_command;
use eframe::egui::{self, Align2, Color32, FontId, Rect, RichText, Vec2};
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Stdio;

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct TrackInfo {
    #[serde(rename = "audioUrl", default)]
    pub(super) audio_url: String,
    #[serde(default)]
    pub(super) title: String,
    #[serde(default)]
    pub(super) artist: String,
    #[serde(default)]
    pub(super) album: Option<String>,
    #[serde(rename = "releaseDate", default)]
    pub(super) release_date: Option<String>,
    #[serde(default)]
    pub(super) cover: Option<String>,
    #[serde(rename = "artistBio", default)]
    pub(super) artist_bio: Option<String>,
    #[serde(rename = "albumBio", default)]
    pub(super) album_bio: Option<String>,
    #[serde(default)]
    pub(super) lyrics: Option<String>,
    #[serde(rename = "lyricsProvider", default)]
    pub(super) lyrics_provider: Option<String>,
    #[serde(rename = "lyricsSynced", default)]
    pub(super) lyrics_synced: bool,
    #[serde(default)]
    pub(super) duration: f64,
    #[serde(default)]
    pub(super) error: Option<String>,
}

pub(super) enum ImportMessage {
    Progress(f32, String, String),
    Done(Result<String, String>),
}

#[derive(Default)]
pub(super) struct UploadState {
    pub(super) open: bool,
    pub(super) url: String,
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) album: String,
    pub(super) release_date: String,
    pub(super) cover: CoverPicker,
    pub(super) artist_bio: String,
    pub(super) album_bio: String,
    pub(super) lyrics: String,
    pub(super) audio_url: String,
    pub(super) lyrics_provider: Option<String>,
    pub(super) lyrics_synced: bool,
    pub(super) duration: f64,
    pub(super) info_loaded: bool,
    pub(super) progress: f32,
    pub(super) stage: String,
    pub(super) message: String,
    pub(super) artist_search: String,
    pub(super) album_search: String,
    pub(super) pending_map: Option<MapPackage>,
    /// Make the imported song playable from its suggestions instead of opening the editor.
    pub(super) auto_map: bool,
    pub(super) fetch: Option<Job<TrackInfo>>,
    pub(super) import: Option<StreamJob<ImportMessage>>,
}

impl UploadState {
    pub(super) fn fetching(&self) -> bool {
        self.fetch.is_some()
    }

    pub(super) fn importing(&self) -> bool {
        self.import.is_some()
    }
}

pub(super) fn paint_placeholder(ui: &egui::Ui, rect: Rect, text: &str, multiline: bool) {
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

pub(super) fn run_song_info(root: &Path, url: &str) -> Result<TrackInfo, String> {
    let script = root.join("dist-tools/song-info.js");
    let output = node_command(root, &script)
        .arg("--url")
        .arg(url)
        .output()
        .map_err(|error| format!("Could not run Node: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stdout
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let parsed: TrackInfo = serde_json::from_str(line.trim()).map_err(|error| {
        format!(
            "Could not read the track info: {error}{}",
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(
                    "\n{}",
                    stderr
                        .lines()
                        .rev()
                        .take(4)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        )
    })?;
    if let Some(error) = &parsed.error {
        return Err(error.clone());
    }
    if !output.status.success() {
        return Err(format!(
            "Track lookup failed: {}",
            parsed.error.unwrap_or_else(|| "unknown error".into())
        ));
    }
    Ok(parsed)
}

impl KaraokeMachineApp {
    pub(super) fn poll_upload(&mut self, ctx: &egui::Context, now: f64) {
        self.upload.cover.poll();

        let fetched = self.upload.fetch.as_mut().and_then(Job::poll);
        let mut start_cover: Option<String> = None;
        if let Some(result) = fetched {
            self.upload.fetch = None;
            match result {
                Ok(info) => {
                    if let Some(error) = info.error {
                        self.upload.message = error;
                    } else {
                        let cover_url = info.cover.clone().filter(|value| !value.trim().is_empty());
                        self.upload.title = info.title;
                        self.upload.artist = info.artist;
                        self.upload.album = info.album.unwrap_or_default();
                        self.upload.release_date = info.release_date.unwrap_or_default();
                        self.upload.cover.set_value(info.cover.unwrap_or_default());
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
                        start_cover = cover_url;
                        self.upload.message = if self.upload.lyrics.trim().is_empty() {
                            "Details fetched. No free lyrics were found — paste or edit them below."
                                .into()
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
            self.upload.cover.start_fetch(&self.root, &self.ctx, url);
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
            // Taken on every outcome: a map must never outlive its own import.
            let pending_map = self.upload.pending_map.take();
            match result {
                Some(Ok(id)) => {
                    self.upload.open = false;
                    if let Some(map) = pending_map {
                        let dir = self.root.join("assets/songs").join(&id);
                        let mut song: Song = match std::fs::read_to_string(dir.join("song.json"))
                            .map_err(|error| format!("Could not read the imported song: {error}"))
                            .and_then(|raw| {
                                serde_json::from_str(&raw).map_err(|error| {
                                    format!("Could not parse the imported song: {error}")
                                })
                            }) {
                            Ok(song) => song,
                            Err(error) => {
                                self.toast(error, now);
                                return;
                            }
                        };
                        match install_package_as_map(&self.root, &mut song, &map) {
                            Ok(_path) => {
                                self.reload_library(ctx, now);
                                if let Some(index) =
                                    self.songs.iter().position(|song| song.id == id)
                                {
                                    self.open_map_picker(index, super::MapPickerMode::Play, now);
                                    self.toast(
                                        "Map installed. Audio was downloaded from its source link.",
                                        now,
                                    );
                                } else {
                                    self.toast(
                                        "Map installed, but the song was not found after reload.",
                                        now,
                                    );
                                }
                            }
                            Err(error) => self.toast(error, now),
                        }
                    } else {
                        self.reload_library(ctx, now);
                        if let Some(index) = self.songs.iter().position(|song| song.id == id) {
                            // An editor opened during the import keeps its
                            // unsaved draft: the song waits in the library.
                            let editing = self.editor.is_some();
                            if self.upload.auto_map {
                                self.auto_map_imported(index, editing, now);
                            } else if editing {
                                self.toast(
                                    "Song prepared. Find it in the library when you finish editing.",
                                    now,
                                );
                            } else {
                                self.open_map_picker(index, super::MapPickerMode::Edit, now);
                                self.toast("Song prepared. Choose a map file to edit.", now);
                            }
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

    /// Map a freshly imported song from its suggestions into a finished level
    /// file and open it on the stage; fall back to the map picker when there
    /// is nothing to map from. While another song is being edited, only the
    /// file is written and the song waits in the library.
    fn auto_map_imported(&mut self, index: usize, editing: bool, now: f64) {
        let Some(song) = self.songs.get(index).cloned() else {
            return;
        };
        let result = automap::auto_mapped_level(&song).and_then(|(level, summary)| {
            let path = maps_dir(&self.root, &song.id)
                .join(format!("{}.json", automap::AUTO_MAP_FILE_STEM));
            save_map_file(&path, &level).map(|()| summary)
        });
        match result {
            Ok(summary) => {
                self.refresh_song_maps(index);
                if editing {
                    self.toast(
                        format!("{summary} Find it in the library when you finish editing."),
                        now,
                    );
                } else {
                    self.open_song(index, now);
                    self.toast(
                        format!("{summary} Open EDIT in the library to fix anything."),
                        now,
                    );
                }
            }
            Err(error) if editing => {
                self.toast(
                    format!("Not auto-mapped: {error} The song is in the library."),
                    now,
                );
            }
            Err(error) => {
                self.open_map_picker(index, super::MapPickerMode::Edit, now);
                self.toast(format!("Not auto-mapped: {error}"), now);
            }
        }
    }

    pub(super) fn start_fetch(&mut self, now: f64) {
        if self.upload.fetching() || self.upload.importing() {
            return;
        }
        self.upload.cover.cancel_fetch();
        let url = self.upload.url.trim().to_string();
        if url.is_empty() {
            self.upload.message = "Paste a YouTube, Spotify or SoundCloud link first.".into();
            return;
        }
        let script = self.root.join("dist-tools/song-info.js");
        if !script.exists() {
            self.upload.message =
                "Import tools are not built. Run `npm install && npm run assets` first.".into();
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

    pub(super) fn start_import(&mut self, now: f64) -> bool {
        if self.upload.fetching() || self.upload.importing() {
            return false;
        }
        if self.upload.url.trim().is_empty() {
            self.upload.message = "Paste a link first.".into();
            return false;
        }
        if self.upload.title.trim().is_empty() || self.upload.artist.trim().is_empty() {
            self.upload.message = "Title and artist are required.".into();
            return false;
        }
        if self.upload.audio_url.trim().is_empty()
            && self.upload.url.to_lowercase().contains("spotify.com")
        {
            self.upload.message =
                "Spotify needs Fetch Details first so KaraokeMachine can match a public audio source."
                    .into();
            return false;
        }
        let script = self.root.join("dist-tools/import-song.js");
        if !script.exists() {
            self.upload.message =
                "Import tools are not built. Run `npm install && npm run assets` first.".into();
            return false;
        }
        let meta = serde_json::json!({
            "audioUrl": self.upload.audio_url,
            "title": self.upload.title,
            "artist": self.upload.artist,
            "album": self.upload.album,
            "releaseDate": self.upload.release_date,
            "cover": self.upload.cover.value,
            "artistBio": self.upload.artist_bio,
            "albumBio": self.upload.album_bio,
            "lyricsText": self.upload.lyrics,
        });
        let work = self.root.join(".work");
        if let Err(error) = std::fs::create_dir_all(&work) {
            self.upload.message = format!("Could not create the work directory: {error}");
            return false;
        }
        let meta_path = work.join("editor-import.json");
        if let Err(error) = std::fs::write(
            &meta_path,
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        ) {
            self.upload.message = format!("Could not write import metadata: {error}");
            return false;
        }
        let log_path = work.join("editor-import.log");
        let log = match std::fs::File::create(&log_path) {
            Ok(file) => file,
            Err(error) => {
                self.upload.message = format!("Could not open the import log: {error}");
                return false;
            }
        };
        let root = self.root.clone();
        let url = self.upload.url.trim().to_string();
        self.upload.progress = 0.0;
        self.upload.stage = "Queued".into();
        self.upload.message =
            "Downloading and separating the audio. This can take several minutes.".into();
        let _ = now;
        self.upload.import = Some(StreamJob::run("The importer", &self.ctx, move |sender| {
            let child = node_command(&root, &script)
                .arg("--url")
                .arg(&url)
                .arg("--meta")
                .arg(&meta_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::from(log))
                .spawn();
            let mut child = match child {
                Ok(child) => child,
                Err(error) => {
                    sender.send(ImportMessage::Done(Err(format!(
                        "Could not start Node: {error}"
                    ))));
                    return;
                }
            };
            let mut result_id: Option<String> = None;
            let mut error_message: Option<String> = None;
            if let Some(stdout) = child.stdout.take() {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if let Some(rest) = line.strip_prefix("PROGRESS\t") {
                        let parts: Vec<&str> = rest.splitn(3, '\t').collect();
                        if let Some(percent) =
                            parts.first().and_then(|value| value.parse::<f32>().ok())
                        {
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
                            "Import finished but did not report a song id.".into(),
                        )));
                    }
                }
                _ => {
                    let log = std::fs::read_to_string(root.join(".work/editor-import.log"))
                        .unwrap_or_default();
                    let tail = log
                        .lines()
                        .rev()
                        .take(8)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n");
                    let message = error_message
                        .or_else(|| {
                            if tail.trim().is_empty() {
                                None
                            } else {
                                Some(tail)
                            }
                        })
                        .unwrap_or_else(|| {
                            "The import failed. Check yt-dlp, ffmpeg and Demucs.".into()
                        });
                    sender.send(ImportMessage::Done(Err(message)));
                }
            }
        }));
        true
    }

    pub(super) fn draw_upload(&mut self, ctx: &egui::Context, now: f64) {
        let mut open = self.upload.open;
        let mut artists: Vec<String> = self
            .songs
            .iter()
            .filter_map(|song| song.artist.clone())
            .collect();
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

        self.upload.cover.update_preview(ctx);
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
                        upload.cover.draw_tile(ui);
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
                        ui.checkbox(
                            &mut upload.auto_map,
                            "Make it playable right away (auto-map from the detected melody)",
                        );
                        let label = if upload.auto_map {
                            "DOWNLOAD, SPLIT & AUTO-MAP"
                        } else {
                            "DOWNLOAD, SPLIT & OPEN EDITOR"
                        };
                        let button =
                            egui::Button::new(label).min_size(Vec2::new(ui.available_width(), 36.0));
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
                if !self.upload.cover.has_value() {
                    if let Some(cover) = &song.cover {
                        let path = self.root.join("assets/songs").join(&song.id).join(cover);
                        self.upload
                            .cover
                            .set_from_path(path, "Cover copied from the existing album.");
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
        self.upload.cover.draw_popup(ctx, &self.root);
    }
}
