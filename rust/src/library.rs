use crate::model::{Album, MapFile, MapInfo, ScoreRecord, Scores, Settings, Song};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Serialize `value` as pretty JSON and move it into place with a rename, so
/// a crash or full disk mid-write leaves the previous file intact instead of
/// a truncated one. The temporary file sits beside the target.
pub fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw).map_err(|error| format!("Could not write {}: {error}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        format!("Could not replace {}: {error}", path.display())
    })
}

/// Read a JSON file, or the type's default when it is missing. A file that
/// exists but does not parse is set aside as `<name>.json.bak` and reported,
/// never silently replaced: the next save would otherwise destroy it.
pub fn load_json_or_default<T: DeserializeOwned + Default>(path: &Path) -> T {
    let Ok(raw) = fs::read_to_string(path) else {
        return T::default();
    };
    match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(error) => {
            let backup = path.with_extension("json.bak");
            eprintln!(
                "could not read {}: {error}; keeping it as {}",
                path.display(),
                backup.display()
            );
            if let Err(error) = fs::rename(path, &backup) {
                eprintln!("could not set aside {}: {error}", path.display());
            }
            T::default()
        }
    }
}

/// Copy a song folder's `song.json` to `song.json.bak` before a mapping is
/// replaced. Returns whether there was a file to back up.
pub fn backup_song_file(dir: &Path) -> Result<bool, String> {
    let path = dir.join("song.json");
    if !path.exists() {
        return Ok(false);
    }
    let backup = dir.join("song.json.bak");
    fs::copy(&path, &backup)
        .map(|_| true)
        .map_err(|error| format!("Could not back up {}: {error}", path.display()))
}

pub fn find_root() -> PathBuf {
    if let Ok(root) = std::env::var("KARAOKEMACHINE_ROOT") {
        let path = PathBuf::from(root);
        if path.join("assets/songs").is_dir() {
            return path;
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join("assets/songs").is_dir() {
        return cwd;
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut cursor = exe.parent().map(Path::to_path_buf);
        for _ in 0..5 {
            if let Some(path) = cursor.clone() {
                if path.join("assets/songs").is_dir() {
                    return path;
                }
                cursor = path.parent().map(Path::to_path_buf);
            }
        }
    }
    cwd
}

pub fn maps_dir(root: &Path, song_id: &str) -> PathBuf {
    root.join("assets/songs").join(song_id).join("maps")
}

pub fn load_map_file(path: &Path) -> Result<MapFile, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let map: MapFile = serde_json::from_str(&raw)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
    if map.format != crate::model::MAP_FILE_FORMAT {
        return Err(format!("{} is not a map file.", path.display()));
    }
    Ok(map)
}

pub fn save_map_file(path: &Path, map: &MapFile) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
    }
    write_json_atomic(path, map)
}

pub fn load_map_files(root: &Path, song_id: &str) -> Vec<MapInfo> {
    let dir = maps_dir(root, song_id);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut maps = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("map")
            .to_string();
        match load_map_file(&path) {
            Ok(map) => maps.push(MapInfo {
                path,
                name,
                draft: map.draft,
                notes: map.notes.len(),
            }),
            Err(error) => eprintln!("bad map file: {error}"),
        }
    }
    maps.sort_by(|a, b| {
        a.draft
            .cmp(&b.draft)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    maps
}

fn prepare_song_maps(root: &Path, song: &mut Song) {
    let mut maps = load_map_files(root, &song.id);
    let had_inline = !song.notes.is_empty()
        || !song.draft_notes.is_empty()
        || !song.placed_words.is_empty()
        || !song.custom_words.is_empty()
        || !song.ribbon_links.is_empty();
    let legacy_path = maps_dir(root, &song.id).join("Legacy.json");
    if had_inline && !legacy_path.exists() {
        if let Some(map) = MapFile::from_legacy_song(song) {
            match save_map_file(&legacy_path, &map) {
                Ok(()) => {
                    eprintln!("migrated {} into {}", song.id, legacy_path.display());
                    maps = load_map_files(root, &song.id);
                }
                Err(error) => eprintln!("could not migrate {}: {error}", song.id),
            }
        }
    }
    if !maps.is_empty() && (!had_inline || legacy_path.exists()) {
        song.notes.clear();
        song.draft_notes.clear();
        song.placed_words.clear();
        song.custom_words.clear();
        song.ribbon_links.clear();
        song.mapped = maps.iter().any(|map| !map.draft);
        if had_inline {
            let song_file = root.join("assets/songs").join(&song.id).join("song.json");
            if let Err(error) = write_json_atomic(&song_file, song) {
                eprintln!("could not clean {}: {error}", song_file.display());
            }
        }
    } else if maps.is_empty() {
        song.mapped = false;
    }
    song.maps = maps;
}

pub fn load_songs(root: &Path) -> Vec<Song> {
    let dir = root.join("assets/songs");
    let mut songs = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return songs;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let file = entry.path().join("song.json");
        let Ok(raw) = fs::read_to_string(&file) else {
            continue;
        };
        match serde_json::from_str::<Song>(&raw) {
            Ok(mut song) => {
                if song.id.is_empty() {
                    song.id = entry.file_name().to_string_lossy().to_string();
                }
                prepare_song_maps(root, &mut song);
                songs.push(song);
            }
            Err(error) => eprintln!("bad song {}: {error}", file.display()),
        }
    }
    songs.sort_by(|a, b| {
        let a_scale = a.source == "scale";
        let b_scale = b.source == "scale";
        if a_scale != b_scale {
            return if a_scale {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        let ao = a.order.unwrap_or(999);
        let bo = b.order.unwrap_or(999);
        ao.cmp(&bo)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    songs
}

pub fn build_albums(songs: &[Song]) -> Vec<Album> {
    let mut albums: Vec<Album> = Vec::new();
    let scales: Vec<usize> = songs
        .iter()
        .enumerate()
        .filter(|(_, song)| song.source == "scale")
        .map(|(i, _)| i)
        .collect();
    if !scales.is_empty() {
        let cover = songs[scales[0]].cover.as_ref().map(|cover| {
            PathBuf::from("assets/songs")
                .join(&songs[scales[0]].id)
                .join(cover)
        });
        albums.push(Album {
            title: "Warm-ups".into(),
            artist: "KaraokeMachine".into(),
            cover_path: cover,
            warmup: true,
            songs: scales,
        });
    }
    let mut groups: BTreeMap<String, Album> = BTreeMap::new();
    for (index, song) in songs.iter().enumerate() {
        if song.source == "scale" {
            continue;
        }
        let title = song.album_display().to_string();
        let artist = song.artist_display().to_string();
        let key = format!("{}::{}", title.to_lowercase(), artist.to_lowercase());
        let album = groups.entry(key).or_insert_with(|| Album {
            title: title.clone(),
            artist: artist.clone(),
            cover_path: song
                .cover
                .as_ref()
                .map(|cover| PathBuf::from("assets/songs").join(&song.id).join(cover)),
            warmup: false,
            songs: Vec::new(),
        });
        album.songs.push(index);
        if album.cover_path.is_none() {
            album.cover_path = song
                .cover
                .as_ref()
                .map(|cover| PathBuf::from("assets/songs").join(&song.id).join(cover));
        }
    }
    let mut imported: Vec<Album> = groups.into_values().collect();
    imported.sort_by_key(|album| album.title.to_lowercase());
    albums.extend(imported);
    albums
}

pub fn song_audio_path(root: &Path, song: &Song, which: &str) -> PathBuf {
    let name = match which {
        "backing" => Some(song.audio.backing.as_str()),
        "vocals" => song.audio.vocals.as_deref().or(song.audio.guide.as_deref()),
        _ => None,
    };
    match name {
        Some(name) => root.join("assets/songs").join(&song.id).join(name),
        None => root.join("assets/songs").join(&song.id).join("backing.mp3"),
    }
}

fn home_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    #[cfg(windows)]
    if let Ok(profile) = std::env::var("USERPROFILE") {
        if !profile.is_empty() {
            return PathBuf::from(profile);
        }
    }
    PathBuf::from(".")
}

fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("KARAOKEMACHINE_CONFIG_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            if !appdata.is_empty() {
                return PathBuf::from(appdata).join("KaraokeMachine");
            }
        }
    }
    home_dir().join(".config/karaoke-machine")
}

pub fn load_settings() -> Settings {
    load_json_or_default(&config_dir().join("settings.json"))
}

pub fn save_settings(settings: &Settings) {
    save_config_file("settings.json", settings);
}

pub fn load_scores() -> Scores {
    load_json_or_default(&config_dir().join("scores.json"))
}

pub fn save_scores(scores: &Scores) {
    save_config_file("scores.json", scores);
}

fn save_config_file(name: &str, value: &impl Serialize) {
    let dir = config_dir();
    if let Err(error) = fs::create_dir_all(&dir) {
        eprintln!("could not create {}: {error}", dir.display());
        return;
    }
    if let Err(error) = write_json_atomic(&dir.join(name), value) {
        eprintln!("{error}");
    }
}

/// Keep `record` when it beats the stored take for `song_id` (see
/// `ScoreRecord::beats`). Returns whether it did.
pub fn record_best(scores: &mut Scores, song_id: &str, record: ScoreRecord) -> bool {
    let better = scores
        .get(song_id)
        .map(|current| record.beats(current))
        .unwrap_or(true);
    if better {
        scores.insert(song_id.to_string(), record);
    }
    better
}

pub fn save_score(scores: &mut Scores, song_id: &str, record: ScoreRecord) -> bool {
    let better = record_best(scores, song_id, record);
    if better {
        save_scores(scores);
    }
    better
}

pub fn read_songlist() -> Vec<String> {
    let file = home_dir().join("Downloads/karaoke-songs.txt");
    fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

pub fn read_songlist_path() -> String {
    home_dir()
        .join("Downloads/karaoke-songs.txt")
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("karaokemachine-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn a_more_accurate_take_replaces_a_higher_scoring_one() {
        let mut scores = Scores::new();
        let long_combos = ScoreRecord {
            score: 5000,
            accuracy: 78.0,
            combo: 40,
        };
        let cleaner = ScoreRecord {
            score: 4500,
            accuracy: 85.0,
            combo: 9,
        };
        assert!(record_best(&mut scores, "song", long_combos.clone()));
        assert!(record_best(&mut scores, "song", cleaner.clone()));
        assert_eq!(scores["song"].accuracy, 85.0);
        assert!(!record_best(&mut scores, "song", long_combos));
        assert_eq!(scores["song"].score, cleaner.score);
    }

    #[test]
    fn atomic_write_leaves_only_the_target() {
        let dir = scratch("atomic");
        let path = dir.join("scores.json");
        let mut scores = Scores::new();
        scores.insert(
            "song".into(),
            ScoreRecord {
                score: 7,
                accuracy: 1.0,
                combo: 2,
            },
        );
        write_json_atomic(&path, &scores).expect("write");
        assert!(path.exists());
        assert!(!dir.join("scores.json.tmp").exists());
        let back: Scores = load_json_or_default(&path);
        assert_eq!(back["song"].score, 7);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_loads_as_default() {
        let dir = scratch("missing");
        let scores: Scores = load_json_or_default(&dir.join("scores.json"));
        assert!(scores.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_set_aside_not_overwritten() {
        let dir = scratch("corrupt");
        let path = dir.join("scores.json");
        fs::write(&path, "{").expect("corrupt file");
        let scores: Scores = load_json_or_default(&path);
        assert!(scores.is_empty());
        assert!(!path.exists());
        assert_eq!(
            fs::read_to_string(dir.join("scores.json.bak")).expect("backup"),
            "{"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn song_backup_copies_the_file() {
        let dir = scratch("backup");
        assert_eq!(backup_song_file(&dir), Ok(false));
        fs::write(dir.join("song.json"), "{}").expect("song file");
        assert_eq!(backup_song_file(&dir), Ok(true));
        assert_eq!(
            fs::read_to_string(dir.join("song.json.bak")).expect("backup"),
            "{}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_inline_mapping_migrates_to_a_map_file() {
        let root = scratch("legacy-map");
        let song_dir = root.join("assets/songs/test-song");
        fs::create_dir_all(&song_dir).expect("song dir");
        fs::write(
            song_dir.join("song.json"),
            serde_json::json!({
                "id": "test-song",
                "title": "Test Song",
                "audio": {"backing": "backing.mp3"},
                "mapped": true,
                "low": 48,
                "high": 72,
                "notes": [
                    {"start": 1.0, "dur": 0.5, "midi": 60},
                    {"start": 2.0, "dur": 0.5, "midi": 62}
                ]
            })
            .to_string(),
        )
        .expect("song json");

        let songs = load_songs(&root);
        assert_eq!(songs.len(), 1);
        let song = &songs[0];
        assert!(song.mapped);
        assert_eq!(song.maps.len(), 1);
        assert!(!song.maps[0].draft);
        assert!(song.notes.is_empty());
        let map = load_map_file(&song.maps[0].path).expect("migrated map");
        assert_eq!(map.notes.len(), 2);
        assert!(song.maps[0].path.ends_with("Legacy.json"));

        let again = load_songs(&root);
        assert_eq!(again[0].maps.len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn map_files_are_listed_without_relying_on_song_notes() {
        let root = scratch("map-files");
        let song_dir = root.join("assets/songs/test-song");
        let maps = song_dir.join("maps");
        fs::create_dir_all(&maps).expect("maps dir");
        fs::write(
            song_dir.join("song.json"),
            serde_json::json!({
                "id": "test-song",
                "title": "Test Song",
                "audio": {"backing": "backing.mp3"}
            })
            .to_string(),
        )
        .expect("song json");
        let map = serde_json::json!({
            "format": "karaokemachine-level",
            "version": 1,
            "draft": false,
            "notes": [{"start": 1.0, "dur": 0.5, "midi": 60}],
            "lines": [],
            "low": 48,
            "high": 72
        });
        fs::write(maps.join("Level.json"), map.to_string()).expect("level");
        let mut draft = map.clone();
        draft["draft"] = serde_json::json!(true);
        fs::write(maps.join("Draft.json"), draft.to_string()).expect("draft");

        let songs = load_songs(&root);
        assert_eq!(songs[0].maps.len(), 2);
        assert!(songs[0].mapped);
        assert_eq!(songs[0].maps.iter().filter(|map| map.draft).count(), 1);
        let _ = fs::remove_dir_all(&root);
    }
}
