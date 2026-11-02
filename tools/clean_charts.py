#!/usr/bin/env python3
"""Clean vocal melody charts using neural pitch tracking.

The offline YIN tracker occasionally picks guitar bleed, octave harmonics and
vibrato as extra notes.  This pass keeps the existing note/word segmentation but
replaces each word's pitch with the median PYIN pitch over that word's exact
time span, then collapses wordless vocal runs to one sustained note.  The
result is a stable, lyric-aligned melody without octave spikes.

Usage: python tools/clean_charts.py <song-dir> [<song-dir> ...]
"""
from __future__ import annotations

import copy
import json
import subprocess
import sys
from pathlib import Path

import librosa
import numpy as np

SR = 22050
HOP = 256
FRAME = 2048


def load_audio(path: Path) -> np.ndarray:
    raw = subprocess.check_output(
        ["ffmpeg", "-v", "error", "-i", str(path), "-ac", "1", "-ar", str(SR), "-f", "f32le", "pipe:1"]
    )
    return np.frombuffer(raw, dtype="<f4").astype(np.float32)


def clean_song(song_path: Path, force: bool = False) -> tuple[int, int, str]:
    song = json.loads(song_path.read_text())
    if song.get("chartCleaned") and not force:
        return 0, 0, "skipped (already cleaned)"
    if song.get("source") == "scale" or not song.get("notes"):
        return 0, 0, "skipped (scale/instrumental)"
    guide_name = (song.get("audio") or {}).get("guide")
    if not guide_name:
        return 0, 0, "skipped (no guide)"
    guide = song_path.parent / guide_name
    if not guide.exists():
        return 0, 0, f"skipped (missing {guide.name})"

    audio = load_audio(guide)
    hop = HOP / SR
    f0, _, vprob = librosa.pyin(
        audio, fmin=65, fmax=1100, sr=SR, frame_length=FRAME, hop_length=HOP, fill_na=np.nan
    )
    frame_count = len(f0)
    midi = np.full(frame_count, np.nan)
    voiced_pitch = np.isfinite(f0)
    midi[voiced_pitch] = 69.0 + 12.0 * np.log2(f0[voiced_pitch] / 440.0)
    rms = np.array(
        [np.sqrt(np.mean(audio[i * HOP : i * HOP + FRAME] ** 2) + 1e-12) for i in range(frame_count)]
    )
    gate = max(2e-4, float(np.percentile(rms, 95)) * 0.02) if rms.size else 2e-4
    voiced = voiced_pitch & (vprob > 0.22) & (rms > gate)

    def pitch_for(start: float, end: float, fallback: int) -> tuple[int, float]:
        lo = max(0, int(start / hop) - 1)
        hi = min(frame_count, int(np.ceil(end / hop)) + 1)
        idx = [i for i in range(lo, hi) if voiced[i]]
        if len(idx) >= 2:
            pitch = float(np.median(midi[idx]))
            return int(round(pitch)), round((pitch - round(pitch)) * 100.0, 1)
        return fallback, 0.0

    backup_dir = Path(".work/chart-backups")
    backup_dir.mkdir(parents=True, exist_ok=True)
    backup = backup_dir / f"{song_path.parent.name}.json"
    if not backup.exists():
        backup.write_text(song_path.read_text())

    notes = copy.deepcopy(song["notes"])
    # group consecutive notes belonging to the same lyric word
    groups: list[list[dict]] = []
    current: list[dict] = []
    key: tuple | None = None
    for note in notes:
        k = (note.get("line"), note.get("wordIndex"), note.get("word"))
        if note.get("word") is None:
            k = None
        same = False
        if current and key is not None and k is not None:
            same = k[0] == key[0] and (
                (k[1] is not None and k[1] == key[1])
                or (k[1] is None and k[2] == key[2])
            )
        if same:
            current.append(note)
        else:
            if current:
                groups.append(current)
            current = [note]
            key = k
    if current:
        groups.append(current)

    for group in groups:
        worded = any(n.get("word") for n in group)
        if worded:
            start = group[0]["start"]
            end = max(n["start"] + n["dur"] for n in group)
            fallback = int(round(float(np.median([n["midi"] for n in group]))))
            pitch, cents = pitch_for(start, end, fallback)
            for note in group:
                note["midi"] = pitch
                note["cents"] = cents

    # Collapse each wordless run to one sustained median note. Intros, outros
    # and vocalisations become one line instead of a pile of stray notes.
    if song.get("lines"):
        collapsed: list[dict] = []
        run: list[dict] = []

        def flush(items: list[dict]) -> None:
            if not items:
                return
            start = items[0]["start"]
            end = max(n["start"] + n["dur"] for n in items)
            if end - start < 0.10:
                return
            fallback = int(round(float(np.median([n["midi"] for n in items]))))
            pitch, cents = pitch_for(start, end, fallback)
            collapsed.append(
                {
                    "start": round(start, 3),
                    "end": round(end, 3),
                    "dur": round(end - start, 3),
                    "midi": pitch,
                    "cents": cents,
                    "conf": float(items[0].get("conf", 0.5)),
                    "vel": float(np.mean([n.get("vel", 0.5) for n in items])),
                }
            )

        previous_end: float | None = None
        for note in notes:
            if note.get("word") is None:
                if run and previous_end is not None and note["start"] - previous_end > 0.45:
                    flush(run)
                    run = []
                run.append(note)
                previous_end = note["start"] + note["dur"]
            else:
                flush(run)
                run = []
                collapsed.append(note)
                previous_end = note["start"] + note["dur"]
        flush(run)
        notes = collapsed

    # A tiny unworded note between two equal pitches is a tracker dip, not a
    # real note.
    for i in range(1, len(notes) - 1):
        a, b, c = notes[i - 1], notes[i], notes[i + 1]
        if b.get("word") is None and b["dur"] < 0.25 and abs(a["midi"] - c["midi"]) <= 1:
            ref = int(round((a["midi"] + c["midi"]) / 2))
            if 0 < abs(b["midi"] - ref) <= 2:
                b["midi"] = ref
                b["cents"] = 0.0

    notes = [n for n in notes if n.get("word") or n["dur"] >= 0.10]
    before = len(song["notes"])
    song["notes"] = notes
    song["chartCleaned"] = True
    song_path.write_text(json.dumps(song, indent=2) + "\n")
    return before, len(notes), "cleaned"


def main() -> int:
    args = sys.argv[1:]
    force = "--force" in args
    targets = [Path(arg) for arg in args if arg != "--force"]
    if not targets:
        targets = [p.parent for p in Path("assets/songs").glob("*/song.json")]
    for directory in targets:
        song_path = directory / "song.json" if directory.is_dir() else directory
        try:
            before, after, message = clean_song(song_path, force)
            print(f"{song_path}: {before} -> {after} notes ({message})")
        except Exception as error:
            print(f"{song_path}: ERROR {error}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
