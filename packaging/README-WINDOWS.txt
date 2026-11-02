KaraokeMachine — Windows build
===============================

QUICK START
1. Extract the whole "KaraokeMachine" folder somewhere. Do not run the exe
   from inside the zip.
2. Double-click karaokemachine.exe.
3. Windows may show a SmartScreen warning because this is an unsigned
   preview build. Click "More info" -> "Run anyway".
4. Allow microphone access if Windows asks. If it does not ask, enable:
   Settings -> Privacy & security -> Microphone -> Let desktop apps access
   your microphone.

IMPORTANT: EVERY SONG STARTS UNMAPPED
- Keep the "assets" folder next to karaokemachine.exe.
- Every song, including the warm-ups, starts as UNMAPPED and cannot be
  played until you manually map it.
- Click an UNMAPPED song in the library to open the manual level editor.
  The old automatic chart is shown translucently as a guide only; it is not
  playable until you draw the notes and place every lyric word yourself.
- Press SUBMIT MAPPING only when the editor says the mapping is complete.
  After that the song becomes playable in the normal singing stage.

UPLOADING NEW SONGS
- Imports need Node.js 22+, ffmpeg, yt-dlp and Demucs. They are not bundled
  with this portable build.
- If those tools are installed on PATH, paste a YouTube, Spotify or
  SoundCloud link in Library -> Imports -> Upload Song. KaraokeMachine will
  fetch editable metadata and lyrics, download the audio, separate the
  backing and vocal stems, and open the editor automatically.
- If the tools are missing, the upload form explains what to install.
  Mapping the songs already in the "assets" folder does not need them.

EDITOR QUICK REFERENCE
  Pan tool                 Drag to pan. Middle-drag pans from any tool.
  Note tool                Click or drag on a lane. Pitch is fixed; drag sets length.
  Ribbon tool              Drag across notes, then drop one word on the selection.
  Eraser                   Click or sweep over notes to delete them.
  Select tool              Click to select, drag the body to move, drag the ends to resize.
  Word bank                Drag a word onto a note. Placed words leave the bank.
  Space                    Play / pause
  Left / Right arrows      Seek 1 second (Shift: 5 seconds)
  Ctrl+Z                   Undo
  Ctrl+Shift+Z / Ctrl+Y    Redo
  Delete                   Delete the selection
  1–5                      Select a tool
  Mouse wheel              Scroll the timeline
  Ctrl + wheel             Zoom

PLAYING
  Space       Play / pause
  R           Restart the take
  L           Lyrics on / off
  V           Original vocals on / off
  B           Backing track on / off
  Esc         Open the album drawer
  Left/Right  Seek back / forward 5 seconds
  Click       Seek on the timeline

OTHER CONTROLS
- Settings has an input-microphone picker, a Play backing track checkbox and a
  vocal level. BACKING ON/OFF and VOCALS ON/OFF are also in the bottom tray.
- Settings -> Test range records your comfortable low and high notes. Save it,
  then use "Fit current song to my range" to apply the best octave shift to the
  chart.
- Singing the right note in a different octave is accepted as a hit and keeps
  the combo, but it shows "OCTAVE" and does not count as perfect.

Version 2.0.0
