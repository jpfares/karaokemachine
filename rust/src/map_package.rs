use crate::model::{CustomWord, LyricLine, Note, PlacedWord, RibbonLink, Song};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const MAP_FORMAT: &str = "karaokemachine-map";
pub const MAP_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MapIndex {
    #[serde(default)]
    pub maps: Vec<MapIndexEntry>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MapIndexEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(rename = "releaseDate", default)]
    pub release_date: String,
    #[serde(default)]
    pub duration: f64,
    #[serde(rename = "sourceUrl", default)]
    pub source_url: String,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub cover: String,
    #[serde(default)]
    pub notes: usize,
    #[serde(default)]
    pub lyrics: usize,
    #[serde(default)]
    pub contributor: String,
}

/// A shareable karaoke mapping. This deliberately contains no audio files:
/// the source URL is the only audio reference.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapPackage {
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default = "default_version")]
    pub version: u32,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(rename = "releaseDate", default)]
    pub release_date: Option<String>,
    #[serde(rename = "sourceUrl")]
    pub source_url: String,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub bpm: Option<f64>,
    #[serde(default)]
    pub low: i32,
    #[serde(default)]
    pub high: i32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(rename = "ribbonLinks", default)]
    pub ribbon_links: Vec<RibbonLink>,
    #[serde(default)]
    pub lines: Vec<LyricLine>,
    #[serde(rename = "placedWords", default)]
    pub placed_words: Vec<PlacedWord>,
    #[serde(rename = "customWords", default)]
    pub custom_words: Vec<CustomWord>,
    #[serde(rename = "lyricsProvider", default)]
    pub lyrics_provider: Option<String>,
    #[serde(rename = "lyricsSynced", default)]
    pub lyrics_synced: Option<bool>,
    /// Cover art as a data: URI. Audio is never embedded.
    #[serde(default)]
    pub cover: Option<String>,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub contributor: Option<String>,
}

fn default_format() -> String {
    MAP_FORMAT.into()
}

fn default_version() -> u32 {
    MAP_VERSION
}

impl MapPackage {
    pub fn from_song(
        root: &Path,
        song: &Song,
        contributor: Option<String>,
    ) -> Result<Self, String> {
        if !song.mapped {
            return Err("Only a submitted mapping can be exported.".into());
        }
        let source_url = song.youtube_url.clone().unwrap_or_default();
        if source_url.trim().is_empty() {
            return Err("This song has no source link, so it cannot be shared safely.".into());
        }
        let cover = read_cover_data_uri(root, song);
        let mut tags = song.tags.clone();
        tags.retain(|tag| tag != "UNMAPPED");
        if !tags.iter().any(|tag| tag == "Mapped") {
            tags.push("Mapped".into());
        }
        Ok(Self {
            format: MAP_FORMAT.into(),
            version: MAP_VERSION,
            id: song.id.clone(),
            title: song.title.clone(),
            artist: song.artist.clone(),
            album: song.album.clone(),
            release_date: song.release_date.clone(),
            source_url,
            duration: song.duration,
            bpm: song.bpm,
            low: song.low,
            high: song.high,
            tags,
            notes: song.notes.clone(),
            ribbon_links: song.ribbon_links.clone(),
            lines: song.lines.clone(),
            placed_words: song.placed_words.clone(),
            custom_words: song.custom_words.clone(),
            lyrics_provider: song.lyrics_provider.clone(),
            lyrics_synced: song.lyrics_synced,
            cover,
            created_at: None,
            contributor,
        })
    }

    pub fn apply_to_song(&self, song: &mut Song) -> Result<(), String> {
        if self.notes.is_empty() {
            return Err("This map has no notes.".into());
        }
        let mut indexed: Vec<(usize, Note)> = self.notes.iter().cloned().enumerate().collect();
        indexed.sort_by(|(_, a), (_, b)| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.midi.cmp(&b.midi))
        });
        let mut remap = vec![None; indexed.len()];
        let mut notes = Vec::with_capacity(indexed.len());
        for (new_index, (old_index, mut note)) in indexed.into_iter().enumerate() {
            remap[old_index] = Some(new_index);
            note.word = None;
            note.word_index = None;
            note.line = None;
            notes.push(note);
        }
        let mut ribbon_links = self.ribbon_links.clone();
        ribbon_links.retain_mut(|link| {
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
        if !ribbon_links.is_empty() {
            for note in &mut notes {
                note.ribbon = None;
            }
        }
        song.title = self.title.clone();
        song.artist = self.artist.clone();
        song.album = self.album.clone();
        song.release_date = self.release_date.clone();
        song.notes = notes;
        song.ribbon_links = ribbon_links;
        song.lines = self.lines.clone();
        song.placed_words = self.placed_words.clone();
        song.custom_words = self.custom_words.clone();
        song.low = self.low;
        song.high = self.high;
        if self.bpm.is_some() {
            song.bpm = self.bpm;
        }
        if !self.source_url.trim().is_empty() {
            song.youtube_url = Some(self.source_url.clone());
        }
        song.lyrics_provider = self.lyrics_provider.clone();
        song.lyrics_synced = self.lyrics_synced;
        song.mapped = true;
        song.draft_notes.clear();
        song.tags.clear();
        for tag in &self.tags {
            if tag != "UNMAPPED" && tag != "Mapped" {
                song.tags.push(tag.clone());
            }
        }
        song.tags.push("Mapped".into());
        song.tags.push("Community map".into());
        Ok(())
    }

    pub fn save_cover(&self, dir: &Path) -> Result<Option<String>, String> {
        let Some(cover) = &self.cover else {
            return Ok(None);
        };
        let Some((meta, payload)) = cover.split_once(";base64,") else {
            return Err("The map cover is not a base64 data URI.".into());
        };
        let mime = meta.strip_prefix("data:").unwrap_or(meta);
        let extension = if mime.contains("png") {
            "png"
        } else if mime.contains("webp") {
            "webp"
        } else if mime.contains("svg") {
            "svg"
        } else {
            "jpg"
        };
        let bytes = STANDARD
            .decode(payload)
            .map_err(|error| format!("Could not decode the map cover: {error}"))?;
        // Use a filename derived from the map id so installing a map cannot
        // overwrite a hand-picked `cover.jpg` (or another map's artwork).
        let id: String = self
            .id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '-'
                }
            })
            .collect();
        let id = if id.is_empty() { "map" } else { &id };
        let name = format!("community-cover-{id}.{extension}");
        let path = dir.join(&name);
        fs::write(&path, bytes)
            .map_err(|error| format!("Could not save the map cover: {error}"))?;
        Ok(Some(name))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        crate::library::write_json_atomic(path, self)
    }

    pub fn from_json(raw: &str) -> Result<Self, String> {
        let map: Self = serde_json::from_str(raw)
            .map_err(|error| format!("Could not parse map package: {error}"))?;
        if map.format != MAP_FORMAT {
            return Err("That file is not a KaraokeMachine map package.".into());
        }
        if map.notes.is_empty() {
            return Err("That map package has no notes.".into());
        }
        Ok(map)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        Self::from_json(&raw)
    }

    pub fn file_name(&self) -> String {
        let mut slug = String::new();
        for ch in format!("{} {}", self.title, self.artist.clone().unwrap_or_default()).chars() {
            if ch.is_ascii_alphanumeric() {
                slug.push(ch.to_ascii_lowercase());
            } else if !slug.ends_with('-') {
                slug.push('-');
            }
        }
        let slug = slug.trim_matches('-');
        if slug.is_empty() {
            format!("{}.json", self.id)
        } else {
            format!("{slug}-{}.json", self.id)
        }
    }
}

fn read_cover_data_uri(root: &Path, song: &Song) -> Option<String> {
    let name = song.cover.as_ref()?;
    let path = PathBuf::from(root)
        .join("assets/songs")
        .join(&song.id)
        .join(name);
    let bytes = fs::read(&path).ok()?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mime = match extension.as_str() {
        "png" => "image/png",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "image/jpeg",
    };
    Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn song() -> Song {
        serde_json::from_value(json!({
            "id": "test-map",
            "title": "Test Song",
            "artist": "Test Artist",
            "album": "Test Album",
            "source": "import",
            "duration": 10.0,
            "mapped": true,
            "notes": [
                {"start": 1.0, "dur": 0.5, "midi": 60, "ribbon": 1}
            ],
            "lines": [{
                "start": 1.0, "end": 2.0, "text": "hello",
                "words": [{"text": "hello", "start": 1.0, "end": 2.0}]
            }],
            "placedWords": [{
                "text": "hello", "start": 1.5, "y": 0.5, "line": 0, "wordIndex": 0
            }],
            "customWords": [{"id": 1, "text": "whoa"}],
            "low": 48,
            "high": 72,
            "audio": {"backing": "backing.mp3", "guide": "guide.mp3"},
            "youtubeUrl": "https://example.com/audio.mp3"
        }))
        .expect("test song")
    }

    #[test]
    fn map_package_round_trips_without_audio() {
        let song = song();
        let package = MapPackage::from_song(Path::new("."), &song, Some("tester".into()))
            .expect("export map");
        assert_eq!(package.source_url, "https://example.com/audio.mp3");
        assert_eq!(package.notes.len(), 1);
        assert_eq!(package.placed_words.len(), 1);
        let value = serde_json::to_value(&package).expect("serialize map");
        assert!(value.get("audio").is_none());
        assert!(value.get("audioFiles").is_none());

        let detached: MapPackage = serde_json::from_value(value).expect("deserialize map package");
        let mut target = song;
        target.mapped = false;
        target.notes.clear();
        detached.apply_to_song(&mut target).expect("apply map");
        assert!(target.mapped);
        assert_eq!(target.notes.len(), 1);
        assert_eq!(target.placed_words.len(), 1);
        assert_eq!(
            target
                .tags
                .iter()
                .filter(|tag| *tag == "Community map")
                .count(),
            1
        );
    }

    #[test]
    fn unmapped_song_cannot_be_exported() {
        let mut song = song();
        song.mapped = false;
        assert!(MapPackage::from_song(Path::new("."), &song, None).is_err());
    }

    #[test]
    fn map_package_carries_independent_ribbon_links() {
        let mut song = song();
        song.notes.push(
            serde_json::from_value(json!({
                "start": 2.0,
                "dur": 0.5,
                "midi": 64
            }))
            .expect("second note"),
        );
        song.ribbon_links = vec![RibbonLink {
            from: 0,
            to: 1,
            from_side: crate::model::RibbonSide::Right,
            to_side: crate::model::RibbonSide::Left,
        }];
        let package = MapPackage::from_song(Path::new("."), &song, None).expect("export map");
        assert_eq!(package.ribbon_links.len(), 1);
        let value = serde_json::to_value(&package).expect("serialize map");
        assert!(value.get("ribbonLinks").is_some());

        let mut target = song;
        target.mapped = false;
        target.notes.clear();
        target.ribbon_links.clear();
        package.apply_to_song(&mut target).expect("apply map");
        assert_eq!(target.notes.len(), 2);
        assert_eq!(target.ribbon_links.len(), 1);
        assert_eq!(target.ribbon_links[0].from, 0);
        assert_eq!(target.ribbon_links[0].to, 1);
    }

    #[test]
    fn map_cover_does_not_overwrite_a_hand_picked_cover() {
        let dir =
            std::env::temp_dir().join(format!("karaokemachine-map-cover-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        fs::write(dir.join("cover.jpg"), b"old cover").expect("old cover");

        let map: MapPackage = serde_json::from_value(json!({
            "id": "yt-test",
            "title": "Test Map",
            "sourceUrl": "https://example.com/audio.mp3",
            "cover": "data:image/png;base64,aGVsbG8=",
            "notes": [{"start": 1.0, "dur": 0.5, "midi": 60}]
        }))
        .expect("map package");

        let name = map
            .save_cover(&dir)
            .expect("save map cover")
            .expect("cover");
        assert_eq!(name, "community-cover-yt-test.png");
        assert_eq!(
            fs::read(dir.join("cover.jpg")).expect("old cover"),
            b"old cover"
        );
        assert_eq!(fs::read(dir.join(&name)).expect("new cover"), b"hello");
        let _ = fs::remove_dir_all(&dir);
    }
}
