//! The lobby and the library drawer: albums, tracks and the imports tab.

use super::stage::add_ribbon_mesh;
use super::{ctx_pointer, pitch_col, Action, DrawerView, KaraokeMachineApp};
use crate::automap;
use crate::library::{build_albums, load_songs, read_songlist};
use crate::model::{format_time, note_name};
use crate::theme::{alpha, BG, GOLD, INK, MINT, MUTED, MUTED2, PINK};
use crate::widgets::{click, glyph_button, pill_button, pill_toggle};
use eframe::egui::{
    self, Align2, Color32, FontId, Id, Mesh, Pos2, Rect, Shape, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2,
};
use std::path::Path;

pub(super) fn paint_cover_fallback(p: &egui::Painter, rect: Rect, title: &str) {
    let c = pitch_col(((title.len() * 37) % 100) as f32 / 100.0);
    p.rect_filled(rect, 12.0, alpha(c, 80));
    p.circle_filled(rect.center(), rect.width() * 0.22, alpha(c, 100));
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        title.chars().next().unwrap_or('K').to_string(),
        FontId::proportional(rect.width() * 0.25),
        INK,
    );
}

pub(super) fn load_texture(ctx: &egui::Context, path: &Path) -> Option<TextureHandle> {
    // Library tiles are at most a few hundred pixels; full-size art only costs memory.
    let image = image::open(path).ok()?.thumbnail(512, 512).to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    let color = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    Some(ctx.load_texture(path.to_string_lossy(), color, TextureOptions::LINEAR))
}

impl KaraokeMachineApp {
    pub(super) fn preload_covers(&mut self, ctx: &egui::Context) {
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

    pub(super) fn reload_library(&mut self, ctx: &egui::Context, _now: f64) {
        self.songs = load_songs(&self.root);
        self.albums = build_albums(&self.songs);
        self.songlist = read_songlist();
        self.cover_paths.clear();
        self.covers.clear();
        self.preload_covers(ctx);
    }

    pub(super) fn draw_lobby(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
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
                Stroke::new(
                    if strong { 1.0f32 } else { 0.6f32 },
                    Color32::from_white_alpha(if strong { 22 } else { 9 }),
                ),
            );
        }
        p.rect_filled(
            Rect::from_min_size(Pos2::ZERO, Vec2::new(left_w, rect.height())),
            0.0,
            Color32::from_rgba_unmultiplied(5, 8, 15, 235),
        );
        p.line_segment(
            [
                Pos2::new(left_w, top),
                Pos2::new(left_w, rect.height() - 30.0),
            ],
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
        add_ribbon_mesh(
            &mut mesh,
            &points,
            24.0,
            low,
            high,
            top,
            rect.height() - 30.0,
            1.0,
        );
        p.add(Shape::mesh(mesh));
        p.text(
            Pos2::new(28.0, 30.0),
            Align2::LEFT_CENTER,
            "KARAOKEMACHINE",
            FontId::proportional(18.0),
            alpha(INK, 200),
        );
        p.text(
            Pos2::new(28.0, 50.0),
            Align2::LEFT_CENTER,
            "MUSIC LIBRARY",
            FontId::monospace(8.0),
            MUTED2,
        );
        let brand = Rect::from_min_size(
            Pos2::new(rect.width() - 170.0, 22.0),
            Vec2::new(140.0, 34.0),
        );
        if pill_button(ui, brand, "LIBRARY", true) {
            actions.push(Action::OpenMenu);
        }
        if !self.drawer_open {
            let center = Rect::from_center_size(
                rect.center() + Vec2::new(-100.0, 0.0),
                Vec2::new(220.0, 46.0),
            );
            if pill_button(ui, center, "OPEN LIBRARY", true) {
                actions.push(Action::OpenMenu);
            }
        }
    }

    pub(super) fn draw_drawer(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>, now: f64) {
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
        let scrim = Rect::from_min_max(
            rect.min,
            Pos2::new((rect.width() - width * anim).max(0.0), rect.height()),
        );
        ui.painter().rect_filled(
            scrim,
            0.0,
            Color32::from_rgba_unmultiplied(3, 6, 12, (110.0 * anim) as u8),
        );
        if click(ui, scrim, "drawer_scrim") {
            actions.push(Action::CloseMenu);
        }
        let p = ui.painter().with_clip_rect(panel);
        p.rect_filled(panel, 0.0, Color32::from_rgb(8, 12, 21));
        p.line_segment(
            [panel.left_top(), panel.left_bottom()],
            Stroke::new(1.0f32, Color32::from_white_alpha(28)),
        );
        // header
        p.text(
            panel.left_top() + Vec2::new(20.0, 22.0),
            Align2::LEFT_TOP,
            "KARAOKEMACHINE · LIBRARY",
            FontId::monospace(8.0),
            MUTED2,
        );
        let drawer_title = if self.drawer_view == DrawerView::Imports {
            "Imports"
        } else if self.drawer_view == DrawerView::Maps {
            "Map Browser"
        } else if self.selected_album.is_some() {
            "Album"
        } else {
            "Records & warm-ups"
        };
        p.text(
            panel.left_top() + Vec2::new(20.0, 38.0),
            Align2::LEFT_TOP,
            drawer_title,
            FontId::proportional(24.0),
            INK,
        );
        let close = Rect::from_min_size(
            panel.right_top() + Vec2::new(-48.0, 16.0),
            Vec2::new(30.0, 30.0),
        );
        if glyph_button(ui, close, "X") {
            actions.push(Action::CloseMenu);
        }
        let body = Rect::from_min_max(
            Pos2::new(panel.left() + 18.0, panel.top() + 80.0),
            Pos2::new(panel.right() - 18.0, panel.bottom() - 72.0),
        );
        p.line_segment(
            [
                Pos2::new(panel.left(), body.top() - 12.0),
                Pos2::new(panel.right(), body.top() - 12.0),
            ],
            Stroke::new(1.0f32, Color32::from_white_alpha(18)),
        );
        // wheel scroll
        if let Some(pos) = ctx_pointer(ui) {
            if body.contains(pos) {
                let delta = ui.input(|i| i.raw_scroll_delta.y);
                if delta.abs() > 0.0 {
                    self.drawer_scroll = (self.drawer_scroll - delta).max(0.0);
                }
            }
        }
        p.text(
            body.left_top(),
            Align2::LEFT_TOP,
            " ",
            FontId::proportional(1.0),
            Color32::TRANSPARENT,
        );
        if self.drawer_view == DrawerView::Imports {
            self.draw_drawer_imports(ui, &p, body, actions);
        } else if self.drawer_view == DrawerView::Maps {
            self.draw_drawer_maps(ui, &p, body, actions);
        } else if let Some(index) = self.selected_album {
            self.draw_album_detail(ui, &p, body, index, actions);
        } else {
            self.draw_album_grid(ui, &p, body, actions);
        }
        // footer
        let foot = Rect::from_min_max(
            Pos2::new(panel.left(), panel.bottom() - 60.0),
            panel.right_bottom(),
        );
        p.rect_filled(foot, 0.0, Color32::from_rgb(8, 12, 21));
        p.line_segment(
            [foot.left_top(), foot.right_top()],
            Stroke::new(1.0f32, Color32::from_white_alpha(22)),
        );
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
            let br = Rect::from_min_size(
                Pos2::new(
                    foot.left() + 6.0 + i as f32 * (btn_w + 4.0),
                    foot.top() + 11.0,
                ),
                Vec2::new(btn_w, 34.0),
            );
            if pill_toggle(
                ui,
                br,
                label,
                (*view == 0
                    && self.drawer_view == DrawerView::Albums
                    && self.selected_album.is_none())
                    || (*view == 1 && self.drawer_view == DrawerView::Imports)
                    || (*view == 2 && self.drawer_view == DrawerView::Maps),
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

    pub(super) fn draw_album_grid(
        &self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        body: Rect,
        actions: &mut Vec<Action>,
    ) {
        let intro_h = 48.0;
        p.text(
            body.left_top(),
            Align2::LEFT_TOP,
            "SELECT A RECORD",
            FontId::monospace(8.0),
            MUTED2,
        );
        p.text(
            body.left_top() + Vec2::new(0.0, 16.0),
            Align2::LEFT_TOP,
            "Every track needs a manual mapping. Pick an album, then a song.",
            FontId::proportional(11.0),
            MUTED,
        );
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
                p.image(
                    tex.id(),
                    art_rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                paint_cover_fallback(p, art_rect, &album.title);
            }
            p.rect_stroke(
                art_rect,
                12.0,
                Stroke::new(1.0f32, Color32::from_white_alpha(30)),
                StrokeKind::Inside,
            );
            if album.warmup {
                let badge = Rect::from_min_size(
                    art_rect.left_top() + Vec2::new(8.0, 8.0),
                    Vec2::new(65.0, 20.0),
                );
                p.rect_filled(badge, 10.0, Color32::from_rgba_unmultiplied(8, 12, 21, 180));
                p.text(
                    badge.center(),
                    Align2::CENTER_CENTER,
                    "WARM-UPS",
                    FontId::monospace(6.0),
                    GOLD,
                );
            }
            let unmapped = album
                .songs
                .iter()
                .filter(|index| self.songs[**index].maps.is_empty())
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
                    format!("{unmapped} NO MAP"),
                    FontId::monospace(6.0),
                    PINK,
                );
            }
            p.text(
                art_rect.left_bottom() + Vec2::new(2.0, 9.0),
                Align2::LEFT_TOP,
                &album.title,
                FontId::proportional(13.0),
                INK,
            );
            p.text(
                art_rect.left_bottom() + Vec2::new(2.0, 26.0),
                Align2::LEFT_TOP,
                format!("{} · {} tracks", album.artist, album.songs.len()),
                FontId::monospace(8.0),
                MUTED2,
            );
            let hit = Rect::from_min_size(Pos2::new(x, y), Vec2::new(art, cell_h));
            if click(ui, hit, ("album", i)) {
                actions.push(Action::SelectAlbum(i));
            }
        }
    }

    pub(super) fn draw_album_detail(
        &self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        body: Rect,
        index: usize,
        actions: &mut Vec<Action>,
    ) {
        let Some(album) = self.albums.get(index) else {
            return;
        };
        let back = Rect::from_min_size(body.left_top(), Vec2::new(90.0, 28.0));
        if pill_toggle(ui, back, "← ALBUMS", false) {
            actions.push(Action::BackToAlbums);
        }
        let art = Rect::from_min_size(
            body.left_top() + Vec2::new(0.0, 42.0),
            Vec2::new(126.0, 126.0),
        );
        p.rect_filled(art, 12.0, Color32::from_rgb(18, 25, 42));
        if let Some(tex) = self.covers.get(&self.songs[album.songs[0]].id) {
            p.image(
                tex.id(),
                art,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            paint_cover_fallback(p, art, &album.title);
        }
        p.rect_stroke(
            art,
            12.0,
            Stroke::new(1.0f32, Color32::from_white_alpha(30)),
            StrokeKind::Inside,
        );
        p.text(
            art.right_top() + Vec2::new(16.0, 2.0),
            Align2::LEFT_TOP,
            if album.warmup {
                "KARAOKEMACHINE SESSIONS"
            } else {
                "ALBUM"
            },
            FontId::monospace(8.0),
            MUTED2,
        );
        p.text(
            art.right_top() + Vec2::new(16.0, 20.0),
            Align2::LEFT_TOP,
            &album.title,
            FontId::proportional(26.0),
            INK,
        );
        let release = album
            .songs
            .iter()
            .find_map(|index| self.songs[*index].release_date.clone());
        let meta = match release {
            Some(date) => format!("{} · {} tracks · {}", album.artist, album.songs.len(), date),
            None => format!("{} · {} tracks", album.artist, album.songs.len()),
        };
        p.text(
            art.right_top() + Vec2::new(16.0, 54.0),
            Align2::LEFT_TOP,
            meta,
            FontId::monospace(9.0),
            MUTED,
        );
        let unmapped = album
            .songs
            .iter()
            .filter(|index| self.songs[**index].maps.is_empty())
            .count();
        p.text(
            art.right_top() + Vec2::new(16.0, 76.0),
            Align2::LEFT_TOP,
            if unmapped > 0 {
                format!("{} of {} tracks need a map.", unmapped, album.songs.len())
            } else {
                "All tracks have a playable map.".into()
            },
            FontId::proportional(10.0),
            if unmapped > 0 { PINK } else { MINT },
        );
        let list_top = art.bottom() + 26.0 - self.drawer_scroll;
        for (i, &song_index) in album.songs.iter().enumerate() {
            let song = &self.songs[song_index];
            let row = Rect::from_min_size(
                Pos2::new(body.left(), list_top + i as f32 * 46.0),
                Vec2::new(body.width(), 42.0),
            );
            if row.bottom() < body.top() || row.top() > body.bottom() {
                continue;
            }
            p.text(
                row.left_center() + Vec2::new(2.0, -8.0),
                Align2::LEFT_CENTER,
                format!("{:02}", i + 1),
                FontId::monospace(8.0),
                MUTED2,
            );
            p.text(
                row.left_center() + Vec2::new(30.0, -7.0),
                Align2::LEFT_CENTER,
                &song.title,
                FontId::proportional(12.5),
                INK,
            );
            let map_count = song.maps.len();
            let sub = if map_count == 0 {
                "NO MAP · choose a map file to edit".to_string()
            } else if map_count > 1 {
                format!("{map_count} MAPS · choose one to play")
            } else if song.maps[0].draft {
                "DRAFT · playable preview".to_string()
            } else if automap::is_auto_mapped(song) {
                "auto-mapped level".to_string()
            } else if song.lyrics_synced.unwrap_or(false) {
                "synced lyrics".to_string()
            } else if !song.lines.is_empty() {
                "lyrics".to_string()
            } else {
                "instrumental".to_string()
            };
            p.text(
                row.left_center() + Vec2::new(30.0, 9.0),
                Align2::LEFT_CENTER,
                sub,
                FontId::monospace(7.0),
                if map_count == 0 { PINK } else { MUTED2 },
            );
            let meta = format!(
                "{} · {}",
                song.bpm
                    .map(|b| format!("{b:.0} BPM · "))
                    .unwrap_or_default(),
                format_time(song.duration())
            );
            p.text(
                Pos2::new(row.right() - 104.0, row.center().y),
                Align2::RIGHT_CENTER,
                meta,
                FontId::monospace(7.0),
                MUTED2,
            );
            p.text(
                Pos2::new(row.right() - 66.0, row.center().y),
                Align2::RIGHT_CENTER,
                if map_count > 0 { "PLAY" } else { "MAP" },
                FontId::monospace(7.0),
                if map_count > 0 { MINT } else { PINK },
            );
            let edit = Rect::from_min_size(
                Pos2::new(row.right() - 36.0, row.top() + 9.0),
                Vec2::new(30.0, 24.0),
            );
            if pill_toggle(ui, edit, "EDIT", map_count == 0) {
                actions.push(Action::EditSong(song_index));
            }
            let main = Rect::from_min_max(row.min, Pos2::new(row.right() - 42.0, row.bottom()));
            if click(ui, main, ("track", song_index)) {
                actions.push(Action::PlaySong(song_index));
            }
        }
    }

    pub(super) fn draw_drawer_imports(
        &self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        body: Rect,
        actions: &mut Vec<Action>,
    ) {
        p.text(
            body.left_top(),
            Align2::LEFT_TOP,
            "IMPORT QUEUE",
            FontId::monospace(8.0),
            MUTED2,
        );
        p.text(
            body.left_top() + Vec2::new(0.0, 18.0),
            Align2::LEFT_TOP,
            "From your list",
            FontId::proportional(26.0),
            INK,
        );
        p.text(
            body.left_top() + Vec2::new(0.0, 52.0),
            Align2::LEFT_TOP,
            &self.songlist_path,
            FontId::monospace(8.0),
            MUTED2,
        );
        let ready = self
            .songlist
            .iter()
            .filter(|url| {
                self.songs
                    .iter()
                    .any(|s| s.youtube_url.as_deref() == Some(url.as_str()))
            })
            .count();
        p.text(
            body.left_top() + Vec2::new(0.0, 76.0),
            Align2::LEFT_TOP,
            format!("{ready}/{} ready", self.songlist.len()),
            FontId::monospace(8.0),
            MUTED,
        );
        let reload = Rect::from_min_size(
            Pos2::new(body.right() - 100.0, body.top() + 70.0),
            Vec2::new(100.0, 30.0),
        );
        if pill_toggle(ui, reload, "RELOAD", false) {
            actions.push(Action::Reload);
        }
        let upload = Rect::from_min_size(
            Pos2::new(body.right() - 250.0, body.top() + 70.0),
            Vec2::new(138.0, 30.0),
        );
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
            let song = self
                .songs
                .iter()
                .find(|s| s.youtube_url.as_deref() == Some(url.as_str()));
            p.line_segment(
                [
                    Pos2::new(row.left(), row.bottom()),
                    Pos2::new(row.right(), row.bottom()),
                ],
                Stroke::new(0.6f32, Color32::from_white_alpha(12)),
            );
            p.text(
                Pos2::new(row.left() + 2.0, row.center().y),
                Align2::LEFT_CENTER,
                format!("{:02}", i + 1),
                FontId::monospace(7.0),
                MUTED2,
            );
            let title = song
                .map(|s| s.title.clone())
                .unwrap_or_else(|| format!("Track {:02}", i + 1));
            p.text(
                Pos2::new(row.left() + 28.0, row.center().y - 7.0),
                Align2::LEFT_CENTER,
                title,
                FontId::proportional(11.0),
                if song.is_some() { INK } else { MUTED },
            );
            p.text(
                Pos2::new(row.left() + 28.0, row.center().y + 8.0),
                Align2::LEFT_CENTER,
                url.split("v=").nth(1).unwrap_or(url),
                FontId::monospace(7.0),
                MUTED2,
            );
            let status = if let Some(song) = song {
                if !song.maps.is_empty() {
                    "PLAY"
                } else {
                    "MAP"
                }
            } else {
                "QUEUED"
            };
            p.text(
                Pos2::new(row.right() - 10.0, row.center().y),
                Align2::RIGHT_CENTER,
                status,
                FontId::monospace(7.0),
                match song {
                    Some(song) if !song.maps.is_empty() => MINT,
                    Some(_) => PINK,
                    None => MUTED2,
                },
            );
            if let Some(song_index) = self
                .songs
                .iter()
                .position(|s| s.youtube_url.as_deref() == Some(url.as_str()))
            {
                if click(ui, row, ("import_track", i)) {
                    actions.push(Action::PlaySong(song_index));
                }
            }
        }
        p.text(
            body.left_bottom() + Vec2::new(0.0, -24.0),
            Align2::LEFT_BOTTOM,
            "Audio stays local. Vocals are optional.",
            FontId::monospace(8.0),
            MUTED2,
        );
    }
}
