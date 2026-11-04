//! The community map browser: fetching the index, downloading and installing maps.

use super::lobby::paint_cover_fallback;
use super::{Action, KaraokeMachineApp, MapPickerMode};
use crate::jobs::Job;
use crate::library::{
    backup_song_file, maps_dir, save_map_file, song_audio_path, write_json_atomic,
};
use crate::map_package::{MapIndex, MapIndexEntry, MapPackage};
use crate::model::{format_time, MapFile, Song};
use crate::theme::{INK, MUTED};
use crate::tools::{http_get_bytes, http_get_text};
use crate::widgets::{click, pill_button, pill_toggle};
use base64::Engine;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke, TextureOptions, Vec2};
use std::path::{Path, PathBuf};

pub(super) const MAPS_BASE_URL: &str = "https://rgbeans.github.io/karaokemachine/maps/";
/// The shared index lists every map with its cover inline.
const MAP_INDEX_LIMIT: u64 = 8 * 1024 * 1024;
/// One map package, cover included.
const MAP_FILE_LIMIT: u64 = 16 * 1024 * 1024;
/// One cover image from the index.
const MAP_COVER_LIMIT: u64 = 4 * 1024 * 1024;

fn maps_base_url() -> String {
    std::env::var("KARAOKEMACHINE_MAPS_BASE_URL").unwrap_or_else(|_| MAPS_BASE_URL.to_string())
}

/// Decode cover bytes into a texture-ready image, shrunk to thumbnail size:
/// the browser draws covers at 54 px, so full-size art would only cost memory.
fn decode_cover(bytes: &[u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory(bytes)
        .ok()?
        .thumbnail(256, 256)
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        &image.into_raw(),
    ))
}

pub(super) fn encode_path_segment(value: &str) -> String {
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

pub(super) fn write_song_file(dir: &Path, song: &Song) -> Result<(), String> {
    write_json_atomic(&dir.join("song.json"), song)
}

/// Store a shared map package as a separate local map file and update the
/// song's metadata/lyrics without touching any existing map file.
pub(super) fn install_package_as_map(
    root: &Path,
    song: &mut Song,
    map: &MapPackage,
) -> Result<PathBuf, String> {
    let dir = root.join("assets/songs").join(&song.id);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
    backup_song_file(&dir)?;
    if let Some(legacy) = MapFile::from_legacy_song(song) {
        let legacy_path = maps_dir(root, &song.id).join("Legacy.json");
        if !legacy_path.exists() {
            save_map_file(&legacy_path, &legacy)?;
        }
    }
    if let Some(cover) = map.save_cover(&dir)? {
        song.cover = Some(cover);
    }
    song.title = map.title.clone();
    song.artist = map.artist.clone();
    song.album = map.album.clone();
    song.release_date = map.release_date.clone();
    song.lines = map.lines.clone();
    song.youtube_url = Some(map.source_url.clone());
    song.notes.clear();
    song.draft_notes.clear();
    song.placed_words.clear();
    song.custom_words.clear();
    song.ribbon_links.clear();
    let path = maps_dir(root, &song.id).join(map.file_name());
    if path.exists() {
        let backup = path.with_extension("json.bak");
        std::fs::copy(&path, &backup)
            .map_err(|error| format!("Could not back up {}: {error}", path.display()))?;
    }
    save_map_file(&path, &map.to_map_file())?;
    write_song_file(&dir, song)?;
    Ok(path)
}

impl KaraokeMachineApp {
    pub(super) fn draw_drawer_maps(
        &self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        body: Rect,
        actions: &mut Vec<Action>,
    ) {
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
                paint_cover_fallback(p, cover, 8.0, &entry.title);
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
                FontId::monospace(10.0),
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
            let row = Rect::from_min_size(Pos2::new(body.left(), y), Vec2::new(body.width(), 62.0));
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
                paint_cover_fallback(p, cover, 8.0, &song.title);
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
                FontId::monospace(10.0),
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

    pub(super) fn start_map_index_fetch(&mut self) {
        if self.map_index_job.is_some() {
            return;
        }
        self.map_index_job = Some(Job::run("The map browser", &self.ctx, || {
            let base = maps_base_url();
            http_get_text(&format!("{base}index.json"), MAP_INDEX_LIMIT).and_then(|raw| {
                serde_json::from_str::<MapIndex>(&raw)
                    .map_err(|error| format!("Could not parse map index: {error}"))
            })
        }));
    }

    pub(super) fn start_remote_map_install(&mut self, entry: MapIndexEntry) {
        if self.map_download.is_some() || entry.file.is_empty() {
            return;
        }
        let url = format!("{}{}", maps_base_url(), encode_path_segment(&entry.file));
        let job = Job::run("The map download", &self.ctx, move || {
            http_get_text(&url, MAP_FILE_LIMIT).and_then(|raw| MapPackage::from_json(&raw))
        });
        self.map_download = Some((entry.id.clone(), job));
    }

    pub(super) fn poll_maps(&mut self, ctx: &egui::Context, now: f64) {
        let index_result = self.map_index_job.as_mut().and_then(Job::poll);
        if let Some(result) = index_result {
            self.map_index_job = None;
            self.map_index_loaded = true;
            match result {
                Ok(index) => {
                    self.map_index = index.maps;
                    let base = maps_base_url();
                    for entry in &self.map_index {
                        let cover = entry.cover.trim();
                        if cover.is_empty() || self.map_covers.contains_key(&entry.id) {
                            continue;
                        }
                        if let Some((_, payload)) = cover
                            .strip_prefix("data:")
                            .and_then(|rest| rest.split_once(";base64,"))
                        {
                            // Older indexes inline the cover.
                            let image = base64::engine::general_purpose::STANDARD
                                .decode(payload)
                                .ok()
                                .and_then(|bytes| decode_cover(&bytes));
                            if let Some(image) = image {
                                let texture = ctx.load_texture(
                                    format!("map-cover-{}", entry.id),
                                    image,
                                    TextureOptions::LINEAR,
                                );
                                self.map_covers.insert(entry.id.clone(), texture);
                            }
                        } else {
                            let url = format!("{base}{cover}");
                            let job = Job::run("The cover download", &self.ctx, move || {
                                let bytes = http_get_bytes(&url, MAP_COVER_LIMIT)?;
                                decode_cover(&bytes).ok_or_else(|| format!("{url} is not an image"))
                            });
                            self.map_cover_jobs.push((entry.id.clone(), job));
                        }
                    }
                }
                Err(error) => {
                    self.map_index.clear();
                    eprintln!("map browser: {error}");
                }
            }
        }

        let mut pending = Vec::new();
        for (id, mut job) in self.map_cover_jobs.drain(..) {
            match job.poll() {
                None => pending.push((id, job)),
                Some(Ok(image)) => {
                    let texture =
                        ctx.load_texture(format!("map-cover-{id}"), image, TextureOptions::LINEAR);
                    self.map_covers.insert(id, texture);
                }
                Some(Err(error)) => eprintln!("map cover: {error}"),
            }
        }
        self.map_cover_jobs = pending;

        let download_result = self.map_download.as_mut().and_then(|(_, job)| job.poll());
        if let Some(result) = download_result {
            self.map_download = None;
            match result {
                Ok(map) => self.install_map(map, now, ctx),
                Err(error) => self.toast(error, now),
            }
        }
    }

    pub(super) fn install_map_file(&mut self, now: f64, ctx: &egui::Context) {
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

    pub(super) fn install_map(&mut self, map: MapPackage, now: f64, ctx: &egui::Context) {
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
            let backing = song_audio_path(&self.root, &existing, "backing");
            if backing.exists() {
                let mut song = existing.clone();
                match install_package_as_map(&self.root, &mut song, &map) {
                    Ok(path) => {
                        self.reload_library(ctx, now);
                        if let Some(index) =
                            self.songs.iter().position(|song| song.id == existing.id)
                        {
                            self.drawer_open = false;
                            self.open_map_picker(index, MapPickerMode::Play, now);
                            self.toast(format!("Map installed as {}.", path.display()), now);
                        } else {
                            self.toast("Map saved. Reload the library to see it.", now);
                        }
                    }
                    Err(error) => self.toast(error, now),
                }
                return;
            }
        }

        if self.upload.fetching() || self.upload.importing() {
            // start_import would refuse silently; say why the map was not queued.
            self.toast("Finish the current import before installing a map.", now);
            return;
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
        match cover {
            Some(path) => self
                .upload
                .cover
                .set_from_path(path, "Cover art from the map."),
            None => self.upload.cover.reset(),
        }
        self.upload.info_loaded = true;
        self.upload.message = "Map loaded. Downloading audio from the source link…".into();
        self.upload.open = true;
        self.upload.pending_map = Some(map);
        if !self.start_import(now) {
            // The import did not start, so the map must not wait around to be
            // applied to whatever the user imports next.
            self.upload.pending_map = None;
        }
    }

    pub(super) fn export_editor_map(&mut self, now: f64) {
        let result: Result<Option<PathBuf>, String> = if let Some(editor) = &self.editor {
            if editor.is_draft() {
                Err("Submit the mapping before exporting it.".into())
            } else {
                MapPackage::from_song(&self.root, &editor.export_song(), None).and_then(|map| {
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
            Ok(Some(path)) => self.toast(format!("Map exported to {}", path.display()), now),
            Ok(None) => {}
            Err(error) => self.toast(error, now),
        }
    }
}
