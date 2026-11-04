# Contributing a karaoke map

KaraokeMachine maps are shared as **JSON packages only**. A map package contains:

- the mapped notes and ribbon groups
- placed lyric words and custom words
- lyrics and word timings
- song metadata
- cover art (embedded as a data URI)
- the original audio source link

A map package **must not contain audio**. Audio is fetched from the source link when
someone installs the map.

## Export a map from the app

1. Map the song in the editor and press **Submit Mapping**.
2. Open the mapped song in the editor again using its **EDIT** button.
3. Press **EXPORT MAP** in the inspector.
4. Save the `.json` package.

The app embeds the cover art and lyrics in the package. It does not copy the audio files.

## Add it to the browser

1. Copy the exported `.json` file into `site/maps/`.
2. Run:

   ```bash
   npm run maps:index
   ```

3. Commit the map JSON, the regenerated `site/maps/index.json`, and the cover image it
   wrote to `site/maps/covers/`.
4. Open a pull request.

The map browser is deployed from `site/` by `.github/workflows/pages.yml`.

## Source links

Use a public link that the app can download from:

- YouTube
- SoundCloud
- a direct HTTP(S) audio file
- Internet Archive

Spotify links do not provide downloadable audio. If a map's source is Spotify, include a
working public audio link instead; Spotify can still appear as metadata in the app's upload
form.

## Map quality checklist

- The mapping was made or reviewed manually in the level editor.
- Every lyric word is placed.
- Notes and ribbons were checked against the vocal waveform.
- The source link matches the audio the mapping was timed to.
- No `.mp3`, `.wav`, `.ogg`, `.m4a`, or `.flac` files are included.
- `npm run maps:index` passes.

## Licensing

Only submit mappings and cover art you have the right to share. KaraokeMachine's code is
MIT licensed; map packages are contributed by their authors and do not grant rights to the
underlying song or recording.
