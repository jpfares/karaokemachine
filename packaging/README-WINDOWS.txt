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

IMPORTANT: NO COMMERCIAL AUDIO IS BUNDLED
- Keep the "assets" folder next to karaokemachine.exe.
- This portable build ships only the app and the generated warm-up tracks.
  It does not contain commercial recordings.
- Use Library -> Maps -> Open Map Browser to find community mappings, then
  install the downloaded map file. The app downloads the audio from the map's
  source link and applies the mapping locally.
- Every included warm-up starts as UNMAPPED and cannot be played until mapped.
- Click an UNMAPPED song in the library to open the manual level editor.
  The old automatic chart is shown translucently as a guide only; it is not
  playable until you draw the notes and place every lyric word yourself.
- Press SUBMIT MAPPING only when the editor says the mapping is complete.
  After that the song becomes playable in the normal singing stage.

MAPS AND AUDIO
- A map package contains notes, lyrics, cover art, and the original source
  link. It never contains audio.
- Library -> Maps -> Install Map File applies a downloaded JSON map. If the
  audio is not local, the app fetches it from the source link and separates it.
- Uploading a brand new song also needs Node.js 22+, ffmpeg, yt-dlp and Demucs.
  They are not bundled with this portable build.
- If those tools are installed on PATH, paste a YouTube, Spotify or
  SoundCloud link in Library -> Imports -> Upload Song. KaraokeMachine will
  fetch editable metadata and lyrics, download the audio, separate the
  backing and vocal stems, and open the editor automatically.
- If the tools are missing, the upload and map-install flows explain what to
  install. Mapping the bundled warm-ups does not need them.

EDITOR QUICK REFERENCE
  Top toolbar              Icon-only; hover an icon for its name.
  Pan tool                 Drag to pan. Middle-drag pans from any tool.
  Right-click + drag       Scrub the yellow now-playing line continuously.
  Vocal analysis strip     Waveform of the isolated vocal plus the raw, unsmoothed pitch
                           contour, auto-scaled across the full vocal track, and a hover
                           readout with exact time, raw pitch, and level.
  Note tool                Click or drag on a lane. Pitch is fixed; drag sets length.
  Ribbon tool              Drag across notes and release over the last note to connect them.
  Eraser                   Click or sweep over notes or word decorations to delete them.
  Select tool              Select notes or words. Drag note bodies to move, note ends to resize.
  Click a note             Audition that note's pitch (Note/Select/Ribbon/Pan tools).
  Word bank                Drag a word anywhere onto the level. Words are decorative and
                           do not have to sit on a note, though they can be dropped on one.
  + button                 Add a custom word to the front of the word bank.
  Space                    Play / pause
  Left / Right arrows      Seek 1 second (Shift: 5 seconds)
  SAVE AS DRAFT            Saves the current mapping without leaving the editor.
  Ctrl+S                   Save as draft.
  EXPORT MAP               Export a submitted map package (JSON; no audio).
  Ctrl+Z                   Undo
  Ctrl+Shift+Z / Ctrl+Y    Redo
  Delete                   Delete the selection
  1–5                      Select a tool
  Mouse wheel              Scroll the timeline
  Wheel over piano rail    Scroll the visible pitch range (Shift = one octave)
  Ctrl + wheel             Zoom the timeline from anywhere over the working area
  Piano rail click         Audition that pitch

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
