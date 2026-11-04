//! The settings window and the vocal-range test.

use super::KaraokeMachineApp;
use crate::audio::input_device_names;
use crate::library::{save_scores, save_settings};
use crate::model::note_name;
use crate::theme::{GOLD, INK, MINT, MUTED, MUTED2, PINK};
use eframe::egui::{self, Align2, RichText, Vec2};

impl KaraokeMachineApp {
    pub(super) fn start_range_test(&mut self) {
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

    pub(super) fn update_range_test(&mut self) {
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

    pub(super) fn draw_range_test(&mut self, ctx: &egui::Context) {
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

    pub(super) fn draw_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        let mut changed = false;
        let octave_before = self.settings.octave_shift;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            // Taller than the 700 pt minimum window: scroll rather than clip.
            .max_height(ctx.screen_rect().height() - 60.0)
            .vscroll(true)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(480.0);
                ui.add_space(4.0);
                ui.label(RichText::new("MICROPHONE").monospace().size(10.0).color(MUTED2));
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
                ui.label(RichText::new("PLAYBACK").monospace().size(10.0).color(MUTED2));
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
                ui.label(RichText::new("VOCAL RANGE").monospace().size(10.0).color(MUTED2));
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
                        .size(10.0)
                        .color(MUTED2),
                );
                ui.add_space(10.0);
                ui.label(RichText::new("STAGE").monospace().size(10.0).color(MUTED2));
                changed |= ui.add(egui::Slider::new(&mut self.settings.lead_time, 4.0..=10.0).text("Notes arrive (s)")).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.sync_ms, -150.0..=150.0).text("Sync trim (ms)")).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.latency_ms, -150.0..=300.0).text("Mic latency (ms)")).changed();
                if ui.checkbox(&mut self.settings.count_in, "Count-in before a take").changed() {
                    changed = true;
                }
                ui.add_space(12.0);
                if self.clear_scores_armed {
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("Yes, clear every high score").color(PINK))
                            .clicked()
                        {
                            self.scores.clear();
                            save_scores(&self.scores);
                            self.clear_scores_armed = false;
                        }
                        if ui.button("Keep them").clicked() {
                            self.clear_scores_armed = false;
                        }
                    });
                } else if ui.button("Clear high scores…").clicked() {
                    self.clear_scores_armed = true;
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Headphones recommended — speaker bleed can register as pitch.").size(10.0).color(MUTED2));
            });
        if open != self.settings_open {
            self.settings_open = open;
            self.clear_scores_armed = false;
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
