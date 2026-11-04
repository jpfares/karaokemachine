# KaraokeMachine

**KaraokeMachine: an app to help you sing your favorite songs in karaoke :)**

KaraokeMachine is a native desktop karaoke machine. It plays a vocal-free backing track,
draws the melody and lyrics, listens through your microphone, and scores the notes you
sing. The level editor exists for one reason: to turn a song into a playable karaoke map.

![Singing stage](docs/stage.png)
![Album library](docs/menu.png)

## Map browser

**https://rgbeans.github.io/karaokemachine/**

The public map browser shares mappings only: notes, ribbons, lyrics, cover art, and the
original source link. It hosts **no audio**. The app's **Library → Maps** tab loads the
public map list automatically. Use **Import Map File** for a downloaded package; it is
installed as a separate level file under the song's `maps/` folder. If the audio is not
already local, KaraokeMachine fetches it from the map's source link.

To share a map, submit a mapping made in the editor. See
[CONTRIBUTING.md](CONTRIBUTING.md). Exported map packages contain no audio files.

## Sing

- **Pitch scoring, not word memorization.** Notes are judged by cents and hold time.
  Perfect holds build a combo. The right note in another octave counts as a hit, keeps
  the combo, and is capped at **OCTAVE** instead of **PERFECT**.
- **Vocal ribbons.** Connect a run of notes into one smooth visual ribbon; separate runs
  stay distinct. Lyrics float independently as decorations over the level.
- **Decorative karaoke lyrics.** Lyrics are positioned independently of notes, so a word
  can sit over the melody or float anywhere that reads well on screen.
- **Backing and guide vocals.** Demucs splits the song into a vocal-free backing and an
  isolated guide vocal. Toggle either one live from the transport.
- **Microphone picker and input gain.** Choose the input device in Settings.
- **Vocal-range test and octave fit.** Sing a comfortable low and high note, save the
  range, then use **Fit current song to my range** to move the chart into it.
- **Per-song vertical fit.** The pitch lane fits the melody, so high and low songs remain
  readable without scrolling vertically.
- **Six warm-up tracks.** Major scale, pentatonic scales, blues, chromatic climb, and a
  maj7 arpeggio, with synthesized backing and guide vocals. They ship pre-mapped and
  playable, and appear in **Library → Maps** alongside community maps.
- **Native and offline.** Rust + `egui`/`eframe`, `cpal` audio, `symphonia` decoding. The
  singing app does not need a browser or Electron.

## Karaoke controls

| Action | Key |
| --- | --- |
| Play / pause | `Space` |
| Restart | `R` |
| Lyrics on/off | `L` |
| Original vocals on/off | `V` |
| Backing track on/off | `B` |
| Open library | `Esc` |
| Seek back/forward 5 s | `←` / `→` |
| Seek | Click the timeline |

Settings also contains the input microphone, backing/vocal levels, count-in, sync trim,
mic latency, octave shift, and the vocal-range test.

## How an imported song becomes playable

Every imported song starts with no level files and cannot be sung until a person maps it,
or until **Auto-map** charts it from the detected melody (below). The old automatic chart
is only a translucent suggestion. The six generated warm-ups are the exception: they ship
with a finished level file and are playable immediately.

Levels and drafts are separate JSON files in `assets/songs/<song-id>/maps/`. The editor
always asks which map file to open; you can also create a new file there.

1. Select a song in the library and press **EDIT**.
2. Choose an existing map file, or create a new map file.
3. Draw or correct the vocal notes over the suggestion.
4. With the Ribbon tool, drag from a dot on one note to a dot on another to connect them.
5. Drag lyric words anywhere onto the level as decorations.
6. Press **SAVE AS DRAFT** to keep working, or **SUBMIT MAPPING** to finish the level.

Drafts can be played as previews, but only a submitted level can be exported or shared.
When a song has exactly one map file, pressing it plays that map. With zero or multiple
map files, choose one from the map selector before pressing **PLAY**.

**Auto-map** skips the drawing. The import pipeline already traces the vocal melody and
aligns the lyric words to it; **AUTO-MAP** in the editor's inspector turns those
suggestions into notes, connectors and placed words in one step, and the upload form's
**Make it playable right away** box does the same automatically, saves the result as
`maps/Auto-map.json` and opens the song on the stage. **AUTO-MAP** replaces whatever is
already drawn in the open map file (it asks first, and `Ctrl+Z` undoes it), and the upload
box leaves a song alone if it already has any map file. The library lists the level as
"auto-mapped" while it keeps that file name; it stays fully editable and is only as good as
the suggestions, so expect to fix a few notes on songs with busy vocals. The status line
says how many words had no detected melody.

## Level editor

![Manual level editor](docs/editor.png)

The top toolbar is icon-only; hover an icon for its name.

The analysis strip above the pitch lanes shows the isolated vocal waveform plus a raw,
unsmoothed vocal pitch contour. The pitch contour is always auto-scaled between the lowest
and highest pitch across the whole vocal track, so piano-rail octave scrolling never cuts
it off. Nothing is reduced to note groups for that display; the mapper decides how to
simplify it. Hover it for a precise timestamp, raw pitch and cents,
and vocal level, so word onsets can be placed exactly. A translucent gray line follows the
mouse across the working area at all times. Click any note to audition its pitch. With the
Select tool, or with the Note tool from just inside either end, drag a note edge to stretch
or contract it. Hold right-click and drag to move the yellow now-playing line continuously.

| Action | Control |
| --- | --- |
| Pan | Pan icon, or middle-drag from any tool |
| Move the now-playing line | **Hold right-click and drag** in the lane, waveform, or ruler |
| Draw a note | Note icon: click or drag either direction on a pitch lane; drag sets length |
| Resize with the Note tool | Note icon: drag just inside either end of a note |
| Connect two notes | Ribbon icon: drag from a dot on one note to a dot on another |
| Select a connector | Ribbon or Select icon: click the line between two notes; Delete removes it |
| Erase | Eraser icon: click or sweep over notes or words |
| Select / move / resize | Select icon: drag a note body to move, drag either end to resize |
| Audition a note | Click a mapped note with the Note, Select, Ribbon, or Pan tool, or click a pitch on the piano rail |
| Move a lyric word | Select icon: drag the word anywhere in the level |
| Place a lyric word | Drag it from the word bank, or click it and then click the level |
| Add a custom word | **+** at the right of the word bank; it goes to the front of the bank |
| Scroll timeline | Mouse wheel |
| Scroll pitch view | Mouse wheel over the piano rail (Shift = octave) |
| Zoom timeline | Ctrl + wheel anywhere over the working area, or the − / + magnifier icons |
| Play / pause | `Space` |
| Seek | Arrow keys, or click/drag the time ruler |
| Save draft | **SAVE AS DRAFT** in the inspector, or `Ctrl+S` |
| Export a shared map | **EDIT** → choose a finished level file → **EXPORT MAP** |
| Undo / redo | `Ctrl+Z` / `Ctrl+Shift+Z` or `Ctrl+Y` |
| Select tool | Keys `1`–`5` |
| Delete selection | `Delete` / `Backspace` |
| Leave editor | `Esc` or the back arrow (drafts save; edited levels submit) |

Word decorations are completely independent of notes. Dropping a word on a note snaps it
there for convenience, but it can then be dragged anywhere horizontally or vertically.
The **+** button adds custom words to the front of the bank; custom words can be placed
and moved exactly like lyric words.

The editor's speed buttons (0.5×–1.5×) are a varispeed timing preview; they change pitch
with speed.

## Upload a new song

![Upload and metadata form](docs/upload.png)

Open **Library → Imports → Upload Song**, paste one of these links, and press **Fetch
Details** (or press `Enter` in the link field):

- YouTube video
- SoundCloud track
- Spotify track (Spotify supplies metadata; KaraokeMachine finds a matching public audio
  source for the stems)

Everything fetched remains editable: title, artist, album, release date, cover art,
artist/album bios, and lyrics. Artist and album dropdowns have their own search boxes.
Click the large cover tile to paste an image link, choose a local file, or paste an image
from the clipboard; a preview is shown before import. You can also copy metadata from an
existing artist/album or create new values. Then **IMPORT & OPEN EDITOR** downloads the
audio, separates the backing and guide vocals, and asks which map file to edit. With
**Make it playable right away** ticked the button reads **IMPORT & AUTO-MAP**, and the
song opens on the stage with its new `maps/Auto-map.json` level instead.

Lyrics are fetched from LRCLIB, lyrics.ovh, or Genius, with manual editing always
available. The automatic chart is stored as an editor suggestion; without Auto-map the
imported song has no level file yet.

## Run it

Prerequisites:

- Rust stable with Cargo
- Node.js 22+ for the import tools
- `ffmpeg`, `yt-dlp`, and Python 3 with **Demucs** for uploading new songs

```bash
git clone https://github.com/rgbeans/karaokemachine
cd karaokemachine
npm install
npm run assets
cargo build --release --manifest-path rust/Cargo.toml
./launch.sh
```

`launch.sh` is a Linux launcher. On Windows and macOS run the binary directly, or use
the development command below, which works everywhere:

```bash
cargo run --release --manifest-path rust/Cargo.toml
```

To force Mesa software GL on a misbehaving driver:

```bash
KARAOKEMACHINE_SOFTWARE_GL=1 ./launch.sh
```

A typical upload-tool setup:

```bash
python3 -m venv .venv
.venv/bin/pip install -U demucs yt-dlp
# ffmpeg must also be available on PATH
```

On Windows:

```bat
py -3 -m venv .venv
.venv\Scripts\pip install -U demucs yt-dlp
```

The import tools look for `yt-dlp`, `demucs` and `python` in the repo's `.venv` first
(`.venv/bin` on Linux and macOS, `.venv\Scripts` on Windows), then fall back to the
commands on `PATH` (`python3` on Linux and macOS, `python` on Windows).

## Import pipeline

1. Resolve the link to a downloadable audio URL.
2. Download audio with `yt-dlp`.
3. Separate vocals and backing with Demucs (`--two-stems=vocals`).
4. Encode a 192 kbps backing and a 160 kbps guide MP3.
5. Trace the vocal melody for the translucent editor suggestions.
6. Fetch lyrics from LRCLIB, lyrics.ovh, or Genius; the mapper can edit them.
7. Fetch album metadata/artwork from iTunes, or accept a custom cover URL/file.
8. Write the song metadata with the automatic chart in `suggestedNotes`.
9. Ask which map file to edit, or let the mapper create a new one.

## Windows package

The Windows build is made only when you run the script. The package contains the app and
the generated warm-ups only; it never bundles the local commercial library.

```bash
# prerequisites: rustup target add x86_64-pc-windows-gnu, mingw-w64, zip
./scripts/build-windows.sh
```

It cross-compiles `karaokemachine.exe`, stages the current `assets/` folder, and writes:

```text
dist-windows/KaraokeMachine-Windows-x64.zip
```

The script prints the zip size and SHA-256 and does not upload anything. The portable
build needs Node, ffmpeg, yt-dlp, and Demucs for **new uploads**; mapping songs already in
the bundled library does not.

## Repository and media

The public repository contains the app, tools, documentation, and the six generated
warm-up tracks with their level files. It does **not** contain the commercial recordings
used during development. Commercial tracks under `assets/songs/*` are gitignored; use the
upload flow or place licensed audio in your local library.

Each song folder keeps its metadata and audio in `song.json`; its mappings live in
`maps/*.json`. Drafts and finished levels use the same file format, with a `draft` flag.

## Project layout

```text
rust/
  src/main.rs          native entry point
  src/app.rs           app state, actions, keys and the frame loop
  src/app/stage.rs     singing stage: scoring, pitch lanes, results
  src/app/lobby.rs     lobby and the library drawer
  src/app/maps.rs      community map browser and map installs
  src/app/upload.rs    upload dialog, metadata fetch, import job
  src/app/cover.rs     cover-art picker inside the upload dialog
  src/app/settings.rs  settings window and vocal-range test
  src/editor.rs        manual level editor, word decorations, ribbons, submission gate
  src/audio.rs         cpal output/mic, symphonia decoding, varispeed playback
  src/pitch.rs         native YIN pitch detector
  src/jobs.rs          background jobs polled by the UI
  src/library.rs       song scan, albums, settings, scores, atomic JSON writes
  src/map_package.rs   shareable map packages (no audio)
  src/model.rs         songs, notes, ribbons, placed words, settings
  src/theme.rs         colour palette
  src/widgets.rs       painter-drawn buttons and toggles
tools/
  analyze_vocals.py    suggested-melody extraction
  clean_charts.py      legacy suggestion cleanup
src/main/              TypeScript import pipeline (download, Demucs, lyrics, metadata)
scripts/               tooling, map index, warm-up generation, Windows packaging
site/                  GitHub Pages map browser and JSON map packages
packaging/             Windows README copied into the portable zip
docs/                  screenshots
assets/songs/<song>/maps/  draft and finished level JSON files
```

## Known limits

- A map is only as good as the time put into it. Suggestions can be wrong; the editor is
  for correcting them by hand.
- The editor speed control changes pitch. Pitch-preserving time-stretch is not implemented.
- Spotify imports need a public YouTube/SoundCloud audio match.
- Successful lyric lookup depends on LRCLIB/lyrics.ovh/Genius coverage. Missing lyrics can
  be pasted or edited in the upload form.
- Headphones give the cleanest score; speaker bleed can register as pitch.
- The Windows build is unsigned, so SmartScreen may show a warning.

## License

MIT.
