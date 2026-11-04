//! Automatic mapping: turn the import pipeline's melody suggestions into a
//! playable mapping without drawing it by hand.
//!
//! The pipeline tags each suggested note with the lyric word it was aligned
//! to. Notes that share a word become one ribbon and the word is placed over
//! the first of them; lyric words no note was aligned to are placed at their
//! own lyric timing, so the mapping is complete and can be submitted. The
//! result is only as good as the suggestions and stays fully editable. The
//! upload form saves it as a finished level named `maps/Auto-map.json`; the
//! library row flags a level that still carries that name (`is_auto_mapped`).

use crate::editor::links_from_legacy_notes;
use crate::model::{lane_range, pitch_range, split_line_words, Note, PlacedWord, Song};
use crate::model::{MapFile, MAP_FILE_FORMAT, MAP_FILE_VERSION};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// File stem of the level the upload form writes: `maps/Auto-map.json`.
pub const AUTO_MAP_FILE_STEM: &str = "Auto-map";

pub struct AutoMapping {
    pub notes: Vec<Note>,
    pub words: Vec<PlacedWord>,
    /// Lyric words no note was aligned to, placed by lyric timing alone.
    pub unmatched_words: usize,
}

impl AutoMapping {
    /// A short description for a toast or status line.
    pub fn summary(&self) -> String {
        let ribbons = self
            .notes
            .iter()
            .filter_map(|note| note.ribbon)
            .collect::<std::collections::BTreeSet<_>>();
        let mut text = format!(
            "Auto-mapped {} notes in {} ribbons and placed {} words.",
            self.notes.len(),
            ribbons.len(),
            self.words.len()
        );
        if self.unmatched_words > 0 {
            text.push_str(&format!(
                " {} words had no detected melody and sit at their lyric time.",
                self.unmatched_words
            ));
        }
        text
    }
}

/// Build a mapping from `suggestions` for `song`.
///
/// `lane` is the (low, high) pitch range word labels are positioned against;
/// the editor passes its own so labels land on their notes, anyone else gets
/// the range the notes imply. Placed custom words in `keep` are carried over,
/// since the suggestions know nothing about them. Fails only when there is
/// nothing to map from.
pub fn auto_map(
    song: &Song,
    suggestions: &[Note],
    lane: Option<(i32, i32)>,
    keep: &[PlacedWord],
) -> Result<AutoMapping, String> {
    let mut notes: Vec<Note> = suggestions
        .iter()
        .filter(|note| note.dur > 0.0)
        .cloned()
        .collect();
    if notes.is_empty() {
        return Err(
            "There are no melody suggestions to map from. Import the song again or draw the notes by hand."
                .into(),
        );
    }
    notes.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(Ordering::Equal));
    // Labels are a fraction of the lane the editor and stage show, so without
    // a caller's lane use the same formula they do.
    let (low, high) = lane.unwrap_or_else(|| {
        let midis: Vec<i32> = notes.iter().map(|note| note.midi).collect();
        lane_range(&midis, (song.low, song.high))
    });
    let line_words: Vec<Vec<(String, f64, f64)>> =
        song.lines.iter().map(split_line_words).collect();

    let mut ribbons: BTreeMap<(usize, Option<usize>, String), u32> = BTreeMap::new();
    let mut words: Vec<PlacedWord> = keep
        .iter()
        .filter(|word| word.custom_id.is_some())
        .cloned()
        .collect();
    let mut next_ribbon_id = notes
        .iter()
        .filter_map(|note| note.ribbon)
        .max()
        .unwrap_or(0)
        + 1;
    for note in &mut notes {
        let (start, dur, midi, line, word_index) =
            (note.start, note.dur, note.midi, note.line, note.word_index);
        let Some(word) = note.word.take() else {
            note.word_index = None;
            note.line = None;
            continue;
        };
        let key = (line.unwrap_or(usize::MAX), word_index, word.clone());
        let ribbon = *ribbons.entry(key).or_insert_with(|| {
            // A note tagged with several words ("hello world") stands for a
            // run of lyric words the tracker could not split; give each its
            // own label, spread across the note, so none is placed twice.
            let parts: Vec<&str> = word.split_whitespace().collect();
            let count = parts.len().max(1);
            for (k, part) in parts.into_iter().enumerate() {
                let index = word_index.map(|index| index + k);
                let valid = match (line, index) {
                    (Some(line), Some(index)) => line_words
                        .get(line)
                        .is_some_and(|words| index < words.len()),
                    _ => false,
                };
                // Without a resolvable lyric slot the label would never match
                // a word-bank token; keep the ribbon and let the lyric-time
                // fallback place the real word instead.
                if valid {
                    words.push(PlacedWord {
                        text: part.to_string(),
                        start: start + dur * k as f64 / count as f64,
                        y: word_y(midi, low, high),
                        line,
                        word_index: index,
                        custom_id: None,
                    });
                }
            }
            let id = next_ribbon_id;
            next_ribbon_id += 1;
            id
        });
        if note.ribbon.is_none() {
            note.ribbon = Some(ribbon);
        }
        note.word_index = None;
        note.line = None;
    }

    // Lyric words no note was aligned to still need a place, or the mapping
    // cannot be submitted. Their own lyric timing is the best guess.
    let mut unmatched_words = 0;
    for (line_index, line) in line_words.iter().enumerate() {
        for (word_index, (text, start, _end)) in line.iter().enumerate() {
            let placed = words
                .iter()
                .any(|word| word.line == Some(line_index) && word.word_index == Some(word_index));
            if !placed {
                unmatched_words += 1;
                words.push(PlacedWord {
                    text: text.clone(),
                    start: *start,
                    y: 0.5,
                    line: Some(line_index),
                    word_index: Some(word_index),
                    custom_id: None,
                });
            }
        }
    }
    words.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(Ordering::Equal));
    Ok(AutoMapping {
        notes,
        words,
        unmatched_words,
    })
}

/// Whether the song has a finished level written by Auto-map (and not since
/// renamed), for the library row.
pub fn is_auto_mapped(song: &Song) -> bool {
    song.maps
        .iter()
        .any(|map| !map.draft && map.name == AUTO_MAP_FILE_STEM)
}

/// A finished level built from a freshly imported song's suggestions, plus
/// the summary for the toast. Refuses a song that already has any map file:
/// the importer returns an existing folder untouched, so a re-import must
/// never add a level beside hand work unasked.
pub fn auto_mapped_level(song: &Song) -> Result<(MapFile, String), String> {
    if !song.maps.is_empty() {
        return Err("This song already has map files, so it was left alone.".into());
    }
    let mapping = auto_map(song, &song.suggested_notes, None, &song.placed_words)?;
    let summary = mapping.summary();
    let mut notes = mapping.notes;
    // Ribbon groups become the explicit connectors the stage and editor use.
    let ribbon_links = links_from_legacy_notes(&notes);
    for note in &mut notes {
        note.ribbon = None;
    }
    let (low, high) = pitch_range(&notes).unwrap_or((song.low, song.high));
    Ok((
        MapFile {
            format: MAP_FILE_FORMAT.into(),
            version: MAP_FILE_VERSION,
            draft: false,
            notes,
            lines: song.lines.clone(),
            placed_words: mapping.words,
            custom_words: song.custom_words.clone(),
            ribbon_links,
            low,
            high,
        },
        summary,
    ))
}

/// Where a word label sits vertically for a note, as a fraction of the lane
/// area: 0 at the top of `high`, 1 at the bottom of `low`, on the note's own
/// lane centre.
fn word_y(midi: i32, low: i32, high: i32) -> f32 {
    (((high - midi) as f32 + 0.5) / (high - low + 1).max(1) as f32).clamp(0.04, 0.96)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn song() -> Song {
        serde_json::from_value(json!({
            "id": "test",
            "title": "Test",
            "duration": 20.0,
            "lines": [
                { "start": 1.0, "end": 3.0, "text": "hello world", "words": [
                    { "text": "hello", "start": 1.0, "end": 2.0 },
                    { "text": "world", "start": 2.0, "end": 3.0 }
                ]},
                { "start": 5.0, "end": 6.0, "text": "lonely", "words": [
                    { "text": "lonely", "start": 5.0, "end": 6.0 }
                ]}
            ],
            "tags": ["Imported", "UNMAPPED"]
        }))
        .expect("test song")
    }

    fn note(start: f64, midi: i32, word: Option<(&str, usize, usize)>) -> Note {
        serde_json::from_value(match word {
            Some((text, line, index)) => json!({
                "start": start, "dur": 0.4, "midi": midi, "word": text, "line": line, "wordIndex": index
            }),
            None => json!({ "start": start, "dur": 0.4, "midi": midi }),
        })
        .expect("test note")
    }

    fn texts(mapping: &AutoMapping) -> Vec<String> {
        mapping.words.iter().map(|word| word.text.clone()).collect()
    }

    #[test]
    fn notes_sharing_a_word_become_one_ribbon_with_the_word_on_the_first() {
        let suggestions = vec![
            note(1.0, 60, Some(("hello", 0, 0))),
            note(1.5, 62, Some(("hello", 0, 0))),
            note(2.0, 64, Some(("world", 0, 1))),
            note(8.0, 55, None),
        ];
        let mapping = auto_map(&song(), &suggestions, None, &[]).expect("mapping");
        assert_eq!(mapping.notes[0].ribbon, mapping.notes[1].ribbon);
        assert_ne!(mapping.notes[0].ribbon, mapping.notes[2].ribbon);
        assert_eq!(mapping.notes[3].ribbon, None);
        assert!(mapping
            .notes
            .iter()
            .all(|note| note.word.is_none() && note.line.is_none()));
        let hello = mapping
            .words
            .iter()
            .find(|word| word.text == "hello")
            .expect("hello placed");
        assert_eq!(hello.start, 1.0);
        assert_eq!((hello.line, hello.word_index), (Some(0), Some(0)));
    }

    #[test]
    fn words_without_notes_are_placed_at_their_lyric_time_and_counted() {
        let suggestions = vec![note(1.0, 60, Some(("hello", 0, 0)))];
        let mapping = auto_map(&song(), &suggestions, None, &[]).expect("mapping");
        assert_eq!(texts(&mapping), vec!["hello", "world", "lonely"]);
        assert_eq!(mapping.unmatched_words, 2);
        assert!(mapping.summary().contains("2 words had no detected melody"));
        let lonely = &mapping.words[2];
        assert_eq!(lonely.start, 5.0);
        assert_eq!((lonely.line, lonely.word_index), (Some(1), Some(0)));
    }

    #[test]
    fn a_phrase_on_one_note_places_each_word_once() {
        let suggestions = vec![note(1.0, 60, Some(("hello world", 0, 0)))];
        let mapping = auto_map(&song(), &suggestions, None, &[]).expect("mapping");
        assert_eq!(texts(&mapping), vec!["hello", "world", "lonely"]);
        assert_eq!(mapping.words[0].start, 1.0);
        assert_eq!(mapping.words[1].start, 1.2);
        assert_eq!(mapping.words[1].word_index, Some(1));
        assert_eq!(mapping.unmatched_words, 1);
    }

    #[test]
    fn a_word_tag_without_a_lyric_slot_keeps_the_ribbon_but_no_orphan_label() {
        let mut no_index = note(1.0, 60, Some(("hello", 0, 0)));
        no_index.word_index = None;
        let out_of_range = note(2.0, 62, Some(("hello", 7, 0)));
        let mapping = auto_map(&song(), &[no_index, out_of_range], None, &[]).expect("mapping");
        assert!(mapping.notes.iter().all(|note| note.ribbon.is_some()));
        assert!(mapping.words.iter().all(|word| word.word_index.is_some()));
        assert_eq!(texts(&mapping), vec!["hello", "world", "lonely"]);
    }

    #[test]
    fn placed_custom_words_are_kept() {
        let keep = vec![PlacedWord {
            text: "whoa".into(),
            start: 4.0,
            y: 0.3,
            line: None,
            word_index: None,
            custom_id: Some(1),
        }];
        let mapping = auto_map(&song(), &[note(1.0, 60, None)], None, &keep).expect("mapping");
        assert!(mapping.words.iter().any(|word| word.custom_id == Some(1)));
    }

    #[test]
    fn the_editor_lane_range_positions_labels() {
        let mapping = auto_map(
            &song(),
            &[note(1.0, 60, Some(("hello", 0, 0)))],
            Some((50, 70)),
            &[],
        )
        .expect("mapping");
        assert_eq!(mapping.words[0].y, 0.5);
    }

    #[test]
    fn without_a_lane_labels_sit_on_the_note_the_stage_draws() {
        // The stage and editor lay words out against lane_range of the notes,
        // so the upload path (no editor lane) must land on the same rows.
        let notes = [
            note(1.0, 60, Some(("hello", 0, 0))),
            note(2.0, 72, Some(("world", 0, 1))),
        ];
        let mapping = auto_map(&song(), &notes, None, &[]).expect("mapping");
        let (low, high) = lane_range(&[60, 72], (48, 72));
        assert_eq!((low, high), (54, 79));
        let lane = |midi: i32| ((high - midi) as f32 + 0.5) / (high - low + 1) as f32;
        assert_eq!(mapping.words[0].y, lane(60));
        assert_eq!(mapping.words[1].y, lane(72));
    }

    #[test]
    fn nothing_to_map_from_is_an_error() {
        assert!(auto_map(&song(), &[], None, &[]).is_err());
        let mut zero = note(1.0, 60, None);
        zero.dur = 0.0;
        assert!(auto_map(&song(), &[zero], None, &[]).is_err());
    }

    #[test]
    fn an_imported_song_becomes_a_finished_level_with_connectors() {
        let mut song = song();
        song.suggested_notes = vec![
            note(1.0, 60, Some(("hello", 0, 0))),
            note(1.5, 62, Some(("hello", 0, 0))),
            note(2.5, 72, None),
        ];
        let (level, summary) = auto_mapped_level(&song).expect("level");
        assert!(!level.draft);
        assert_eq!(level.notes.len(), 3);
        assert!(level.notes.iter().all(|note| note.ribbon.is_none()));
        assert_eq!(
            level.ribbon_links.len(),
            1,
            "one word over two notes is one connector"
        );
        assert_eq!(level.placed_words.len(), 3);
        assert_eq!((level.low, level.high), (58, 74));
        assert!(summary.contains("3 notes"), "{summary}");
    }

    #[test]
    fn a_song_with_map_files_is_left_alone() {
        use crate::model::MapInfo;
        let mut song = song();
        song.suggested_notes = vec![note(1.0, 60, None)];
        assert!(auto_mapped_level(&song).is_ok());
        song.maps.push(MapInfo {
            path: std::path::PathBuf::from("maps/Level.json"),
            name: "Level".into(),
            draft: true,
            notes: 0,
        });
        assert!(auto_mapped_level(&song).is_err());
        assert!(!is_auto_mapped(&song));
        song.maps[0].name = AUTO_MAP_FILE_STEM.into();
        song.maps[0].draft = false;
        assert!(is_auto_mapped(&song));
    }
}
