use crate::audio::AudioEngine;
use crate::model::{format_time, note_name, pitch_color, LyricLine, Note, Song};
use eframe::egui::{
    self, Align2, Color32, FontId, Id, LayerId, Order, PointerButton, Pos2, Rect, Sense,
    Stroke, StrokeKind, Ui, Vec2,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const BG: Color32 = Color32::from_rgb(5, 7, 14);
const INK: Color32 = Color32::from_rgb(244, 247, 251);
const MUTED: Color32 = Color32::from_rgb(142, 150, 170);
const MUTED2: Color32 = Color32::from_rgb(94, 102, 121);
const MINT: Color32 = Color32::from_rgb(124, 247, 196);
const PINK: Color32 = Color32::from_rgb(255, 93, 158);
const GOLD: Color32 = Color32::from_rgb(255, 209, 102);
const PANEL: Color32 = Color32::from_rgb(9, 13, 23);
const PANEL2: Color32 = Color32::from_rgb(12, 17, 29);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorTool {
    Pan,
    Note,
    Ribbon,
    Eraser,
    Select,
}

impl EditorTool {
    fn label(self) -> &'static str {
        match self {
            EditorTool::Pan => "PAN",
            EditorTool::Note => "NOTE",
            EditorTool::Ribbon => "RIBBON",
            EditorTool::Eraser => "ERASER",
            EditorTool::Select => "SELECT",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            EditorTool::Pan => "Drag to pan. Middle-click pans from any tool.",
            EditorTool::Note => "Click or drag on the lane to draw one pitch. Drag sets length.",
            EditorTool::Ribbon => "Drag across notes to select a ribbon, then drop a word on it.",
            EditorTool::Eraser => "Click or sweep over notes to erase them.",
            EditorTool::Select => "Click notes to select. Drag the body to move, the edges to resize.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorEvent {
    None,
    Back,
    Submit,
}

#[derive(Clone, Debug)]
struct WordToken {
    text: String,
    line: usize,
    index: usize,
    start: f64,
    placed: bool,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    NewNote {
        index: usize,
        start: f64,
    },
    Move {
        index: usize,
        grab: f64,
        midi_offset: i32,
    },
    ResizeLeft {
        index: usize,
        end: f64,
    },
    ResizeRight {
        index: usize,
        start: f64,
    },
    Ribbon {
        anchor_start: f64,
    },
    Erase,
}

struct Layout {
    note_area: Rect,
    rail: Rect,
    inspector: Rect,
    ruler: Rect,
    transport: Rect,
    bank: Rect,
}

pub struct Editor {
    pub song: Song,
    suggestions: Vec<Note>,
    notes: Vec<Note>,
    word_bank: Vec<WordToken>,
    tool: EditorTool,
    scroll: f64,
    playhead: f64,
    playing: bool,
    speed: f32,
    zoom: f32,
    low: i32,
    high: i32,
    selected: BTreeSet<usize>,
    drag: Option<Drag>,
    word_drag: Option<usize>,
    pending_word: Option<usize>,
    history: Vec<Vec<Note>>,
    redo: Vec<Vec<Note>>,
    dirty: bool,
    audio_ready: bool,
    backing_on: bool,
    vocals_on: bool,
    show_suggestions: bool,
    follow: bool,
    snap: bool,
    duration: f64,
    status: String,
    last_seek: f64,
}

impl Editor {
    pub fn new(
        mut song: Song,
        suggestions: Vec<Note>,
        initial_notes: Vec<Note>,
        duration: f64,
    ) -> Self {
        song.notes.clear();
        let duration = duration.max(song.duration()).max(1.0);
        let word_bank = build_word_bank(&song, &initial_notes);
        let mut low = song.low.max(0);
        let mut high = song.high.min(127);
        let mut source: Vec<i32> = suggestions
            .iter()
            .chain(initial_notes.iter())
            .map(|n| n.midi)
            .collect();
        if source.is_empty() {
            source = song.notes.iter().map(|n| n.midi).collect();
        }
        if let (Some(lo), Some(hi)) = (source.iter().min(), source.iter().max()) {
            low = *lo - 2;
            high = *hi + 3;
        }
        if high - low < 24 {
            let center = (high + low) as f32 / 2.0;
            low = (center - 12.0).floor() as i32;
            high = (center + 12.0).ceil() as i32;
        }
        low = low.clamp(0, 108);
        high = high.clamp(low + 18, 127);
        let zoom = 90.0;
        let first_note = suggestions
            .iter()
            .chain(initial_notes.iter())
            .map(|note| note.start)
            .fold(f64::INFINITY, f64::min);
        let scroll = if first_note.is_finite() && first_note > 3.0 {
            (first_note - 1.0).max(0.0)
        } else {
            0.0
        };
        let status = if initial_notes.is_empty() {
            "Draw notes over the translucent suggestions, then place every word from the bank."
        } else {
            "Draft loaded. Finish the word bank and submit when the mapping is complete."
        };
        Self {
            song,
            suggestions,
            notes: initial_notes,
            word_bank,
            tool: EditorTool::Note,
            scroll,
            playhead: scroll,
            playing: false,
            speed: 1.0,
            zoom,
            low,
            high,
            selected: BTreeSet::new(),
            drag: None,
            word_drag: None,
            pending_word: None,
            history: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            audio_ready: false,
            backing_on: true,
            vocals_on: true,
            show_suggestions: true,
            follow: true,
            snap: true,
            duration,
            status: status.to_string(),
            last_seek: 0.0,
        }
    }

    pub fn set_audio_ready(&mut self, ready: bool) {
        self.audio_ready = ready;
    }

    pub fn pause_audio(&self, audio: &Option<AudioEngine>) {
        if let Some(audio) = audio {
            audio.pause();
        }
    }

    pub fn save_draft(&mut self, root: &Path) -> Result<(), String> {
        if self.song.mapped {
            return Ok(());
        }
        self.notes.sort_by(note_order);
        self.song.notes.clear();
        self.song.draft_notes = self.notes.clone();
        self.song.suggested_notes = self.suggestions.clone();
        self.song.mapped = false;
        write_song(root, &self.song)
    }

    pub fn submit(&mut self, root: &Path) -> Result<(), String> {
        self.validate()?;
        self.notes.sort_by(note_order);
        let midis: Vec<i32> = self.notes.iter().map(|n| n.midi).collect();
        if let (Some(lo), Some(hi)) = (midis.iter().min(), midis.iter().max()) {
            self.song.low = (*lo - 2).clamp(0, 108);
            self.song.high = (*hi + 2).clamp(self.song.low + 12, 127);
        }
        self.song.notes = self.notes.clone();
        self.song.draft_notes.clear();
        self.song.suggested_notes = self.suggestions.clone();
        self.song.mapped = true;
        self.song.tags.retain(|tag| tag != "UNMAPPED");
        if !self.song.tags.iter().any(|tag| tag == "Mapped") {
            self.song.tags.push("Mapped".into());
        }
        self.dirty = false;
        write_song(root, &self.song)
    }

    fn validate(&self) -> Result<(), String> {
        if self.notes.is_empty() {
            return Err("Place at least one vocal note before submitting.".into());
        }
        if !self.word_bank.is_empty() {
            let missing: Vec<&WordToken> = self.word_bank.iter().filter(|w| !w.placed).collect();
            if let Some(first) = missing.first() {
                return Err(format!(
                    "{} word{} still in the bank (next: “{}”). Place every lyric word first.",
                    missing.len(),
                    if missing.len() == 1 { "" } else { "s" },
                    first.text
                ));
            }
        }
        Ok(())
    }

    pub fn draw(&mut self, ui: &mut Ui, audio: &Option<AudioEngine>) -> EditorEvent {
        let dt = ui.input(|i| i.stable_dt).min(0.1) as f64;
        let rect = ui.max_rect();
        let layout = self.layout(rect);
        let mut event = EditorEvent::None;
        let ctx = ui.ctx().clone();

        self.handle_keys(ui, audio, &layout, &mut event);

        // Advance the visible playhead from the audio clock. When no device is
        // available the editor still previews timing on a wall clock.
        if self.playing {
            if self.audio_ready {
                if let Some(audio) = audio {
                    self.playhead = audio.position();
                }
            } else {
                self.playhead += dt * self.speed as f64;
            }
            if self.playhead >= self.duration {
                self.playhead = self.duration;
                self.playing = false;
                if let Some(audio) = audio {
                    audio.pause();
                }
            }
            if self.follow {
                let span = (self.note_area_width(&layout) / self.zoom) as f64;
                let span = span.max(1.0);
                if self.playhead > self.scroll + span * 0.82 {
                    self.scroll = (self.playhead - span * 0.18).max(0.0);
                } else if self.playhead < self.scroll {
                    self.scroll = (self.playhead - span * 0.08).max(0.0);
                }
            }
        }

        // The note lane gets first crack at the pointer so the wheel can scroll
        // the timeline without fighting nested scroll areas.
        let canvas_response = ui.interact(
            layout.note_area,
            Id::new("editor_note_canvas"),
            Sense::click_and_drag(),
        );
        self.handle_navigation(ui, &layout, &canvas_response);
        self.handle_canvas(ui, &layout, &canvas_response, audio, &mut event);

        // ---- paint -----------------------------------------------------
        let p = ui.painter().clone();
        p.rect_filled(rect, 0.0, BG);
        self.draw_header(ui, &p, &layout, &mut event);
        self.draw_toolbar(ui, &p, &layout);
        self.draw_lanes(ui, &p, &layout);
        self.draw_ruler(ui, &p, &layout, &mut event, audio);
        self.draw_transport(ui, &p, &layout, audio);
        self.draw_word_bank(ui, &p, &layout, audio);
        self.draw_inspector(ui, &p, &layout, &mut event);

        if self.playing || self.drag.is_some() || self.word_drag.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        event
    }

    fn layout(&self, rect: Rect) -> Layout {
        let header_h = 60.0;
        let toolbar_h = 44.0;
        let bank_h = 112.0;
        let transport_h = 50.0;
        let ruler_h = 26.0;
        let inspector_w = 216.0;
        let rail_w = 58.0;
        let bank = Rect::from_min_max(
            Pos2::new(rect.left(), rect.bottom() - bank_h),
            rect.right_bottom(),
        );
        let transport = Rect::from_min_max(
            Pos2::new(rect.left(), bank.top() - transport_h),
            Pos2::new(rect.right(), bank.top()),
        );
        let ruler = Rect::from_min_max(
            Pos2::new(rect.left() + rail_w, transport.top() - ruler_h),
            Pos2::new(rect.right() - inspector_w, transport.top()),
        );
        let note_area = Rect::from_min_max(
            Pos2::new(rect.left() + rail_w, rect.top() + header_h + toolbar_h),
            Pos2::new(rect.right() - inspector_w, transport.top() - ruler_h),
        );
        let rail = Rect::from_min_max(
            Pos2::new(rect.left(), note_area.top()),
            Pos2::new(note_area.left(), note_area.bottom()),
        );
        let inspector = Rect::from_min_max(
            Pos2::new(rect.right() - inspector_w + 8.0, note_area.top() + 6.0),
            Pos2::new(rect.right() - 8.0, transport.top() - 8.0),
        );
        Layout {
            note_area,
            rail,
            inspector,
            ruler,
            transport,
            bank,
        }
    }

    fn note_area_width(&self, layout: &Layout) -> f32 {
        layout.note_area.width().max(100.0)
    }

    fn handle_keys(
        &mut self,
        ui: &mut Ui,
        audio: &Option<AudioEngine>,
        layout: &Layout,
        event: &mut EditorEvent,
    ) {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            *event = EditorEvent::Back;
        }
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
            if ui.input(|i| i.modifiers.shift) {
                self.redo();
            } else {
                self.undo();
            }
        }
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Y)) {
            self.redo();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_play(audio);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
            self.delete_selected();
        }
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)) {
            if self.validate().is_ok() {
                *event = EditorEvent::Submit;
            } else if let Err(error) = self.validate() {
                self.status = error;
            }
        }
        for (key, tool) in [
            (egui::Key::Num1, EditorTool::Pan),
            (egui::Key::Num2, EditorTool::Note),
            (egui::Key::Num3, EditorTool::Ribbon),
            (egui::Key::Num4, EditorTool::Eraser),
            (egui::Key::Num5, EditorTool::Select),
        ] {
            if ui.input(|i| i.key_pressed(key)) {
                self.tool = tool;
                self.status = tool.hint().to_string();
            }
        }
        let (left, right, shift) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.modifiers.shift,
            )
        });
        if left || right {
            let step = if shift { 5.0 } else { 1.0 };
            let target = if left {
                (self.playhead - step).max(0.0)
            } else {
                (self.playhead + step).min(self.duration)
            };
            self.seek(target, audio);
            let span = (self.note_area_width(layout) / self.zoom) as f64;
            if target < self.scroll || target > self.scroll + span {
                self.scroll = (target - span * 0.25).max(0.0);
            }
        }
    }

    fn handle_navigation(
        &mut self,
        ui: &mut Ui,
        layout: &Layout,
        response: &egui::Response,
    ) {
        if response.hovered() {
            let (scroll, modifiers) = ui.input(|i| (i.raw_scroll_delta, i.modifiers));
            if modifiers.command || modifiers.ctrl {
                if scroll.y.abs() > 0.0 {
                    let anchor = ui
                        .input(|i| i.pointer.hover_pos())
                        .unwrap_or_else(|| layout.note_area.center());
                    let anchor_time = self.x_to_time(anchor.x, layout);
                    self.zoom = (self.zoom * (1.0 + scroll.y * 0.0015)).clamp(24.0, 420.0);
                    self.scroll = (anchor_time - (anchor.x - layout.note_area.left()) as f64 / self.zoom as f64)
                        .max(0.0);
                }
            } else {
                let dx = scroll.x + scroll.y;
                if dx.abs() > 0.0 {
                    self.scroll = (self.scroll - dx as f64 / self.zoom as f64).max(0.0);
                }
            }
        }

        // Middle-button horizontal panning works from every tool, including
        // while a word is being dragged.
        let (middle_down, pointer, delta) = ui.input(|i| {
            (
                i.pointer.button_down(PointerButton::Middle),
                i.pointer.hover_pos(),
                i.pointer.delta(),
            )
        });
        if middle_down && pointer.map(|pos| layout.note_area.contains(pos)).unwrap_or(false) {
            if self.drag.is_none() {
                self.scroll = (self.scroll - delta.x as f64 / self.zoom as f64).max(0.0);
            }
        }

        if self.tool == EditorTool::Pan && response.dragged_by(PointerButton::Primary) {
            self.scroll = (self.scroll - delta.x as f64 / self.zoom as f64).max(0.0);
        }
    }

    fn handle_canvas(
        &mut self,
        ui: &mut Ui,
        layout: &Layout,
        response: &egui::Response,
        audio: &Option<AudioEngine>,
        _event: &mut EditorEvent,
    ) {
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let Some(pos) = pointer else {
            return;
        };

        // A word clicked in the bank can be placed with the next note click.
        if response.clicked_by(PointerButton::Primary) {
            if let Some(token) = self.pending_word.take() {
                if let Some(note) = self.hit_note(pos, layout) {
                    self.assign_word(token, vec![note], None, layout, audio);
                } else if layout.note_area.contains(pos) {
                    self.assign_word(token, Vec::new(), Some(pos), layout, audio);
                } else {
                    self.pending_word = Some(token);
                }
                return;
            }
        }

        match self.tool {
            EditorTool::Pan => {}
            EditorTool::Note => {
                if response.drag_started_by(PointerButton::Primary) && layout.note_area.contains(pos) {
                    self.push_history();
                    let midi = self.midi_at_y(pos.y, layout);
                    let start = self.snap_time(self.x_to_time(pos.x, layout));
                    let index = self.notes.len();
                    self.notes.push(Note {
                        start,
                        dur: 0.25,
                        midi,
                        word: None,
                        word_index: None,
                        line: None,
                        vel: None,
                    });
                    self.selected.clear();
                    self.selected.insert(index);
                    self.drag = Some(Drag::NewNote { index, start });
                    self.dirty = true;
                } else if response.clicked_by(PointerButton::Primary) && layout.note_area.contains(pos) {
                    self.push_history();
                    let midi = self.midi_at_y(pos.y, layout);
                    let start = self.snap_time(self.x_to_time(pos.x, layout));
                    self.notes.push(Note {
                        start,
                        dur: 0.28,
                        midi,
                        word: None,
                        word_index: None,
                        line: None,
                        vel: None,
                    });
                    self.selected.clear();
                    self.selected.insert(self.notes.len() - 1);
                    self.dirty = true;
                    self.status = "Note placed. Drag with NOTE for a longer syllable.".into();
                }
            }
            EditorTool::Select => {
                if response.drag_started_by(PointerButton::Primary) {
                    if let Some(index) = self.hit_note(pos, layout) {
                        self.selected.clear();
                        self.selected.insert(index);
                        let (note_start, note_dur, note_midi) = {
                            let note = &self.notes[index];
                            (note.start, note.dur, note.midi)
                        };
                        let x0 = self.time_to_x(note_start, layout);
                        let x1 = self.time_to_x(note_start + note_dur, layout);
                        self.push_history();
                        self.drag = if (pos.x - x0).abs() <= 8.0 {
                            Some(Drag::ResizeLeft {
                                index,
                                end: note_start + note_dur,
                            })
                        } else if (pos.x - x1).abs() <= 8.0 {
                            Some(Drag::ResizeRight {
                                index,
                                start: note_start,
                            })
                        } else {
                            Some(Drag::Move {
                                index,
                                grab: self.x_to_time(pos.x, layout) - note_start,
                                midi_offset: self.midi_at_y(pos.y, layout) - note_midi,
                            })
                        };
                    }
                } else if response.clicked_by(PointerButton::Primary) {
                    if let Some(index) = self.hit_note(pos, layout) {
                        if ui.input(|i| i.modifiers.shift) {
                            if !self.selected.insert(index) {
                                self.selected.remove(&index);
                            }
                        } else {
                            self.selected.clear();
                            self.selected.insert(index);
                        }
                    } else {
                        self.selected.clear();
                    }
                }
            }
            EditorTool::Ribbon => {
                if response.drag_started_by(PointerButton::Primary) {
                    if let Some(index) = self.hit_note(pos, layout) {
                        self.selected.clear();
                        self.selected.insert(index);
                        self.drag = Some(Drag::Ribbon {
                            anchor_start: self.notes[index].start,
                        });
                        self.status = "Ribbon selection — release over the last note, then drop a word.".into();
                    }
                } else if response.clicked_by(PointerButton::Primary) {
                    if let Some(index) = self.hit_note(pos, layout) {
                        if !ui.input(|i| i.modifiers.shift) {
                            self.selected.clear();
                        }
                        if !self.selected.insert(index) {
                            self.selected.remove(&index);
                        }
                    } else {
                        self.selected.clear();
                    }
                }
            }
            EditorTool::Eraser => {
                if response.drag_started_by(PointerButton::Primary) {
                    if self.hit_note(pos, layout).is_some() {
                        self.push_history();
                        self.drag = Some(Drag::Erase);
                        self.erase_at(pos, layout);
                    }
                } else if response.clicked_by(PointerButton::Primary) {
                    if self.hit_note(pos, layout).is_some() {
                        self.push_history();
                        self.erase_at(pos, layout);
                    }
                }
            }
        }

        // Continue an active drag with the current pointer position.
        match self.drag {
            Some(Drag::NewNote { index, start }) => {
                if response.dragged_by(PointerButton::Primary) {
                    let end = self.snap_time(self.x_to_time(pos.x, layout)).max(start + 0.06);
                    if let Some(note) = self.notes.get_mut(index) {
                        note.dur = (end - start).clamp(0.06, 30.0);
                    }
                } else if response.drag_stopped_by(PointerButton::Primary) {
                    if let Some(note) = self.notes.get_mut(index) {
                        note.dur = note.dur.max(0.28);
                    }
                    self.drag = None;
                    self.sort_notes();
                    self.status = "Note placed. Add a word from the bank to make it singable.".into();
                }
            }
            Some(Drag::Move {
                index,
                grab,
                midi_offset,
            }) => {
                if response.dragged_by(PointerButton::Primary) {
                    let start = self.snap_time(self.x_to_time(pos.x, layout) - grab);
                    let midi = (self.midi_at_y(pos.y, layout) - midi_offset).clamp(0, 127);
                    if let Some(note) = self.notes.get_mut(index) {
                        note.start = start.max(0.0);
                        note.midi = midi;
                    }
                } else if response.drag_stopped_by(PointerButton::Primary) {
                    self.drag = None;
                    self.sort_notes();
                }
            }
            Some(Drag::ResizeLeft { index, end }) => {
                if response.dragged_by(PointerButton::Primary) {
                    let start = self.snap_time(self.x_to_time(pos.x, layout));
                    if let Some(note) = self.notes.get_mut(index) {
                        let new_start = start.min(end - 0.06).max(0.0);
                        note.start = new_start;
                        note.dur = end - new_start;
                    }
                } else if response.drag_stopped_by(PointerButton::Primary) {
                    self.drag = None;
                    self.sort_notes();
                }
            }
            Some(Drag::ResizeRight { index, start }) => {
                if response.dragged_by(PointerButton::Primary) {
                    let end = self.snap_time(self.x_to_time(pos.x, layout));
                    if let Some(note) = self.notes.get_mut(index) {
                        note.dur = (end - start).clamp(0.06, 30.0);
                    }
                } else if response.drag_stopped_by(PointerButton::Primary) {
                    self.drag = None;
                    self.sort_notes();
                }
            }
            Some(Drag::Ribbon { anchor_start }) => {
                if response.dragged_by(PointerButton::Primary) {
                    let end = self.x_to_time(pos.x, layout);
                    let (lo, hi) = if anchor_start <= end {
                        (anchor_start, end)
                    } else {
                        (end, anchor_start)
                    };
                    self.selected.clear();
                    for (i, note) in self.notes.iter().enumerate() {
                        if note.start >= lo - 0.02 && note.start <= hi + 0.02 {
                            self.selected.insert(i);
                        }
                    }
                } else if response.drag_stopped_by(PointerButton::Primary) {
                    self.drag = None;
                    if self.selected.len() > 1 {
                        self.status = format!(
                            "{} notes selected — drop a lyric word on any of them.",
                            self.selected.len()
                        );
                    }
                }
            }
            Some(Drag::Erase) => {
                if response.dragged_by(PointerButton::Primary) {
                    self.erase_at(pos, layout);
                } else if response.drag_stopped_by(PointerButton::Primary) {
                    self.drag = None;
                }
            }
            None => {}
        }
    }

    fn draw_header(&self, ui: &mut Ui, p: &egui::Painter, layout: &Layout, event: &mut EditorEvent) {
        let rect = ui.max_rect();
        let header = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + 60.0));
        p.rect_filled(header, 0.0, Color32::from_rgba_unmultiplied(5, 8, 15, 248));
        p.line_segment(
            [header.left_bottom(), header.right_bottom()],
            Stroke::new(1.0f32, Color32::from_white_alpha(24)),
        );
        if paint_button(ui, Rect::from_min_size(Pos2::new(16.0, 14.0), Vec2::new(82.0, 32.0)), "← LIBRARY", false, true) {
            *event = EditorEvent::Back;
        }
        p.text(
            Pos2::new(116.0, 13.0),
            Align2::LEFT_TOP,
            "KARAOKEMACHINE · MANUAL LEVEL EDITOR",
            FontId::monospace(8.0),
            MUTED2,
        );
        p.text(
            Pos2::new(116.0, 26.0),
            Align2::LEFT_TOP,
            &self.song.title,
            FontId::proportional(20.0),
            INK,
        );
        p.text(
            Pos2::new(116.0, 48.0),
            Align2::LEFT_TOP,
            format!(
                "{}   ·   {}   ·   {} notes",
                self.song.artist_display(),
                self.song.album_display(),
                self.notes.len()
            ),
            FontId::monospace(7.0),
            MUTED,
        );
        let badge = Rect::from_min_size(Pos2::new(rect.right() - 390.0, 17.0), Vec2::new(96.0, 26.0));
        p.rect_filled(badge, 13.0, Color32::from_rgba_unmultiplied(255, 93, 158, 22));
        p.rect_stroke(
            badge,
            13.0,
            Stroke::new(1.0f32, Color32::from_rgba_unmultiplied(255, 93, 158, 90)),
            StrokeKind::Inside,
        );
        p.text(
            badge.center(),
            Align2::CENTER_CENTER,
            if self.song.mapped { "MAPPED" } else { "UNMAPPED" },
            FontId::monospace(8.0),
            if self.song.mapped { MINT } else { PINK },
        );
        let progress = self.word_progress();
        let progress_rect = Rect::from_min_size(Pos2::new(rect.right() - 284.0, 21.0), Vec2::new(124.0, 18.0));
        p.rect_filled(progress_rect, 9.0, Color32::from_white_alpha(8));
        let fill = Rect::from_min_size(
            progress_rect.min,
            Vec2::new(progress_rect.width() * progress, progress_rect.height()),
        );
        p.rect_filled(fill, 9.0, alpha(MINT, 150));
        p.text(
            progress_rect.center(),
            Align2::CENTER_CENTER,
            format!("{}% WORDS", (progress * 100.0).round() as i32),
            FontId::monospace(7.0),
            INK,
        );
        let submit_rect = Rect::from_min_size(Pos2::new(rect.right() - 148.0, 12.0), Vec2::new(132.0, 36.0));
        let ok = self.validate().is_ok();
        if paint_button(ui, submit_rect, "SUBMIT MAPPING", ok, ok) {
            if ok {
                *event = EditorEvent::Submit;
            }
        }
        let _ = layout;
    }

    fn draw_toolbar(&mut self, ui: &mut Ui, p: &egui::Painter, layout: &Layout) {
        let rect = ui.max_rect();
        let bar = Rect::from_min_max(
            Pos2::new(rect.left(), rect.top() + 60.0),
            Pos2::new(rect.right(), layout.note_area.top()),
        );
        p.rect_filled(bar, 0.0, PANEL);
        p.line_segment(
            [bar.left_bottom(), bar.right_bottom()],
            Stroke::new(1.0f32, Color32::from_white_alpha(18)),
        );
        let mut x = 16.0;
        for tool in [
            EditorTool::Pan,
            EditorTool::Note,
            EditorTool::Ribbon,
            EditorTool::Eraser,
            EditorTool::Select,
        ] {
            let w = 72.0;
            let r = Rect::from_min_size(Pos2::new(x, bar.top() + 6.0), Vec2::new(w, 32.0));
            if paint_toggle(ui, r, tool.label(), self.tool == tool) {
                self.tool = tool;
                self.status = tool.hint().to_string();
            }
            x += w + 6.0;
        }
        let hint = Rect::from_min_size(Pos2::new(x + 10.0, bar.top() + 10.0), Vec2::new(300.0, 24.0));
        p.text(
            hint.left_center(),
            Align2::LEFT_CENTER,
            self.tool.hint(),
            FontId::monospace(7.0),
            MUTED2,
        );
        let snap = Rect::from_min_size(Pos2::new(bar.right() - 430.0, bar.top() + 6.0), Vec2::new(88.0, 32.0));
        if paint_toggle(ui, snap, if self.snap { "SNAP ON" } else { "SNAP OFF" }, self.snap) {
            self.snap = !self.snap;
        }
        let sugg = Rect::from_min_size(Pos2::new(bar.right() - 334.0, bar.top() + 6.0), Vec2::new(104.0, 32.0));
        if paint_toggle(ui, sugg, "SUGGESTIONS", self.show_suggestions) {
            self.show_suggestions = !self.show_suggestions;
        }
        let follow = Rect::from_min_size(Pos2::new(bar.right() - 222.0, bar.top() + 6.0), Vec2::new(80.0, 32.0));
        if paint_toggle(ui, follow, "FOLLOW", self.follow) {
            self.follow = !self.follow;
        }
        let zoom_out = Rect::from_min_size(Pos2::new(bar.right() - 132.0, bar.top() + 6.0), Vec2::new(28.0, 32.0));
        let zoom_in = Rect::from_min_size(Pos2::new(bar.right() - 98.0, bar.top() + 6.0), Vec2::new(28.0, 32.0));
        if paint_button(ui, zoom_out, "−", false, true) {
            self.zoom = (self.zoom / 1.25).clamp(24.0, 420.0);
        }
        if paint_button(ui, zoom_in, "+", false, true) {
            self.zoom = (self.zoom * 1.25).clamp(24.0, 420.0);
        }
        p.text(
            Pos2::new(bar.right() - 62.0, bar.center().y),
            Align2::LEFT_CENTER,
            format!("{:.0}px/s", self.zoom),
            FontId::monospace(7.0),
            MUTED2,
        );
    }

    fn draw_lanes(&self, ui: &mut Ui, p: &egui::Painter, layout: &Layout) {
        let area = layout.note_area;
        let clip = Rect::from_min_max(
            Pos2::new(layout.rail.left(), area.top()),
            Pos2::new(area.right(), area.bottom()),
        );
        let painter = ui.painter().with_clip_rect(clip);
        painter.rect_filled(area, 0.0, Color32::from_rgb(6, 9, 17));
        let lane_h = area.height() / (self.high - self.low + 1) as f32;
        for midi in self.low..=self.high {
            let top = area.top() + (self.high - midi) as f32 * lane_h;
            let black = [1, 3, 6, 8, 10].contains(&(((midi % 12) + 12) % 12));
            if black {
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(area.left(), top), Vec2::new(area.width(), lane_h)),
                    0.0,
                    Color32::from_white_alpha(8),
                );
            }
            if midi % 12 == 0 {
                painter.line_segment(
                    [Pos2::new(area.left(), top + lane_h), Pos2::new(area.right(), top + lane_h)],
                    Stroke::new(1.0f32, Color32::from_white_alpha(34)),
                );
            }
        }

        // Time grid. Beat lines follow the BPM when the import found one.
        let interval = self.song.bpm.map(|bpm| 60.0 / bpm).unwrap_or(1.0);
        let first = (self.scroll / interval).floor() as i64 - 1;
        let last = ((self.scroll + (area.width() / self.zoom) as f64) / interval).ceil() as i64 + 1;
        for tick in first..=last {
            let x = self.time_to_x(tick as f64 * interval, layout);
            if x < area.left() || x > area.right() {
                continue;
            }
            let bar = self.song.bpm.map(|_| tick % 4 == 0).unwrap_or(false);
            painter.line_segment(
                [Pos2::new(x, area.top()), Pos2::new(x, area.bottom())],
                Stroke::new(if bar { 1.1f32 } else { 0.6f32 }, Color32::from_white_alpha(if bar { 22 } else { 9 })),
            );
        }

        // Suggested chart: a ghost of the old automatic melody.
        if self.show_suggestions {
            let h = (lane_h * 0.62).clamp(4.0, 18.0);
            for note in &self.suggestions {
                let x0 = self.time_to_x(note.start, layout);
                let x1 = self.time_to_x(note.start + note.dur, layout);
                if x1 < area.left() || x0 > area.right() {
                    continue;
                }
                let y = self.midi_to_y(note.midi, layout);
                let t = ((y - area.top()) / area.height().max(1.0)).clamp(0.0, 1.0);
                let c = pitch_color(t);
                let color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 34);
                let outline = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 64);
                let r = Rect::from_min_max(Pos2::new(x0, y - h / 2.0), Pos2::new(x1.max(x0 + 2.0), y + h / 2.0));
                painter.rect_filled(r, h / 2.0, color);
                painter.rect_stroke(r, h / 2.0, Stroke::new(0.8f32, outline), StrokeKind::Inside);
            }
        }

        // Mapped notes, including the thin neck that makes a word one ribbon.
        let h = (lane_h * 0.62).clamp(6.0, 22.0);
        let mut previous: Option<(usize, f32, f32)> = None;
        for (i, note) in self.notes.iter().enumerate() {
            let x0 = self.time_to_x(note.start, layout);
            let x1 = self.time_to_x(note.start + note.dur, layout);
            if x1 < area.left() - 4.0 || x0 > area.right() + 4.0 {
                previous = None;
                continue;
            }
            let y = self.midi_to_y(note.midi, layout);
            let t = ((y - area.top()) / area.height().max(1.0)).clamp(0.0, 1.0);
            let c = pitch_color(t);
            let color = Color32::from_rgb(c[0], c[1], c[2]);
            if let Some((prev_i, prev_x1, prev_y)) = previous {
                if same_word(&self.notes[prev_i], note) && note.start - (self.notes[prev_i].start + self.notes[prev_i].dur) <= 3.0 {
                    painter.line_segment(
                        [Pos2::new(prev_x1, prev_y), Pos2::new(x0, y)],
                        Stroke::new((h * 0.34).max(2.0), alpha(color, 150)),
                    );
                }
            }
            let r = Rect::from_min_max(
                Pos2::new(x0.max(area.left()), y - h / 2.0),
                Pos2::new(x1.max(x0 + 2.0).min(area.right()), y + h / 2.0),
            );
            if r.width() > 0.5 {
                painter.rect_filled(r, h / 2.0, color);
                let selected = self.selected.contains(&i);
                painter.rect_stroke(
                    r,
                    h / 2.0,
                    Stroke::new(if selected { 1.8f32 } else { 0.7f32 }, if selected { Color32::WHITE } else { Color32::from_white_alpha(50) }),
                    StrokeKind::Inside,
                );
                if selected && r.width() > 18.0 {
                    for hx in [r.left() + 3.0, r.right() - 3.0] {
                        painter.line_segment(
                            [Pos2::new(hx, r.top() + 2.0), Pos2::new(hx, r.bottom() - 2.0)],
                            Stroke::new(2.0f32, Color32::from_white_alpha(220)),
                        );
                    }
                }
                if let Some(word) = &note.word {
                    if !is_word_continuation(&self.notes, i) {
                        let text_w = word.chars().count() as f32 * 6.4;
                        if r.width() > text_w + 6.0 && h >= 13.0 {
                            painter.text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                word,
                                FontId::proportional((h * 0.58).clamp(8.0, 12.0)),
                                Color32::from_rgb(6, 12, 21),
                            );
                        } else {
                            painter.text(
                                Pos2::new(r.center().x, r.top() - 2.0),
                                Align2::CENTER_BOTTOM,
                                word,
                                FontId::proportional(9.0),
                                alpha(INK, 190),
                            );
                        }
                    }
                }
            }
            previous = Some((i, x1, y));
        }

        // Playhead.
        let px = self.time_to_x(self.playhead, layout);
        if px >= area.left() && px <= area.right() {
            painter.line_segment(
                [Pos2::new(px, area.top()), Pos2::new(px, area.bottom())],
                Stroke::new(1.4f32, alpha(GOLD, 220)),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(px - 5.0, area.top()),
                    Pos2::new(px + 5.0, area.top()),
                    Pos2::new(px, area.top() + 9.0),
                ],
                GOLD,
                Stroke::NONE,
            ));
        }

        // Piano rail.
        p.rect_filled(layout.rail, 0.0, Color32::from_rgb(7, 10, 18));
        p.line_segment(
            [layout.rail.right_top(), layout.rail.right_bottom()],
            Stroke::new(1.0f32, alpha(MINT, 65)),
        );
        for midi in self.low..=self.high {
            let y = self.midi_to_y(midi, layout);
            let black = [1, 3, 6, 8, 10].contains(&(((midi % 12) + 12) % 12));
            if black {
                p.rect_filled(
                    Rect::from_min_size(Pos2::new(layout.rail.left(), y - lane_h / 2.0), Vec2::new(layout.rail.width(), lane_h)),
                    0.0,
                    Color32::from_white_alpha(10),
                );
            }
            if midi % 12 == 0 || lane_h >= 16.0 {
                p.text(
                    Pos2::new(layout.rail.right() - 8.0, y),
                    Align2::RIGHT_CENTER,
                    note_name(midi),
                    FontId::monospace((lane_h * 0.42).clamp(6.5, 9.5)),
                    if midi % 12 == 0 { alpha(INK, 135) } else { alpha(INK, 55) },
                );
            }
        }
        p.text(
            Pos2::new(layout.rail.left() + 7.0, layout.rail.top() + 5.0),
            Align2::LEFT_TOP,
            "PITCH",
            FontId::monospace(6.0),
            MUTED2,
        );
    }

    fn draw_ruler(
        &mut self,
        ui: &mut Ui,
        p: &egui::Painter,
        layout: &Layout,
        event: &mut EditorEvent,
        audio: &Option<AudioEngine>,
    ) {
        let ruler = layout.ruler;
        p.rect_filled(ruler, 0.0, Color32::from_rgb(8, 12, 21));
        p.line_segment([ruler.left_top(), ruler.right_top()], Stroke::new(1.0f32, Color32::from_white_alpha(16)));
        let visible = (ruler.width() / self.zoom) as f64;
        let candidates = [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0];
        let step = candidates
            .iter()
            .copied()
            .find(|step| step * self.zoom as f64 >= 66.0)
            .unwrap_or(120.0);
        let first = (self.scroll / step).floor() as i64;
        let last = ((self.scroll + visible) / step).ceil() as i64 + 1;
        for tick in first..=last {
            let time = tick as f64 * step;
            let x = self.time_to_x(time, layout);
            if x < ruler.left() || x > ruler.right() {
                continue;
            }
            p.line_segment([Pos2::new(x, ruler.top()), Pos2::new(x, ruler.top() + 7.0)], Stroke::new(1.0f32, Color32::from_white_alpha(35)));
            p.text(
                Pos2::new(x + 5.0, ruler.center().y + 1.0),
                Align2::LEFT_CENTER,
                format_time(time),
                FontId::monospace(7.0),
                MUTED2,
            );
        }
        let px = self.time_to_x(self.playhead, layout);
        if px >= ruler.left() && px <= ruler.right() {
            p.line_segment(
                [Pos2::new(px, ruler.top() + 12.0), Pos2::new(px, ruler.bottom())],
                Stroke::new(2.0f32, GOLD),
            );
        }
        let response = ui.interact(ruler, Id::new("editor_ruler"), Sense::click_and_drag());
        if (response.clicked() || response.dragged()) && ui.input(|i| i.pointer.primary_down()) {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                let t = self.x_to_time(pos.x.clamp(ruler.left(), ruler.right()), layout).clamp(0.0, self.duration);
                self.seek(t, audio);
                if self.playing {
                    if let Some(audio) = audio {
                        audio.play();
                    }
                }
            }
        }
        let _ = event;
    }

    fn draw_transport(
        &mut self,
        ui: &mut Ui,
        p: &egui::Painter,
        layout: &Layout,
        audio: &Option<AudioEngine>,
    ) {
        let rect = ui.max_rect();
        let tray = layout.transport;
        p.rect_filled(tray, 0.0, PANEL);
        p.line_segment([tray.left_top(), tray.right_top()], Stroke::new(1.0f32, Color32::from_white_alpha(18)));
        let play = Rect::from_min_size(Pos2::new(tray.left() + 16.0, tray.top() + 8.0), Vec2::new(86.0, 34.0));
        if paint_button(ui, play, if self.playing { "PAUSE" } else { "PLAY" }, true, true) {
            self.toggle_play(audio);
        }
        let stop = Rect::from_min_size(Pos2::new(play.right() + 7.0, play.top()), Vec2::new(62.0, 34.0));
        if paint_button(ui, stop, "STOP", false, true) {
            self.playing = false;
            if let Some(audio) = audio {
                audio.pause();
            }
            self.seek(0.0, audio);
        }
        p.text(
            Pos2::new(stop.right() + 18.0, tray.center().y),
            Align2::LEFT_CENTER,
            format!("{} / {}", format_time(self.playhead), format_time(self.duration)),
            FontId::monospace(11.0),
            INK,
        );
        p.text(
            Pos2::new(stop.right() + 150.0, tray.center().y - 8.0),
            Align2::LEFT_CENTER,
            "SPEED",
            FontId::monospace(6.0),
            MUTED2,
        );
        let mut x = stop.right() + 146.0;
        for speed in [0.5f32, 0.75, 1.0, 1.25, 1.5] {
            let r = Rect::from_min_size(Pos2::new(x, tray.top() + 8.0), Vec2::new(44.0, 34.0));
            if paint_toggle(ui, r, &format!("{speed:.2}×").replace("1.00×", "1×").replace("0.50×", "0.5×").replace("1.50×", "1.5×"), (self.speed - speed).abs() < 0.01) {
                self.speed = speed;
                if let Some(audio) = audio {
                    audio.set_rate(speed);
                }
            }
            x += 48.0;
        }
        let vocals = Rect::from_min_size(Pos2::new(tray.right() - 290.0, tray.top() + 8.0), Vec2::new(94.0, 34.0));
        if paint_toggle(ui, vocals, if self.vocals_on { "VOCALS ON" } else { "VOCALS OFF" }, self.vocals_on) {
            self.vocals_on = !self.vocals_on;
            self.apply_editor_volumes(audio);
        }
        let backing = Rect::from_min_size(Pos2::new(vocals.right() + 7.0, vocals.top()), Vec2::new(100.0, 34.0));
        if paint_toggle(ui, backing, if self.backing_on { "BACKING ON" } else { "BACKING OFF" }, self.backing_on) {
            self.backing_on = !self.backing_on;
            self.apply_editor_volumes(audio);
        }
        let hint = Rect::from_min_size(Pos2::new(tray.right() - 430.0, tray.top() + 8.0), Vec2::new(124.0, 34.0));
        p.text(
            hint.center(),
            Align2::CENTER_CENTER,
            if self.audio_ready { "AUDIO READY" } else { "CHART PREVIEW" },
            FontId::monospace(7.0),
            if self.audio_ready { MINT } else { GOLD },
        );
        let _ = rect;
    }

    fn draw_word_bank(&mut self, ui: &mut Ui, p: &egui::Painter, layout: &Layout, audio: &Option<AudioEngine>) {
        let bank = layout.bank;
        p.rect_filled(bank, 0.0, Color32::from_rgb(8, 12, 21));
        p.line_segment([bank.left_top(), bank.right_top()], Stroke::new(1.0f32, Color32::from_white_alpha(20)));
        let remaining = self.word_bank.iter().filter(|word| !word.placed).count();
        p.text(
            Pos2::new(bank.left() + 16.0, bank.top() + 10.0),
            Align2::LEFT_TOP,
            "WORD BANK",
            FontId::monospace(8.0),
            MUTED2,
        );
        p.text(
            Pos2::new(bank.left() + 94.0, bank.top() + 10.0),
            Align2::LEFT_TOP,
            format!("{remaining} remaining · drag a word onto a note, or click it and then click a note"),
            FontId::proportional(10.0),
            MUTED,
        );
        if self.word_bank.is_empty() {
            p.text(
                Pos2::new(bank.left() + 16.0, bank.top() + 38.0),
                Align2::LEFT_TOP,
                "No lyric words were found. This is an instrumental mapping — draw the vocal melody and submit.",
                FontId::proportional(12.0),
                GOLD,
            );
            return;
        }
        let inner = Rect::from_min_max(
            Pos2::new(bank.left() + 12.0, bank.top() + 32.0),
            Pos2::new(bank.right() - 12.0, bank.bottom() - 8.0),
        );
        let mut chip_actions: Vec<(usize, bool)> = Vec::new();
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("editor_word_bank_scroll")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut last_line = usize::MAX;
                        for (i, token) in self.word_bank.iter().enumerate() {
                            if token.placed {
                                continue;
                            }
                            if token.line != last_line {
                                if last_line != usize::MAX {
                                    let (sep, _) = ui.allocate_exact_size(Vec2::new(8.0, 34.0), Sense::hover());
                                    let painter = ui.painter();
                                    painter.line_segment(
                                        [sep.center_top() + Vec2::new(4.0, 3.0), sep.center_bottom() - Vec2::new(4.0, 3.0)],
                                        Stroke::new(1.0f32, Color32::from_white_alpha(25)),
                                    );
                                }
                                last_line = token.line;
                            }
                            let width = (token.text.chars().count() as f32 * 8.0 + 26.0).clamp(48.0, 150.0);
                            let (chip, response) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::click_and_drag());
                            let active = self.pending_word == Some(i);
                            let fill = if active { alpha(MINT, 44) } else { PANEL2 };
                            let border = if active { MINT } else { Color32::from_white_alpha(30) };
                            let painter = ui.painter();
                            painter.rect_filled(chip, 9.0, fill);
                            painter.rect_stroke(chip, 9.0, Stroke::new(1.0f32, border), StrokeKind::Inside);
                            painter.text(
                                chip.center(),
                                Align2::CENTER_CENTER,
                                &token.text,
                                FontId::proportional(12.0),
                                if active { MINT } else { INK },
                            );
                            painter.text(
                                chip.right_top() + Vec2::new(-6.0, 3.0),
                                Align2::RIGHT_TOP,
                                format!("{}", token.line + 1),
                                FontId::monospace(5.5),
                                MUTED2,
                            );
                            if response.drag_started() {
                                chip_actions.push((i, true));
                            }
                            if response.clicked() {
                                chip_actions.push((i, false));
                            }
                        }
                    });
                });
        });
        for (token, dragged) in chip_actions {
            if dragged {
                self.word_drag = Some(token);
                self.pending_word = None;
                self.status = "Drag the word onto a note (or a ribbon selection).".into();
            } else if self.selected.is_empty() {
                self.pending_word = if self.pending_word == Some(token) { None } else { Some(token) };
                self.status = if self.pending_word.is_some() {
                    "Word armed — click a note to place it.".into()
                } else {
                    "Word selection cleared.".into()
                };
            } else {
                let targets: Vec<usize> = self.selected.iter().copied().collect();
                self.assign_word(token, targets, None, layout, audio);
            }
        }

        // Ghost follows the pointer during a bank drag. The bank interaction
        // owns the drag, so this paints above the canvas on the tooltip layer.
        if let Some(token) = self.word_drag {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                let ghost = ui.ctx().layer_painter(LayerId::new(Order::Tooltip, Id::new("editor_word_ghost")));
                let text = &self.word_bank[token].text;
                let width = (text.chars().count() as f32 * 8.0 + 30.0).clamp(54.0, 160.0);
                let rect = Rect::from_center_size(pos + Vec2::new(0.0, -18.0), Vec2::new(width, 32.0));
                ghost.rect_filled(rect, 9.0, Color32::from_rgba_unmultiplied(124, 247, 196, 235));
                ghost.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.0), Color32::from_rgb(5, 18, 13));
                if layout.note_area.contains(pos) {
                    ghost.rect_stroke(layout.note_area, 0.0, Stroke::new(1.0f32, alpha(MINT, 65)), StrokeKind::Inside);
                }
            }
            if ui.input(|i| !i.pointer.any_down()) {
                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                    if layout.note_area.contains(pos) {
                        if let Some(note) = self.hit_note(pos, layout) {
                            let targets = if self.selected.contains(&note) && self.selected.len() > 1 {
                                self.selected.iter().copied().collect()
                            } else {
                                vec![note]
                            };
                            self.assign_word(token, targets, None, layout, audio);
                        } else {
                            self.assign_word(token, Vec::new(), Some(pos), layout, audio);
                        }
                    } else {
                        self.status = "Word returned to the bank.".into();
                    }
                }
                self.word_drag = None;
            }
        }
    }

    fn draw_inspector(
        &mut self,
        ui: &mut Ui,
        p: &egui::Painter,
        layout: &Layout,
        event: &mut EditorEvent,
    ) {
        let r = layout.inspector;
        p.rect_filled(r, 12.0, PANEL);
        p.rect_stroke(r, 12.0, Stroke::new(1.0f32, Color32::from_white_alpha(20)), StrokeKind::Inside);
        let mut y = r.top() + 14.0;
        let x = r.left() + 14.0;
        p.text(Pos2::new(x, y), Align2::LEFT_TOP, "MAPPING", FontId::monospace(7.0), MUTED2);
        y += 18.0;
        let total = self.word_bank.len();
        let placed = total.saturating_sub(self.word_bank.iter().filter(|w| !w.placed).count());
        p.text(
            Pos2::new(x, y),
            Align2::LEFT_TOP,
            format!("{} / {} words", placed, total),
            FontId::proportional(16.0),
            INK,
        );
        y += 26.0;
        p.text(
            Pos2::new(x, y),
            Align2::LEFT_TOP,
            format!("{} notes · {} lines", self.notes.len(), self.song.lines.len()),
            FontId::monospace(8.0),
            MUTED,
        );
        y += 24.0;
        p.line_segment([Pos2::new(x, y), Pos2::new(r.right() - 14.0, y)], Stroke::new(1.0f32, Color32::from_white_alpha(18)));
        y += 12.0;

        if !self.selected.is_empty() {
            let count = self.selected.len();
            p.text(
                Pos2::new(x, y),
                Align2::LEFT_TOP,
                if count == 1 { "SELECTED NOTE" } else { "SELECTED NOTES" },
                FontId::monospace(7.0),
                MUTED2,
            );
            y += 16.0;
            if count == 1 {
                let index = *self.selected.iter().next().unwrap();
                if let Some(note) = self.notes.get(index) {
                    p.text(
                        Pos2::new(x, y),
                        Align2::LEFT_TOP,
                        format!("{}  ·  {:.2}s", note_name(note.midi), note.dur),
                        FontId::proportional(20.0),
                        GOLD,
                    );
                    y += 28.0;
                    p.text(
                        Pos2::new(x, y),
                        Align2::LEFT_TOP,
                        format!("at {}", format_time(note.start)),
                        FontId::monospace(8.0),
                        MUTED,
                    );
                    y += 18.0;
                    p.text(
                        Pos2::new(x, y),
                        Align2::LEFT_TOP,
                        note.word.as_deref().unwrap_or("no word"),
                        FontId::proportional(13.0),
                        if note.word.is_some() { MINT } else { MUTED2 },
                    );
                    y += 24.0;
                }
            } else {
                p.text(
                    Pos2::new(x, y),
                    Align2::LEFT_TOP,
                    format!("{count} notes in ribbon selection"),
                    FontId::proportional(13.0),
                    MINT,
                );
                y += 24.0;
            }
            let bw = (r.width() - 38.0) / 2.0;
            for (i, label) in ["− OCT", "+ OCT"].iter().enumerate() {
                let b = Rect::from_min_size(Pos2::new(x + i as f32 * (bw + 10.0), y), Vec2::new(bw, 30.0));
                if paint_button(ui, b, label, false, true) {
                    self.transpose_selected(if i == 0 { -12 } else { 12 });
                }
            }
            y += 38.0;
            let clear = Rect::from_min_size(Pos2::new(x, y), Vec2::new(r.width() - 28.0, 28.0));
            if paint_button(ui, clear, "CLEAR WORD", false, true) {
                self.clear_selected_words();
            }
            y += 36.0;
            let delete = Rect::from_min_size(Pos2::new(x, y), Vec2::new(r.width() - 28.0, 28.0));
            if paint_button(ui, delete, "DELETE NOTE", false, true) {
                self.delete_selected();
            }
            y += 44.0;
        } else {
            p.text(
                Pos2::new(x, y),
                Align2::LEFT_TOP,
                "Select a note or ribbon to edit it.",
                FontId::proportional(11.0),
                MUTED,
            );
            y += 34.0;
        }

        p.line_segment([Pos2::new(x, y), Pos2::new(r.right() - 14.0, y)], Stroke::new(1.0f32, Color32::from_white_alpha(18)));
        y += 12.0;
        p.text(Pos2::new(x, y), Align2::LEFT_TOP, "STATUS", FontId::monospace(7.0), MUTED2);
        y += 16.0;
        let galley = p.layout(
            self.status.clone(),
            FontId::proportional(10.0),
            MUTED,
            r.width() - 28.0,
        );
        let status_h = galley.size().y;
        p.galley(Pos2::new(x, y), galley, MUTED);
        y += status_h + 14.0;
        p.text(Pos2::new(x, y), Align2::LEFT_TOP, "SHORTCUTS", FontId::monospace(7.0), MUTED2);
        y += 16.0;
        for line in [
            "1–5  tools",
            "Space  play / pause",
            "← →  seek 1s (Shift 5s)",
            "Ctrl+Z / Ctrl+Shift+Z  undo",
            "Delete  remove selection",
            "Middle-drag  pan from any tool",
        ] {
            p.text(Pos2::new(x, y), Align2::LEFT_TOP, line, FontId::monospace(7.5), MUTED2);
            y += 15.0;
        }
        let submit = Rect::from_min_size(
            Pos2::new(x, r.bottom() - 52.0),
            Vec2::new(r.width() - 28.0, 38.0),
        );
        let ok = self.validate().is_ok();
        if let Err(reason) = self.validate() {
            let galley = p.layout(
                reason,
                FontId::proportional(9.0),
                PINK,
                r.width() - 28.0,
            );
            let h = galley.size().y;
            p.galley(Pos2::new(x, submit.top() - h - 4.0), galley, PINK);
        } else {
            p.text(
                Pos2::new(x, submit.top() - 16.0),
                Align2::LEFT_BOTTOM,
                "Ready to submit.",
                FontId::proportional(9.0),
                MINT,
            );
        }
        if paint_button(ui, submit, "SUBMIT MAPPING", ok, ok) && ok {
            *event = EditorEvent::Submit;
        }
    }

    fn word_progress(&self) -> f32 {
        if self.word_bank.is_empty() {
            if self.notes.is_empty() { 0.0 } else { 1.0 }
        } else {
            let placed = self.word_bank.iter().filter(|word| word.placed).count() as f32;
            placed / self.word_bank.len() as f32
        }
    }

    fn time_to_x(&self, time: f64, layout: &Layout) -> f32 {
        layout.note_area.left() + ((time - self.scroll) * self.zoom as f64) as f32
    }

    fn x_to_time(&self, x: f32, layout: &Layout) -> f64 {
        self.scroll + (x - layout.note_area.left()) as f64 / self.zoom as f64
    }

    fn midi_to_y(&self, midi: i32, layout: &Layout) -> f32 {
        let lane_h = layout.note_area.height() / (self.high - self.low + 1) as f32;
        layout.note_area.top() + (self.high - midi) as f32 * lane_h + lane_h / 2.0
    }

    fn midi_at_y(&self, y: f32, layout: &Layout) -> i32 {
        let lane_h = layout.note_area.height() / (self.high - self.low + 1) as f32;
        (self.high - ((y - layout.note_area.top()) / lane_h).floor() as i32).clamp(self.low, self.high)
    }

    fn snap_time(&self, time: f64) -> f64 {
        if !self.snap {
            return time.max(0.0);
        }
        let step = self
            .song
            .bpm
            .map(|bpm| 60.0 / bpm / 4.0)
            .unwrap_or(0.05)
            .clamp(0.025, 0.25);
        ((time.max(0.0) / step).round() * step).max(0.0)
    }

    fn hit_note(&self, pos: Pos2, layout: &Layout) -> Option<usize> {
        if !layout.note_area.contains(pos) {
            return None;
        }
        let lane_h = layout.note_area.height() / (self.high - self.low + 1) as f32;
        let h = (lane_h * 0.62).clamp(6.0, 22.0);
        for (i, note) in self.notes.iter().enumerate().rev() {
            let x0 = self.time_to_x(note.start, layout);
            let x1 = self.time_to_x(note.start + note.dur, layout);
            let y = self.midi_to_y(note.midi, layout);
            if pos.x >= x0 - 3.0 && pos.x <= x1 + 3.0 && (pos.y - y).abs() <= h / 2.0 + 3.0 {
                return Some(i);
            }
        }
        None
    }

    fn erase_at(&mut self, pos: Pos2, layout: &Layout) {
        if let Some(index) = self.hit_note(pos, layout) {
            self.notes.remove(index);
            self.selected.clear();
            self.refresh_word_bank();
            self.dirty = true;
            self.status = "Note erased.".into();
        }
    }

    fn seek(&mut self, time: f64, audio: &Option<AudioEngine>) {
        self.playhead = time.clamp(0.0, self.duration);
        self.last_seek = self.playhead;
        if let Some(audio) = audio {
            audio.seek(self.playhead);
        }
    }

    fn toggle_play(&mut self, audio: &Option<AudioEngine>) {
        if self.playing {
            self.playing = false;
            if let Some(audio) = audio {
                audio.pause();
            }
            return;
        }
        if self.playhead >= self.duration - 0.04 {
            self.seek(0.0, audio);
        }
        self.playing = true;
        if let Some(audio) = audio {
            audio.set_rate(self.speed);
            if self.audio_ready {
                audio.play();
            }
        }
    }

    fn apply_editor_volumes(&self, audio: &Option<AudioEngine>) {
        if let Some(audio) = audio {
            audio.set_volumes(
                if self.backing_on { 0.9 } else { 0.0 },
                if self.vocals_on { 0.85 } else { 0.0 },
            );
        }
    }

    fn push_history(&mut self) {
        self.history.push(self.notes.clone());
        if self.history.len() > 100 {
            self.history.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    fn undo(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.redo.push(self.notes.clone());
            self.notes = previous;
            self.selected.clear();
            self.refresh_word_bank();
            self.dirty = true;
            self.status = "Undo.".into();
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.history.push(self.notes.clone());
            self.notes = next;
            self.selected.clear();
            self.refresh_word_bank();
            self.dirty = true;
            self.status = "Redo.".into();
        }
    }

    fn delete_selected(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        self.push_history();
        let targets = self.selected.clone();
        self.notes = self
            .notes
            .iter()
            .enumerate()
            .filter(|(i, _)| !targets.contains(i))
            .map(|(_, note)| note.clone())
            .collect();
        self.selected.clear();
        self.refresh_word_bank();
        self.dirty = true;
        self.status = "Selection deleted.".into();
    }

    fn transpose_selected(&mut self, semitones: i32) {
        if self.selected.is_empty() {
            return;
        }
        self.push_history();
        for index in self.selected.iter().copied() {
            if let Some(note) = self.notes.get_mut(index) {
                note.midi = (note.midi + semitones).clamp(0, 127);
            }
        }
        self.dirty = true;
        self.status = format!("Transposed {} semitones.", semitones);
    }

    fn clear_selected_words(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        self.push_history();
        for index in self.selected.iter().copied() {
            if let Some(note) = self.notes.get_mut(index) {
                note.word = None;
                note.word_index = None;
                note.line = None;
            }
        }
        self.refresh_word_bank();
        self.dirty = true;
        self.status = "Word removed from selection; it is back in the bank.".into();
    }

    fn assign_word(
        &mut self,
        token_index: usize,
        mut targets: Vec<usize>,
        create_at: Option<Pos2>,
        layout: &Layout,
        audio: &Option<AudioEngine>,
    ) {
        let Some(token) = self.word_bank.get(token_index).cloned() else {
            return;
        };
        self.push_history();
        if targets.is_empty() {
            if let Some(pos) = create_at {
                let midi = self.midi_at_y(pos.y, layout);
                let start = self.snap_time(self.x_to_time(pos.x, layout));
                self.notes.push(Note {
                    start,
                    dur: 0.32,
                    midi,
                    word: None,
                    word_index: None,
                    line: None,
                    vel: None,
                });
                targets.push(self.notes.len() - 1);
            }
        }
        for index in targets.iter().copied() {
            if let Some(note) = self.notes.get_mut(index) {
                note.word = Some(token.text.clone());
                note.word_index = Some(token.index);
                note.line = Some(token.line);
            }
        }
        self.selected = targets.iter().copied().collect();
        self.pending_word = None;
        self.word_drag = None;
        self.refresh_word_bank();
        self.dirty = true;
        self.status = format!(
            "Placed “{}” on {} note{}.",
            token.text,
            targets.len(),
            if targets.len() == 1 { "" } else { "s" }
        );
        if self.playing {
            if let Some(audio) = audio {
                audio.play();
            }
        }
    }

    fn refresh_word_bank(&mut self) {
        for token in &mut self.word_bank {
            token.placed = self.notes.iter().any(|note| {
                (note.line == Some(token.line) && note.word_index == Some(token.index))
                    || (note.line == Some(token.line)
                        && note.word_index.is_none()
                        && note.word.as_deref() == Some(token.text.as_str()))
            });
        }
    }

    fn sort_notes(&mut self) {
        let selected_keys: Vec<(u64, i32)> = self
            .selected
            .iter()
            .filter_map(|i| self.notes.get(*i))
            .map(|note| (note.start.to_bits(), note.midi))
            .collect();
        self.notes.sort_by(note_order);
        self.selected.clear();
        for (i, note) in self.notes.iter().enumerate() {
            if selected_keys.contains(&(note.start.to_bits(), note.midi)) {
                self.selected.insert(i);
            }
        }
        self.refresh_word_bank();
    }
}

fn note_order(a: &Note, b: &Note) -> std::cmp::Ordering {
    a.start
        .partial_cmp(&b.start)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a.midi.cmp(&b.midi))
}

fn same_word(a: &Note, b: &Note) -> bool {
    if a.word.is_none() || b.word.is_none() || a.word != b.word || a.line != b.line {
        return false;
    }
    match (a.word_index, b.word_index) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

fn is_word_continuation(notes: &[Note], index: usize) -> bool {
    index > 0 && same_word(&notes[index - 1], &notes[index])
}

fn alpha(color: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
}

fn build_word_bank(song: &Song, notes: &[Note]) -> Vec<WordToken> {
    let mut bank = Vec::new();
    for (line_index, line) in song.lines.iter().enumerate() {
        for (word_index, (text, start, _end)) in split_line_words(line).into_iter().enumerate() {
            bank.push(WordToken {
                text,
                line: line_index,
                index: word_index,
                start,
                placed: false,
            });
        }
    }
    bank.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.index.cmp(&b.index))
    });
    for token in &mut bank {
        token.placed = notes.iter().any(|note| {
            (note.line == Some(token.line) && note.word_index == Some(token.index))
                || (note.line == Some(token.line)
                    && note.word_index.is_none()
                    && note.word.as_deref() == Some(token.text.as_str()))
        });
    }
    bank
}

fn split_line_words(line: &LyricLine) -> Vec<(String, f64, f64)> {
    let mut source: Vec<(String, f64, f64)> = Vec::new();
    if line.words.is_empty() {
        source.push((line.text.clone(), line.start, line.end.max(line.start + 0.3)));
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
        result.push((line.text.clone(), line.start, line.end.max(line.start + 0.3)));
    }
    result
}

fn write_song(root: &Path, song: &Song) -> Result<(), String> {
    let path = PathBuf::from(root).join("assets/songs").join(&song.id).join("song.json");
    let raw = serde_json::to_string_pretty(song).map_err(|error| error.to_string())?;
    fs::write(&path, raw).map_err(|error| format!("Could not save {}: {error}", path.display()))
}

fn paint_button(ui: &mut Ui, rect: Rect, text: &str, primary: bool, enabled: bool) -> bool {
    let response = ui.interact(
        rect,
        Id::new(("editor_button", text, rect.left() as i32, rect.top() as i32)),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let hovered = response.hovered() && enabled;
    let p = ui.painter();
    let fill = if primary {
        if hovered { Color32::from_rgb(152, 255, 213) } else { MINT }
    } else if hovered {
        Color32::from_rgb(24, 32, 49)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 9)
    };
    p.rect_filled(rect, 7.0, if enabled { fill } else { Color32::from_white_alpha(5) });
    if !primary {
        p.rect_stroke(
            rect,
            7.0,
            Stroke::new(1.0f32, if enabled { Color32::from_white_alpha(35) } else { Color32::from_white_alpha(12) }),
            StrokeKind::Inside,
        );
    }
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::monospace(if text.len() > 8 { 7.5 } else { 9.0 }),
        if !enabled {
            MUTED2
        } else if primary {
            Color32::from_rgb(5, 18, 13)
        } else {
            INK
        },
    );
    enabled && response.clicked()
}

fn paint_toggle(ui: &mut Ui, rect: Rect, text: &str, active: bool) -> bool {
    let response = ui.interact(
        rect,
        Id::new(("editor_toggle", text, rect.left() as i32, rect.top() as i32)),
        Sense::click(),
    );
    let p = ui.painter();
    let fill = if active {
        Color32::from_rgba_unmultiplied(124, 247, 196, 24)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 7)
    };
    p.rect_filled(rect, 7.0, fill);
    p.rect_stroke(
        rect,
        7.0,
        Stroke::new(1.0f32, if active { alpha(MINT, 95) } else { Color32::from_white_alpha(26) }),
        StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::monospace(if text.len() > 10 { 6.5 } else { 8.0 }),
        if active { MINT } else { MUTED },
    );
    response.clicked()
}
