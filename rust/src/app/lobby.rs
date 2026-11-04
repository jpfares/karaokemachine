//! The lobby and the library drawer: albums, tracks and the imports tab.

use super::stage::add_ribbon_mesh;
use super::{ctx_pointer, pitch_col, Action, DrawerView, KaraokeMachineApp};
use crate::automap;
use crate::library::{build_albums, load_songs, read_songlist};
use crate::model::{accuracy_label, format_time, next_rank, rank_for, ScoreRecord, Song};
use crate::theme::{alpha, rank_colour, BG, GOLD, INK, LINE, MINT, MUTED, MUTED2, PANEL, PINK};
use crate::widgets::{click, glyph_button, pill_button, pill_toggle};
use eframe::egui::{
    self, Align2, Color32, FontId, Id, Mesh, Pos2, Rect, Shape, Stroke, StrokeKind, TextureHandle,
    TextureOptions, Vec2,
};
use std::path::Path;

pub(super) fn paint_cover_fallback(p: &egui::Painter, rect: Rect, rounding: f32, title: &str) {
    let c = pitch_col(((title.len() * 37) % 100) as f32 / 100.0);
    p.rect_filled(rect, rounding, alpha(c, 80));
    p.circle_filled(rect.center(), rect.width() * 0.22, alpha(c, 100));
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        title.chars().next().unwrap_or('K').to_string(),
        FontId::proportional((rect.width() * 0.25).max(10.0)),
        INK,
    );
}

/// Any map file plays, draft or level, so the home offers every song with one.
fn playable(song: &Song) -> bool {
    !song.maps.is_empty()
}

/// Cover art at `rect`, or the generated fallback, with a hairline frame.
fn paint_cover(
    p: &egui::Painter,
    rect: Rect,
    rounding: f32,
    tex: Option<&TextureHandle>,
    title: &str,
) {
    p.rect_filled(rect, rounding, Color32::from_rgb(18, 25, 42));
    match tex {
        Some(tex) => {
            p.image(
                tex.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => paint_cover_fallback(p, rect, rounding, title),
    }
    p.rect_stroke(
        rect,
        rounding,
        Stroke::new(1.0f32, Color32::from_white_alpha(30)),
        StrokeKind::Inside,
    );
}

/// "+N% TO A" in gold, or "TOP RANK" in mint at S.
fn nudge_text(accuracy: f32) -> (String, Color32) {
    match next_rank(accuracy) {
        Some((rank, needed)) => (format!("+{needed:.0}% TO {rank}"), GOLD),
        None => ("TOP RANK".to_string(), MINT),
    }
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}S")
    }
}

/// `ScoreRecord::beats` as an ordering, for sorting.
fn compare_records(a: &ScoreRecord, b: &ScoreRecord) -> std::cmp::Ordering {
    if a.beats(b) {
        std::cmp::Ordering::Greater
    } else if b.beats(a) {
        std::cmp::Ordering::Less
    } else {
        std::cmp::Ordering::Equal
    }
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

    /// The song the home screen leads with: the last one sung, else the
    /// best-scored, else the first warm-up, else the first mapped song.
    /// The flag says whether it came from `last_song`.
    fn home_hero(&self) -> Option<(usize, bool)> {
        if let Some(id) = &self.settings.last_song {
            if let Some(i) = self.songs.iter().position(|s| &s.id == id && playable(s)) {
                return Some((i, true));
            }
        }
        let best = self
            .scored_mapped()
            .into_iter()
            .max_by(|a, b| compare_records(a.1, b.1));
        if let Some((i, _)) = best {
            return Some((i, false));
        }
        self.songs
            .iter()
            .position(|s| playable(s) && s.source == "scale")
            .or_else(|| self.songs.iter().position(playable))
            .map(|i| (i, false))
    }

    /// Every high score that still belongs to a mapped song.
    fn scored_mapped(&self) -> Vec<(usize, &ScoreRecord)> {
        self.scores
            .iter()
            .filter_map(|(id, record)| {
                self.songs
                    .iter()
                    .position(|s| &s.id == id && playable(s))
                    .map(|i| (i, record))
            })
            .collect()
    }

    pub(super) fn draw_lobby(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let rect = ui.max_rect();
        let p = ui.painter().clone();
        p.rect_filled(rect, 0.0, BG);
        let w = rect.width();
        let h = rect.height();
        let x0 = 40.0;
        // The drawer covers the right 430 px; everything clickable stays left of it.
        let safe_right = w - 470.0;
        let cw = safe_right - x0;
        let hero = Rect::from_min_max(Pos2::new(x0, 86.0), Pos2::new(safe_right, 386.0));
        // Widgets are painted whether or not the drawer is open, but only the
        // scrim answers clicks while it is: a tap on the dimmed home closes
        // the drawer instead of starting a song.
        let live = !self.drawer_open;

        // The pitch ribbon, the app's signature, flows out from under the hero
        // card into the strip the drawer covers.
        let ribbon_x0 = safe_right - 200.0;
        let ribbon_x1 = w - 40.0;
        let points: Vec<Pos2> = (0..=180)
            .map(|i| {
                let t = i as f32 / 180.0;
                let y = 236.0
                    + (t * std::f32::consts::TAU * 2.2).sin() * 100.0
                    + (t * std::f32::consts::TAU * 5.0 + 0.7).sin() * 20.0;
                Pos2::new(ribbon_x0 + t * (ribbon_x1 - ribbon_x0), y)
            })
            .collect();
        let mut mesh = Mesh::default();
        add_ribbon_mesh(
            &mut mesh,
            &points,
            24.0,
            52,
            80,
            hero.top(),
            hero.bottom(),
            0.9,
        );
        p.add(Shape::mesh(mesh));

        // header
        p.text(
            Pos2::new(x0, 30.0),
            Align2::LEFT_CENTER,
            "KARAOKEMACHINE",
            FontId::proportional(18.0),
            alpha(INK, 200),
        );
        p.text(
            Pos2::new(x0, 50.0),
            Align2::LEFT_CENTER,
            "HOME",
            FontId::monospace(10.0),
            MUTED,
        );
        let brand = Rect::from_min_size(Pos2::new(w - 170.0, 22.0), Vec2::new(140.0, 34.0));
        if pill_button(ui, brand, "LIBRARY", false) && live {
            actions.push(Action::OpenMenu);
        }

        // right strip: caption under the ribbon and a footer count
        p.text(
            Pos2::new(safe_right + 40.0, hero.bottom() + 24.0),
            Align2::LEFT_TOP,
            "ON STAGE, NOTES SCROLL BY AS",
            FontId::monospace(10.0),
            MUTED,
        );
        p.text(
            Pos2::new(safe_right + 40.0, hero.bottom() + 40.0),
            Align2::LEFT_TOP,
            "COLOURED RIBBONS. YOU SING ALONG.",
            FontId::monospace(10.0),
            MUTED,
        );
        let playable_count = self.songs.iter().filter(|s| playable(s)).count();
        p.text(
            Pos2::new(w - 40.0, h - 30.0),
            Align2::RIGHT_CENTER,
            format!(
                "{} · {playable_count} PLAYABLE · {}",
                plural(self.songs.len(), "SONG"),
                plural(self.albums.len(), "ALBUM")
            ),
            FontId::monospace(10.0),
            MUTED2,
        );

        // hero card
        let hp = p.with_clip_rect(hero);
        hp.rect_filled(hero, 18.0, PANEL);
        hp.rect_stroke(hero, 18.0, Stroke::new(1.0f32, LINE), StrokeKind::Inside);
        // The interior scales with the card so the window minimum (1100 pt
        // wide) still fits the stat columns and both pills.
        let cover_size = (cw * 0.27).clamp(160.0, 252.0);
        let cover_rect = Rect::from_min_size(
            Pos2::new(x0 + 24.0, hero.center().y - cover_size / 2.0),
            Vec2::splat(cover_size),
        );
        let text_x = cover_rect.right() + 28.0;
        let avail = hero.right() - 24.0 - text_x;
        let primary_w = (avail * 0.56).clamp(150.0, 240.0);
        let secondary_w = (avail - primary_w - 16.0).min(170.0);
        let show_secondary = secondary_w >= 110.0;
        let column_pitch = ((avail - 84.0) / 3.0).min(160.0);
        let hero_pick = self.home_hero();
        let hero_idx = hero_pick.map(|(i, _)| i);
        let primary = Rect::from_min_size(Pos2::new(text_x, 320.0), Vec2::new(primary_w, 48.0));
        let secondary = Rect::from_min_size(
            Pos2::new(text_x + primary_w + 16.0, 320.0),
            Vec2::new(secondary_w, 48.0),
        );
        match hero_pick {
            Some((index, from_last)) => {
                let song = &self.songs[index];
                let is_scale = song.source == "scale";
                let record = self.scores.get(&song.id);
                paint_cover(
                    &hp,
                    cover_rect,
                    14.0,
                    self.covers.get(&song.id),
                    &song.title,
                );
                if click(ui, cover_rect, "home_hero_cover") && live {
                    actions.push(Action::PlaySong(index));
                }
                let mut eyebrow = if from_last && record.is_none() {
                    "CONTINUE"
                } else if from_last {
                    "SING AGAIN"
                } else if record.is_some() {
                    "YOUR BEST TAKE"
                } else {
                    "START HERE"
                }
                .to_string();
                if is_scale {
                    eyebrow.push_str(" · WARM-UP");
                }
                hp.text(
                    Pos2::new(text_x, 110.0),
                    Align2::LEFT_TOP,
                    eyebrow,
                    FontId::monospace(10.0),
                    MUTED,
                );
                hp.text(
                    Pos2::new(text_x, 130.0),
                    Align2::LEFT_TOP,
                    &song.title,
                    FontId::proportional(24.0),
                    INK,
                );
                hp.text(
                    Pos2::new(text_x, 164.0),
                    Align2::LEFT_TOP,
                    format!(
                        "{} · {}",
                        song.artist_display(),
                        format_time(song.duration())
                    ),
                    FontId::proportional(13.0),
                    MUTED,
                );
                match record {
                    Some(record) => {
                        let rank = rank_for(record.accuracy);
                        hp.text(
                            Pos2::new(text_x, 192.0),
                            Align2::LEFT_TOP,
                            rank.to_string(),
                            FontId::proportional(56.0),
                            rank_colour(rank),
                        );
                        let columns = [
                            ("SCORE", record.score.to_string()),
                            ("ACCURACY", accuracy_label(record.accuracy)),
                            ("COMBO", format!("{}×", record.combo)),
                        ];
                        for (i, (label, value)) in columns.iter().enumerate() {
                            let cx = text_x + 84.0 + i as f32 * column_pitch;
                            hp.text(
                                Pos2::new(cx, 204.0),
                                Align2::LEFT_TOP,
                                *label,
                                FontId::monospace(10.0),
                                MUTED,
                            );
                            hp.text(
                                Pos2::new(cx, 220.0),
                                Align2::LEFT_TOP,
                                value,
                                FontId::proportional(22.0),
                                INK,
                            );
                        }
                        let (nudge, colour) = nudge_text(record.accuracy);
                        hp.text(
                            Pos2::new(text_x + 84.0, 260.0),
                            Align2::LEFT_TOP,
                            nudge,
                            FontId::monospace(10.0),
                            colour,
                        );
                    }
                    None => {
                        hp.text(
                            Pos2::new(text_x, 200.0),
                            Align2::LEFT_TOP,
                            "Not scored yet. Sing it once to set a personal best.",
                            FontId::proportional(13.0),
                            MUTED,
                        );
                    }
                }
                let verb = if from_last && record.is_some() {
                    "SING AGAIN"
                } else {
                    "SING THIS"
                };
                if pill_button(ui, primary, verb, true) && live {
                    actions.push(Action::PlaySong(index));
                }
                if show_secondary && pill_button(ui, secondary, "PICK ANOTHER", false) && live {
                    actions.push(Action::OpenMenu);
                }
            }
            None => {
                paint_cover(&hp, cover_rect, 14.0, None, "K");
                hp.text(
                    Pos2::new(text_x, 110.0),
                    Align2::LEFT_TOP,
                    "GET STARTED",
                    FontId::monospace(10.0),
                    MUTED,
                );
                hp.text(
                    Pos2::new(text_x, 130.0),
                    Align2::LEFT_TOP,
                    "Add your first song",
                    FontId::proportional(24.0),
                    INK,
                );
                hp.text(
                    Pos2::new(text_x, 164.0),
                    Align2::LEFT_TOP,
                    "A YouTube link or an audio file is enough. Auto-map charts it.",
                    FontId::proportional(13.0),
                    MUTED,
                );
                if pill_button(ui, primary, "UPLOAD A SONG", true) && live {
                    actions.push(Action::OpenUpload);
                }
            }
        }

        // three doors
        let first_scale = self
            .songs
            .iter()
            .position(|s| playable(s) && s.source == "scale");
        let hero_is_scale = hero_idx
            .map(|i| self.songs[i].source == "scale")
            .unwrap_or(false);
        let scale_count = self
            .songs
            .iter()
            .filter(|s| playable(s) && s.source == "scale")
            .count();
        let card_w = (cw - 40.0) / 3.0;
        let narrow = card_w < 260.0;
        let warmup_sub;
        let (warmup_sub_text, warmup_verb): (&str, &str) = match first_scale {
            Some(index) if !hero_is_scale => {
                let song = &self.songs[index];
                warmup_sub = format!("{} · {}", song.title, format_time(song.duration()));
                (&warmup_sub, "SING")
            }
            Some(_) if narrow => {
                warmup_sub = format!("{} scales", scale_count);
                (&warmup_sub, "BROWSE")
            }
            Some(_) => {
                warmup_sub = format!("{} short scales · pick one", scale_count);
                (&warmup_sub, "BROWSE")
            }
            None => ("No warm-ups installed", "LIBRARY"),
        };
        let cards = [
            ("01", "Sing a warm-up", warmup_sub_text, warmup_verb),
            (
                "02",
                "Upload a song",
                if narrow {
                    "Link or file · optional auto-map"
                } else {
                    "YouTube link or audio file · optional auto-map"
                },
                "IMPORT",
            ),
            (
                "03",
                "Browse maps",
                if narrow {
                    "Published charts"
                } else {
                    "Charts other singers published"
                },
                "BROWSE",
            ),
        ];
        for (i, (numeral, title, sub, verb)) in cards.iter().enumerate() {
            let c = Rect::from_min_size(
                Pos2::new(x0 + i as f32 * (card_w + 20.0), 406.0),
                Vec2::new(card_w, 100.0),
            );
            let cp = p.with_clip_rect(c);
            cp.rect_filled(c, 14.0, PANEL);
            cp.rect_stroke(
                c,
                14.0,
                Stroke::new(1.0f32, Color32::from_white_alpha(28)),
                StrokeKind::Inside,
            );
            cp.text(
                c.left_top() + Vec2::new(20.0, 16.0),
                Align2::LEFT_TOP,
                *numeral,
                FontId::monospace(10.0),
                MUTED2,
            );
            cp.text(
                c.right_top() + Vec2::new(-20.0, 16.0),
                Align2::RIGHT_TOP,
                *verb,
                FontId::monospace(10.0),
                MINT,
            );
            cp.text(
                c.left_top() + Vec2::new(20.0, 38.0),
                Align2::LEFT_TOP,
                *title,
                FontId::proportional(16.0),
                INK,
            );
            cp.text(
                c.left_top() + Vec2::new(20.0, 66.0),
                Align2::LEFT_TOP,
                *sub,
                FontId::proportional(12.0),
                MUTED,
            );
            if click(ui, c, ("home_card", i)) && live {
                match i {
                    0 => match first_scale {
                        Some(index) if !hero_is_scale => actions.push(Action::PlaySong(index)),
                        Some(_) => {
                            actions.push(Action::OpenMenu);
                            if let Some(album) = self.albums.iter().position(|a| a.warmup) {
                                actions.push(Action::SelectAlbum(album));
                            }
                        }
                        None => actions.push(Action::OpenMenu),
                    },
                    1 => actions.push(Action::OpenUpload),
                    _ => {
                        actions.push(Action::OpenMenu);
                        actions.push(Action::ShowMaps);
                    }
                }
            }
        }

        // personal bests, padded with songs still to sing
        let rows_top = 552.0;
        let row_h = 52.0;
        let n_rows = (((h - 40.0 - rows_top) / row_h).floor() as usize).min(6);
        let mut scored = self.scored_mapped();
        let n_scored = scored.len();
        scored.retain(|(i, _)| Some(*i) != hero_idx);
        scored.sort_by(|a, b| compare_records(b.1, a.1));
        let mut rows: Vec<(usize, Option<&ScoreRecord>)> =
            scored.iter().map(|(i, r)| (*i, Some(*r))).collect();
        rows.truncate(n_rows);
        if rows.len() < n_rows {
            let mut pad: Vec<usize> = (0..self.songs.len())
                .filter(|i| {
                    let song = &self.songs[*i];
                    playable(song) && Some(*i) != hero_idx && !self.scores.contains_key(&song.id)
                })
                .collect();
            pad.sort_by_key(|i| self.songs[*i].source != "scale");
            for i in pad {
                if rows.len() >= n_rows {
                    break;
                }
                rows.push((i, None));
            }
        }
        let header_y = 530.0;
        p.text(
            Pos2::new(x0, header_y),
            Align2::LEFT_TOP,
            if n_scored > 0 {
                "PERSONAL BESTS · RANKED BY ACCURACY"
            } else {
                "UP NEXT"
            },
            FontId::monospace(10.0),
            MUTED,
        );
        p.text(
            Pos2::new(safe_right, header_y),
            Align2::RIGHT_TOP,
            if n_scored > 0 {
                format!("{} SCORED", plural(n_scored, "SONG"))
            } else {
                "NOTHING SCORED YET".to_string()
            },
            FontId::monospace(10.0),
            MUTED,
        );
        if rows.is_empty() {
            p.text(
                Pos2::new(x0, rows_top + 4.0),
                Align2::LEFT_TOP,
                if n_scored > 0 {
                    "Your only score so far is on the song above."
                } else {
                    "No scores yet. Finish a song and your best runs land here."
                },
                FontId::proportional(12.0),
                MUTED,
            );
        }
        for (i, (index, record)) in rows.iter().enumerate() {
            let song = &self.songs[*index];
            let r = Rect::from_min_size(
                Pos2::new(x0, rows_top + i as f32 * row_h),
                Vec2::new(cw, row_h),
            );
            p.line_segment(
                [r.left_bottom(), r.right_bottom()],
                Stroke::new(1.0f32, Color32::from_white_alpha(12)),
            );
            // Keep the title and subtitle clear of the right-hand columns.
            let right_cols = if record.is_some() { 362.0 } else { 120.0 };
            let lp = p.with_clip_rect(Rect::from_min_max(
                r.left_top(),
                Pos2::new(r.right() - right_cols, r.bottom()),
            ));
            match record {
                Some(record) => {
                    let rank = rank_for(record.accuracy);
                    lp.text(
                        Pos2::new(r.left() + 30.0, r.center().y),
                        Align2::CENTER_CENTER,
                        rank.to_string(),
                        FontId::proportional(20.0),
                        rank_colour(rank),
                    );
                }
                None => {
                    lp.text(
                        Pos2::new(r.left() + 30.0, r.center().y),
                        Align2::CENTER_CENTER,
                        "SING",
                        FontId::monospace(10.0),
                        MINT,
                    );
                }
            }
            let thumb = Rect::from_min_size(
                Pos2::new(r.left() + 56.0, r.top() + 8.0),
                Vec2::new(36.0, 36.0),
            );
            paint_cover(&lp, thumb, 8.0, self.covers.get(&song.id), &song.title);
            lp.text(
                Pos2::new(r.left() + 104.0, r.top() + 9.0),
                Align2::LEFT_TOP,
                &song.title,
                FontId::proportional(13.0),
                INK,
            );
            let mut sub = if cw < 800.0 && song.source == "scale" {
                format_time(song.duration())
            } else {
                format!(
                    "{} · {}",
                    song.artist_display(),
                    format_time(song.duration())
                )
            };
            if song.source == "scale" {
                sub.push_str(" · WARM-UP");
            }
            lp.text(
                Pos2::new(r.left() + 104.0, r.top() + 29.0),
                Align2::LEFT_TOP,
                sub,
                FontId::monospace(10.0),
                MUTED,
            );
            match record {
                Some(record) => {
                    let (nudge, colour) = nudge_text(record.accuracy);
                    let columns = [
                        (
                            r.right() - 16.0,
                            record.score.to_string(),
                            "SCORE".to_string(),
                            MUTED,
                        ),
                        (
                            r.right() - 140.0,
                            format!("{}×", record.combo),
                            "COMBO".to_string(),
                            MUTED,
                        ),
                        (
                            r.right() - 270.0,
                            accuracy_label(record.accuracy),
                            nudge,
                            colour,
                        ),
                    ];
                    for (x, value, label, colour) in columns {
                        p.text(
                            Pos2::new(x, r.top() + 8.0),
                            Align2::RIGHT_TOP,
                            value,
                            FontId::proportional(16.0),
                            INK,
                        );
                        p.text(
                            Pos2::new(x, r.top() + 30.0),
                            Align2::RIGHT_TOP,
                            label,
                            FontId::monospace(10.0),
                            colour,
                        );
                    }
                }
                None => {
                    p.text(
                        Pos2::new(r.right() - 16.0, r.center().y),
                        Align2::RIGHT_CENTER,
                        "NOT YET SUNG",
                        FontId::monospace(10.0),
                        MUTED,
                    );
                }
            }
            if click(ui, r, ("home_best", i)) && live {
                actions.push(Action::PlaySong(*index));
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
            FontId::monospace(10.0),
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
        let intro_h = 62.0;
        p.text(
            body.left_top(),
            Align2::LEFT_TOP,
            "SELECT A RECORD",
            FontId::monospace(10.0),
            MUTED2,
        );
        p.text(
            body.left_top() + Vec2::new(0.0, 16.0),
            Align2::LEFT_TOP,
            "Pick an album, then a song.",
            FontId::proportional(11.0),
            MUTED,
        );
        p.text(
            body.left_top() + Vec2::new(0.0, 31.0),
            Align2::LEFT_TOP,
            "Unmapped songs pick a map file first; AUTO-MAP charts them in the editor.",
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
                paint_cover_fallback(p, art_rect, 12.0, &album.title);
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
                FontId::monospace(10.0),
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
            paint_cover_fallback(p, art, 12.0, &album.title);
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
            FontId::monospace(10.0),
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
            FontId::monospace(10.0),
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
                FontId::monospace(10.0),
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
                "NO MAP · EDIT to map it".to_string()
            } else if map_count > 1 {
                format!("{map_count} MAPS · PLAY to choose")
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
                FontId::monospace(10.0),
                if map_count == 0 { PINK } else { MUTED2 },
            );
            let meta = match song.bpm {
                Some(bpm) => format!("{bpm:.0} BPM · {}", format_time(song.duration())),
                None => format_time(song.duration()),
            };
            p.text(
                Pos2::new(row.right() - 104.0, row.center().y),
                Align2::RIGHT_CENTER,
                meta,
                FontId::monospace(10.0),
                MUTED2,
            );
            p.text(
                Pos2::new(row.right() - 66.0, row.center().y),
                Align2::RIGHT_CENTER,
                if map_count > 0 { "PLAY" } else { "MAP" },
                FontId::monospace(10.0),
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
            FontId::monospace(10.0),
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
            FontId::monospace(10.0),
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
            FontId::monospace(10.0),
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
                FontId::monospace(10.0),
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
                FontId::monospace(10.0),
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
                FontId::monospace(10.0),
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
            FontId::monospace(10.0),
            MUTED2,
        );
    }
}
