# KaraokeMachine

**KaraokeMachine: an app to help you sing your favorite songs in karaoke :)**

KaraokeMachine is a native desktop karaoke machine. It plays a vocal-free backing track,
draws the melody and lyrics, listens through your microphone, and scores the notes you
sing. The level editor exists for one reason: to turn a song into a playable karaoke map.

![Singing stage](docs/stage.png)
![Album library](docs/menu.png)

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
  maj7 arpeggio, with synthesized backing and guide vocals.
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

## How a song becomes playable

Every song starts **UNMAPPED** and cannot be sung until a person maps it. The old
automatic chart is only a translucent suggestion.

1. Click an UNMAPPED song in the library.
2. Draw or correct the vocal notes over the suggestion.
3. Connect notes into ribbons where a word or vocal run should glide.
4. Drag lyric words anywhere onto the level as decorations.
5. Place every word, then press **SUBMIT MAPPING**.
6. The song becomes playable in the normal singing stage.

Unfinished work is saved as a draft and remains UNMAPPED. Submitted maps are stored in
the song's `song.json`.

## Level editor

![Manual level editor](docs/editor.png)

The top toolbar is icon-only; hover an icon for its name.

The analysis strip above the pitch lanes shows the isolated vocal waveform plus a raw,
unsmoothed vocal pitch contour. Nothing is reduced to note groups for that display; the
mapper decides how to simplify it. Hover it for a precise timestamp, raw pitch and cents,
and vocal level, so word onsets can be placed exactly. A translucent gray line follows the
mouse across the working area at all times. Click any note to audition its pitch. With the
Select tool, drag either note edge to stretch or contract it. Hold right-click and drag to
move the yellow now-playing line continuously.

| Action | Control |
| --- | --- |
| Pan | Pan icon, or middle-drag from any tool |
| Move the now-playing line | **Hold right-click and drag** in the lane, waveform, or ruler |
| Draw a note | Note icon: click or drag on a pitch lane; drag sets length |
| Make a ribbon | Ribbon icon: drag across notes, release over the last note |
| Erase | Eraser icon: click or sweep over notes or words |
| Select / move / resize | Select icon: drag a note body to move, drag either end to resize |
| Audition a note | Click a note with the Note, Select, Ribbon, or Pan tool |
| Move a lyric word | Select icon: drag the word anywhere in the level |
| Place a lyric word | Drag it from the word bank, or click it and then click the level |
| Add a custom word | **+** at the right of the word bank; it goes to the front of the bank |
| Scroll timeline | Mouse wheel |
| Zoom | Ctrl + wheel, or the − / + magnifier icons |
| Play / pause | `Space` |
| Seek | Arrow keys, or click/drag the time ruler |
| Save draft | **SAVE AS DRAFT** in the inspector, or `Ctrl+S` |
| Undo / redo | `Ctrl+Z` / `Ctrl+Shift+Z` or `Ctrl+Y` |
| Select tool | Keys `1`–`5` |
| Delete selection | `Delete` / `Backspace` |
| Leave editor | `Esc` or the back arrow (saves a draft for unmapped songs) |

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
existing artist/album or create new values. Then **Download & Open Editor** downloads the
audio, separates the backing and guide vocals, and opens the manual mapper.

Lyrics are fetched from LRCLIB, lyrics.ovh, or Genius, with manual editing always
available. The automatic chart is stored only as an editor suggestion; the imported song
is still UNMAPPED.

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

For development:

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

The app looks for `.venv/bin/yt-dlp` and `.venv/bin/demucs` first, then falls back to
commands on `PATH`.

## Import pipeline

1. Resolve the link to a downloadable audio URL.
2. Download audio with `yt-dlp`.
3. Separate vocals and backing with Demucs (`--two-stems=vocals`).
4. Encode a 192 kbps backing and a 160 kbps guide MP3.
5. Trace the vocal melody for the translucent editor suggestions.
6. Fetch lyrics from LRCLIB, lyrics.ovh, or Genius; the mapper can edit them.
7. Fetch album metadata/artwork from iTunes, or accept a custom cover URL/file.
8. Write the song as `mapped: false` with empty `notes` and the automatic chart in
   `suggestedNotes`.
9. Open the level editor.

## Windows package

The Windows build is made only when you run the script:

```bash
# prerequisites: rustup target add x86_64-pc-windows-gnu, mingw-w64, zip, rsync
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
warm-up tracks. It does **not** contain the commercial recordings used during
development. Commercial tracks under `assets/songs/*` are gitignored; use the upload flow
or place licensed audio in your local library.

## Project layout

```text
rust/
  src/main.rs          native entry point
  src/app.rs           library, singing stage, settings, upload form
  src/editor.rs        manual level editor, word decorations, ribbons, submission gate
  src/audio.rs         cpal output/mic, symphonia decoding, varispeed playback
  src/pitch.rs         native YIN pitch detector
  src/library.rs       song scan, albums, settings, scores
  src/model.rs         songs, notes, ribbons, placed words, settings
tools/
  analyze_vocals.py    suggested-melody extraction
  clean_charts.py      legacy suggestion cleanup
src/main/              TypeScript import pipeline (download, Demucs, lyrics, metadata)
scripts/               tooling, warm-up generation, Windows packaging
packaging/             Windows README copied into the portable zip
docs/                  screenshots
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
