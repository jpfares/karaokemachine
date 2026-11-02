use crate::model::{Album, ScoreRecord, Scores, Settings, Song};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn find_root() -> PathBuf {
    if let Ok(root) = std::env::var("KARAOKEMACHINE_ROOT").or_else(|_| std::env::var("KEYLINE_ROOT")) {
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
    imported.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
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
    if let Ok(dir) = std::env::var("KARAOKEMACHINE_CONFIG_DIR").or_else(|_| std::env::var("KEYLINE_CONFIG_DIR")) {
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
    let file = config_dir().join("settings.json");
    fs::read_to_string(file)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_settings(settings: &Settings) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);
    let file = dir.join("settings.json");
    if let Ok(raw) = serde_json::to_string_pretty(settings) {
        let _ = fs::write(file, raw);
    }
}

pub fn load_scores() -> Scores {
    let file = config_dir().join("scores.json");
    fs::read_to_string(file)
        .ok()
        .and_then(|raw| serde_json::from_str::<Scores>(&raw).ok())
        .unwrap_or_default()
}

pub fn save_scores(scores: &Scores) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);
    let file = dir.join("scores.json");
    if let Ok(raw) = serde_json::to_string_pretty(scores) {
        let _ = fs::write(file, raw);
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
