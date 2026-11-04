use crate::model::{Album, ScoreRecord, Scores, Settings, Song};
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
                songs.push(song);
            }
            Err(error) => eprintln!("bad song {}: {error}", file.display()),
        }
    }
    songs.sort_by(|a, b| {
        let a_scale = a.source == "scale";
        let b_scale = b.source == "scale";
        if a_scale != b_scale {
            return if a_scale { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
        }
        let ao = a.order.unwrap_or(999);
        let bo = b.order.unwrap_or(999);
        ao.cmp(&bo).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
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
            cover_path: song.cover.as_ref().map(|cover| {
                PathBuf::from("assets/songs").join(&song.id).join(cover)
            }),
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

pub fn save_score(scores: &mut Scores, song_id: &str, record: ScoreRecord) -> bool {
    let better = scores
        .get(song_id)
        .map(|current| record.score > current.score)
        .unwrap_or(true);
    if better {
        scores.insert(song_id.to_string(), record);
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
        let dir = std::env::temp_dir().join(format!("karaokemachine-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn atomic_write_leaves_only_the_target() {
        let dir = scratch("atomic");
        let path = dir.join("scores.json");
        let mut scores = Scores::new();
        scores.insert("song".into(), ScoreRecord { score: 7, accuracy: 1.0, combo: 2 });
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
        assert_eq!(fs::read_to_string(dir.join("scores.json.bak")).expect("backup"), "{");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn song_backup_copies_the_file() {
        let dir = scratch("backup");
        assert_eq!(backup_song_file(&dir), Ok(false));
        fs::write(dir.join("song.json"), "{}").expect("song file");
        assert_eq!(backup_song_file(&dir), Ok(true));
        assert_eq!(fs::read_to_string(dir.join("song.json.bak")).expect("backup"), "{}");
        let _ = fs::remove_dir_all(&dir);
    }
}
