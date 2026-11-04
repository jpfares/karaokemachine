//! The singing stage: scoring, the pitch-lane layout and its painting.

use super::{ctx_pointer, pitch_col, Action, KaraokeMachineApp, MapPickerMode};
use crate::audio::Microphone;
use crate::library::{load_map_file, save_score, save_settings, song_audio_path};
use crate::model::{
    accuracy_label, format_time, note_name, rank_for, Note, RibbonSide, ScoreRecord, Scores,
    Settings, Song,
};
use crate::pitch::{detect_pitch, PitchReading};
use crate::theme::{
    alpha, rank_colour, BG, GOLD, INK, LINE, MINT, MUTED, MUTED2, OCTAVE_BLUE, PINK,
};
use crate::widgets::{click, glyph_button, pill_button, pill_toggle, round_button};
use eframe::egui::{
    self, Align2, Color32, FontId, Mesh, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2,
};
use std::collections::VecDeque;
use std::time::Duration;

pub(super) const OCTAVE_TOLERANCE_CENTS: f32 = 50.0;
pub(super) const IN_TUNE_TOLERANCE_CENTS: f32 = 85.0;
/// Where the hit line sits across the lane area: notes approach from the
/// right and stay visible for a moment after they are judged.
const HIT_LINE_FRACTION: f32 = 0.22;
/// How much sung-pitch history the trail behind the voice bubble keeps.
const TRAIL_SECONDS: f64 = 4.0;
/// How fast the active note's hit glow fades, in units per second.
const GLOW_DECAY: f32 = 4.0;

/// One sung pitch, kept for the trail that follows the voice bubble.
#[derive(Clone, Copy, Debug)]
pub(super) struct TrailPoint {
    pub(super) time: f64,
    pub(super) midi: f32,
    pub(super) color: Color32,
}

/// Fold a sung pitch into the octave of the target note.
///
/// Returns the cents error against that octave and the octave shift that was
/// removed. A singer an octave below the chart therefore lands on the right
/// note class with a small cents error, but the shift stays non-zero so the
/// note can be capped below "perfect".
pub(super) fn fold_octave(midi: f32, target: i32) -> (f32, i32) {
    let shift = ((midi - target as f32) / 12.0).round() as i32;
    let folded = midi - 12.0 * shift as f32;
    ((folded - target as f32) * 100.0, shift)
}

pub(super) fn same_lyric_word(a: &Note, b: &Note) -> bool {
    if a.word.is_none() || a.word != b.word || a.line != b.line {
        return false;
    }
    match (a.word_index, b.word_index) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct NoteStat {
    pub(super) total: u32,
    pub(super) hit: u32,
    pub(super) cents_sum: f64,
    pub(super) cents_count: u32,
    pub(super) octave_frames: u32,
    pub(super) judged: bool,
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
pub(super) struct Judgment {
    pub(super) text: String,
    pub(super) color: Color32,
    pub(super) at: f64,
}

#[derive(Clone, Debug)]
pub(super) struct ResultData {
    pub(super) accuracy: f32,
    pub(super) rank: char,
    pub(super) perfect: u32,
    pub(super) hit: u32,
    pub(super) combo: u32,
    pub(super) score: u32,
    pub(super) new_best: bool,
}

pub(super) struct Stage {
    pub(super) song: Song,
    pub(super) duration: f64,
    pub(super) time: f64,
    pub(super) playing: bool,
    pub(super) started: bool,
    pub(super) count_in_until: Option<f64>,
    pub(super) score: u32,
    pub(super) combo: u32,
    pub(super) best_combo: u32,
    pub(super) hit_notes: u32,
    pub(super) perfects: u32,
    pub(super) total_probe: u32,
    pub(super) hit_probe: u32,
    pub(super) finalize_ptr: usize,
    pub(super) active: i32,
    pub(super) stats: Vec<NoteStat>,
    pub(super) judgment: Option<Judgment>,
    pub(super) result: Option<ResultData>,
    pub(super) audio_loaded: bool,
    /// A decode job is running for this song; playback waits for it.
    pub(super) audio_loading: bool,
    /// Start playing as soon as the decode job finishes.
    pub(super) play_when_loaded: bool,
    /// Wall-clock time of the previous tick, for the no-audio fallback clock.
    pub(super) last_tick: f64,
    /// Recent sung pitches, newest last, in chart time.
    pub(super) trail: VecDeque<TrailPoint>,
    /// 1.0 on a frame where the active note was hit, fading to 0.
    pub(super) hit_glow: f32,
    pub(super) octave_shift: i32,
}

impl Stage {
    pub(super) fn new(mut song: Song, settings: &Settings) -> Self {
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
            audio_loading: false,
            play_when_loaded: false,
            last_tick: 0.0,
            trail: VecDeque::new(),
            hit_glow: 0.0,
            octave_shift: shift,
        }
    }

    pub(super) fn set_octave_shift(&mut self, shift: i32) {
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
    pub(super) fn reset_scoring(&mut self, from_time: f64) {
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
        self.trail.clear();
        self.hit_glow = 0.0;
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
    pub(super) fn seek(&mut self, time: f64) {
        if self.result.is_some() {
            return;
        }
        self.time = time.clamp(0.0, self.duration);
        self.reset_scoring(self.time);
    }

    pub(super) fn reset(&mut self) {
        self.time = 0.0;
        self.playing = false;
        self.started = false;
        self.count_in_until = None;
        self.reset_scoring(0.0);
    }

    /// Record what the singer is producing right now for the on-screen trail,
    /// and light the active note when the reading is a hit.
    fn record_reading(&mut self, chart_time: f64, reading: Option<PitchReading>) {
        while self
            .trail
            .front()
            .is_some_and(|point| point.time < chart_time - TRAIL_SECONDS)
        {
            self.trail.pop_front();
        }
        let Some(reading) = reading else {
            return;
        };
        let target = usize::try_from(self.active)
            .ok()
            .and_then(|index| self.song.notes.get(index))
            .map(|note| note.midi);
        let (midi, color, hit) = reading_feedback(reading, target);
        if hit {
            self.hit_glow = 1.0;
        }
        self.trail.push_back(TrailPoint {
            time: chart_time,
            midi,
            color,
        });
    }

    pub(super) fn active_note_at(&self, time: f64) -> i32 {
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

    pub(super) fn finish(&mut self, scores: &mut Scores) {
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
        let rank = rank_for(accuracy);
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

    pub(super) fn finalize_note(&mut self, index: usize) {
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

    pub(super) fn score_tick(
        &mut self,
        chart_time: f64,
        reading: Option<PitchReading>,
        mic_ok: bool,
    ) {
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

pub(super) struct Layout {
    pub(super) header_h: f32,
    pub(super) tray_h: f32,
    pub(super) left_w: f32,
    pub(super) hit_x: f32,
    pub(super) lane_top: f32,
    pub(super) lane_bottom: f32,
    pub(super) lane_h: f32,
    pub(super) low: i32,
    pub(super) high: i32,
    pub(super) pps: f32,
}

impl Layout {
    pub(super) fn new(size: Vec2, song: &Song, settings: &Settings) -> Self {
        let width = size.x;
        let height = size.y;
        let header_h = 64.0;
        let tray_h = 104.0;
        let left_w = (width * 0.07).clamp(96.0, 132.0);
        let hit_x = left_w + (width - left_w) * HIT_LINE_FRACTION;
        let lane_top = header_h + 20.0;
        let lane_bottom = height - tray_h - 18.0;
        // Fit the lanes to this song's actual melody, not a fixed window.
        // The extra lanes above the top note leave room for its lyric label.
        let mut midis: Vec<i32> = song.notes.iter().map(|n| n.midi).collect();
        midis.sort_unstable();
        let (lowest, highest) = if let (Some(lowest), Some(highest)) = (midis.first(), midis.last())
        {
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

    pub(super) fn lane_y(&self, midi: i32) -> f32 {
        self.lane_top + (self.high - midi) as f32 * self.lane_h + self.lane_h / 2.0
    }

    pub(super) fn note_height(&self) -> f32 {
        (self.lane_h * 0.62).clamp(7.0, 22.0)
    }

    pub(super) fn y_norm(&self, y: f32) -> f32 {
        ((y - self.lane_top) / (self.lane_bottom - self.lane_top).max(1.0)).clamp(0.0, 1.0)
    }

    /// Vertical position of a fractional pitch, for the voice trail.
    pub(super) fn pitch_y(&self, midi: f32) -> f32 {
        self.lane_top + (self.high as f32 - midi) * self.lane_h + self.lane_h / 2.0
    }
}

/// Where a sung pitch sits and what colour it earns against `target`: mint
/// in tune, gold a little off, blue the right note in another octave, pink a
/// miss. Without a target the pitch is shown as sung, uncoloured.
fn reading_feedback(reading: PitchReading, target: Option<i32>) -> (f32, Color32, bool) {
    let Some(target) = target else {
        return (reading.midi, MUTED, false);
    };
    let (cents, octave_shift) = fold_octave(reading.midi, target);
    if octave_shift != 0 {
        return if cents.abs() <= OCTAVE_TOLERANCE_CENTS {
            (target as f32 + cents / 100.0, OCTAVE_BLUE, true)
        } else {
            (reading.midi, PINK, false)
        };
    }
    if cents.abs() <= 30.0 {
        (reading.midi, MINT, true)
    } else if cents.abs() <= IN_TUNE_TOLERANCE_CENTS {
        (reading.midi, GOLD, true)
    } else {
        (reading.midi, PINK, false)
    }
}

pub(super) fn stage_needs_input(stage: &Option<Stage>) -> bool {
    stage
        .as_ref()
        .map(|s| s.playing || s.result.is_none())
        .unwrap_or(false)
}

pub(super) fn draw_melody(
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
    let use_links = !stage.song.ribbon_links.is_empty();
    let mut mesh = Mesh::default();
    let mut previous: Option<(f32, f32)> = None;
    let mut active_body: Option<Rect> = None;

    for (i, note) in notes.iter().enumerate() {
        let x0 = layout.hit_x + (note.start - t) as f32 * layout.pps;
        let x1 = layout.hit_x + (note.start + note.dur - t) as f32 * layout.pps;
        if x1 < layout.left_w - 2.0 {
            previous = None;
            continue;
        }
        // A note that has passed the hit line keeps its colour if it was
        // sung and fades if it was missed, so the singer sees the verdict.
        let opacity = match stage.stats.get(i) {
            Some(stat) if stat.judged => {
                let ratio = if stat.total > 0 {
                    stat.hit as f32 / stat.total as f32
                } else {
                    0.0
                };
                if ratio >= 0.55 {
                    0.95
                } else if ratio >= 0.28 {
                    0.55
                } else {
                    0.28
                }
            }
            _ => 0.95,
        };
        if x0 > width + 60.0 {
            break;
        }
        let y = layout.lane_y(note.midi);
        let group_prev = !use_links && i > 0 && {
            let prev = &notes[i - 1];
            let gap = note.start - (prev.start + prev.dur);
            let limit = if note.ribbon.is_some() { 3.0 } else { 0.85 };
            gap <= limit && prev.shares_ribbon(note)
        };
        let group_next = !use_links
            && notes
                .get(i + 1)
                .map(|next| {
                    let gap = next.start - (note.start + note.dur);
                    let limit = if note.ribbon.is_some() { 3.0 } else { 0.85 };
                    gap <= limit && note.shares_ribbon(next)
                })
                .unwrap_or(false);
        // Each note keeps its own rounded body. A thin neck connects it to the
        // next note of the same word/phrase, so the word remains one ribbon but
        // every note is visible.
        let sx = x0.max(layout.left_w) + if group_prev { 0.6 } else { 2.0 };
        let ex = (x1 - if group_next { 0.6 } else { 2.0 }).max(sx + 1.0);
        if i as i32 == active {
            active_body = Some(Rect::from_min_max(
                Pos2::new(sx, y - h / 2.0),
                Pos2::new(ex, y + h / 2.0),
            ));
        }

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
                    opacity,
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
            opacity,
        );
        previous = Some((ex, y));
    }
    if use_links {
        let mut links_mesh = Mesh::default();
        for link in &stage.song.ribbon_links {
            let (Some(from), Some(to)) = (notes.get(link.from), notes.get(link.to)) else {
                continue;
            };
            let from_time = match link.from_side {
                RibbonSide::Left => from.start,
                RibbonSide::Right => from.start + from.dur,
            };
            let to_time = match link.to_side {
                RibbonSide::Left => to.start,
                RibbonSide::Right => to.start + to.dur,
            };
            let ax = layout.hit_x + (from_time - t) as f32 * layout.pps;
            let bx = layout.hit_x + (to_time - t) as f32 * layout.pps;
            let ay = layout.lane_y(from.midi);
            let by = layout.lane_y(to.midi);
            if ax.max(bx) < layout.hit_x - 4.0 || ax.min(bx) > width + 60.0 {
                continue;
            }
            let steps = 8usize;
            let mut bridge = Vec::with_capacity(steps + 1);
            for step in 0..=steps {
                let f = step as f32 / steps as f32;
                let smooth = f * f * (3.0 - 2.0 * f);
                bridge.push(Pos2::new(ax + (bx - ax) * f, ay + (by - ay) * smooth));
            }
            add_ribbon_mesh(
                &mut links_mesh,
                &bridge,
                (h * 0.38).max(3.0),
                layout.low,
                layout.high,
                layout.lane_top,
                layout.lane_bottom,
                0.95,
            );
        }
        p.add(Shape::mesh(links_mesh));
    }
    p.add(Shape::mesh(mesh));
    // The note being sung lights up while the voice is on it.
    if let Some(body) = active_body {
        let glow = stage.hit_glow;
        if glow > 0.0 {
            let outer = body.expand(4.0 + glow * 3.0);
            p.rect_filled(
                outer,
                outer.height() / 2.0,
                alpha(Color32::WHITE, (glow * 40.0) as u8),
            );
            p.rect_stroke(
                body.expand(2.0),
                body.height() / 2.0 + 2.0,
                Stroke::new(2.0f32, alpha(Color32::WHITE, (glow * 220.0) as u8)),
                StrokeKind::Outside,
            );
        }
    }
    if active >= 0 {
        if let Some(note) = notes.get(active as usize) {
            let x0 = layout.hit_x + (note.start - t) as f32 * layout.pps;
            let x1 = layout.hit_x + (note.start + note.dur - t) as f32 * layout.pps;
            let y = layout.lane_y(note.midi);
            let r = Rect::from_min_max(
                Pos2::new(x0.max(layout.hit_x), y - h / 2.0),
                Pos2::new(x1.max(layout.hit_x + 8.0), y + h / 2.0),
            );
            p.rect_stroke(
                r,
                h / 2.0,
                Stroke::new(1.4f32, Color32::WHITE),
                StrokeKind::Inside,
            );
        }
    }
}

// The four lane parameters belong in a shared lane-geometry type; that
// extraction is tracked in REVIEW.md and would remove this allowance.
#[allow(clippy::too_many_arguments)]
pub(super) fn add_ribbon_mesh(
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
        if clean
            .last()
            .map(|last| (point.x - last.x).abs() > 0.01 || (point.y - last.y).abs() > 0.01)
            .unwrap_or(true)
        {
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

pub(super) fn draw_placed_words(
    p: &egui::Painter,
    stage: &Stage,
    layout: &Layout,
    time: f64,
    width: f32,
) {
    for word in &stage.song.placed_words {
        if word.start < time - 1.5 || word.start > time + 11.0 {
            continue;
        }
        let x = layout.hit_x + (word.start - time) as f32 * layout.pps;
        if x < layout.left_w - 80.0 || x > width + 100.0 {
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
        let cx = x.max(layout.left_w + half).min(width - half);
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
            if active {
                Color32::WHITE
            } else {
                alpha(INK, 175)
            },
        );
    }
}

pub(super) fn draw_words(p: &egui::Painter, stage: &Stage, layout: &Layout, time: f64, width: f32) {
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
            .max_by(|a, b| {
                a.dur
                    .partial_cmp(&b.dur)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(&group[0]);
        let centre = if has_rest {
            group[0].start + group[0].dur * 0.5
        } else {
            (start + end) * 0.5
        };
        let x = layout.hit_x + (centre - time) as f32 * layout.pps;
        if x < layout.left_w + 4.0 || x > width + 80.0 {
            continue;
        }
        let y = layout.lane_y(anchor.midi) - h / 2.0 - 17.0;
        if y < layout.lane_top + 8.0 || y > layout.lane_bottom - 2.0 {
            continue;
        }
        let active = time >= start - 0.02 && time <= end + 0.02;
        let fg = if active {
            Color32::WHITE
        } else {
            alpha(INK, 170)
        };
        let font = FontId::proportional(if active { 15.0 } else { 13.5 });
        let width_est = text.chars().count() as f32 * 7.4 + 12.0;
        let pill_w = width_est.min(170.0);
        // Keep the whole word readable while it passes the playhead.
        let half = pill_w * 0.5 + 2.0;
        let cx = x.max(layout.left_w + half).min(width - half);
        let pill = Rect::from_center_size(Pos2::new(cx, y), Vec2::new(pill_w, 24.0));
        p.rect_filled(
            pill,
            5.0,
            Color32::from_rgba_unmultiplied(5, 8, 15, if active { 200 } else { 115 }),
        );
        if active {
            p.rect_stroke(
                pill,
                5.0,
                Stroke::new(1.0f32, alpha(GOLD, 90)),
                StrokeKind::Inside,
            );
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

impl KaraokeMachineApp {
    pub(super) fn start_mic(&mut self) {
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

    pub(super) fn apply_octave_shift(&mut self) {
        self.settings.octave_shift = self.settings.octave_shift.clamp(-2, 2);
        if let Some(stage) = &mut self.stage {
            stage.set_octave_shift(self.settings.octave_shift);
        }
        save_settings(&self.settings);
    }

    pub(super) fn fit_current_song_to_range(&mut self) {
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
        let low = stage
            .song
            .notes
            .iter()
            .map(|note| note.midi - shift_now)
            .min()
            .unwrap_or(60);
        let high = stage
            .song
            .notes
            .iter()
            .map(|note| note.midi - shift_now)
            .max()
            .unwrap_or(60);
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

    pub(super) fn backing_level(&self) -> f32 {
        if self.settings.backing_enabled {
            self.settings.backing_volume
        } else {
            0.0
        }
    }

    pub(super) fn apply_audio_volumes(&self) {
        if let Some(audio) = &self.audio {
            audio.set_volumes(self.backing_level(), self.settings.vocals_volume);
        }
    }

    pub(super) fn open_song(&mut self, index: usize, now: f64) {
        self.open_map_picker(index, MapPickerMode::Play, now);
    }

    pub(super) fn open_song_with_map(
        &mut self,
        index: usize,
        map_path: &std::path::Path,
        now: f64,
    ) {
        let Some(mut song) = self.songs.get(index).cloned() else {
            return;
        };
        let map = match load_map_file(map_path) {
            Ok(map) => map,
            Err(error) => {
                self.toast(error, now);
                return;
            }
        };
        map.apply_to_song(&mut song);
        self.editor = None;
        self.map_picker = None;
        if let Some(audio) = &self.audio {
            audio.stop();
        }
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
            stage.audio_loading = true;
        }
        self.stage = Some(stage);
        self.start_audio_load(backing, Some(vocals), false);
        if self.settings.last_song.as_deref() != Some(self.songs[index].id.as_str()) {
            self.settings.last_song = Some(self.songs[index].id.clone());
            save_settings(&self.settings);
        }
        self.drawer_open = false;
        self.settings_open = false;
        self.started_at = now;
        self.tracker.reset();
        self.reading = None;
    }

    pub(super) fn toggle_play(&mut self, now: f64) {
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
        if stage.audio_loading {
            stage.play_when_loaded = true;
            self.toast(
                "Audio is still loading; playback starts when it is ready.",
                now,
            );
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

    pub(super) fn restart_song(&mut self, now: f64) {
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

    pub(super) fn toggle_lyrics(&mut self) {
        self.settings.show_lyrics = !self.settings.show_lyrics;
        save_settings(&self.settings);
    }

    pub(super) fn toggle_vocals(&mut self) {
        self.settings.vocals_volume = if self.settings.vocals_volume > 0.01 {
            0.0
        } else {
            0.8
        };
        self.apply_audio_volumes();
        save_settings(&self.settings);
    }

    pub(super) fn toggle_backing(&mut self) {
        self.settings.backing_enabled = !self.settings.backing_enabled;
        if self.settings.backing_enabled && self.settings.backing_volume <= 0.01 {
            self.settings.backing_volume = 0.9;
        }
        self.apply_audio_volumes();
        save_settings(&self.settings);
    }

    pub(super) fn tick_stage(&mut self, ctx: &egui::Context, now: f64) {
        if self.editor.is_some() {
            return;
        }
        let Some(stage) = &mut self.stage else {
            return;
        };
        if let Some(until) = stage.count_in_until {
            if now >= until {
                stage.count_in_until = None;
                stage.time = 0.0;
                if stage.audio_loading {
                    // The count-in ended before the song decoded; start as
                    // soon as it has.
                    stage.play_when_loaded = true;
                } else {
                    if let Some(audio) = &self.audio {
                        if stage.audio_loaded {
                            audio.play();
                        }
                    }
                    stage.playing = true;
                    stage.started = true;
                }
            }
        }
        let elapsed = if stage.last_tick > 0.0 {
            (now - stage.last_tick).clamp(0.0, 0.1)
        } else {
            0.0
        };
        stage.last_tick = now;
        if stage.playing {
            match &self.audio {
                Some(audio) if stage.audio_loaded => stage.time = audio.position(),
                // No audio: run the chart on the wall clock, not once per repaint.
                _ => stage.time += elapsed,
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
            // A reading arriving now was sung `latency_ms` ago, so judge it
            // against the note that was playing then.
            let heard_at = chart_time - self.settings.latency_ms as f64 / 1000.0;
            stage.score_tick(heard_at, self.reading, mic_ok);
            stage.hit_glow = (stage.hit_glow - elapsed as f32 * GLOW_DECAY).max(0.0);
            stage.record_reading(chart_time, self.reading);
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

    pub(super) fn update_pitch(&mut self) {
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
            let mut samples = mic.latest(2048);
            // Input gain lifts a quiet microphone over the detector's level gate.
            let gain = self.settings.input_gain;
            if (gain - 1.0).abs() > 0.001 {
                for sample in &mut samples {
                    *sample *= gain;
                }
            }
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

    pub(super) fn draw_stage(&self, ui: &mut egui::Ui, actions: &mut Vec<Action>, now: f64) {
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
        p.line_segment(
            [header.left_bottom(), header.right_bottom()],
            Stroke::new(1.0f32, LINE),
        );
        let back = Rect::from_min_size(Pos2::new(16.0, 15.0), Vec2::new(36.0, 34.0));
        if glyph_button(ui, back, "←") {
            actions.push(Action::Home);
        }
        p.text(
            Pos2::new(66.0, 17.0),
            Align2::LEFT_TOP,
            "NOW SINGING",
            FontId::monospace(10.0),
            MUTED2,
        );
        p.text(
            Pos2::new(66.0, 30.0),
            Align2::LEFT_TOP,
            &stage.song.title,
            FontId::proportional(20.0),
            INK,
        );
        p.text(
            Pos2::new(66.0, 50.0),
            Align2::LEFT_TOP,
            format!(
                "{}   ·   {}",
                stage.song.artist_display(),
                stage.song.tags.join(" · ")
            ),
            FontId::proportional(10.0),
            MUTED,
        );
        let accuracy = if stage.total_probe > 0 {
            (stage.hit_probe as f32 / stage.total_probe as f32 * 100.0).round() as i32
        } else {
            0
        };
        let stats_x = rect.width() - 400.0;
        for (i, (label, value, color)) in [
            ("SCORE", stage.score.to_string(), INK),
            ("COMBO", format!("{}×", stage.combo), GOLD),
            (
                "ACCURACY",
                if stage.total_probe > 0 {
                    format!("{accuracy}%")
                } else {
                    "—".into()
                },
                MINT,
            ),
        ]
        .iter()
        .enumerate()
        {
            let x = stats_x + i as f32 * 118.0;
            p.text(
                Pos2::new(x, 12.0),
                Align2::LEFT_TOP,
                *label,
                FontId::monospace(10.0),
                MUTED,
            );
            p.text(
                Pos2::new(x, 24.0),
                Align2::LEFT_TOP,
                value,
                FontId::monospace(26.0),
                *color,
            );
        }
        // lanes
        let clip = Rect::from_min_max(
            Pos2::new(layout.left_w, layout.lane_top),
            Pos2::new(rect.width(), layout.lane_bottom),
        );
        let painter = ui.painter().with_clip_rect(clip);
        for midi in layout.low..=layout.high {
            let top = layout.lane_top + (layout.high - midi) as f32 * layout.lane_h;
            let y = top + layout.lane_h;
            let black = [1, 3, 6, 8, 10].contains(&(((midi % 12) + 12) % 12));
            if black {
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(layout.left_w, top),
                        Vec2::new(rect.width() - layout.left_w, layout.lane_h),
                    ),
                    0.0,
                    Color32::from_white_alpha(9),
                );
            }
            let is_c = midi % 12 == 0;
            painter.line_segment(
                [Pos2::new(layout.left_w, y), Pos2::new(rect.width(), y)],
                Stroke::new(
                    if is_c { 1.2f32 } else { 0.7f32 },
                    Color32::from_white_alpha(if is_c { 44 } else { 17 }),
                ),
            );
        }
        // grid
        let interval = stage.song.bpm.map(|bpm| 60.0 / bpm).unwrap_or(1.0);
        let first = (chart_time / interval).floor() as i64 - 1;
        let last =
            ((chart_time + self.settings.lead_time as f64 + 2.0) / interval).ceil() as i64 + 1;
        for tick in first..=last {
            let t = tick as f64 * interval;
            let x = layout.hit_x + (t - chart_time) as f32 * layout.pps;
            if x < layout.left_w || x > rect.width() {
                continue;
            }
            let bar = stage
                .song
                .bpm
                .map(|_| tick % 4 == 0)
                .unwrap_or((t.round() as i64) % 5 == 0);
            if stage.song.bpm.is_none() && !bar {
                continue;
            }
            painter.line_segment(
                [
                    Pos2::new(x, layout.lane_top),
                    Pos2::new(x, layout.lane_bottom),
                ],
                Stroke::new(
                    if bar { 1.2f32 } else { 0.7f32 },
                    Color32::from_white_alpha(if bar { 30 } else { 13 }),
                ),
            );
        }
        // What has already scrolled past the hit line sits in shadow.
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(layout.left_w, layout.lane_top),
                Pos2::new(layout.hit_x, layout.lane_bottom),
            ),
            0.0,
            Color32::from_black_alpha(70),
        );
        let active = stage.active_note_at(chart_time);
        if active >= 0 {
            let note = &stage.song.notes[active as usize];
            let y = layout.lane_y(note.midi);
            let color = pitch_col(layout.y_norm(y));
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(layout.left_w, y - layout.lane_h / 2.0),
                    Vec2::new((rect.width() - layout.left_w).min(520.0), layout.lane_h),
                ),
                0.0,
                alpha(color, 28),
            );
        }
        // The hit line: notes are judged as they cross it.
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(layout.hit_x - 6.0, layout.lane_top),
                Pos2::new(layout.hit_x + 6.0, layout.lane_bottom),
            ),
            0.0,
            alpha(MINT, 14),
        );
        painter.line_segment(
            [
                Pos2::new(layout.hit_x, layout.lane_top),
                Pos2::new(layout.hit_x, layout.lane_bottom),
            ],
            Stroke::new(2.0f32, alpha(MINT, 150)),
        );

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
        let rail = Rect::from_min_size(
            Pos2::new(0.0, layout.header_h),
            Vec2::new(
                layout.left_w,
                rect.height() - layout.tray_h - layout.header_h,
            ),
        );
        p.rect_filled(rail, 0.0, Color32::from_rgba_unmultiplied(5, 8, 15, 242));
        p.line_segment(
            [
                Pos2::new(layout.left_w, layout.lane_top),
                Pos2::new(layout.left_w, layout.lane_bottom),
            ],
            Stroke::new(1.0f32, alpha(MINT, 70)),
        );
        for midi in layout.low..=layout.high {
            let y = layout.lane_y(midi);
            let is_target = active >= 0 && stage.song.notes[active as usize].midi == midi;
            let color = if is_target {
                INK
            } else if midi % 12 == 0 {
                alpha(INK, 120)
            } else {
                alpha(INK, 55)
            };
            if is_target {
                p.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(0.0, y - layout.lane_h / 2.0),
                        Vec2::new(layout.left_w, layout.lane_h),
                    ),
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
            p.rect_stroke(
                bracket,
                6.0,
                Stroke::new(1.0f32, Color32::from_white_alpha(60)),
                StrokeKind::Inside,
            );
        }
        // The voice trail: where the singer has been over the last seconds.
        let mut last: Option<(Pos2, Color32)> = None;
        for point in &stage.trail {
            let age = (chart_time - point.time).max(0.0);
            let x = layout.hit_x + (point.time - chart_time) as f32 * layout.pps;
            if x < layout.left_w {
                last = None;
                continue;
            }
            let y = layout
                .pitch_y(point.midi)
                .clamp(layout.lane_top + 4.0, layout.lane_bottom - 4.0);
            let fade = (1.0 - age / TRAIL_SECONDS).clamp(0.0, 1.0) as f32;
            let color = alpha(point.color, (60.0 + fade * 150.0) as u8);
            let pos = Pos2::new(x, y);
            // Join points that are close in time and pitch; an octave flip or a
            // rest breaks the line rather than drawing a vertical bar.
            if let Some((prev, _)) = last {
                if (prev.x - x).abs() < layout.pps * 0.25
                    && (prev.y - y).abs() < layout.lane_h * 2.5
                {
                    painter.line_segment([prev, pos], Stroke::new(3.0f32, color));
                }
            }
            painter.circle_filled(pos, 2.5 + fade * 1.5, color);
            last = Some((pos, color));
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
            let color = if target.is_some() {
                if octave_match {
                    OCTAVE_BLUE
                } else if folded_cents.abs() <= 30.0 {
                    MINT
                } else if folded_cents.abs() <= 65.0 {
                    GOLD
                } else if folded_cents.abs() <= 120.0 {
                    Color32::from_rgb(255, 161, 92)
                } else {
                    PINK
                }
            } else {
                pitch_col(layout.y_norm(y))
            };
            p.circle_filled(Pos2::new(layout.hit_x, y), 38.0, alpha(color, 25));
            p.circle_filled(Pos2::new(layout.hit_x, y), 23.0, alpha(color, 190));
            p.circle_filled(Pos2::new(layout.hit_x, y), 16.0, color);
            p.circle_filled(Pos2::new(layout.hit_x - 4.0, y - 4.0), 5.0, Color32::WHITE);
            let label = if target.is_some() {
                if octave_match {
                    format!(
                        "{}   OCT {:+}",
                        note_name(reading.midi.round() as i32),
                        octave_shift
                    )
                } else {
                    // Octave-folded, like the colours: a singer an octave off
                    // is told sharp or flat within that octave, not "-1200".
                    let verdict = if folded_cents.abs() <= 30.0 {
                        "IN TUNE"
                    } else if folded_cents.abs() > IN_TUNE_TOLERANCE_CENTS {
                        "WAY OFF"
                    } else if folded_cents > 0.0 {
                        "SHARP"
                    } else {
                        "FLAT"
                    };
                    format!(
                        "{}  {:+.0}¢  {verdict}",
                        note_name(reading.midi.round() as i32),
                        folded_cents
                    )
                }
            } else {
                format!(
                    "{}   {:+.0}¢",
                    note_name(reading.midi.round() as i32),
                    reading.cents
                )
            };
            let label_rect = Rect::from_min_size(
                Pos2::new(layout.hit_x + 34.0, y - 13.0),
                Vec2::new(150.0, 26.0),
            );
            p.rect_filled(
                label_rect,
                6.0,
                Color32::from_rgba_unmultiplied(5, 8, 15, 220),
            );
            p.rect_stroke(
                label_rect,
                6.0,
                Stroke::new(1.0f32, alpha(color, 70)),
                StrokeKind::Inside,
            );
            p.text(
                label_rect.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::monospace(10.0),
                color,
            );
        } else if self.mic.is_some() {
            p.text(
                Pos2::new(
                    layout.hit_x + 18.0,
                    layout.lane_y((layout.low + layout.high) / 2),
                ),
                Align2::LEFT_CENTER,
                "LISTENING",
                FontId::monospace(10.0),
                alpha(INK, 45),
            );
        } else if let Some(error) = &self.mic_error {
            p.text(
                Pos2::new(
                    layout.hit_x + 18.0,
                    layout.lane_y((layout.low + layout.high) / 2),
                ),
                Align2::LEFT_CENTER,
                format!("MIC OFF · {error}"),
                FontId::proportional(11.0),
                PINK,
            );
        }

        // count-in
        if let Some(until) = stage.count_in_until {
            let left = (until - now).max(0.0).ceil();
            let text = if left <= 0.3 {
                "SING".to_string()
            } else {
                format!("{left:.0}")
            };
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(150.0),
                alpha(MINT, 220),
            );
            p.text(
                rect.center() + Vec2::new(0.0, 110.0),
                Align2::CENTER_CENTER,
                "GET READY",
                FontId::monospace(10.0),
                MUTED,
            );
        }
        // Combo grows beside the hit line so the streak is felt, not read.
        if stage.combo >= 2 && stage.result.is_none() {
            let size = 18.0 + stage.combo.min(40) as f32 * 0.45;
            p.text(
                Pos2::new(layout.hit_x + 16.0, layout.lane_top + 10.0),
                Align2::LEFT_TOP,
                format!("{}×", stage.combo),
                FontId::proportional(size),
                GOLD,
            );
            p.text(
                Pos2::new(layout.hit_x + 16.0, layout.lane_top + 12.0 + size),
                Align2::LEFT_TOP,
                "COMBO",
                FontId::monospace(10.0),
                alpha(GOLD, 170),
            );
        }
        if let Some(judgment) = &stage.judgment {
            let age = now - judgment.at;
            if age < 0.7 {
                let fade = (1.0 - age / 0.7) as f32;
                let size = 34.0 + (1.0 - fade) * 10.0;
                // Inside the lane area so the flash never crosses the header.
                p.text(
                    Pos2::new(
                        layout.hit_x + 24.0,
                        layout.lane_top + 14.0 - (1.0 - fade) * 12.0,
                    ),
                    Align2::LEFT_TOP,
                    &judgment.text,
                    FontId::proportional(size),
                    alpha(judgment.color, (fade * 255.0) as u8),
                );
            }
        }

        // timeline
        let tray_top = rect.height() - layout.tray_h;
        let timeline = Rect::from_min_size(
            Pos2::new(layout.hit_x + 12.0, tray_top - 28.0),
            Vec2::new(rect.width() - layout.hit_x - 36.0, 5.0),
        );
        p.rect_filled(timeline, 3.0, Color32::from_white_alpha(18));
        let ratio = (stage.time / stage.duration.max(0.01)).clamp(0.0, 1.0) as f32;
        p.rect_filled(
            Rect::from_min_size(
                timeline.min,
                Vec2::new(timeline.width() * ratio, timeline.height()),
            ),
            3.0,
            MINT,
        );
        p.circle_filled(
            Pos2::new(
                timeline.left() + timeline.width() * ratio,
                timeline.center().y,
            ),
            5.0,
            Color32::WHITE,
        );
        p.text(
            Pos2::new(layout.hit_x + 12.0, tray_top - 19.0),
            Align2::LEFT_TOP,
            format_time(stage.time),
            FontId::monospace(10.0),
            MUTED2,
        );
        p.text(
            Pos2::new(rect.width() - 24.0, tray_top - 19.0),
            Align2::RIGHT_TOP,
            format_time(stage.duration),
            FontId::monospace(10.0),
            MUTED2,
        );
        if click(ui, timeline, "timeline_click") {
            if let Some(pos) = ctx_pointer(ui) {
                let t = ((pos.x - timeline.left()) / timeline.width()).clamp(0.0, 1.0) as f64
                    * stage.duration;
                actions.push(Action::Seek(t));
            }
        }

        // bottom transport
        let tray = Rect::from_min_size(
            Pos2::new(24.0, rect.height() - layout.tray_h + 18.0),
            Vec2::new(rect.width() - 48.0, layout.tray_h - 28.0),
        );
        p.rect_filled(tray, 24.0, Color32::from_rgba_unmultiplied(9, 13, 23, 235));
        p.rect_stroke(
            tray,
            24.0,
            Stroke::new(1.0f32, Color32::from_white_alpha(26)),
            StrokeKind::Inside,
        );
        let play_rect = Rect::from_center_size(
            Pos2::new(rect.width() * 0.5, tray.center().y),
            Vec2::new(58.0, 58.0),
        );
        let transport_label = if stage.audio_loading {
            "LOADING"
        } else if stage.playing {
            "PAUSE"
        } else {
            "PLAY"
        };
        if round_button(ui, play_rect, transport_label, true) {
            actions.push(Action::TogglePlay);
        }
        let restart = Rect::from_min_size(
            Pos2::new(play_rect.left() - 126.0, tray.center().y - 17.0),
            Vec2::new(96.0, 34.0),
        );
        if pill_toggle(ui, restart, "RESTART", false) {
            actions.push(Action::Restart);
        }
        let lyrics = Rect::from_min_size(
            Pos2::new(play_rect.right() + 36.0, tray.center().y - 17.0),
            Vec2::new(96.0, 34.0),
        );
        if pill_toggle(
            ui,
            lyrics,
            if self.settings.show_lyrics {
                "LYRICS ON"
            } else {
                "LYRICS OFF"
            },
            self.settings.show_lyrics,
        ) {
            actions.push(Action::ToggleLyrics);
        }
        let vocals = Rect::from_min_size(lyrics.right_top() + Vec2::new(8.0, 0.0), lyrics.size());
        if pill_toggle(
            ui,
            vocals,
            if self.settings.vocals_volume > 0.01 {
                "VOCALS ON"
            } else {
                "VOCALS OFF"
            },
            self.settings.vocals_volume > 0.01,
        ) {
            actions.push(Action::ToggleVocals);
        }
        let backing = Rect::from_min_size(vocals.right_top() + Vec2::new(8.0, 0.0), lyrics.size());
        if pill_toggle(
            ui,
            backing,
            if self.settings.backing_enabled {
                "BACKING ON"
            } else {
                "BACKING OFF"
            },
            self.settings.backing_enabled,
        ) {
            actions.push(Action::ToggleBacking);
        }
        let menu_btn = Rect::from_min_size(
            Pos2::new(tray.left() + 16.0, tray.center().y - 17.0),
            Vec2::new(100.0, 34.0),
        );
        if pill_toggle(ui, menu_btn, "LIBRARY", false) {
            actions.push(Action::OpenMenu);
        }
        let settings_tray = Rect::from_min_size(
            Pos2::new(tray.right() - 120.0, tray.center().y - 17.0),
            Vec2::new(100.0, 34.0),
        );
        if pill_toggle(ui, settings_tray, "SETTINGS", false) {
            actions.push(Action::OpenSettings);
        }

        // results
        if let Some(result) = &stage.result {
            p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(3, 6, 12, 190));
            let card = Rect::from_center_size(rect.center(), Vec2::new(620.0, 470.0));
            p.rect_filled(card, 24.0, Color32::from_rgb(9, 14, 24));
            p.rect_stroke(
                card,
                24.0,
                Stroke::new(1.0f32, Color32::from_white_alpha(30)),
                StrokeKind::Inside,
            );
            p.text(
                card.center_top() + Vec2::new(0.0, 28.0),
                Align2::CENTER_CENTER,
                "SESSION COMPLETE",
                FontId::monospace(10.0),
                MUTED2,
            );
            if result.new_best {
                let badge = Rect::from_center_size(
                    card.center_top() + Vec2::new(0.0, 54.0),
                    Vec2::new(84.0, 20.0),
                );
                p.rect_filled(
                    badge,
                    10.0,
                    Color32::from_rgba_unmultiplied(255, 209, 102, 24),
                );
                p.text(
                    badge.center(),
                    Align2::CENTER_CENTER,
                    "NEW BEST",
                    FontId::monospace(10.0),
                    GOLD,
                );
            }
            p.text(
                card.center_top() + Vec2::new(0.0, 92.0),
                Align2::CENTER_CENTER,
                result.rank.to_string(),
                FontId::proportional(64.0),
                rank_colour(result.rank),
            );
            let copy = match result.rank {
                'S' => "Flawless. Every note landed.",
                'A' => "Great take. A few cents off here and there.",
                'B' => "Solid. Tighten the long notes and this is an A.",
                'C' => "Getting there. Hold each note a little longer.",
                _ => "Keep going. A warm-up scale helps find the key.",
            };
            p.text(
                card.center_top() + Vec2::new(0.0, 160.0),
                Align2::CENTER_CENTER,
                copy,
                FontId::proportional(22.0),
                INK,
            );
            let stats = [
                ("SCORE", result.score.to_string()),
                ("PITCH ACCURACY", accuracy_label(result.accuracy)),
                (
                    "NOTES HIT",
                    format!("{}/{}", result.hit, stage.song.notes.len()),
                ),
                ("PERFECT", result.perfect.to_string()),
                ("BEST COMBO", format!("{}×", result.combo)),
            ];
            for (i, (label, value)) in stats.iter().enumerate() {
                let x = card.left() + 40.0 + i as f32 * 110.0;
                p.text(
                    Pos2::new(x, card.center().y + 56.0),
                    Align2::LEFT_TOP,
                    *label,
                    FontId::monospace(10.0),
                    MUTED,
                );
                p.text(
                    Pos2::new(x, card.center().y + 74.0),
                    Align2::LEFT_TOP,
                    value,
                    FontId::monospace(22.0),
                    INK,
                );
            }
            let retry = Rect::from_center_size(
                Pos2::new(card.center().x - 90.0, card.bottom() - 70.0),
                Vec2::new(160.0, 44.0),
            );
            if pill_button(ui, retry, "SING IT AGAIN", true) {
                actions.push(Action::RetrySong);
            }
            let library = Rect::from_center_size(
                Pos2::new(card.center().x + 110.0, card.bottom() - 70.0),
                Vec2::new(170.0, 44.0),
            );
            if pill_button(ui, library, "HOME", false) {
                actions.push(Action::Home);
            }
        }
        let _ = now;
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
        stage
            .judgment
            .as_ref()
            .map(|j| j.text.as_str())
            .unwrap_or("")
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
