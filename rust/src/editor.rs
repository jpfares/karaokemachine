use crate::audio::AudioEngine;
use crate::automap;
use crate::jobs::Job;
use crate::model::{
    format_time, lane_range, note_name, pitch_color, split_line_words, CustomWord, LyricLine,
    MapFile, Note, PlacedWord, RibbonLink, RibbonSide, Song, MAP_FILE_FORMAT, MAP_FILE_VERSION,
};
use crate::theme::{alpha, BG, GOLD, INK, MINT, MUTED, MUTED2, PANEL, PANEL2, PINK};
use crate::widgets::{button, toggle};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Id, LayerId, Order, PointerButton, Pos2, Rect,
    RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

pub const VOCAL_BINS_PER_SECOND: f32 = 80.0;
/// How long a first AUTO-MAP click over an existing chart stays armed.
const AUTO_MAP_CONFIRM_SECONDS: f64 = 6.0;
pub const RAW_PITCH_SAMPLE_RATE: f32 = 8000.0;

/// Keep a dragged note long enough to remain selectable without changing the
/// length the pointer asked for.
const MIN_NOTE_DURATION: f64 = 0.06;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorTool {
    Pan,
    Note,
    Ribbon,
    Eraser,
    Select,
}

impl EditorTool {
    fn hint(self) -> &'static str {
        match self {
            EditorTool::Pan => "Drag to pan. Middle-drag pans from any tool. Right-click moves the now-playing line.",
            EditorTool::Note => {
                "Click or drag on the lane to draw one pitch in either direction. Drag a note edge from inside to resize."
            }
            EditorTool::Ribbon => {
                "Drag from a dot on one note to a dot on another to connect them. Click a connector to select it."
            }
            EditorTool::Eraser => "Click or sweep over notes and word decorations to erase them.",
            EditorTool::Select => {
                "Select notes, connectors, or word decorations. Drag note ends to resize."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorIcon {
    Pan,
    Note,
    Ribbon,
    Eraser,
    Select,
    Snap,
    Suggestions,
    Follow,
    ZoomOut,
    ZoomIn,
    Plus,
    Back,
    Submit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorEvent {
    None,
    AutoMap,
    Back,
    SaveDraft,
    ExportMap,
    Submit,
}

#[derive(Clone, Debug)]
struct WordToken {
    text: String,
    line: Option<usize>,
    index: Option<usize>,
    custom_id: Option<u32>,
    start: f64,
    placed: bool,
}

#[derive(Clone)]
struct Snapshot {
    notes: Vec<Note>,
    words: Vec<PlacedWord>,
    custom_words: Vec<CustomWord>,
    ribbon_links: Vec<RibbonLink>,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    NewNote {
        index: usize,
        anchor: f64,
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
    MoveWord {
        index: usize,
        grab_time: f64,
        grab_y: f32,
    },
    RibbonConnect {
        from_note: usize,
        from_side: RibbonSide,
    },
    Erase,
}

struct Layout {
    wave: Rect,
    note_area: Rect,
    rail: Rect,
    inspector: Rect,
    ruler: Rect,
    transport: Rect,
    bank: Rect,
}

pub struct Editor {
    pub song: Song,
    map_path: PathBuf,
    map_name: String,
    map_draft: bool,
    suggestions: Vec<Note>,
    notes: Vec<Note>,
    vocal_peaks: Vec<f32>,
    raw_pitch: Vec<Option<f32>>,
    pitch_job: Option<Job<Vec<Option<f32>>>>,
    words: Vec<PlacedWord>,
    custom_words: Vec<CustomWord>,
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
    selected_word: Option<usize>,
    selected_ribbon: Option<usize>,
    ribbon_links: Vec<RibbonLink>,
    drag: Option<Drag>,
    word_drag: Option<usize>,
    pending_word: Option<usize>,
    history: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    dirty: bool,
    audio_ready: bool,
    /// A decode is in flight; cleared when it lands or fails.
    audio_loading: bool,
    backing_on: bool,
    vocals_on: bool,
    show_suggestions: bool,
    follow: bool,
    snap: bool,
    duration: f64,
    right_scrubbing: bool,
    next_custom_id: u32,
    custom_word_open: bool,
    custom_word_input: String,
    custom_word_focus: bool,
    status: String,
    last_seek: f64,
    /// When AUTO-MAP was clicked over an existing chart, awaiting a second click.
    auto_map_armed: Option<f64>,
}

impl Editor {
    pub fn new(
        mut song: Song,
        suggestions: Vec<Note>,
        mut initial_notes: Vec<Note>,
        duration: f64,
        map_path: PathBuf,
        map_draft: bool,
    ) -> Self {
        song.notes.clear();
        let map_name = map_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("map")
            .to_string();
        let duration = duration.max(song.duration()).max(1.0);
        let mut words = song.placed_words.clone();
        let custom_words = song.custom_words.clone();
        let mut next_ribbon_id = initial_notes
            .iter()
            .chain(suggestions.iter())
            .filter_map(|note| note.ribbon)
            .max()
            .unwrap_or(0)
            + 1;
        if words.is_empty() {
            migrate_legacy_notes(&song, &mut initial_notes, &mut words, &mut next_ribbon_id);
        } else {
            migrate_legacy_ribbons(&mut initial_notes, &mut next_ribbon_id);
        }
        let mut ribbon_links = if song.ribbon_links.is_empty() {
            links_from_legacy_notes(&initial_notes)
        } else {
            song.ribbon_links.clone()
        };
        ribbon_links.retain(|link| {
            link.from < initial_notes.len() && link.to < initial_notes.len() && link.from != link.to
        });
        for note in &mut initial_notes {
            note.ribbon = None;
        }
        let next_custom_id = custom_words.iter().map(|word| word.id).max().unwrap_or(0) + 1;
        let word_bank = build_word_bank(&song.lines, &custom_words, &words);
        let mut source: Vec<i32> = suggestions
            .iter()
            .chain(initial_notes.iter())
            .map(|n| n.midi)
            .collect();
        if source.is_empty() {
            source = song.notes.iter().map(|n| n.midi).collect();
        }
        let (low, high) = lane_range(&source, (song.low, song.high));
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
            "Draw notes over the translucent suggestions. Drag lyric words anywhere as decorations."
        } else {
            "Draft loaded. Finish the notes, ribbons, and word decorations, then submit."
        };
        Self {
            song,
            map_path,
            map_name,
            map_draft,
            suggestions,
            notes: initial_notes,
            vocal_peaks: Vec::new(),
            raw_pitch: Vec::new(),
            pitch_job: None,
            words,
            custom_words,
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
            selected_word: None,
            selected_ribbon: None,
            ribbon_links,
            drag: None,
            word_drag: None,
            pending_word: None,
            history: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            audio_ready: false,
            audio_loading: false,
            backing_on: true,
            vocals_on: true,
            show_suggestions: true,
            follow: false,
            snap: true,
            duration,
            right_scrubbing: false,
            next_custom_id,
            custom_word_open: false,
            custom_word_input: String::new(),
            custom_word_focus: false,
            status: status.to_string(),
            last_seek: 0.0,
            auto_map_armed: None,
        }
    }

    pub fn set_audio_ready(&mut self, ready: bool) {
        self.audio_ready = ready;
        self.audio_loading = false;
    }

    pub fn set_audio_loading(&mut self, loading: bool) {
        self.audio_loading = loading;
    }

    /// Bring a freshly installed engine in line with the editor's transport.
    /// The decode can finish while the chart preview is already playing, or
    /// the engine may have been rebuilt after a device change.
    pub fn sync_audio(&self, audio: &AudioEngine) {
        let (backing, vocals) = self.mix_levels();
        audio.set_volumes(backing, vocals);
        audio.set_rate(self.speed);
        audio.seek(self.playhead);
        if self.playing {
            audio.play();
        }
    }

    fn mix_levels(&self) -> (f32, f32) {
        (
            if self.backing_on { 0.9 } else { 0.0 },
            if self.vocals_on { 0.85 } else { 0.0 },
        )
    }

    pub fn set_vocal_peaks(&mut self, peaks: Vec<f32>) {
        self.vocal_peaks = peaks;
    }

    pub fn set_pitch_job(&mut self, job: Job<Vec<Option<f32>>>) {
        self.pitch_job = Some(job);
    }

    fn poll_pitch_track(&mut self) {
        let result = self.pitch_job.as_mut().and_then(Job::poll);
        if let Some(result) = result {
            self.pitch_job = None;
            // A failed analysis simply leaves the raw pitch lane empty.
            if let Ok(track) = result {
                self.raw_pitch = track;
            }
        }
    }

    pub fn pause_audio(&self, audio: &Option<AudioEngine>) {
        if let Some(audio) = audio {
            audio.pause();
        }
    }

    pub fn mark_draft_saved(&mut self) {
        self.status = format!("Draft saved to {}.", self.map_name);
    }

    /// Unsaved edits since the last draft save or submission.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn is_draft(&self) -> bool {
        self.map_draft
    }

    pub fn save_draft(&mut self) -> Result<(), String> {
        if !self.map_draft {
            return Err("This file is a finished level. Submit Mapping to save changes.".into());
        }
        self.sort_notes();
        let map = self.to_map_file(true);
        crate::library::save_map_file(&self.map_path, &map)?;
        self.dirty = false;
        self.status = format!("Draft saved to {}.", self.map_name);
        Ok(())
    }

    pub fn submit(&mut self) -> Result<(), String> {
        self.validate()?;
        self.sort_notes();
        for note in &mut self.notes {
            note.word = None;
            note.word_index = None;
            note.line = None;
        }
        let map = self.to_map_file(false);
        crate::library::save_map_file(&self.map_path, &map)?;
        self.map_draft = false;
        self.dirty = false;
        self.status = format!("Level saved to {}.", self.map_name);
        Ok(())
    }

    fn to_map_file(&self, draft: bool) -> MapFile {
        let mut low = self.low;
        let mut high = self.high;
        if let (Some(lo), Some(hi)) = (
            self.notes.iter().map(|note| note.midi).min(),
            self.notes.iter().map(|note| note.midi).max(),
        ) {
            low = (lo - 2).clamp(0, 108);
            high = (hi + 2).clamp(low + 12, 127);
        }
        MapFile {
            format: MAP_FILE_FORMAT.into(),
            version: MAP_FILE_VERSION,
            draft,
            notes: self.notes.clone(),
            lines: self.song.lines.clone(),
            placed_words: self.words.clone(),
            custom_words: self.custom_words.clone(),
            ribbon_links: self.ribbon_links.clone(),
            low,
            high,
        }
    }

    /// A song carrying this editor's current mapping, for map export.
    pub fn export_song(&self) -> Song {
        let map = self.to_map_file(false);
        let mut song = self.song.clone();
        map.apply_to_song(&mut song);
        song.mapped = true;
        song.tags.retain(|tag| tag != "UNMAPPED");
        if !song.tags.iter().any(|tag| tag == "Mapped") {
            song.tags.push("Mapped".into());
        }
        song
    }

    /// Replace the current notes, connectors and words with a mapping built
    /// from the suggestions. Undoable like any other edit.
    pub fn auto_map(&mut self) -> Result<(), String> {
        let lane = Some((self.low, self.high));
        let mapping = automap::auto_map(&self.song, &self.suggestions, lane, &self.words)?;
        self.auto_map_armed = None;
        self.push_history();
        self.status = mapping.summary() + " Check them, then submit.";
        let mut notes = mapping.notes;
        self.ribbon_links = links_from_legacy_notes(&notes);
        for note in &mut notes {
            note.ribbon = None;
        }
        self.notes = notes;
        self.words = mapping.words;
        self.selected.clear();
        self.selected_word = None;
        self.selected_ribbon = None;
        self.drag = None;
        self.rebuild_word_bank();
        self.sort_notes();
        Ok(())
    }

    /// AUTO-MAP over an empty chart runs at once; over existing notes or
    /// words it asks for a second click within a few seconds first.
    fn request_auto_map(&mut self, now: f64) {
        let chart_is_empty =
            self.notes.is_empty() && self.words.iter().all(|word| word.custom_id.is_some());
        let armed = self
            .auto_map_armed
            .is_some_and(|at| now - at < AUTO_MAP_CONFIRM_SECONDS);
        if !chart_is_empty && !armed {
            self.auto_map_armed = Some(now);
            self.status = format!(
                "AUTO-MAP replaces the {} notes and {} words already drawn. Click it again to confirm; Ctrl+Z undoes it.",
                self.notes.len(),
                self.words.len()
            );
            return;
        }
        if let Err(error) = self.auto_map() {
            self.status = error;
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.notes.is_empty() {
            return Err("Place at least one vocal note before submitting.".into());
        }
        if !self.word_bank.is_empty() {
            let missing: Vec<&WordToken> = self.word_bank.iter().filter(|w| !w.placed).collect();
            if let Some(first) = missing.first() {
                return Err(format!(
                    "{} word{} left in the bank — next: “{}”.",
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
        self.poll_pitch_track();

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

        // Apply the scrub after the transport update so the yellow line always
        // follows the cursor while the secondary button is held.
        self.handle_right_scrub(ui, &layout, audio);

        // The note lane gets first crack at the pointer so the wheel can scroll
        // the timeline without fighting nested scroll areas.
        let canvas_response = ui.interact(
            layout.note_area,
            Id::new("editor_note_canvas"),
            Sense::click_and_drag(),
        );
        self.handle_navigation(ui, &layout, &canvas_response, audio);
        self.handle_canvas(ui, &layout, &canvas_response, audio, &mut event);

        // ---- paint -----------------------------------------------------
        let p = ui.painter().clone();
        p.rect_filled(rect, 0.0, BG);
        self.draw_header(ui, &p, &layout, &mut event);
        self.draw_toolbar(ui, &p, &layout);
        self.draw_lanes(ui, &p, &layout);
        self.draw_wave(ui, &p, &layout);
        self.draw_hover_cursor(ui, &layout);
        self.draw_ruler(ui, &p, &layout, &mut event, audio);
        self.draw_transport(ui, &p, &layout, audio);
        self.draw_word_bank(ui, &p, &layout, audio);
        self.draw_inspector(ui, &p, &layout, &mut event);
        if matches!(event, EditorEvent::AutoMap) {
            event = EditorEvent::None;
            self.request_auto_map(ui.input(|i| i.time));
        }
        self.draw_custom_word_dialog(&ctx);

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
        let wave_h = 64.0;
        let note_top = rect.top() + header_h + toolbar_h;
        let wave = Rect::from_min_max(
            Pos2::new(rect.left() + rail_w, note_top),
            Pos2::new(rect.right() - inspector_w, note_top + wave_h),
        );
        let note_area = Rect::from_min_max(
            Pos2::new(rect.left() + rail_w, wave.bottom()),
            Pos2::new(rect.right() - inspector_w, transport.top() - ruler_h),
        );
        let rail = Rect::from_min_max(
            Pos2::new(rect.left(), wave.top()),
            Pos2::new(note_area.left(), note_area.bottom()),
        );
        let inspector = Rect::from_min_max(
            Pos2::new(rect.right() - inspector_w + 8.0, wave.top() + 6.0),
            Pos2::new(rect.right() - 8.0, transport.top() - 8.0),
        );
        Layout {
            wave,
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
        if self.custom_word_open {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.custom_word_open = false;
                self.custom_word_input.clear();
            }
            return;
        }
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
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            *event = EditorEvent::SaveDraft;
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
        audio: &Option<AudioEngine>,
    ) {
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let (scroll, modifiers) = ui.input(|i| (i.raw_scroll_delta, i.modifiers));
        let ctrl = modifiers.command || modifiers.ctrl;
        let zoom_area = layout
            .rail
            .union(layout.wave)
            .union(layout.note_area)
            .union(layout.ruler);

        // Ctrl+wheel behaves like the zoom in/out buttons anywhere over the
        // working area, including the piano rail and the analysis strip.
        let mut zoom_handled = false;
        if ctrl && scroll.y.abs() > 0.0 {
            if let Some(pos) = pointer.filter(|pos| zoom_area.contains(*pos)) {
                let anchor_x = pos
                    .x
                    .clamp(layout.note_area.left(), layout.note_area.right());
                let anchor_time = self.x_to_time(anchor_x, layout);
                self.zoom = (self.zoom * (1.0 + scroll.y * 0.0015)).clamp(24.0, 420.0);
                self.scroll = (anchor_time
                    - (anchor_x - layout.note_area.left()) as f64 / self.zoom as f64)
                    .max(0.0);
                zoom_handled = true;
            }
        }

        // The leftmost piano rail is a pitch scrollbar and an audition keyboard.
        if let Some(pos) = pointer {
            if layout.rail.contains(pos) {
                if !zoom_handled && scroll.y.abs() > 0.0 {
                    let step = if modifiers.shift { 12 } else { 1 };
                    self.scroll_pitches(if scroll.y > 0.0 { step } else { -step });
                }
                if layout.note_area.contains(pos)
                    && ui.input(|i| i.pointer.button_clicked(PointerButton::Primary))
                {
                    let midi = self.midi_at_y(pos.y, layout);
                    if let Some(audio) = audio {
                        audio.play_preview(midi, 0.55);
                        self.status = format!("Preview {} from the piano rail.", note_name(midi));
                    } else {
                        self.status = "No audio output is available for note preview.".into();
                    }
                }
            }
        }

        if response.hovered() && !zoom_handled && !ctrl {
            let dx = scroll.x + scroll.y;
            if dx.abs() > 0.0 {
                self.scroll = (self.scroll - dx as f64 / self.zoom as f64).max(0.0);
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
        if middle_down
            && self.drag.is_none()
            && pointer
                .map(|pos| layout.note_area.contains(pos))
                .unwrap_or(false)
        {
            self.scroll = (self.scroll - delta.x as f64 / self.zoom as f64).max(0.0);
        }

        if self.tool == EditorTool::Pan && response.dragged_by(PointerButton::Primary) {
            self.scroll = (self.scroll - delta.x as f64 / self.zoom as f64).max(0.0);
        }
    }

    fn handle_right_scrub(&mut self, ui: &mut Ui, layout: &Layout, audio: &Option<AudioEngine>) {
        if self.custom_word_open {
            self.right_scrubbing = false;
            return;
        }
        let clicked = ui.input(|i| i.pointer.button_clicked(PointerButton::Secondary));
        let down = ui.input(|i| i.pointer.button_down(PointerButton::Secondary));
        if !(clicked || down) || self.drag.is_some() {
            if self.right_scrubbing {
                self.status = format!("Playhead at {}.", format_time_precise(self.playhead));
            }
            self.right_scrubbing = false;
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.interact_pos()) else {
            self.right_scrubbing = false;
            return;
        };
        let area = layout.wave.union(layout.note_area).union(layout.ruler);
        if !area.contains(pos) {
            self.right_scrubbing = false;
            return;
        }
        if !self.right_scrubbing {
            self.right_scrubbing = true;
            self.status = "Scrubbing the now-playing line. Release to leave it here.".into();
        }
        let time = self.x_to_time(pos.x, layout).clamp(0.0, self.duration);
        if (time - self.playhead).abs() > 0.0005 {
            self.seek(time, audio);
        }
    }

    fn preview_note(&mut self, index: usize, audio: &Option<AudioEngine>) {
        let Some(note) = self.notes.get(index) else {
            return;
        };
        let midi = note.midi;
        let start = note.start;
        let duration = note.dur.clamp(0.18, 2.0) as f32;
        if let Some(audio) = audio {
            audio.play_preview(midi, duration);
            self.status = format!(
                "Preview {} at {}. Click another note to hear it.",
                note_name(midi),
                format_time(start)
            );
        } else {
            self.status = "No audio output is available for note preview.".into();
        }
    }

    fn begin_edge_resize(&mut self, index: usize, left_edge: bool) {
        self.selected.clear();
        self.selected.insert(index);
        self.selected_word = None;
        self.selected_ribbon = None;
        let (note_start, note_dur) = {
            let note = &self.notes[index];
            (note.start, note.dur)
        };
        self.push_history();
        self.drag = Some(if left_edge {
            Drag::ResizeLeft {
                index,
                end: note_start + note_dur,
            }
        } else {
            Drag::ResizeRight {
                index,
                start: note_start,
            }
        });
    }

    /// Set a note to span from the press anchor to the current pointer. This
    /// works in both directions, so dragging left is not forced to grow right.
    fn update_new_note(&mut self, index: usize, anchor: f64, current: f64) {
        let (start, end) = if current >= anchor {
            (anchor, current)
        } else {
            (current, anchor)
        };
        let start = start.max(0.0);
        let end = end.max(start);
        if let Some(note) = self.notes.get_mut(index) {
            note.start = start;
            note.dur = (end - start).max(MIN_NOTE_DURATION);
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
        let over_edge = match self.tool {
            EditorTool::Select => self.note_edge_at(pos, layout).is_some(),
            EditorTool::Note => self.note_edge_at_inside(pos, layout).is_some(),
            _ => false,
        };
        if over_edge {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }

        // A word armed with a click in the bank drops at the next click,
        // anywhere in the note area.
        if response.clicked_by(PointerButton::Primary) {
            if let Some(token) = self.pending_word.take() {
                if layout.note_area.contains(pos) {
                    self.place_word(token, pos, layout, audio);
                } else {
                    self.pending_word = Some(token);
                }
                return;
            }
        }

        match self.tool {
            EditorTool::Pan => {
                if response.clicked_by(PointerButton::Primary) {
                    if let Some(index) = self.hit_note(pos, layout) {
                        self.preview_note(index, audio);
                    }
                }
            }
            EditorTool::Note => {
                if response.drag_started_by(PointerButton::Primary) {
                    // `drag_started` fires after the pointer has moved a few
                    // pixels. Use the press point so the note still begins
                    // where the user put the pointer down.
                    let press_pos = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
                    if layout.note_area.contains(press_pos) {
                        if let Some((index, left_edge)) =
                            self.note_edge_at_inside(press_pos, layout)
                        {
                            self.begin_edge_resize(index, left_edge);
                        } else {
                            self.push_history();
                            let midi = self.midi_at_y(press_pos.y, layout);
                            let start = self.snap_time(self.x_to_time(press_pos.x, layout));
                            let index = self.notes.len();
                            self.notes.push(Note {
                                start,
                                dur: MIN_NOTE_DURATION,
                                midi,
                                word: None,
                                word_index: None,
                                line: None,
                                vel: None,
                                ribbon: None,
                            });
                            self.selected.clear();
                            self.selected_word = None;
                            self.selected_ribbon = None;
                            self.selected.insert(index);
                            self.drag = Some(Drag::NewNote {
                                index,
                                anchor: start,
                            });
                            self.dirty = true;
                        }
                    }
                } else if response.clicked_by(PointerButton::Primary)
                    && layout.note_area.contains(pos)
                {
                    if let Some(index) = self.hit_note(pos, layout) {
                        self.selected.clear();
                        self.selected.insert(index);
                        self.selected_word = None;
                        self.selected_ribbon = None;
                        self.preview_note(index, audio);
                    } else {
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
                            ribbon: None,
                        });
                        self.selected.clear();
                        self.selected_word = None;
                        self.selected_ribbon = None;
                        self.selected.insert(self.notes.len() - 1);
                        self.dirty = true;
                        self.status =
                            "Note placed. Use RIBBON to connect notes into one vocal run.".into();
                    }
                }
            }
            EditorTool::Select => {
                if response.drag_started_by(PointerButton::Primary) {
                    if let Some(word_index) = self.hit_word(pos, layout) {
                        self.selected.clear();
                        self.selected_word = Some(word_index);
                        self.selected_ribbon = None;
                        let word = &self.words[word_index];
                        let grab_time = self.x_to_time(pos.x, layout) - word.start;
                        let grab_y = (pos.y - layout.note_area.top())
                            / layout.note_area.height().max(1.0)
                            - word.y;
                        self.push_history();
                        self.drag = Some(Drag::MoveWord {
                            index: word_index,
                            grab_time,
                            grab_y,
                        });
                    } else if let Some((index, left_edge)) = self.note_edge_at(pos, layout) {
                        self.begin_edge_resize(index, left_edge);
                    } else if let Some(index) = self.hit_note(pos, layout) {
                        self.selected.clear();
                        self.selected.insert(index);
                        self.selected_word = None;
                        self.selected_ribbon = None;
                        let (note_start, note_midi) = {
                            let note = &self.notes[index];
                            (note.start, note.midi)
                        };
                        self.push_history();
                        self.drag = Some(Drag::Move {
                            index,
                            grab: self.x_to_time(pos.x, layout) - note_start,
                            midi_offset: self.midi_at_y(pos.y, layout) - note_midi,
                        });
                    } else {
                        self.selected.clear();
                        self.selected_word = None;
                        self.selected_ribbon = None;
                    }
                } else if response.clicked_by(PointerButton::Primary) {
                    if let Some(word_index) = self.hit_word(pos, layout) {
                        self.selected.clear();
                        self.selected_word = Some(word_index);
                        self.selected_ribbon = None;
                    } else if let Some(index) = self.hit_note(pos, layout) {
                        self.selected_word = None;
                        self.selected_ribbon = None;
                        if ui.input(|i| i.modifiers.shift) {
                            if !self.selected.insert(index) {
                                self.selected.remove(&index);
                            }
                        } else {
                            self.selected.clear();
                            self.selected.insert(index);
                        }
                        self.preview_note(index, audio);
                    } else if let Some(index) = self.hit_ribbon(pos, layout) {
                        self.selected.clear();
                        self.selected_word = None;
                        self.selected_ribbon = Some(index);
                        self.status = "Connector selected. Press Delete to remove it.".into();
                    } else {
                        self.selected.clear();
                        self.selected_word = None;
                        self.selected_ribbon = None;
                    }
                }
            }
            EditorTool::Ribbon => {
                if response.drag_started_by(PointerButton::Primary) {
                    let press_pos = ui.input(|i| i.pointer.press_origin()).unwrap_or(pos);
                    if let Some((note, side)) = self.ribbon_dot_at(press_pos, layout) {
                        self.selected.clear();
                        self.selected_word = None;
                        self.selected_ribbon = None;
                        self.drag = Some(Drag::RibbonConnect {
                            from_note: note,
                            from_side: side,
                        });
                        self.status = "Drag to another note's dot to connect the two notes.".into();
                    }
                } else if response.clicked_by(PointerButton::Primary) {
                    if let Some(index) = self.hit_ribbon(pos, layout) {
                        self.selected.clear();
                        self.selected_word = None;
                        self.selected_ribbon = Some(index);
                        self.status = "Connector selected. Press Delete to remove it.".into();
                    } else if let Some((note, _)) = self.ribbon_dot_at(pos, layout) {
                        self.selected.clear();
                        self.selected_word = None;
                        self.selected_ribbon = None;
                        self.selected.insert(note);
                        self.preview_note(note, audio);
                    } else {
                        self.selected.clear();
                        self.selected_word = None;
                        self.selected_ribbon = None;
                    }
                }
            }
            EditorTool::Eraser => {
                if response.drag_started_by(PointerButton::Primary) {
                    if self.hit_word(pos, layout).is_some() || self.hit_note(pos, layout).is_some()
                    {
                        self.push_history();
                        self.drag = Some(Drag::Erase);
                        self.erase_at(pos, layout);
                    }
                } else if response.clicked_by(PointerButton::Primary)
                    && (self.hit_word(pos, layout).is_some()
                        || self.hit_note(pos, layout).is_some())
                {
                    self.push_history();
                    self.erase_at(pos, layout);
                }
            }
        }

        // Continue an active drag with the current pointer position.
        match self.drag {
            Some(Drag::NewNote { index, anchor }) => {
                let stopped = response.drag_stopped_by(PointerButton::Primary);
                if response.dragged_by(PointerButton::Primary) || stopped {
                    let current = self.snap_time(self.x_to_time(pos.x, layout));
                    self.update_new_note(index, anchor, current);
                }
                if stopped {
                    self.drag = None;
                    self.sort_notes();
                    self.status =
                        "Note placed. Use RIBBON to connect notes into one vocal run.".into();
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
            Some(Drag::MoveWord {
                index,
                grab_time,
                grab_y,
            }) => {
                if response.dragged_by(PointerButton::Primary) {
                    let start = self.snap_time(self.x_to_time(pos.x, layout) - grab_time);
                    let y = ((pos.y - layout.note_area.top()) / layout.note_area.height().max(1.0)
                        - grab_y)
                        .clamp(0.02, 0.98);
                    if let Some(word) = self.words.get_mut(index) {
                        word.start = start.max(0.0);
                        word.y = y;
                    }
                    self.dirty = true;
                } else if response.drag_stopped_by(PointerButton::Primary) {
                    self.drag = None;
                    self.status = "Word moved. Words are decorative and can float anywhere.".into();
                }
            }
            Some(Drag::RibbonConnect {
                from_note,
                from_side,
            }) => {
                if response.drag_stopped_by(PointerButton::Primary) {
                    self.drag = None;
                    let target = self.ribbon_dot_at(pos, layout);
                    match target {
                        Some((to_note, to_side)) if to_note != from_note => {
                            if let Some(index) = self.ribbon_links.iter().position(|link| {
                                (link.from == from_note && link.to == to_note)
                                    || (link.from == to_note && link.to == from_note)
                            }) {
                                self.selected_ribbon = Some(index);
                                self.status = "Those notes are already connected.".into();
                            } else {
                                self.push_history();
                                self.ribbon_links.push(RibbonLink {
                                    from: from_note,
                                    to: to_note,
                                    from_side,
                                    to_side,
                                });
                                self.selected.clear();
                                self.selected_word = None;
                                self.selected_ribbon = Some(self.ribbon_links.len() - 1);
                                self.dirty = true;
                                self.status =
                                    "Notes connected. Press Delete to remove the connector.".into();
                            }
                        }
                        Some(_) => {
                            self.status = "Choose a dot on a different note.".into();
                        }
                        None => {
                            self.status =
                                "Release on another note's dot to connect the two notes.".into();
                        }
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

    fn draw_header(
        &self,
        ui: &mut Ui,
        p: &egui::Painter,
        layout: &Layout,
        event: &mut EditorEvent,
    ) {
        let rect = ui.max_rect();
        let header = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + 60.0));
        p.rect_filled(header, 0.0, Color32::from_rgba_unmultiplied(5, 8, 15, 248));
        p.line_segment(
            [header.left_bottom(), header.right_bottom()],
            Stroke::new(1.0f32, Color32::from_white_alpha(24)),
        );
        if paint_icon_button(
            ui,
            Rect::from_min_size(Pos2::new(16.0, 14.0), Vec2::new(38.0, 32.0)),
            EditorIcon::Back,
            false,
            true,
            "Back to the library",
        ) {
            *event = EditorEvent::Back;
        }
        p.text(
            Pos2::new(70.0, 13.0),
            Align2::LEFT_TOP,
            "KARAOKEMACHINE · MANUAL LEVEL EDITOR",
            FontId::monospace(10.0),
            MUTED2,
        );
        p.text(
            Pos2::new(70.0, 26.0),
            Align2::LEFT_TOP,
            &self.song.title,
            FontId::proportional(20.0),
            INK,
        );
        p.text(
            Pos2::new(70.0, 48.0),
            Align2::LEFT_TOP,
            format!(
                "{}   ·   {}   ·   {} notes   ·   {}",
                self.song.artist_display(),
                self.song.album_display(),
                self.notes.len(),
                self.map_name
            ),
            FontId::monospace(10.0),
            MUTED,
        );
        let badge =
            Rect::from_min_size(Pos2::new(rect.right() - 390.0, 17.0), Vec2::new(96.0, 26.0));
        let badge_color = if self.map_draft {
            Color32::from_rgba_unmultiplied(255, 209, 102, 22)
        } else {
            Color32::from_rgba_unmultiplied(124, 247, 196, 22)
        };
        p.rect_filled(badge, 13.0, badge_color);
        p.rect_stroke(
            badge,
            13.0,
            Stroke::new(
                1.0f32,
                if self.map_draft {
                    alpha(GOLD, 90)
                } else {
                    alpha(MINT, 90)
                },
            ),
            StrokeKind::Inside,
        );
        p.text(
            badge.center(),
            Align2::CENTER_CENTER,
            if self.map_draft { "DRAFT" } else { "LEVEL" },
            FontId::monospace(10.0),
            if self.map_draft { GOLD } else { MINT },
        );
        let progress = self.word_progress();
        let progress_rect = Rect::from_min_size(
            Pos2::new(rect.right() - 284.0, 21.0),
            Vec2::new(124.0, 18.0),
        );
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
            FontId::monospace(10.0),
            INK,
        );
        let submit_rect =
            Rect::from_min_size(Pos2::new(rect.right() - 54.0, 12.0), Vec2::new(38.0, 36.0));
        let ok = self.validate().is_ok();
        if paint_icon_button(
            ui,
            submit_rect,
            EditorIcon::Submit,
            ok,
            ok,
            if ok {
                "Submit mapping — this song becomes playable"
            } else {
                "Place all notes and word decorations before submitting"
            },
        ) && ok
        {
            *event = EditorEvent::Submit;
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
        for (tool, icon, tip) in [
            (
                EditorTool::Pan,
                EditorIcon::Pan,
                "Pan — drag; middle-drag works from any tool",
            ),
            (
                EditorTool::Note,
                EditorIcon::Note,
                "Note — click or drag to draw one pitch",
            ),
            (
                EditorTool::Ribbon,
                EditorIcon::Ribbon,
                "Ribbon — drag across notes to connect them",
            ),
            (
                EditorTool::Eraser,
                EditorIcon::Eraser,
                "Eraser — click or sweep over notes and words",
            ),
            (
                EditorTool::Select,
                EditorIcon::Select,
                "Select — move notes, words, and note ends",
            ),
        ] {
            let r = Rect::from_min_size(Pos2::new(x, bar.top() + 6.0), Vec2::new(38.0, 32.0));
            if paint_icon_button(ui, r, icon, self.tool == tool, true, tip) {
                self.tool = tool;
                self.status = tool.hint().to_string();
            }
            x += 42.0;
        }
        let hint = Rect::from_min_max(
            Pos2::new(x + 8.0, bar.top() + 4.0),
            Pos2::new(bar.right() - 344.0, bar.bottom() - 4.0),
        );
        p.text(
            hint.left_center(),
            Align2::LEFT_CENTER,
            self.tool.hint(),
            FontId::monospace(10.0),
            MUTED2,
        );
        let snap = Rect::from_min_size(
            Pos2::new(bar.right() - 332.0, bar.top() + 6.0),
            Vec2::new(38.0, 32.0),
        );
        if paint_icon_button(
            ui,
            snap,
            EditorIcon::Snap,
            self.snap,
            true,
            if self.snap {
                "Snap to the grid — on"
            } else {
                "Snap to the grid — off"
            },
        ) {
            self.snap = !self.snap;
        }
        let sugg = Rect::from_min_size(
            Pos2::new(bar.right() - 290.0, bar.top() + 6.0),
            Vec2::new(38.0, 32.0),
        );
        if paint_icon_button(
            ui,
            sugg,
            EditorIcon::Suggestions,
            self.show_suggestions,
            true,
            "Show or hide the translucent suggested chart",
        ) {
            self.show_suggestions = !self.show_suggestions;
        }
        let follow = Rect::from_min_size(
            Pos2::new(bar.right() - 248.0, bar.top() + 6.0),
            Vec2::new(38.0, 32.0),
        );
        if paint_icon_button(
            ui,
            follow,
            EditorIcon::Follow,
            self.follow,
            true,
            "Follow the playhead while previewing",
        ) {
            self.follow = !self.follow;
        }
        let zoom_out = Rect::from_min_size(
            Pos2::new(bar.right() - 190.0, bar.top() + 6.0),
            Vec2::new(38.0, 32.0),
        );
        let zoom_in = Rect::from_min_size(
            Pos2::new(bar.right() - 148.0, bar.top() + 6.0),
            Vec2::new(38.0, 32.0),
        );
        if paint_icon_button(ui, zoom_out, EditorIcon::ZoomOut, false, true, "Zoom out") {
            self.zoom = (self.zoom / 1.25).clamp(24.0, 420.0);
        }
        if paint_icon_button(ui, zoom_in, EditorIcon::ZoomIn, false, true, "Zoom in") {
            self.zoom = (self.zoom * 1.25).clamp(24.0, 420.0);
        }
        p.text(
            Pos2::new(bar.right() - 102.0, bar.center().y),
            Align2::LEFT_CENTER,
            format!("{:.0}px/s", self.zoom),
            FontId::monospace(10.0),
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
                    Rect::from_min_size(
                        Pos2::new(area.left(), top),
                        Vec2::new(area.width(), lane_h),
                    ),
                    0.0,
                    Color32::from_white_alpha(8),
                );
            }
            if midi % 12 == 0 {
                painter.line_segment(
                    [
                        Pos2::new(area.left(), top + lane_h),
                        Pos2::new(area.right(), top + lane_h),
                    ],
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
                Stroke::new(
                    if bar { 1.1f32 } else { 0.6f32 },
                    Color32::from_white_alpha(if bar { 22 } else { 9 }),
                ),
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
                let r = Rect::from_min_max(
                    Pos2::new(x0, y - h / 2.0),
                    Pos2::new(x1.max(x0 + 2.0), y + h / 2.0),
                );
                painter.rect_filled(r, h / 2.0, color);
                painter.rect_stroke(r, h / 2.0, Stroke::new(0.8f32, outline), StrokeKind::Inside);
            }
        }

        // Explicit note-to-note connectors.
        let h = (lane_h * 0.62).clamp(6.0, 22.0);
        for (link_index, link) in self.ribbon_links.iter().enumerate() {
            let Some((start, end)) = self.ribbon_line(link, layout) else {
                continue;
            };
            if start.x.max(end.x) < area.left() - 4.0 || start.x.min(end.x) > area.right() + 4.0 {
                continue;
            }
            let selected = self.selected_ribbon == Some(link_index);
            if selected {
                painter.line_segment(
                    [start, end],
                    Stroke::new((h * 0.58).max(5.0), alpha(MINT, 60)),
                );
            }
            painter.line_segment(
                [start, end],
                Stroke::new(
                    (h * 0.34).max(2.5),
                    if selected {
                        MINT
                    } else {
                        Color32::from_white_alpha(105)
                    },
                ),
            );
        }

        // Mapped note bodies.
        for (i, note) in self.notes.iter().enumerate() {
            let x0 = self.time_to_x(note.start, layout);
            let x1 = self.time_to_x(note.start + note.dur, layout);
            if x1 < area.left() - 4.0 || x0 > area.right() + 4.0 {
                continue;
            }
            let y = self.midi_to_y(note.midi, layout);
            let t = ((y - area.top()) / area.height().max(1.0)).clamp(0.0, 1.0);
            let c = pitch_color(t);
            let color = Color32::from_rgb(c[0], c[1], c[2]);
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
                    Stroke::new(
                        if selected { 1.8f32 } else { 0.7f32 },
                        if selected {
                            Color32::WHITE
                        } else {
                            Color32::from_white_alpha(50)
                        },
                    ),
                    StrokeKind::Inside,
                );
                if selected && r.width() > 18.0 {
                    for hx in [r.left() + 3.0, r.right() - 3.0] {
                        painter.line_segment(
                            [
                                Pos2::new(hx, r.top() + 2.0),
                                Pos2::new(hx, r.bottom() - 2.0),
                            ],
                            Stroke::new(2.0f32, Color32::from_white_alpha(220)),
                        );
                    }
                }
            }
        }

        // Ribbon-tool handles and a live preview while connecting two notes.
        if self.tool == EditorTool::Ribbon {
            let radius = self.ribbon_dot_radius(layout);
            let drag_start = match self.drag {
                Some(Drag::RibbonConnect {
                    from_note,
                    from_side,
                }) => Some((from_note, from_side)),
                _ => None,
            };
            let pointer = ui.input(|i| i.pointer.interact_pos());
            let hovered = pointer.and_then(|pos| self.ribbon_dot_at(pos, layout));
            if let Some((from_note, from_side)) = drag_start {
                if let (Some(start), Some(pointer)) = (
                    self.ribbon_dot_center(from_note, from_side, layout),
                    pointer,
                ) {
                    let end = hovered
                        .filter(|(note, _)| *note != from_note)
                        .and_then(|(note, side)| self.ribbon_dot_center(note, side, layout))
                        .unwrap_or(pointer);
                    painter.line_segment(
                        [start, end],
                        Stroke::new((h * 0.34).max(2.5), alpha(MINT, 160)),
                    );
                }
            }
            for (index, _) in self.notes.iter().enumerate() {
                for side in [RibbonSide::Left, RibbonSide::Right] {
                    let Some(center) = self.ribbon_dot_center(index, side, layout) else {
                        continue;
                    };
                    if center.x < area.left() - radius || center.x > area.right() + radius {
                        continue;
                    }
                    let source = drag_start == Some((index, side));
                    let target = hovered == Some((index, side))
                        && drag_start.map(|(note, _)| note != index).unwrap_or(false);
                    painter.circle_filled(
                        center,
                        radius,
                        if source || target { GOLD } else { MINT },
                    );
                    painter.circle_stroke(
                        center,
                        radius,
                        Stroke::new(1.5f32, Color32::from_rgb(5, 18, 13)),
                    );
                }
            }
        }

        // Decorative words are not attached to notes. They are drawn on top of
        // the note layer and can be moved to any time or vertical position.
        for (index, word) in self.words.iter().enumerate() {
            let rect = self.word_rect(word, layout);
            if rect.right() < area.left() - 4.0 || rect.left() > area.right() + 4.0 {
                continue;
            }
            let selected = self.selected_word == Some(index);
            let custom = word.custom_id.is_some();
            let fill = Color32::from_rgba_unmultiplied(8, 13, 23, 235);
            let border = if selected {
                Color32::WHITE
            } else if custom {
                alpha(GOLD, 150)
            } else {
                alpha(MINT, 100)
            };
            painter.rect_filled(rect, 11.0, fill);
            painter.rect_stroke(
                rect,
                11.0,
                Stroke::new(if selected { 1.6f32 } else { 0.9f32 }, border),
                StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                &word.text,
                FontId::proportional(12.0),
                if selected { Color32::WHITE } else { INK },
            );
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
                    Rect::from_min_size(
                        Pos2::new(layout.rail.left(), y - lane_h / 2.0),
                        Vec2::new(layout.rail.width(), lane_h),
                    ),
                    0.0,
                    Color32::from_white_alpha(10),
                );
            }
            p.text(
                Pos2::new(layout.rail.right() - 8.0, y),
                Align2::RIGHT_CENTER,
                note_name(midi),
                FontId::monospace((lane_h * 0.5).clamp(5.0, 9.5)),
                if midi % 12 == 0 {
                    alpha(INK, 150)
                } else if black {
                    alpha(INK, 88)
                } else {
                    alpha(INK, 108)
                },
            );
        }
        p.text(
            Pos2::new(layout.rail.left() + 7.0, layout.wave.top() + 4.0),
            Align2::LEFT_TOP,
            "RAW",
            FontId::monospace(5.5),
            GOLD,
        );
        p.text(
            Pos2::new(layout.rail.left() + 7.0, layout.wave.top() + 16.0),
            Align2::LEFT_TOP,
            "PITCH",
            FontId::monospace(5.5),
            GOLD,
        );
        p.text(
            Pos2::new(layout.rail.left() + 7.0, layout.wave.top() + 33.0),
            Align2::LEFT_TOP,
            "LEVEL",
            FontId::monospace(5.5),
            MUTED2,
        );
        p.text(
            Pos2::new(layout.rail.left() + 7.0, layout.rail.bottom() - 8.0),
            Align2::LEFT_BOTTOM,
            "SCROLL",
            FontId::monospace(5.0),
            MUTED2,
        );
    }

    fn draw_wave(&self, ui: &mut Ui, _p: &egui::Painter, layout: &Layout) {
        let area = layout.wave;
        let painter = ui.painter().with_clip_rect(area);
        painter.rect_filled(area, 0.0, Color32::from_rgb(7, 10, 18));
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
                Stroke::new(
                    if bar { 1.0f32 } else { 0.55f32 },
                    Color32::from_white_alpha(if bar { 18 } else { 7 }),
                ),
            );
        }
        let pitch_h = 20.0f32;
        let pitch_area = Rect::from_min_max(
            area.left_top(),
            Pos2::new(area.right(), area.top() + pitch_h),
        );
        let level_area = Rect::from_min_max(
            Pos2::new(area.left(), area.top() + pitch_h + 1.0),
            area.right_bottom(),
        );
        painter.line_segment(
            [
                Pos2::new(area.left(), area.top() + pitch_h),
                Pos2::new(area.right(), area.top() + pitch_h),
            ],
            Stroke::new(0.8f32, Color32::from_white_alpha(24)),
        );
        let center = level_area.center().y;
        painter.line_segment(
            [
                Pos2::new(area.left(), center),
                Pos2::new(area.right(), center),
            ],
            Stroke::new(0.7f32, Color32::from_white_alpha(20)),
        );

        let max_peak = self
            .vocal_peaks
            .iter()
            .copied()
            .fold(0.0f32, f32::max)
            .max(1e-6);
        if self.vocal_peaks.is_empty() {
            painter.text(
                Pos2::new(area.left() + 12.0, center),
                Align2::LEFT_CENTER,
                if self.audio_loading {
                    "Loading audio…"
                } else if self.audio_ready {
                    "This song has no isolated vocal stem"
                } else {
                    "Audio unavailable"
                },
                FontId::proportional(10.0),
                if self.audio_loading { GOLD } else { MUTED2 },
            );
        } else {
            let half = level_area.height() * 0.45;
            for x in (area.left().round() as i32)..(area.right().round() as i32) {
                let time = self.x_to_time(x as f32, layout);
                if time < 0.0 {
                    continue;
                }
                let index = (time * VOCAL_BINS_PER_SECOND as f64).floor() as usize;
                let Some(peak) = self.vocal_peaks.get(index) else {
                    continue;
                };
                let amplitude = (peak / max_peak).sqrt().clamp(0.0, 1.0);
                let h = (amplitude * half).max(0.6);
                painter.line_segment(
                    [
                        Pos2::new(x as f32, center - h),
                        Pos2::new(x as f32, center + h),
                    ],
                    Stroke::new(1.0f32, Color32::from_rgba_unmultiplied(100, 200, 255, 120)),
                );
            }
        }

        // Raw pitch, frame by frame. No note grouping or smoothing is applied
        // here: simplifying the contour is the mapper's decision.
        let has_pitch = self.raw_pitch.iter().any(|value| value.is_some());
        if !has_pitch {
            painter.text(
                Pos2::new(pitch_area.left() + 8.0, pitch_area.center().y),
                Align2::LEFT_CENTER,
                if self.pitch_job.is_some() {
                    "Analyzing raw vocal pitch…"
                } else if self.audio_loading {
                    "Loading audio…"
                } else {
                    "No raw pitch for this song"
                },
                FontId::proportional(10.0),
                if self.pitch_job.is_some() || self.audio_loading {
                    GOLD
                } else {
                    MUTED2
                },
            );
        } else {
            // The raw pitch overview is independent of the piano-rail scroll.
            // Scale it to the full vocal track so the contour is always in frame.
            let mut pitch_low = self
                .raw_pitch
                .iter()
                .filter_map(|value| *value)
                .fold(f32::INFINITY, f32::min);
            let mut pitch_high = self
                .raw_pitch
                .iter()
                .filter_map(|value| *value)
                .fold(f32::NEG_INFINITY, f32::max);
            if !pitch_low.is_finite() || !pitch_high.is_finite() {
                pitch_low = 48.0;
                pitch_high = 72.0;
            }
            pitch_low -= 0.75;
            pitch_high += 0.75;
            if pitch_high - pitch_low < 1.0 {
                let center = (pitch_high + pitch_low) * 0.5;
                pitch_low = center - 0.5;
                pitch_high = center + 0.5;
            }
            let span = (pitch_high - pitch_low).max(1.0);
            let mut previous: Option<(usize, f32, f32)> = None;
            for (bin, value) in self.raw_pitch.iter().enumerate() {
                let Some(midi) = *value else {
                    previous = None;
                    continue;
                };
                let x = self.time_to_x(bin as f64 / VOCAL_BINS_PER_SECOND as f64, layout);
                if x < area.left() - 4.0 || x > area.right() + 4.0 {
                    previous = None;
                    continue;
                }
                let y = (pitch_area.top() + (pitch_high - midi) / span * pitch_area.height())
                    .clamp(pitch_area.top() + 2.0, pitch_area.bottom() - 2.0);
                let t = ((y - pitch_area.top()) / pitch_area.height().max(1.0)).clamp(0.0, 1.0);
                let color = pitch_color(t);
                let color = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], 235);
                if let Some((previous_bin, px, py)) = previous {
                    if bin.saturating_sub(previous_bin) <= 2 {
                        painter.line_segment(
                            [Pos2::new(px, py), Pos2::new(x, y)],
                            Stroke::new(3.0f32, Color32::from_rgba_unmultiplied(5, 8, 15, 180)),
                        );
                        painter.line_segment(
                            [Pos2::new(px, py), Pos2::new(x, y)],
                            Stroke::new(1.2f32, color),
                        );
                    }
                }
                painter.circle_filled(Pos2::new(x, y), 1.15, color);
                previous = Some((bin, x, y));
            }
        }

        // Hover readout: exact timestamp plus raw pitch and vocal level at the
        // cursor. The vertical gray cursor is drawn globally below.
        let hover = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|pos| area.contains(*pos));
        if let Some(pos) = hover {
            let x = pos.x.clamp(area.left(), area.right());
            let time = self.x_to_time(x, layout).clamp(0.0, self.duration);
            let pitch_bin = (time * VOCAL_BINS_PER_SECOND as f64).floor() as usize;
            let pitch = self
                .raw_pitch
                .get(pitch_bin)
                .and_then(|value| *value)
                .map(|midi| {
                    let rounded = midi.round();
                    let cents = ((midi - rounded) * 100.0).round();
                    format!("{} {:+.0}¢", note_name(rounded as i32), cents)
                })
                .unwrap_or_else(|| {
                    if self.pitch_job.is_some() {
                        "analyzing…".into()
                    } else {
                        "—".into()
                    }
                });
            let level = if self.vocal_peaks.is_empty() {
                0.0
            } else {
                let index = (time * VOCAL_BINS_PER_SECOND as f64).floor() as usize;
                self.vocal_peaks.get(index).copied().unwrap_or(0.0) / max_peak
            };
            let label = Rect::from_min_size(
                Pos2::new(area.left() + 8.0, area.bottom() - 23.0),
                Vec2::new(196.0, 18.0),
            );
            painter.rect_filled(label, 4.0, Color32::from_rgba_unmultiplied(5, 8, 15, 220));
            painter.text(
                label.left_center() + Vec2::new(6.0, 0.0),
                Align2::LEFT_CENTER,
                format!(
                    "{}  ·  {}  ·  {:.0}%",
                    format_time_precise(time),
                    pitch,
                    level * 100.0
                ),
                FontId::monospace(10.0),
                INK,
            );
        }

        let px = self.time_to_x(self.playhead, layout);
        if px >= area.left() && px <= area.right() {
            painter.line_segment(
                [Pos2::new(px, area.top()), Pos2::new(px, area.bottom())],
                Stroke::new(1.6f32, alpha(GOLD, 235)),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(px - 5.0, area.top()),
                    Pos2::new(px + 5.0, area.top()),
                    Pos2::new(px, area.top() + 8.0),
                ],
                GOLD,
                Stroke::NONE,
            ));
        }

        // Borders make the analysis strip read as a separate timeline lane.
        painter.line_segment(
            [area.left_top(), area.right_top()],
            Stroke::new(1.0f32, Color32::from_white_alpha(24)),
        );
        painter.line_segment(
            [area.left_bottom(), area.right_bottom()],
            Stroke::new(1.0f32, Color32::from_white_alpha(24)),
        );
    }

    fn draw_hover_cursor(&self, ui: &Ui, layout: &Layout) {
        if self.custom_word_open {
            return;
        }
        let Some(pos) = ui.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let area = layout.wave.union(layout.note_area).union(layout.ruler);
        if !area.contains(pos) {
            return;
        }
        let x = pos
            .x
            .clamp(layout.note_area.left(), layout.note_area.right());
        ui.painter().line_segment(
            [
                Pos2::new(x, layout.wave.top()),
                Pos2::new(x, layout.note_area.bottom()),
            ],
            Stroke::new(1.0f32, Color32::from_rgba_unmultiplied(185, 190, 202, 72)),
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
        p.line_segment(
            [ruler.left_top(), ruler.right_top()],
            Stroke::new(1.0f32, Color32::from_white_alpha(16)),
        );
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
            p.line_segment(
                [Pos2::new(x, ruler.top()), Pos2::new(x, ruler.top() + 7.0)],
                Stroke::new(1.0f32, Color32::from_white_alpha(35)),
            );
            p.text(
                Pos2::new(x + 5.0, ruler.center().y + 1.0),
                Align2::LEFT_CENTER,
                format_time(time),
                FontId::monospace(10.0),
                MUTED2,
            );
        }
        let px = self.time_to_x(self.playhead, layout);
        if px >= ruler.left() && px <= ruler.right() {
            p.line_segment(
                [
                    Pos2::new(px, ruler.top() + 12.0),
                    Pos2::new(px, ruler.bottom()),
                ],
                Stroke::new(2.0f32, GOLD),
            );
        }
        let response = ui.interact(ruler, Id::new("editor_ruler"), Sense::click_and_drag());
        if (response.clicked() || response.dragged()) && ui.input(|i| i.pointer.primary_down()) {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                let t = self
                    .x_to_time(pos.x.clamp(ruler.left(), ruler.right()), layout)
                    .clamp(0.0, self.duration);
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
        p.line_segment(
            [tray.left_top(), tray.right_top()],
            Stroke::new(1.0f32, Color32::from_white_alpha(18)),
        );
        let play = Rect::from_min_size(
            Pos2::new(tray.left() + 16.0, tray.top() + 8.0),
            Vec2::new(86.0, 34.0),
        );
        if button(
            ui,
            play,
            if self.playing { "PAUSE" } else { "PLAY" },
            true,
            true,
        ) {
            self.toggle_play(audio);
        }
        let stop = Rect::from_min_size(
            Pos2::new(play.right() + 7.0, play.top()),
            Vec2::new(62.0, 34.0),
        );
        if button(ui, stop, "STOP", false, true) {
            self.playing = false;
            if let Some(audio) = audio {
                audio.pause();
            }
            self.seek(0.0, audio);
        }
        p.text(
            Pos2::new(stop.right() + 18.0, tray.center().y),
            Align2::LEFT_CENTER,
            format!(
                "{} / {}",
                format_time(self.playhead),
                format_time(self.duration)
            ),
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
            if toggle(
                ui,
                r,
                &format!("{speed:.2}×")
                    .replace("1.00×", "1×")
                    .replace("0.50×", "0.5×")
                    .replace("1.50×", "1.5×"),
                (self.speed - speed).abs() < 0.01,
            ) {
                self.speed = speed;
                if let Some(audio) = audio {
                    audio.set_rate(speed);
                }
            }
            x += 48.0;
        }
        let vocals = Rect::from_min_size(
            Pos2::new(tray.right() - 290.0, tray.top() + 8.0),
            Vec2::new(94.0, 34.0),
        );
        if toggle(
            ui,
            vocals,
            if self.vocals_on {
                "VOCALS ON"
            } else {
                "VOCALS OFF"
            },
            self.vocals_on,
        ) {
            self.vocals_on = !self.vocals_on;
            self.apply_editor_volumes(audio);
        }
        let backing = Rect::from_min_size(
            Pos2::new(vocals.right() + 7.0, vocals.top()),
            Vec2::new(100.0, 34.0),
        );
        if toggle(
            ui,
            backing,
            if self.backing_on {
                "BACKING ON"
            } else {
                "BACKING OFF"
            },
            self.backing_on,
        ) {
            self.backing_on = !self.backing_on;
            self.apply_editor_volumes(audio);
        }
        let hint = Rect::from_min_size(
            Pos2::new(tray.right() - 430.0, tray.top() + 8.0),
            Vec2::new(124.0, 34.0),
        );
        p.text(
            hint.center(),
            Align2::CENTER_CENTER,
            if self.audio_ready {
                "AUDIO READY"
            } else {
                "CHART PREVIEW"
            },
            FontId::monospace(10.0),
            if self.audio_ready { MINT } else { GOLD },
        );
        let _ = rect;
    }

    fn draw_word_bank(
        &mut self,
        ui: &mut Ui,
        p: &egui::Painter,
        layout: &Layout,
        audio: &Option<AudioEngine>,
    ) {
        let bank = layout.bank;
        p.rect_filled(bank, 0.0, Color32::from_rgb(8, 12, 21));
        p.line_segment(
            [bank.left_top(), bank.right_top()],
            Stroke::new(1.0f32, Color32::from_white_alpha(20)),
        );
        let remaining = self.word_bank.iter().filter(|word| !word.placed).count();
        p.text(
            Pos2::new(bank.left() + 16.0, bank.top() + 10.0),
            Align2::LEFT_TOP,
            "WORD BANK",
            FontId::monospace(10.0),
            MUTED2,
        );
        p.text(
            Pos2::new(bank.left() + 94.0, bank.top() + 10.0),
            Align2::LEFT_TOP,
            format!("{remaining} remaining · drag a word anywhere on the level"),
            FontId::proportional(10.0),
            MUTED,
        );
        let add = Rect::from_min_size(
            Pos2::new(bank.right() - 50.0, bank.top() + 10.0),
            Vec2::new(34.0, 34.0),
        );
        if paint_icon_button(
            ui,
            add,
            EditorIcon::Plus,
            self.custom_word_open,
            true,
            "Add a custom word to the front of the bank",
        ) {
            self.custom_word_open = true;
            self.custom_word_input.clear();
            self.custom_word_focus = true;
            self.status = "Type a custom word. It will go to the front of the bank.".into();
        }
        if self.word_bank.is_empty() {
            p.text(
                Pos2::new(bank.left() + 16.0, bank.top() + 38.0),
                Align2::LEFT_TOP,
                "No lyric words were found. Draw the melody, add custom words with + if needed, then submit.",
                FontId::proportional(12.0),
                GOLD,
            );
            return;
        }
        let inner = Rect::from_min_max(
            Pos2::new(bank.left() + 12.0, bank.top() + 32.0),
            Pos2::new(bank.right() - 62.0, bank.bottom() - 8.0),
        );
        let mut chip_actions: Vec<(usize, bool)> = Vec::new();
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("editor_word_bank_scroll")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut last_line: Option<usize> = None;
                        for (i, token) in self.word_bank.iter().enumerate() {
                            if token.placed {
                                continue;
                            }
                            if let Some(line) = token.line {
                                if last_line != Some(line) {
                                    if last_line.is_some() {
                                        let (sep, _) = ui.allocate_exact_size(
                                            Vec2::new(8.0, 34.0),
                                            Sense::hover(),
                                        );
                                        let painter = ui.painter();
                                        painter.line_segment(
                                            [
                                                sep.center_top() + Vec2::new(4.0, 3.0),
                                                sep.center_bottom() - Vec2::new(4.0, 3.0),
                                            ],
                                            Stroke::new(1.0f32, Color32::from_white_alpha(25)),
                                        );
                                    }
                                    last_line = Some(line);
                                }
                            }
                            let width =
                                (token.text.chars().count() as f32 * 8.0 + 26.0).clamp(48.0, 150.0);
                            let (chip, response) = ui.allocate_exact_size(
                                Vec2::new(width, 34.0),
                                Sense::click_and_drag(),
                            );
                            let active = self.pending_word == Some(i);
                            let custom = token.custom_id.is_some();
                            let fill = if active { alpha(MINT, 44) } else { PANEL2 };
                            let border = if active {
                                MINT
                            } else if custom {
                                alpha(GOLD, 120)
                            } else {
                                Color32::from_white_alpha(30)
                            };
                            let painter = ui.painter();
                            painter.rect_filled(chip, 9.0, fill);
                            painter.rect_stroke(
                                chip,
                                9.0,
                                Stroke::new(1.0f32, border),
                                StrokeKind::Inside,
                            );
                            painter.text(
                                chip.center(),
                                Align2::CENTER_CENTER,
                                &token.text,
                                FontId::proportional(12.0),
                                if active { MINT } else { INK },
                            );
                            if let Some(line) = token.line {
                                painter.text(
                                    chip.right_top() + Vec2::new(-6.0, 3.0),
                                    Align2::RIGHT_TOP,
                                    format!("{}", line + 1),
                                    FontId::monospace(5.5),
                                    MUTED2,
                                );
                            } else {
                                painter.text(
                                    chip.right_top() + Vec2::new(-6.0, 3.0),
                                    Align2::RIGHT_TOP,
                                    "NEW",
                                    FontId::monospace(5.5),
                                    GOLD,
                                );
                            }
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
                self.status = "Drag the word anywhere in the level; notes are only a guide.".into();
            } else {
                self.pending_word = if self.pending_word == Some(token) {
                    None
                } else {
                    Some(token)
                };
                self.status = if self.pending_word.is_some() {
                    "Word armed — click anywhere in the level to place it.".into()
                } else {
                    "Word selection cleared.".into()
                };
            }
        }

        // Ghost follows the pointer during a bank drag. The bank interaction
        // owns the drag, so this paints above the canvas on the tooltip layer.
        if let Some(token) = self.word_drag {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                let ghost = ui
                    .ctx()
                    .layer_painter(LayerId::new(Order::Tooltip, Id::new("editor_word_ghost")));
                let text = &self.word_bank[token].text;
                let width = (text.chars().count() as f32 * 8.0 + 30.0).clamp(54.0, 160.0);
                let rect =
                    Rect::from_center_size(pos + Vec2::new(0.0, -18.0), Vec2::new(width, 32.0));
                ghost.rect_filled(
                    rect,
                    9.0,
                    Color32::from_rgba_unmultiplied(124, 247, 196, 235),
                );
                ghost.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    text,
                    FontId::proportional(12.0),
                    Color32::from_rgb(5, 18, 13),
                );
                if layout.note_area.contains(pos) {
                    ghost.rect_stroke(
                        layout.note_area,
                        0.0,
                        Stroke::new(1.0f32, alpha(MINT, 65)),
                        StrokeKind::Inside,
                    );
                }
            }
            if ui.input(|i| !i.pointer.any_down()) {
                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                    if layout.note_area.contains(pos) {
                        self.place_word(token, pos, layout, audio);
                    } else {
                        self.status = "Word returned to the bank.".into();
                    }
                }
                self.word_drag = None;
            }
        }
    }

    fn draw_custom_word_dialog(&mut self, ctx: &egui::Context) {
        if !self.custom_word_open {
            return;
        }
        let mut open = self.custom_word_open;
        let mut add_clicked = false;
        let mut cancel_clicked = false;
        egui::Window::new("Add custom word")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.label(
                    RichText::new(
                        "The word goes to the front of the bank. Place it anywhere in the level as a decoration.",
                    )
                    .size(10.0)
                    .color(MUTED),
                );
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.custom_word_input)
                        .hint_text("custom word")
                        .desired_width(f32::INFINITY),
                );
                if self.custom_word_focus {
                    response.request_focus();
                    self.custom_word_focus = false;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !self.custom_word_input.trim().is_empty()
                {
                    add_clicked = true;
                }
                let _ = response;
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.custom_word_input.trim().is_empty(),
                            egui::Button::new("ADD"),
                        )
                        .clicked()
                    {
                        add_clicked = true;
                    }
                    if ui.button("CANCEL").clicked() {
                        cancel_clicked = true;
                    }
                });
            });
        if !open || cancel_clicked {
            self.custom_word_open = false;
            self.custom_word_input.clear();
            return;
        }
        if add_clicked {
            self.add_custom_word();
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
        p.rect_stroke(
            r,
            12.0,
            Stroke::new(1.0f32, Color32::from_white_alpha(20)),
            StrokeKind::Inside,
        );
        let mut y = r.top() + 14.0;
        let x = r.left() + 14.0;
        p.text(
            Pos2::new(x, y),
            Align2::LEFT_TOP,
            "MAPPING",
            FontId::monospace(10.0),
            MUTED2,
        );
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
            format!(
                "{} notes · {} connectors · {} lines",
                self.notes.len(),
                self.ribbon_count(),
                self.song.lines.len()
            ),
            FontId::monospace(10.0),
            MUTED,
        );
        y += 24.0;
        if !self.suggestions.is_empty() {
            let now = ui.input(|i| i.time);
            if self
                .auto_map_armed
                .is_some_and(|at| now - at >= AUTO_MAP_CONFIRM_SECONDS)
            {
                self.auto_map_armed = None;
            }
            let auto = Rect::from_min_size(Pos2::new(x, y), Vec2::new(r.width() - 28.0, 28.0));
            let label = if self.auto_map_armed.is_some() {
                format!("REPLACE {} NOTES?", self.notes.len())
            } else {
                "AUTO-MAP".to_string()
            };
            if button(ui, auto, &label, self.auto_map_armed.is_some(), true) {
                *event = EditorEvent::AutoMap;
            }
            y += 36.0;
        }
        p.line_segment(
            [Pos2::new(x, y), Pos2::new(r.right() - 14.0, y)],
            Stroke::new(1.0f32, Color32::from_white_alpha(18)),
        );
        y += 12.0;

        if let Some(index) = self.selected_word {
            let word = self.words.get(index).cloned();
            if let Some(word) = word {
                p.text(
                    Pos2::new(x, y),
                    Align2::LEFT_TOP,
                    "WORD DECORATION",
                    FontId::monospace(10.0),
                    MUTED2,
                );
                y += 18.0;
                p.text(
                    Pos2::new(x, y),
                    Align2::LEFT_TOP,
                    format!("“{}”", word.text),
                    FontId::proportional(20.0),
                    GOLD,
                );
                y += 30.0;
                p.text(
                    Pos2::new(x, y),
                    Align2::LEFT_TOP,
                    format!(
                        "{} · lane {:.0}%",
                        format_time(word.start),
                        word.y.clamp(0.0, 1.0) * 100.0
                    ),
                    FontId::monospace(10.0),
                    MUTED,
                );
                y += 18.0;
                p.text(
                    Pos2::new(x, y),
                    Align2::LEFT_TOP,
                    "Decorative: not attached to a note.",
                    FontId::proportional(10.0),
                    MUTED2,
                );
                y += 28.0;
                let delete =
                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(r.width() - 28.0, 30.0));
                if button(ui, delete, "DELETE WORD", false, true) {
                    self.delete_selected();
                }
                y += 40.0;
            }
        } else if let Some(index) = self.selected_ribbon {
            if let Some(link) = self.ribbon_links.get(index).copied() {
                p.text(
                    Pos2::new(x, y),
                    Align2::LEFT_TOP,
                    "CONNECTOR",
                    FontId::monospace(7.0),
                    MUTED2,
                );
                y += 16.0;
                let from = self
                    .notes
                    .get(link.from)
                    .map(|note| format!("{} at {}", note_name(note.midi), format_time(note.start)))
                    .unwrap_or_else(|| format!("note {}", link.from + 1));
                let to = self
                    .notes
                    .get(link.to)
                    .map(|note| format!("{} at {}", note_name(note.midi), format_time(note.start)))
                    .unwrap_or_else(|| format!("note {}", link.to + 1));
                p.text(
                    Pos2::new(x, y),
                    Align2::LEFT_TOP,
                    format!("{from}  ↔  {to}"),
                    FontId::proportional(12.0),
                    GOLD,
                );
                y += 26.0;
                p.text(
                    Pos2::new(x, y),
                    Align2::LEFT_TOP,
                    "Click the line to select it; Delete removes it.",
                    FontId::proportional(10.0),
                    MUTED2,
                );
                y += 24.0;
                let delete =
                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(r.width() - 28.0, 30.0));
                if button(ui, delete, "DELETE CONNECTOR", false, true) {
                    self.delete_selected();
                }
                y += 42.0;
            } else {
                p.text(
                    Pos2::new(x, y),
                    Align2::LEFT_TOP,
                    "Select a note, connector, or word.",
                    FontId::proportional(11.0),
                    MUTED,
                );
                y += 34.0;
            }
        } else if !self.selected.is_empty() {
            let count = self.selected.len();
            p.text(
                Pos2::new(x, y),
                Align2::LEFT_TOP,
                if count == 1 {
                    "SELECTED NOTE"
                } else {
                    "SELECTED NOTES"
                },
                FontId::monospace(10.0),
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
                        FontId::monospace(10.0),
                        MUTED,
                    );
                    y += 18.0;
                    let connectors = self
                        .ribbon_links
                        .iter()
                        .filter(|link| link.from == index || link.to == index)
                        .count();
                    let label = match connectors {
                        0 => "no connectors".to_string(),
                        1 => "1 connector".to_string(),
                        count => format!("{count} connectors"),
                    };
                    p.text(
                        Pos2::new(x, y),
                        Align2::LEFT_TOP,
                        label,
                        FontId::proportional(13.0),
                        if connectors > 0 { MINT } else { MUTED2 },
                    );
                    y += 24.0;
                }
            } else {
                p.text(
                    Pos2::new(x, y),
                    Align2::LEFT_TOP,
                    format!("{count} notes selected"),
                    FontId::proportional(13.0),
                    MINT,
                );
                y += 24.0;
            }
            let bw = (r.width() - 38.0) / 2.0;
            for (i, label) in ["− OCT", "+ OCT"].iter().enumerate() {
                let b = Rect::from_min_size(
                    Pos2::new(x + i as f32 * (bw + 10.0), y),
                    Vec2::new(bw, 30.0),
                );
                if button(ui, b, label, false, true) {
                    self.transpose_selected(if i == 0 { -12 } else { 12 });
                }
            }
            y += 38.0;
            let detach = Rect::from_min_size(Pos2::new(x, y), Vec2::new(r.width() - 28.0, 28.0));
            let has_connectors = self.selected.iter().any(|index| {
                self.ribbon_links
                    .iter()
                    .any(|link| link.from == *index || link.to == *index)
            });
            if button(ui, detach, "DETACH CONNECTORS", false, has_connectors) && has_connectors {
                self.detach_selected_ribbons();
            }
            y += 36.0;
            let delete = Rect::from_min_size(Pos2::new(x, y), Vec2::new(r.width() - 28.0, 28.0));
            if button(ui, delete, "DELETE NOTE", false, true) {
                self.delete_selected();
            }
            y += 44.0;
        } else {
            p.text(
                Pos2::new(x, y),
                Align2::LEFT_TOP,
                "Select a note, connector, or word.",
                FontId::proportional(11.0),
                MUTED,
            );
            y += 34.0;
        }

        p.line_segment(
            [Pos2::new(x, y), Pos2::new(r.right() - 14.0, y)],
            Stroke::new(1.0f32, Color32::from_white_alpha(18)),
        );
        y += 12.0;
        p.text(
            Pos2::new(x, y),
            Align2::LEFT_TOP,
            "STATUS",
            FontId::monospace(10.0),
            MUTED2,
        );
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
        p.text(
            Pos2::new(x, y),
            Align2::LEFT_TOP,
            "SHORTCUTS",
            FontId::monospace(10.0),
            MUTED2,
        );
        y += 16.0;
        for line in [
            "1–5  tools",
            "Space  play / pause",
            "← →  seek 1s (Shift 5s)",
            "Right-click  move now line",
            "Ctrl+Z / ⇧Z  undo",
            "Ctrl+S  save draft",
            "Delete  remove selection",
            "Middle-drag  pan",
        ] {
            p.text(
                Pos2::new(x, y),
                Align2::LEFT_TOP,
                line,
                FontId::monospace(10.0),
                MUTED2,
            );
            y += 15.0;
        }
        let save = Rect::from_min_size(
            Pos2::new(x, r.bottom() - 96.0),
            Vec2::new(r.width() - 28.0, 32.0),
        );
        let export = Rect::from_min_size(
            Pos2::new(x, r.bottom() - 134.0),
            Vec2::new(r.width() - 28.0, 28.0),
        );
        let submit = Rect::from_min_size(
            Pos2::new(x, r.bottom() - 52.0),
            Vec2::new(r.width() - 28.0, 38.0),
        );
        let top_action = if self.map_draft {
            save.top()
        } else {
            export.top()
        };
        let ok = self.validate().is_ok();
        if let Err(reason) = self.validate() {
            let galley = p.layout(reason, FontId::proportional(10.0), PINK, r.width() - 28.0);
            let h = galley.size().y;
            p.galley(Pos2::new(x, top_action - h - 4.0), galley, PINK);
        } else {
            p.text(
                Pos2::new(x, top_action - 16.0),
                Align2::LEFT_BOTTOM,
                "Ready to submit.",
                FontId::proportional(10.0),
                MINT,
            );
        }
        if !self.map_draft && button(ui, export, "EXPORT MAP", false, true) {
            *event = EditorEvent::ExportMap;
        }
        if button(ui, save, "SAVE AS DRAFT", false, self.map_draft) {
            *event = EditorEvent::SaveDraft;
        }
        if button(ui, submit, "SUBMIT MAPPING", ok, ok) && ok {
            *event = EditorEvent::Submit;
        }
    }

    fn ribbon_count(&self) -> usize {
        self.ribbon_links.len()
    }

    fn word_progress(&self) -> f32 {
        if self.word_bank.is_empty() {
            if self.notes.is_empty() {
                0.0
            } else {
                1.0
            }
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
        (self.high - ((y - layout.note_area.top()) / lane_h).floor() as i32)
            .clamp(self.low, self.high)
    }

    fn word_rect(&self, word: &PlacedWord, layout: &Layout) -> Rect {
        let x = self.time_to_x(word.start, layout);
        let y =
            layout.note_area.top() + word.y.clamp(0.0, 1.0) * layout.note_area.height().max(1.0);
        let width = (word.text.chars().count() as f32 * 8.0 + 28.0).clamp(52.0, 190.0);
        Rect::from_center_size(Pos2::new(x, y), Vec2::new(width, 24.0))
    }

    fn hit_word(&self, pos: Pos2, layout: &Layout) -> Option<usize> {
        if !layout.note_area.contains(pos) {
            return None;
        }
        self.words
            .iter()
            .enumerate()
            .rev()
            .find(|(_, word)| self.word_rect(word, layout).contains(pos))
            .map(|(index, _)| index)
    }

    fn scroll_pitches(&mut self, semitones: i32) {
        let span = (self.high - self.low).clamp(12, 127);
        let low = (self.low + semitones).clamp(0, 127 - span);
        if low != self.low {
            self.low = low;
            self.high = low + span;
            self.status = format!(
                "Pitch view: {} – {}. Scroll the piano rail to move it.",
                note_name(self.low),
                note_name(self.high)
            );
        }
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

    fn note_edge_at(&self, pos: Pos2, layout: &Layout) -> Option<(usize, bool)> {
        self.note_edge_at_with(pos, layout, false)
    }

    /// The Note tool only grabs an edge when the press starts inside the note.
    /// Pressing exactly on the boundary therefore starts the next note instead
    /// of resizing the previous one, which keeps notes truly end to end.
    fn note_edge_at_inside(&self, pos: Pos2, layout: &Layout) -> Option<(usize, bool)> {
        self.note_edge_at_with(pos, layout, true)
    }

    fn note_edge_at_with(
        &self,
        pos: Pos2,
        layout: &Layout,
        inside_only: bool,
    ) -> Option<(usize, bool)> {
        if !layout.note_area.contains(pos) {
            return None;
        }
        let lane_h = layout.note_area.height() / (self.high - self.low + 1) as f32;
        let h = (lane_h * 0.62).clamp(6.0, 22.0);
        let mut best: Option<(f32, usize, bool)> = None;
        for (index, note) in self.notes.iter().enumerate().rev() {
            let y = self.midi_to_y(note.midi, layout);
            if (pos.y - y).abs() > h / 2.0 + 6.0 {
                continue;
            }
            let x0 = self.time_to_x(note.start, layout);
            let x1 = self.time_to_x(note.start + note.dur, layout);
            if inside_only && (pos.x <= x0 || pos.x >= x1) {
                continue;
            }
            let width = (x1 - x0).max(1.0);
            let tolerance = 12.0f32.min(width * 0.4).max(4.0);
            let left = (pos.x - x0).abs();
            let right = (pos.x - x1).abs();
            let (distance, is_left) = if left <= right {
                (left, true)
            } else {
                (right, false)
            };
            if distance <= tolerance
                && best
                    .map(|(best_distance, _, _)| distance < best_distance)
                    .unwrap_or(true)
            {
                best = Some((distance, index, is_left));
            }
        }
        best.map(|(_, index, left)| (index, left))
    }

    fn ribbon_dot_radius(&self, layout: &Layout) -> f32 {
        let lane_h = layout.note_area.height() / (self.high - self.low + 1) as f32;
        (lane_h * 0.62).clamp(6.0, 22.0) * 0.6
    }

    fn ribbon_dot_center(&self, index: usize, side: RibbonSide, layout: &Layout) -> Option<Pos2> {
        let note = self.notes.get(index)?;
        let time = match side {
            RibbonSide::Left => note.start,
            RibbonSide::Right => note.start + note.dur,
        };
        Some(Pos2::new(
            self.time_to_x(time, layout),
            self.midi_to_y(note.midi, layout),
        ))
    }

    fn ribbon_dot_at(&self, pos: Pos2, layout: &Layout) -> Option<(usize, RibbonSide)> {
        let radius = self.ribbon_dot_radius(layout);
        if !layout.note_area.expand(radius).contains(pos) {
            return None;
        }
        let mut best: Option<(f32, usize, RibbonSide)> = None;
        for (index, _) in self.notes.iter().enumerate().rev() {
            for side in [RibbonSide::Left, RibbonSide::Right] {
                let Some(center) = self.ribbon_dot_center(index, side, layout) else {
                    continue;
                };
                let distance = pos.distance(center);
                if distance <= radius + 2.0
                    && best
                        .map(|(best_distance, _, _)| distance < best_distance)
                        .unwrap_or(true)
                {
                    best = Some((distance, index, side));
                }
            }
        }
        best.map(|(_, index, side)| (index, side))
    }

    fn ribbon_line(&self, link: &RibbonLink, layout: &Layout) -> Option<(Pos2, Pos2)> {
        let start = self.ribbon_dot_center(link.from, link.from_side, layout)?;
        let end = self.ribbon_dot_center(link.to, link.to_side, layout)?;
        Some((start, end))
    }

    fn hit_ribbon(&self, pos: Pos2, layout: &Layout) -> Option<usize> {
        let tolerance = (self.ribbon_dot_radius(layout) * 0.9).max(6.0);
        for (index, link) in self.ribbon_links.iter().enumerate().rev() {
            let Some((start, end)) = self.ribbon_line(link, layout) else {
                continue;
            };
            if distance_to_segment(pos, start, end) <= tolerance {
                return Some(index);
            }
        }
        None
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
        if let Some(index) = self.hit_word(pos, layout) {
            self.words.remove(index);
            self.selected_word = None;
            self.selected_ribbon = None;
            self.refresh_word_bank();
            self.dirty = true;
            self.status = "Word decoration erased and returned to the bank.".into();
            return;
        }
        if let Some(index) = self.hit_note(pos, layout) {
            let mut remap = vec![None; self.notes.len()];
            self.notes.remove(index);
            for (old, slot) in remap.iter_mut().enumerate() {
                if old == index {
                    continue;
                }
                *slot = Some(if old < index { old } else { old - 1 });
            }
            self.remap_ribbon_links(&remap);
            self.selected.clear();
            self.selected_ribbon = None;
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
            let (backing, vocals) = self.mix_levels();
            audio.set_volumes(backing, vocals);
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            notes: self.notes.clone(),
            words: self.words.clone(),
            custom_words: self.custom_words.clone(),
            ribbon_links: self.ribbon_links.clone(),
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.notes = snapshot.notes;
        self.words = snapshot.words;
        self.custom_words = snapshot.custom_words;
        self.ribbon_links = snapshot.ribbon_links;
        self.selected.clear();
        self.selected_word = None;
        self.selected_ribbon = None;
        self.rebuild_word_bank();
        self.refresh_word_bank();
    }

    fn push_history(&mut self) {
        self.history.push(self.snapshot());
        if self.history.len() > 100 {
            self.history.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    fn undo(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.redo.push(self.snapshot());
            self.restore(previous);
            self.dirty = true;
            self.status = "Undo.".into();
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.history.push(self.snapshot());
            self.restore(next);
            self.dirty = true;
            self.status = "Redo.".into();
        }
    }

    fn delete_selected(&mut self) {
        if let Some(index) = self.selected_ribbon.take() {
            if index < self.ribbon_links.len() {
                self.push_history();
                self.ribbon_links.remove(index);
                self.dirty = true;
                self.status = "Connector deleted.".into();
            }
            return;
        }
        if let Some(index) = self.selected_word {
            self.push_history();
            if index < self.words.len() {
                self.words.remove(index);
            }
            self.selected_word = None;
            self.refresh_word_bank();
            self.dirty = true;
            self.status = "Word decoration deleted and returned to the bank.".into();
            return;
        }
        if self.selected.is_empty() {
            return;
        }
        self.push_history();
        let targets = self.selected.clone();
        let mut remap = vec![None; self.notes.len()];
        let mut notes = Vec::with_capacity(self.notes.len());
        for (old_index, note) in self.notes.iter().enumerate() {
            if targets.contains(&old_index) {
                continue;
            }
            remap[old_index] = Some(notes.len());
            notes.push(note.clone());
        }
        self.notes = notes;
        self.remap_ribbon_links(&remap);
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

    fn detach_selected_ribbons(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        self.push_history();
        let selected = self.selected.clone();
        self.ribbon_links
            .retain(|link| !selected.contains(&link.from) && !selected.contains(&link.to));
        self.selected_ribbon = None;
        self.dirty = true;
        self.status = "Selected notes detached from their connectors.".into();
    }

    fn place_word(
        &mut self,
        token_index: usize,
        pos: Pos2,
        layout: &Layout,
        audio: &Option<AudioEngine>,
    ) {
        let Some(token) = self.word_bank.get(token_index).cloned() else {
            return;
        };
        self.push_history();
        self.selected_ribbon = None;
        let (start, y) = if let Some(index) = self.hit_note(pos, layout) {
            let note = &self.notes[index];
            let y = ((self.midi_to_y(note.midi, layout) - layout.note_area.top())
                / layout.note_area.height().max(1.0))
            .clamp(0.02, 0.98);
            (note.start, y)
        } else {
            (
                self.snap_time(self.x_to_time(pos.x, layout)),
                ((pos.y - layout.note_area.top()) / layout.note_area.height().max(1.0))
                    .clamp(0.02, 0.98),
            )
        };
        self.words.push(PlacedWord {
            text: token.text.clone(),
            start,
            y,
            line: token.line,
            word_index: token.index,
            custom_id: token.custom_id,
        });
        self.selected.clear();
        self.selected_word = Some(self.words.len() - 1);
        self.pending_word = None;
        self.word_drag = None;
        self.refresh_word_bank();
        self.dirty = true;
        self.status = format!(
            "Placed “{}” as a decorative word. Drag it anywhere with SELECT.",
            token.text
        );
        if self.playing {
            if let Some(audio) = audio {
                audio.play();
            }
        }
    }

    fn add_custom_word(&mut self) {
        let text = self.custom_word_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.push_history();
        let id = self.next_custom_id;
        self.next_custom_id = self.next_custom_id.saturating_add(1);
        self.custom_words.insert(
            0,
            CustomWord {
                id,
                text: text.clone(),
            },
        );
        self.rebuild_word_bank();
        self.custom_word_input.clear();
        self.custom_word_open = false;
        self.dirty = true;
        self.status = format!("Added custom word “{text}” to the front of the bank.");
    }

    fn rebuild_word_bank(&mut self) {
        self.word_bank = build_word_bank(&self.song.lines, &self.custom_words, &self.words);
        self.pending_word = None;
        self.word_drag = None;
    }

    fn refresh_word_bank(&mut self) {
        for token in &mut self.word_bank {
            token.placed = self.words.iter().any(|word| match token.custom_id {
                Some(id) => word.custom_id == Some(id),
                None => word.line == token.line && word.word_index == token.index,
            });
        }
    }

    fn sort_notes(&mut self) {
        let mut order: Vec<usize> = (0..self.notes.len()).collect();
        order.sort_by(|&a, &b| note_order(&self.notes[a], &self.notes[b]));
        let mut remap = vec![None; self.notes.len()];
        for (new_index, &old_index) in order.iter().enumerate() {
            remap[old_index] = Some(new_index);
        }
        let old_notes = std::mem::take(&mut self.notes);
        self.notes = order
            .iter()
            .map(|&index| old_notes[index].clone())
            .collect();
        self.selected = self
            .selected
            .iter()
            .filter_map(|&index| remap.get(index).copied().flatten())
            .collect();
        self.remap_ribbon_links(&remap);
        self.selected_ribbon = None;
        self.refresh_word_bank();
    }

    fn remap_ribbon_links(&mut self, remap: &[Option<usize>]) {
        self.ribbon_links.retain_mut(|link| {
            let (Some(from), Some(to)) = (
                remap.get(link.from).copied().flatten(),
                remap.get(link.to).copied().flatten(),
            ) else {
                return false;
            };
            link.from = from;
            link.to = to;
            from != to
        });
    }
}

fn distance_to_segment(point: Pos2, start: Pos2, end: Pos2) -> f32 {
    let segment = end - start;
    let length_sq = segment.length_sq();
    if length_sq <= f32::EPSILON {
        return point.distance(start);
    }
    let t = ((point - start).dot(segment) / length_sq).clamp(0.0, 1.0);
    point.distance(start + segment * t)
}

fn note_order(a: &Note, b: &Note) -> std::cmp::Ordering {
    a.start
        .partial_cmp(&b.start)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a.midi.cmp(&b.midi))
}

fn legacy_key(note: &Note) -> Option<(usize, Option<usize>, String)> {
    let word = note.word.clone()?;
    Some((note.line.unwrap_or(usize::MAX), note.word_index, word))
}

fn word_y_for_midi(song: &Song, midi: i32) -> f32 {
    let low = if song.high > song.low { song.low } else { 48 };
    let high = if song.high > song.low { song.high } else { 84 };
    (1.0 - (midi - low) as f32 / (high - low).max(1) as f32).clamp(0.04, 0.96)
}

/// Connectors implied by legacy `Note.ribbon` groups: consecutive notes of
/// one group close enough in time become one link.
pub(crate) fn links_from_legacy_notes(notes: &[Note]) -> Vec<RibbonLink> {
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by(|&a, &b| note_order(&notes[a], &notes[b]));
    let mut links = Vec::new();
    for pair in order.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let (anchor, next) = (&notes[from], &notes[to]);
        let gap = next.start - (anchor.start + anchor.dur);
        let limit = if anchor.ribbon.is_some() || next.ribbon.is_some() {
            3.0
        } else {
            0.85
        };
        if gap <= limit && anchor.shares_ribbon(next) {
            links.push(RibbonLink {
                from,
                to,
                from_side: RibbonSide::Right,
                to_side: RibbonSide::Left,
            });
        }
    }
    links
}

fn migrate_legacy_notes(
    song: &Song,
    notes: &mut [Note],
    words: &mut Vec<PlacedWord>,
    next_ribbon_id: &mut u32,
) {
    let mut seen_words: BTreeSet<(usize, Option<usize>, String)> = BTreeSet::new();
    let mut ribbons: BTreeMap<(usize, Option<usize>, String), u32> = BTreeMap::new();
    for note in notes.iter_mut() {
        let Some(key) = legacy_key(note) else {
            note.word = None;
            note.word_index = None;
            note.line = None;
            continue;
        };
        if seen_words.insert(key.clone()) {
            words.push(PlacedWord {
                text: key.2.clone(),
                start: note.start,
                y: word_y_for_midi(song, note.midi),
                line: note.line,
                word_index: note.word_index,
                custom_id: None,
            });
        }
        if note.ribbon.is_none() {
            let ribbon = *ribbons.entry(key).or_insert_with(|| {
                let ribbon = *next_ribbon_id;
                *next_ribbon_id = next_ribbon_id.saturating_add(1);
                ribbon
            });
            note.ribbon = Some(ribbon);
        }
        note.word = None;
        note.word_index = None;
        note.line = None;
    }
}

fn migrate_legacy_ribbons(notes: &mut [Note], next_ribbon_id: &mut u32) {
    let mut ribbons: BTreeMap<(usize, Option<usize>, String), u32> = BTreeMap::new();
    for note in notes.iter() {
        if note.ribbon.is_some() {
            continue;
        }
        if let Some(key) = legacy_key(note) {
            ribbons.entry(key).or_insert_with(|| {
                let ribbon = *next_ribbon_id;
                *next_ribbon_id = next_ribbon_id.saturating_add(1);
                ribbon
            });
        }
    }
    for note in notes.iter_mut() {
        if note.ribbon.is_none() {
            if let Some(key) = legacy_key(note) {
                if let Some(ribbon) = ribbons.get(&key) {
                    note.ribbon = Some(*ribbon);
                }
            }
        }
        note.word = None;
        note.word_index = None;
        note.line = None;
    }
}

/// Analyze raw vocal pitch in a worker thread. The result is deliberately not
/// smoothed or reduced to note groups: the mapper decides how to simplify it.
pub fn spawn_raw_pitch_track(
    ctx: &egui::Context,
    mono: Vec<f32>,
    bins_per_second: f32,
) -> Job<Vec<Option<f32>>> {
    Job::run("The pitch analysis", ctx, move || {
        Ok(analyze_raw_pitch(&mono, bins_per_second))
    })
}

fn analyze_raw_pitch(mono: &[f32], bins_per_second: f32) -> Vec<Option<f32>> {
    const WINDOW: usize = 512;
    if mono.len() < WINDOW {
        return Vec::new();
    }
    let bins =
        (mono.len() as f32 / RAW_PITCH_SAMPLE_RATE * bins_per_second.max(1.0)).ceil() as usize;
    let mut track = vec![None; bins];
    for (bin, slot) in track.iter_mut().enumerate() {
        let center_seconds = bin as f32 / bins_per_second.max(1.0);
        let start =
            ((center_seconds * RAW_PITCH_SAMPLE_RATE - WINDOW as f32 * 0.5).max(0.0)) as usize;
        if start + WINDOW > mono.len() {
            break;
        }
        let Some(reading) =
            crate::pitch::detect_pitch(&mono[start..start + WINDOW], RAW_PITCH_SAMPLE_RATE)
        else {
            continue;
        };
        if reading.clarity >= 0.18 && reading.rms >= 0.0015 {
            *slot = Some(reading.midi);
        }
    }
    track
}

fn format_time_precise(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    let minutes = (seconds / 60.0).floor() as i64;
    let rest = seconds - minutes as f64 * 60.0;
    format!("{minutes}:{rest:05.2}")
}

fn build_word_bank(
    lines: &[LyricLine],
    custom_words: &[CustomWord],
    words: &[PlacedWord],
) -> Vec<WordToken> {
    let mut bank = Vec::new();
    // Custom words are inserted at the front by the + button and stay first.
    for custom in custom_words {
        bank.push(WordToken {
            text: custom.text.clone(),
            line: None,
            index: None,
            custom_id: Some(custom.id),
            start: 0.0,
            placed: words.iter().any(|word| word.custom_id == Some(custom.id)),
        });
    }
    let mut lyric_tokens = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        for (word_index, (text, start, _end)) in split_line_words(line).into_iter().enumerate() {
            lyric_tokens.push(WordToken {
                text,
                line: Some(line_index),
                index: Some(word_index),
                custom_id: None,
                start,
                placed: false,
            });
        }
    }
    lyric_tokens.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.index.cmp(&b.index))
    });
    for mut token in lyric_tokens {
        token.placed = words
            .iter()
            .any(|word| word.line == token.line && word.word_index == token.index);
        bank.push(token);
    }
    bank
}

fn paint_icon_button(
    ui: &mut Ui,
    rect: Rect,
    icon: EditorIcon,
    active: bool,
    enabled: bool,
    tooltip: &str,
) -> bool {
    let response = ui
        .interact(
            rect,
            Id::new((
                "editor_icon",
                icon as u8,
                rect.left() as i32,
                rect.top() as i32,
            )),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        )
        .on_hover_text(tooltip);
    let p = ui.painter();
    let fill = if active {
        Color32::from_rgba_unmultiplied(124, 247, 196, 26)
    } else if response.hovered() && enabled {
        Color32::from_rgba_unmultiplied(255, 255, 255, 13)
    } else {
        Color32::from_rgba_unmultiplied(255, 255, 255, 6)
    };
    p.rect_filled(
        rect,
        8.0,
        if enabled {
            fill
        } else {
            Color32::from_white_alpha(4)
        },
    );
    p.rect_stroke(
        rect,
        8.0,
        Stroke::new(
            1.0f32,
            if active {
                alpha(MINT, 95)
            } else {
                Color32::from_white_alpha(24)
            },
        ),
        StrokeKind::Inside,
    );
    let color = if !enabled {
        MUTED2
    } else if active {
        MINT
    } else {
        INK
    };
    paint_editor_icon(p, rect.shrink(5.0), icon, color, active);
    enabled && response.clicked()
}

fn paint_editor_icon(
    p: &egui::Painter,
    rect: Rect,
    icon: EditorIcon,
    color: Color32,
    active: bool,
) {
    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.44;
    let stroke = Stroke::new(1.9f32, color);
    match icon {
        EditorIcon::Pan => {
            p.line_segment([c - Vec2::new(s, 0.0), c + Vec2::new(s, 0.0)], stroke);
            p.line_segment([c - Vec2::new(0.0, s), c + Vec2::new(0.0, s)], stroke);
            for tip in [
                c + Vec2::new(s, 0.0),
                c - Vec2::new(s, 0.0),
                c + Vec2::new(0.0, s),
                c - Vec2::new(0.0, s),
            ] {
                let dir = (tip - c).normalized() * (s * 0.32);
                let side = Vec2::new(-dir.y, dir.x) * 0.5;
                p.line_segment([tip, tip - dir + side], stroke);
                p.line_segment([tip, tip - dir - side], stroke);
            }
        }
        EditorIcon::Note => {
            p.line_segment(
                [
                    c + Vec2::new(s * 0.48, -s),
                    c + Vec2::new(s * 0.48, s * 0.48),
                ],
                stroke,
            );
            p.rect_filled(
                Rect::from_center_size(c + Vec2::new(0.0, s * 0.5), Vec2::new(s * 1.25, s * 0.62)),
                s * 0.3,
                color,
            );
        }
        EditorIcon::Ribbon => {
            p.rect_filled(
                Rect::from_center_size(
                    c + Vec2::new(-s * 0.56, 0.0),
                    Vec2::new(s * 0.92, s * 0.46),
                ),
                s * 0.22,
                color,
            );
            p.rect_filled(
                Rect::from_center_size(c + Vec2::new(s * 0.56, 0.0), Vec2::new(s * 0.92, s * 0.46)),
                s * 0.22,
                color,
            );
            p.line_segment(
                [c + Vec2::new(-s * 0.12, 0.0), c + Vec2::new(s * 0.12, 0.0)],
                Stroke::new(1.5f32, color),
            );
        }
        EditorIcon::Eraser => {
            let points = vec![
                Pos2::new(
                    rect.left() + rect.width() * 0.12,
                    rect.bottom() - rect.height() * 0.12,
                ),
                Pos2::new(
                    rect.left() + rect.width() * 0.56,
                    rect.top() + rect.height() * 0.1,
                ),
                Pos2::new(
                    rect.right() - rect.width() * 0.06,
                    rect.top() + rect.height() * 0.32,
                ),
                Pos2::new(
                    rect.right() - rect.width() * 0.42,
                    rect.bottom() - rect.height() * 0.04,
                ),
            ];
            p.add(egui::Shape::convex_polygon(
                points,
                alpha(color, 80),
                Stroke::new(1.4f32, color),
            ));
            p.line_segment(
                [
                    Pos2::new(
                        rect.left() + rect.width() * 0.4,
                        rect.bottom() - rect.height() * 0.1,
                    ),
                    Pos2::new(
                        rect.right() - rect.width() * 0.22,
                        rect.top() + rect.height() * 0.2,
                    ),
                ],
                Stroke::new(1.0f32, color),
            );
        }
        EditorIcon::Select => {
            let points = vec![
                c + Vec2::new(-s * 0.55, -s),
                c + Vec2::new(-s * 0.55, s * 0.85),
                c + Vec2::new(-s * 0.08, s * 0.4),
                c + Vec2::new(s * 0.55, s * 0.28),
            ];
            p.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
        }
        EditorIcon::Snap => {
            for row in -1..=1 {
                for col in -1..=1 {
                    let point = c + Vec2::new(col as f32 * s * 0.8, row as f32 * s * 0.8);
                    let radius = if row == 0 && col == 0 { 2.2 } else { 1.3 };
                    p.circle_filled(
                        point,
                        radius,
                        if row == 0 && col == 0 { MINT } else { color },
                    );
                }
            }
            p.line_segment(
                [c - Vec2::new(s * 1.0, 0.0), c + Vec2::new(s * 1.0, 0.0)],
                Stroke::new(0.8f32, alpha(color, 80)),
            );
        }
        EditorIcon::Suggestions => {
            p.rect_stroke(
                Rect::from_center_size(
                    c + Vec2::new(-s * 0.16, -s * 0.16),
                    Vec2::new(s * 1.5, s * 1.1),
                ),
                3.0,
                Stroke::new(1.2f32, alpha(color, 110)),
                StrokeKind::Inside,
            );
            p.rect_stroke(
                Rect::from_center_size(
                    c + Vec2::new(s * 0.16, s * 0.16),
                    Vec2::new(s * 1.5, s * 1.1),
                ),
                3.0,
                Stroke::new(1.5f32, color),
                StrokeKind::Inside,
            );
        }
        EditorIcon::Follow => {
            p.circle_stroke(c, s * 0.92, Stroke::new(1.5f32, color));
            p.line_segment(
                [c - Vec2::new(s * 1.15, 0.0), c - Vec2::new(s * 0.5, 0.0)],
                stroke,
            );
            p.line_segment(
                [c + Vec2::new(s * 0.5, 0.0), c + Vec2::new(s * 1.15, 0.0)],
                stroke,
            );
            p.line_segment(
                [c - Vec2::new(0.0, s * 1.15), c - Vec2::new(0.0, s * 0.5)],
                stroke,
            );
            p.line_segment(
                [c + Vec2::new(0.0, s * 0.5), c + Vec2::new(0.0, s * 1.15)],
                stroke,
            );
            p.circle_filled(c, 1.8, if active { color } else { MINT });
        }
        EditorIcon::ZoomOut | EditorIcon::ZoomIn => {
            let lens = c + Vec2::new(-s * 0.18, -s * 0.18);
            p.circle_stroke(lens, s * 0.62, Stroke::new(1.6f32, color));
            p.line_segment(
                [
                    lens + Vec2::new(s * 0.46, s * 0.46),
                    c + Vec2::new(s * 1.1, s * 1.1),
                ],
                Stroke::new(1.8f32, color),
            );
            p.line_segment(
                [
                    lens - Vec2::new(s * 0.33, 0.0),
                    lens + Vec2::new(s * 0.33, 0.0),
                ],
                Stroke::new(1.5f32, color),
            );
            if matches!(icon, EditorIcon::ZoomIn) {
                p.line_segment(
                    [
                        lens - Vec2::new(0.0, s * 0.33),
                        lens + Vec2::new(0.0, s * 0.33),
                    ],
                    Stroke::new(1.5f32, color),
                );
            }
        }
        EditorIcon::Plus => {
            p.line_segment(
                [c - Vec2::new(s * 0.8, 0.0), c + Vec2::new(s * 0.8, 0.0)],
                stroke,
            );
            p.line_segment(
                [c - Vec2::new(0.0, s * 0.8), c + Vec2::new(0.0, s * 0.8)],
                stroke,
            );
        }
        EditorIcon::Back => {
            p.line_segment(
                [c + Vec2::new(s * 0.7, 0.0), c - Vec2::new(s * 0.7, 0.0)],
                stroke,
            );
            p.line_segment(
                [
                    c - Vec2::new(s * 0.7, 0.0),
                    c - Vec2::new(s * 0.15, -s * 0.55),
                ],
                stroke,
            );
            p.line_segment(
                [
                    c - Vec2::new(s * 0.7, 0.0),
                    c - Vec2::new(s * 0.15, s * 0.55),
                ],
                stroke,
            );
        }
        EditorIcon::Submit => {
            p.circle_stroke(c, s * 0.95, Stroke::new(1.5f32, color));
            p.line_segment(
                [
                    c + Vec2::new(-s * 0.45, s * 0.02),
                    c + Vec2::new(-s * 0.08, s * 0.4),
                ],
                Stroke::new(1.9f32, color),
            );
            p.line_segment(
                [
                    c + Vec2::new(-s * 0.08, s * 0.4),
                    c + Vec2::new(s * 0.5, -s * 0.38),
                ],
                Stroke::new(1.9f32, color),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor() -> Editor {
        let song: Song = serde_json::from_value(serde_json::json!({
            "id": "test",
            "title": "Test",
            "low": 48,
            "high": 72,
            "audio": {"backing": "backing.mp3"}
        }))
        .expect("test song");
        Editor::new(
            song,
            Vec::new(),
            Vec::new(),
            30.0,
            PathBuf::from("test-level.json"),
            true,
        )
    }

    fn note(start: f64) -> Note {
        Note {
            start,
            dur: MIN_NOTE_DURATION,
            midi: 60,
            word: None,
            word_index: None,
            line: None,
            vel: None,
            ribbon: None,
        }
    }

    #[test]
    fn drafts_and_levels_save_to_the_selected_map_file() {
        let dir =
            std::env::temp_dir().join(format!("karaokemachine-editor-map-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("Level.json");
        let song: Song = serde_json::from_value(serde_json::json!({
            "id": "test",
            "title": "Test",
            "low": 48,
            "high": 72,
            "audio": {"backing": "backing.mp3"}
        }))
        .expect("test song");
        let mut editor = Editor::new(song, Vec::new(), Vec::new(), 20.0, path.clone(), true);
        editor.notes.push(note(1.0));
        editor.save_draft().expect("save draft");
        let draft = crate::library::load_map_file(&path).expect("draft file");
        assert!(draft.draft);
        assert_eq!(draft.notes.len(), 1);

        editor.submit().expect("submit level");
        let level = crate::library::load_map_file(&path).expect("level file");
        assert!(!level.draft);
        assert_eq!(level.notes.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_note_can_be_dragged_right_to_left() {
        let mut editor = editor();
        editor.snap = false;
        editor.notes.push(note(5.0));
        editor.update_new_note(0, 5.0, 2.0);
        assert!((editor.notes[0].start - 2.0).abs() < 1e-9);
        assert!((editor.notes[0].dur - 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_new_note_keeps_the_pointer_span() {
        let mut editor = editor();
        editor.snap = false;
        editor.notes.push(note(2.0));
        editor.update_new_note(0, 2.0, 2.4);
        assert!((editor.notes[0].start - 2.0).abs() < 1e-9);
        assert!((editor.notes[0].dur - 0.4).abs() < 1e-9);
    }

    #[test]
    fn a_new_note_has_a_tiny_minimum_not_a_full_beat() {
        let mut editor = editor();
        editor.snap = false;
        editor.notes.push(note(2.0));
        editor.update_new_note(0, 2.0, 2.0);
        assert!((editor.notes[0].start - 2.0).abs() < 1e-9);
        assert!((editor.notes[0].dur - MIN_NOTE_DURATION).abs() < 1e-9);
    }

    #[test]
    fn legacy_ribbon_groups_become_explicit_links() {
        let song: Song = serde_json::from_value(serde_json::json!({
            "id": "legacy",
            "title": "Legacy",
            "mapped": true,
            "low": 48,
            "high": 72,
            "audio": {"backing": "backing.mp3"},
            "notes": [
                {"start": 1.0, "dur": 0.5, "midi": 60, "ribbon": 4},
                {"start": 1.6, "dur": 0.5, "midi": 62, "ribbon": 4}
            ]
        }))
        .expect("legacy song");
        let initial = song.notes.clone();
        let editor = Editor::new(
            song,
            Vec::new(),
            initial,
            20.0,
            PathBuf::from("legacy-level.json"),
            false,
        );
        assert_eq!(editor.ribbon_links.len(), 1);
        assert_eq!(editor.ribbon_links[0].from, 0);
        assert_eq!(editor.ribbon_links[0].to, 1);
        assert!(editor.notes.iter().all(|note| note.ribbon.is_none()));
    }

    #[test]
    fn independent_connectors_do_not_replace_one_another() {
        let mut editor = editor();
        editor.notes.push(note(1.0));
        editor.notes.push(note(2.0));
        editor.notes.push(note(3.0));
        editor.ribbon_links.push(RibbonLink {
            from: 0,
            to: 1,
            from_side: RibbonSide::Right,
            to_side: RibbonSide::Left,
        });
        editor.ribbon_links.push(RibbonLink {
            from: 1,
            to: 2,
            from_side: RibbonSide::Right,
            to_side: RibbonSide::Left,
        });
        assert_eq!(editor.ribbon_links.len(), 2);
    }

    #[test]
    fn connectors_follow_notes_when_they_are_sorted() {
        let mut editor = editor();
        editor.notes.push(note(3.0));
        editor.notes.push(note(1.0));
        editor.ribbon_links.push(RibbonLink {
            from: 0,
            to: 1,
            from_side: RibbonSide::Right,
            to_side: RibbonSide::Left,
        });
        editor.sort_notes();
        assert_eq!(editor.ribbon_links.len(), 1);
        assert_eq!(editor.ribbon_links[0].from, 1);
        assert_eq!(editor.ribbon_links[0].to, 0);
    }

    #[test]
    fn deleting_a_selected_connector_leaves_the_notes_alone() {
        let mut editor = editor();
        editor.notes.push(note(1.0));
        editor.notes.push(note(2.0));
        editor.ribbon_links.push(RibbonLink {
            from: 0,
            to: 1,
            from_side: RibbonSide::Right,
            to_side: RibbonSide::Left,
        });
        editor.selected_ribbon = Some(0);
        editor.delete_selected();
        assert_eq!(editor.notes.len(), 2);
        assert!(editor.ribbon_links.is_empty());
        assert!(editor.selected_ribbon.is_none());
    }

    #[test]
    fn deleting_a_note_drops_its_connectors() {
        let mut editor = editor();
        editor.notes.push(note(1.0));
        editor.notes.push(note(2.0));
        editor.ribbon_links.push(RibbonLink {
            from: 0,
            to: 1,
            from_side: RibbonSide::Right,
            to_side: RibbonSide::Left,
        });
        editor.selected.insert(0);
        editor.delete_selected();
        assert_eq!(editor.notes.len(), 1);
        assert!(editor.ribbon_links.is_empty());
    }

    #[test]
    fn the_note_tool_grabs_edges_from_inside_only() {
        let mut editor = editor();
        editor.notes.push(note(2.0));
        editor.notes[0].dur = 1.0;
        let layout = editor.layout(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 980.0)));
        let x1 = editor.time_to_x(3.0, &layout);
        let y = editor.midi_to_y(60, &layout);
        assert!(editor
            .note_edge_at_inside(Pos2::new(x1 - 1.0, y), &layout)
            .is_some());
        assert!(editor
            .note_edge_at_inside(Pos2::new(x1 + 1.0, y), &layout)
            .is_none());
    }

    /// A private folder per test: tests run in parallel and must not share one.
    fn automap_scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "karaokemachine-automap-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn unmapped_song() -> Song {
        serde_json::from_value(serde_json::json!({
            "id": "automap",
            "title": "Automap",
            "duration": 10.0,
            "mapped": false,
            "tags": ["Imported", "UNMAPPED"],
            "lines": [{ "start": 1.0, "end": 2.0, "text": "la la", "words": [
                { "text": "la", "start": 1.0, "end": 1.5 },
                { "text": "la", "start": 1.5, "end": 2.0 }
            ]}],
            "suggestedNotes": [
                { "start": 1.0, "dur": 0.4, "midi": 60, "word": "la", "line": 0, "wordIndex": 0 },
                { "start": 1.5, "dur": 0.4, "midi": 62, "word": "la", "line": 0, "wordIndex": 1 }
            ]
        }))
        .expect("song")
    }

    /// An editor on a fresh draft file in `dir`, with the song's suggestions.
    fn automap_editor(song: Song, initial: Vec<Note>, dir: &std::path::Path) -> Editor {
        let suggestions = song.suggested_notes.clone();
        Editor::new(
            song,
            suggestions,
            initial,
            10.0,
            dir.join("Level.json"),
            true,
        )
    }

    #[test]
    fn auto_map_fills_the_editor_and_submits_a_level() {
        let dir = automap_scratch("submit");
        let mut editor = automap_editor(unmapped_song(), Vec::new(), &dir);
        assert!(editor.validate().is_err(), "an empty editor cannot submit");

        editor.auto_map().expect("auto-map");
        assert_eq!(editor.notes.len(), 2);
        assert!(editor.notes.iter().all(|note| note.ribbon.is_none()));
        assert!(editor.word_bank.iter().all(|token| token.placed));
        assert!(editor.is_dirty());
        assert!(editor.validate().is_ok());

        editor.submit().expect("submit");
        assert!(!editor.is_draft());
        let saved = crate::library::load_map_file(&dir.join("Level.json")).expect("level");
        assert!(!saved.draft);
        assert_eq!(saved.notes.len(), 2);
        assert_eq!(saved.placed_words.len(), 2);

        // Undo restores the empty chart the editor started with.
        editor.undo();
        assert!(editor.notes.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_map_turns_a_word_held_over_two_notes_into_a_connector() {
        let dir = automap_scratch("links");
        let mut song = unmapped_song();
        song.suggested_notes[1].word_index = Some(0);
        let mut editor = automap_editor(song, Vec::new(), &dir);
        editor.auto_map().expect("auto-map");
        assert_eq!(editor.notes.len(), 2);
        assert_eq!(editor.ribbon_links.len(), 1);
        assert_eq!(
            (editor.ribbon_links[0].from, editor.ribbon_links[0].to),
            (0, 1)
        );
        assert_eq!(
            editor.words.len(),
            2,
            "the second lyric word sits at its own time"
        );
        editor.undo();
        assert!(editor.ribbon_links.is_empty());
    }

    #[test]
    fn auto_map_keeps_placed_custom_words_and_is_idempotent() {
        let dir = automap_scratch("custom");
        let mut song = unmapped_song();
        song.custom_words = vec![CustomWord {
            id: 1,
            text: "whoa".into(),
        }];
        song.placed_words = vec![PlacedWord {
            text: "whoa".into(),
            start: 3.0,
            y: 0.2,
            line: None,
            word_index: None,
            custom_id: Some(1),
        }];
        let mut editor = automap_editor(song, Vec::new(), &dir);
        editor.auto_map().expect("auto-map");
        assert!(editor.words.iter().any(|word| word.custom_id == Some(1)));
        assert!(editor.validate().is_ok());
        let first: Vec<_> = editor
            .notes
            .iter()
            .map(|note| (note.start, note.midi))
            .collect();
        let first_links = editor.ribbon_links.clone();
        editor.auto_map().expect("auto-map twice");
        let second: Vec<_> = editor
            .notes
            .iter()
            .map(|note| (note.start, note.midi))
            .collect();
        assert_eq!(first, second);
        assert_eq!(first_links, editor.ribbon_links);
        assert_eq!(editor.words.len(), 3);
    }

    #[test]
    fn auto_map_over_a_draft_asks_first_then_replaces_and_undoes() {
        let dir = automap_scratch("confirm");
        let song = unmapped_song();
        let hand = vec![song.suggested_notes[0].clone()];
        let mut editor = automap_editor(song, hand, &dir);
        editor.request_auto_map(10.0);
        assert_eq!(editor.notes.len(), 1, "first click only arms");
        assert!(editor.auto_map_armed.is_some());
        editor.request_auto_map(11.0);
        assert_eq!(editor.notes.len(), 2);
        editor.undo();
        assert_eq!(editor.notes.len(), 1);
    }

    #[test]
    fn auto_map_without_suggestions_reports_why() {
        let dir = automap_scratch("empty");
        let mut song = unmapped_song();
        song.suggested_notes.clear();
        let mut editor = automap_editor(song, Vec::new(), &dir);
        assert!(editor.auto_map().is_err());
        assert!(editor.notes.is_empty());
    }
}
