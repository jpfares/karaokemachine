#!/usr/bin/env python3
"""Offline vocal melody extraction.

Reads the isolated vocal stem, tracks F0 with YIN, segments stable pitches into
notes and writes a compact JSON chart. Uses only numpy + ffmpeg.

The live app uses the same YIN idea in TypeScript for microphone input.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys

import numpy as np

SR = 22050
FRAME = 2048
HOP = 256
FMIN = 65.0
FMAX = 1100.0


def load_audio(path: str) -> np.ndarray:
    cmd = [
        "ffmpeg", "-v", "error", "-i", path,
        "-ac", "1", "-ar", str(SR), "-f", "f32le", "pipe:1",
    ]
    raw = subprocess.check_output(cmd)
    audio = np.frombuffer(raw, dtype="<f4").astype(np.float64)
    if audio.size == 0:
        raise ValueError("empty audio")
    return audio


def pitch_frame(frame: np.ndarray) -> tuple[float, float, float]:
    """Return (f0, clarity, rms) for one frame."""
    frame = frame - frame.mean()
    rms = float(np.sqrt(np.mean(frame * frame)) + 1e-12)
    if rms < 1e-5:
        return 0.0, 0.0, rms

    n = frame.size
    nfft = 1 << (2 * n - 1).bit_length()
    spec = np.fft.rfft(frame, n=nfft)
    acf = np.fft.irfft(spec * np.conj(spec), n=nfft)[:n]
    cum = np.concatenate(([0.0], np.cumsum(frame * frame)))

    min_lag = max(2, int(SR / FMAX))
    max_lag = min(int(SR / FMIN), n - 2)
    lags = np.arange(1, max_lag + 1)
    # difference with a tapered comparison window
    e0 = cum[n - lags]
    e1 = cum[n] - cum[lags]
    diff = e0 + e1 - 2.0 * acf[1:max_lag + 1]
    diff = np.maximum(diff, 0.0)

    cmnd = np.ones(max_lag + 1, dtype=np.float64)
    running = 0.0
    for tau in range(1, max_lag + 1):
        running += diff[tau - 1]
        cmnd[tau] = 1.0 if running <= 1e-12 else diff[tau - 1] * tau / running

    threshold = 0.20
    tau_est = -1
    for tau in range(min_lag, max_lag + 1):
        if cmnd[tau] < threshold:
            t = tau
            while t + 1 <= max_lag and cmnd[t + 1] < cmnd[t]:
                t += 1
            tau_est = t
            break
    if tau_est < 0:
        segment = cmnd[min_lag:max_lag + 1]
        best_rel = int(np.argmin(segment))
        if segment[best_rel] > 0.55:
            return 0.0, 0.0, rms
        tau_est = min_lag + best_rel

    # Prefer an earlier (higher-frequency) period if it is nearly as strong.
    for divisor in (2, 3):
        cand = int(round(tau_est / divisor))
        if cand >= min_lag and cand < tau_est and cmnd[cand] < 0.30:
            local = cand
            while local + 1 < tau_est and cmnd[local + 1] < cmnd[local]:
                local += 1
            if cmnd[local] < cmnd[tau_est] + 0.08:
                tau_est = local
                break

    tau = float(tau_est)
    if min_lag < tau_est < max_lag:
        s0, s1, s2 = cmnd[tau_est - 1], cmnd[tau_est], cmnd[tau_est + 1]
        denom = 2.0 * (2.0 * s1 - s2 - s0)
        if abs(denom) > 1e-12:
            tau = tau_est + (s2 - s0) / denom
    f0 = SR / tau
    clarity = float(max(0.0, min(1.0, 1.0 - cmnd[tau_est])))
    if not np.isfinite(f0) or f0 < FMIN * 0.9 or f0 > FMAX * 1.1:
        return 0.0, 0.0, rms
    return float(f0), clarity, rms


def track(audio: np.ndarray) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    count = max(0, 1 + (len(audio) - FRAME) // HOP)
    f0 = np.zeros(count, dtype=np.float64)
    clarity = np.zeros(count, dtype=np.float64)
    rms = np.zeros(count, dtype=np.float64)
    for i in range(count):
        start = i * HOP
        f0[i], clarity[i], rms[i] = pitch_frame(audio[start:start + FRAME])
    return f0, clarity, rms


def median_smooth(values: np.ndarray, mask: np.ndarray, radius: int = 2) -> np.ndarray:
    out = values.copy()
    idx = np.where(mask)[0]
    for i in idx:
        lo, hi = max(0, i - radius), min(len(values), i + radius + 1)
        window = values[lo:hi][mask[lo:hi]]
        if window.size:
            out[i] = float(np.median(window))
    return out


def extract_notes(audio: np.ndarray) -> tuple[list[dict], dict]:
    f0, clarity, rms = track(audio)
    count = len(f0)
    hop_sec = HOP / SR

    rms_max = float(np.percentile(rms, 95)) if rms.size else 0.0
    rms_gate = max(2e-4, rms_max * 0.025)
    voiced = (f0 > 0) & (clarity > 0.32) & (rms > rms_gate)
    if voiced.sum() < 4:
        return [], {"frames": count, "duration": len(audio) / SR, "voicedFrames": 0}

    midi = np.zeros(count, dtype=np.float64)
    midi[voiced] = 69.0 + 12.0 * np.log2(f0[voiced] / 440.0)
    midi = median_smooth(midi, voiced, radius=2)

    # Correct isolated octave jumps against a local median of the surrounding
    # voiced frames. This mainly fixes the classic half/double-period error.
    voiced_idx = np.where(voiced)[0]
    for i in voiced_idx:
        lo, hi = max(0, i - 8), min(count, i + 9)
        neigh = midi[lo:hi][voiced[lo:hi]]
        if neigh.size < 4:
            continue
        ref = float(np.median(neigh))
        for shift in (-12.0, 12.0):
            if abs(midi[i] + shift - ref) < 1.2 and abs(midi[i] - ref) > 7.0:
                midi[i] += shift
                break

    labels = np.full(count, -999, dtype=np.int32)
    labels[voiced] = np.rint(midi[voiced]).astype(np.int32)

    # Stable run segmentation with hysteresis.
    segments: list[dict] = []
    current: dict | None = None
    gap = 0
    pending = 0
    pending_label = -999

    def close(end_frame: int) -> None:
        nonlocal current
        if current is None:
            return
        frames = list(range(current["startFrame"], end_frame))
        voiced_frames = [f for f in frames if labels[f] != -999]
        if len(voiced_frames) < 3:
            current = None
            return
        pitches = midi[voiced_frames]
        conf = clarity[voiced_frames]
        energy = rms[voiced_frames]
        median_pitch = float(np.median(pitches))
        start_sec = voiced_frames[0] * hop_sec
        end_sec = (voiced_frames[-1] + 1) * hop_sec
        length = end_sec - start_sec
        if length < 0.09:
            current = None
            return
        segments.append({
            "start": start_sec,
            "end": end_sec,
            "dur": length,
            "midi": int(round(median_pitch)),
            "cents": round((median_pitch - round(median_pitch)) * 100.0, 1),
            "conf": round(float(np.mean(conf)), 3),
            "vel": round(float(np.clip(np.mean(energy) / max(rms_max, 1e-9), 0, 1)), 3),
        })
        current = None

    for i in range(count):
        label = labels[i]
        if label == -999:
            if current is not None:
                gap += 1
                if gap > 6:
                    close(i - gap + 1)
                    gap = 0
            continue
        gap = 0
        if current is None:
            current = {"startFrame": i, "label": int(label)}
            pending = 0
            continue
        if label == current["label"]:
            pending = 0
            continue
        if label == pending_label:
            pending += 1
        else:
            pending_label = int(label)
            pending = 1
        if pending >= 3:
            close(i - pending + 1)
            current = {"startFrame": i - pending + 1, "label": int(label)}
            pending = 0
    if current is not None:
        close(count)

    # Merge same-pitch fragments and clean up.
    merged: list[dict] = []
    for seg in segments:
        if seg["conf"] < 0.30:
            continue
        if merged:
            prev = merged[-1]
            if seg["midi"] == prev["midi"] and seg["start"] - prev["end"] < 0.12:
                prev["end"] = seg["end"]
                prev["dur"] = prev["end"] - prev["start"]
                prev["conf"] = (prev["conf"] + seg["conf"]) / 2
                continue
        merged.append(seg)

    # Octave folding for short orphan notes.
    for i, note in enumerate(merged):
        if note["dur"] > 0.32 or len(merged) < 3:
            continue
        neighbors = [merged[j]["midi"] for j in (i - 1, i + 1) if 0 <= j < len(merged)]
        if len(neighbors) == 2 and abs(neighbors[0] - neighbors[1]) <= 2:
            ref = (neighbors[0] + neighbors[1]) / 2
            if abs(note["midi"] - ref - 12) < 1:
                note["midi"] -= 12
            elif abs(note["midi"] - ref + 12) < 1:
                note["midi"] += 12

    merged = [n for n in merged if 38 <= n["midi"] <= 90]
    for note in merged:
        note["start"] = round(float(note["start"]), 3)
        note["end"] = round(float(note["end"]), 3)
        note["dur"] = round(max(0.09, float(note["dur"])), 3)
        note.pop("startFrame", None)
        note.pop("label", None)

    stats = {
        "frames": count,
        "duration": round(len(audio) / SR, 3),
        "voicedFrames": int(voiced.sum()),
        "voicedRatio": round(float(voiced.mean()), 3),
        "medianClarity": round(float(np.median(clarity[voiced])), 3),
        "notes": len(merged),
    }
    return merged, stats


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("input")
    parser.add_argument("output")
    args = parser.parse_args()

    audio = load_audio(args.input)
    notes, stats = extract_notes(audio)
    result = {"notes": notes, "stats": stats}
    with open(args.output, "w", encoding="utf-8") as handle:
        json.dump(result, handle)
    print(
        f"analyzed {stats['duration']:.1f}s: {len(notes)} notes, "
        f"voiced {stats.get('voicedRatio', 0) * 100:.0f}%, clarity {stats.get('medianClarity', 0):.2f}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
