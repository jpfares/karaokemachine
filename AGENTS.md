# Working on KaraokeMachine with an AI coding assistant

This file is read by coding assistants at the start of every session (Codex, Cursor and
Copilot read `AGENTS.md`; Claude Code reads `CLAUDE.md`, which imports this file). It
exists because a full review of the codebase in November 2026 found that almost every
real bug came from the same handful of habits, not from hard problems. The rules below
are those habits, inverted. Follow them literally.

## What this project is

A desktop karaoke game: Rust + egui (`rust/`), a TypeScript/Python import pipeline
(`src/`, `scripts/`, `tools/`), and a GitHub Pages map browser (`site/`). The stage
scores a singer's pitch against a chart; the editor draws charts; the upload form imports
songs from a link. Read `README.md` for the user-facing model and `CONTRIBUTING.md` for
map packages.

Where things live, and where new things go:

| Concern | Place | Rule |
| --- | --- | --- |
| Colours, alpha helper, rank colours | `rust/src/theme.rs` | The only file that may contain a colour literal. |
| Painted buttons, pills, toggles | `rust/src/widgets.rs` | Every clickable painted control comes from here. No one-off button painters. |
| Background work | `rust/src/jobs.rs` (`Job<T>`, `StreamJob<M>`) | Anything that can take more than a frame runs here, never on the UI thread. |
| HTTP | `rust/src/tools.rs` (`http_get_text`, `http_get_bytes`) | Timeouts and size caps are built in. Never shell out to `node`/`curl` for a fetch. |
| Subprocesses | `rust/src/tools.rs` (`node_command`) | Sets `CREATE_NO_WINDOW` on Windows and the working directory. |
| Data model, serde, pure helpers | `rust/src/model.rs` | Pure functions with tests live here (`rank_for`, `next_rank`, `lane_range`, `pitch_range`). |
| Loading and saving files | `rust/src/library.rs` | Atomic writes (`write_json_atomic`), config dir via `KARAOKEMACHINE_CONFIG_DIR`. |
| One screen each | `rust/src/app/{stage,lobby,maps,upload,cover,settings}.rs` | A new screen is a new module with `pub(super)` items, not more lines in `app.rs`. |
| The scoring stage | `rust/src/app/stage.rs` | All score resets go through `Stage::reset_scoring`. |
| Chart editor | `rust/src/editor.rs` | Saves only to the open map file. Never writes `song.json`. |
| Auto-map | `rust/src/automap.rs` | Pure conversion from suggestions to a `MapFile`. |

Data model facts that must stay true: `song.json` holds metadata and audio paths only;
mappings are `assets/songs/<id>/maps/*.json` (`MapFile`, with a `draft` flag); any map file
plays; `Song.mapped` and `Song.maps` are derived by the library loader, never written by
hand; map installs and imports never overwrite an existing map file.

## The definition of done

A change is not finished until all of these pass locally. CI runs the same gate, so a red
CI means the step was skipped.

```bash
cargo fmt --check --manifest-path rust/Cargo.toml
```

```bash
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
```

```bash
cargo test --manifest-path rust/Cargo.toml
```

```bash
npx tsc --noEmit
```

Anything visible also needs a screenshot through the app's hooks (below), looked at, with
overlaps and clipped text fixed before the commit. "I think it renders fine" is not a check.

## Rules, each with the bug it prevents

1. **One function per state transition.** The stage once had three hand-written copies of
   the score reset that disagreed, so seeking corrupted the score. If two places need the
   same reset, sync or recompute, write one function and call it from both. Never
   copy-paste a block that mutates state.
2. **Never lose user work.** A map install used to overwrite hand-made charts matched by
   title alone; the editor's back button used to drop unsaved edits on a failed save. Any
   write that could replace a user's file needs: a match on id or exact source, a backup or
   a separate file, and on failure the UI stays where it was with the data still in memory.
   Say in the commit message what the failure path does.
3. **Nothing slow on the UI thread.** Audio decode, HTTP, subprocesses, directory scans of
   covers: all through `Job`/`StreamJob` with a cloned `egui::Context` so the UI repaints
   when the result lands. If a function can take longer than a frame, it is a job.
4. **Every network call has a timeout and a size cap.** Use the helpers in `tools.rs`.
   Never add a new way to fetch.
5. **No colour, size or threshold literal outside its home.** Colours in `theme.rs`.
   Scoring thresholds in one `const` (see `RANK_THRESHOLDS`). The ribbon-join rule used to
   exist three times with different magic numbers; it must exist once.
6. **Painted text is 10 pt or larger**, labels use `MUTED` or brighter, and layout derives
   from `rect.width()` / `rect.height()` so the supported minimum window (1100x700 logical)
   still fits. Check every new screen at that size with `KARAOKEMACHINE_WINDOW=1100x700`.
7. **Functions under about 80 lines, files under about 1,000.** `app.rs` was 4,500 lines and
   two 400-line functions hid most of the bugs. When a function grows, split by what the
   pieces are, not by line count.
8. **Errors are `Result<_, String>` with a sentence a user can read**, shown as a toast.
   No `unwrap`/`expect` outside tests; no `panic!` on user data. Diagnostics go to
   `eprintln!`, not to the UI.
9. **Every logic change ships with a test.** Scoring, parsing, mapping, library loading,
   file formats. Tests use their own scratch directory (see `scratch(name)` in
   `library.rs` tests) because they run in parallel, and never touch the user's real
   config directory.
10. **Windows is a first-class target.** No `python3` assumptions, no `/` string paths,
    subprocesses through `node_command`, CRLF-tolerant parsing, and the Windows zip must
    be able to do what the README says.
11. **Docs describe the code as it is.** When behaviour changes, the README sentence that
    described the old behaviour changes in the same commit. A review found the README
    promising three things the code did not do.
12. **One behaviour change per commit, named in the message.** A gameplay rule change
    (how scores are kept, what a button does) is called out in its own paragraph so the
    other person sees it without reading the diff.

## How to work: process is what closes the gap

Most of the quality difference between assistants is not intelligence. It is whether the
assistant reads before writing, states what could break, and checks its own work as if it
were someone else's. Do these every time.

**Before writing code**

- Read the whole function you are changing and every caller (`grep -rn` the name). Read the
  module's doc comment. Do not guess a signature.
- Write a five-line plan: what changes, which files, which existing function already does
  part of it (reuse it), what could lose data or hang the UI, and how you will prove it
  works.
- If the change touches saving, loading, scoring or the audio thread, list the failure
  scenario first: "if X fails midway, the user sees Y and keeps Z."

**While writing**

- Prefer editing an existing function over adding a parallel one. If you find two copies of
  something, merge them as part of the change.
- Keep the shape of the module: `Action` enum for UI events, painter-drawn widgets from
  `widgets.rs`, jobs for anything slow.
- Add the test in the same edit as the code, not "later".

**After writing: refute your own change**

Run the gate. Then, before saying it is done, answer these in writing (in the reply, not in
your head):

1. What input or state makes this wrong? First run with an empty library, a song with no
   map file, a draft map, a missing audio stem, a window at 1100x700, the drawer open on
   top, the upload running while the editor is open.
2. Which other code reads the fields I changed? Did I grep?
3. What does the user see if the write fails halfway?
4. Did I duplicate anything that already existed in `theme.rs`, `widgets.rs`, `model.rs` or
   `library.rs`?
5. Which screenshot proves it? Take it.

If any answer is "I'm not sure", that is the next task, not a caveat.

## Screenshot hooks

The app can run headless-ish and save one frame. Build with `cargo build`, then run
`rust/target/debug/karaokemachine` from the repository root with environment variables
(all prefixed `KARAOKEMACHINE_`):

| Variable | Effect |
| --- | --- |
| `SHOT=<path.png>` | Save a screenshot and exit. |
| `SHOT_DELAY=<seconds>` | Frames to wait first (12 or more for the stage in a debug build, since audio decodes slowly). |
| `NOMIC=1` | No microphone. |
| `CONFIG_DIR=<dir>` | Scores and settings from this folder instead of the real one. Always set this for test runs. |
| `WINDOW=1100x700` | Open at another size. |
| `SONG=<id or title>` | Open that song (through the map picker). |
| `AUTOPLAY=1`, `T=<seconds>`, `PITCH=<midi>` | Play, seek, and feed a fake sung pitch. |
| `DRAWER=imports|maps|album`, `ALBUM=<index>` | Open the library drawer on a tab. |
| `SETTINGS=1`, `UPLOAD=1`, `COVER_POPUP=1` | Open those windows. |
| `MAPS_BASE_URL=<url>` | Point the map browser at a local index. |

Example (PowerShell, from the repository root):

```powershell
$env:KARAOKEMACHINE_SHOT = "$PWD\shot.png"; $env:KARAOKEMACHINE_SHOT_DELAY = "12"
$env:KARAOKEMACHINE_NOMIC = "1"; $env:KARAOKEMACHINE_CONFIG_DIR = "$PWD\.work\cfg"
$env:KARAOKEMACHINE_SONG = "scale-major"; $env:KARAOKEMACHINE_AUTOPLAY = "1"; $env:KARAOKEMACHINE_T = "20"
Start-Process -FilePath rust\target\debug\karaokemachine.exe -PassThru | ForEach-Object { $_.WaitForExit() }
```

Then open `shot.png` and look at it. On Linux or macOS the same variables work with
`env VAR=value ./rust/target/debug/karaokemachine`.

## Session kickoff prompt

Paste this at the start of a session, after the task description:

> Read AGENTS.md and follow it literally. Before changing anything, read the full function
> and every caller of what you will touch, then give me a five-line plan that names the
> files, the existing code you will reuse, what could lose data or block the UI, and how you
> will prove it works. Make the change with its test in the same step. Then run `cargo fmt
> --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` (all with
> `--manifest-path rust/Cargo.toml`) and paste the last lines of each. For anything visible,
> take a screenshot with the `KARAOKEMACHINE_SHOT` hooks and describe what you checked. Then
> try to refute your own change: answer the five questions in the "refute your own change"
> section in writing. Do not tell me it is done until the gate is green and the five answers
> are concrete. Commit with a message that says why, and call out any behaviour change in
> its own paragraph.
