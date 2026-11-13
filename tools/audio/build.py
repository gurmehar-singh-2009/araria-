"""Synthesizes Araria's sound effects and music loops.

    python3 tools/audio/build.py

Everything is generated from oscillators and noise with numpy, written as
WAV and encoded to Ogg Vorbis with ffmpeg, then listed in
`assets/manifest.json`. No samples are used, so the audio is original and
freely redistributable.
"""

import json
import os
import subprocess
import tempfile
import wave

import numpy as np

RATE = 44100
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "assets", "audio")
MANIFEST = os.path.join(ROOT, "assets", "manifest.json")
entries = {}


def t(seconds):
    return np.arange(int(seconds * RATE)) / RATE


def env(n, attack=0.005, release=0.1, curve=2.0):
    """Attack/decay envelope over n samples."""
    e = np.ones(n)
    a = max(1, int(attack * RATE))
    e[:a] = np.linspace(0, 1, a)
    decay = np.linspace(1, 0, n - a) ** curve
    e[a:] = decay
    r = min(n, max(1, int(release * RATE)))
    e[-r:] *= np.linspace(1, 0, r)
    return e


def osc(freq, seconds, shape="sine"):
    """Oscillator; `freq` may be a scalar or a per-sample array (sweeps)."""
    x = t(seconds)
    phase = 2 * np.pi * np.cumsum(np.broadcast_to(freq, x.shape)) / RATE
    if shape == "sine":
        return np.sin(phase)
    if shape == "square":
        return np.tanh(np.sin(phase) * 4)
    if shape == "saw":
        return 2 * ((phase / (2 * np.pi)) % 1) - 1
    if shape == "tri":
        return 2 * np.abs(2 * ((phase / (2 * np.pi)) % 1) - 1) - 1
    raise ValueError(shape)


def noise(seconds, seed=0):
    return np.random.default_rng(seed).uniform(-1, 1, int(seconds * RATE))


def lowpass(x, cutoff):
    """One-pole low-pass; cutoff may vary per sample."""
    a = np.broadcast_to(np.clip(2 * np.pi * np.asarray(cutoff) / RATE, 0, 1), x.shape)
    y = np.zeros_like(x)
    acc = 0.0
    for i in range(len(x)):
        acc += a[i] * (x[i] - acc)
        y[i] = acc
    return y


def sweep(f0, f1, seconds):
    return np.geomspace(f0, f1, int(seconds * RATE))


def mix(*tracks):
    n = max(len(x) for x in tracks)
    out = np.zeros(n)
    for x in tracks:
        out[:len(x)] += x
    return out


def at(sound, seconds, total):
    out = np.zeros(int(total * RATE))
    start = int(seconds * RATE)
    end = min(len(out), start + len(sound))
    out[start:end] += sound[:end - start]
    return out


def save(name, samples, display, category, looping=False, peak=0.85):
    samples = samples / max(1e-9, np.max(np.abs(samples))) * peak
    if not looping:
        fade = min(len(samples), int(0.01 * RATE))
        samples[-fade:] *= np.linspace(1, 0, fade)
    os.makedirs(OUT, exist_ok=True)
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp:
        with wave.open(tmp, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(RATE)
            w.writeframes((samples * 32767).astype(np.int16).tobytes())
    path = os.path.join(OUT, name + ".ogg")
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", tmp.name, "-c:a", "libvorbis", "-q:a", "4", path], check=True)
    os.unlink(tmp.name)
    entries["audio/" + name] = {"file": f"audio/{name}.ogg", "name": display, "category": category, "duration": round(len(samples) / RATE, 2), "looping": looping}
    print(f"{name}: {len(samples) / RATE:.2f}s, {os.path.getsize(path) // 1024} KiB")


def note(freq, seconds, shape="tri", attack=0.005, curve=2.0):
    n = int(seconds * RATE)
    return osc(freq, seconds, shape)[:n] * env(n, attack, 0.03, curve)


def effects():
    save("jump", osc(sweep(260, 620, 0.16), 0.16, "square") * env(int(0.16 * RATE), curve=1.2), "Jump", "Movement")
    save("land", mix(lowpass(noise(0.14, 1), sweep(1400, 200, 0.14)) * env(int(0.14 * RATE)), 0.6 * note(90, 0.14, "sine")), "Land", "Movement")
    save("footstep", lowpass(noise(0.07, 2), 900) * env(int(0.07 * RATE), curve=3), "Footstep", "Movement", peak=0.5)
    save("bounce", osc(sweep(180, 900, 0.3), 0.3, "sine") * env(int(0.3 * RATE), curve=1.5), "Bounce", "Movement")
    save("coin", mix(note(988, 0.08, "square"), at(note(1319, 0.3, "square", curve=3), 0.07, 0.4)), "Coin", "Pickups", peak=0.6)
    save("gem", mix(*[at(note(f, 0.35, "sine", curve=3), i * 0.06, 0.7) for i, f in enumerate([1047, 1319, 1568, 2093])]), "Gem", "Pickups", peak=0.7)
    save("pickup", osc(sweep(500, 1200, 0.12), 0.12, "tri") * env(int(0.12 * RATE)), "Pickup", "Pickups", peak=0.7)
    save("powerup", mix(*[at(note(f, 0.14, "square"), i * 0.07, 0.75) for i, f in enumerate([392, 494, 587, 784, 988, 1175])]), "Power Up", "Pickups", peak=0.6)
    save("checkpoint", mix(*[at(note(f, 0.5, "tri", curve=2.5), i * 0.11, 1.0) for i, f in enumerate([523, 659, 784, 1047])]), "Checkpoint", "Game")
    save("win", mix(*[at(note(f, d, "square", curve=2), s, 1.9) for f, s, d in [(523, 0, .15), (659, .15, .15), (784, .3, .15), (1047, .45, .4), (784, .85, .15), (1047, 1.0, .8)]],
                    *[at(0.5 * note(f, 0.8, "tri"), 1.0, 1.9) for f in (523, 659)]), "Victory", "Game", peak=0.6)
    save("lose", mix(*[at(note(f, 0.35, "saw", curve=1.5), i * 0.28, 1.5) for i, f in enumerate([392, 349, 311, 262])]), "Defeat", "Game", peak=0.5)
    save("hurt", (osc(sweep(420, 140, 0.22), 0.22, "saw") + 0.5 * noise(0.22, 3)) * env(int(0.22 * RATE), curve=1.5), "Hurt", "Game", peak=0.7)
    save("death", mix(osc(sweep(500, 60, 0.7), 0.7, "square") * env(int(0.7 * RATE), curve=1.2), 0.4 * lowpass(noise(0.7, 4), 1200) * env(int(0.7 * RATE))), "Defeated", "Game", peak=0.7)
    save("explosion", lowpass(noise(0.9, 5), sweep(4000, 80, 0.9)) * env(int(0.9 * RATE), curve=1.6) + 0.8 * osc(sweep(120, 35, 0.9), 0.9) * env(int(0.9 * RATE)), "Explosion", "Game")
    save("click", note(880, 0.04, "square", curve=4), "Click", "Interface", peak=0.4)
    save("hover", note(1200, 0.025, "sine", curve=4), "Hover", "Interface", peak=0.25)
    save("error", mix(note(196, 0.12, "square"), at(note(156, 0.2, "square"), 0.13, 0.35)), "Error", "Interface", peak=0.5)
    save("notify", mix(note(784, 0.12, "sine"), at(note(1175, 0.3, "sine", curve=3), 0.1, 0.45)), "Notify", "Interface", peak=0.6)
    save("beep", note(660, 0.18, "square", curve=1.0), "Countdown Beep", "Game", peak=0.5)
    save("go", note(1320, 0.5, "square", curve=1.5), "Go", "Game", peak=0.5)
    save("door", lowpass(osc(sweep(90, 60, 0.5), 0.5, "saw") * (1 + 0.5 * osc(23, 0.5)), 700) * env(int(0.5 * RATE), 0.05, 0.1, 1.0), "Door", "World", peak=0.7)
    save("chop", mix(lowpass(noise(0.12, 6), sweep(3000, 300, 0.12)) * env(int(0.12 * RATE), curve=3), 0.7 * note(160, 0.1, "tri", curve=3)), "Chop", "World")
    save("mine", mix(note(1800, 0.06, "tri", curve=4), 0.8 * lowpass(noise(0.1, 7), 2500) * env(int(0.1 * RATE), curve=3), at(0.4 * note(2400, 0.2, "sine", curve=4), 0.02, 0.25)), "Mine", "World")
    save("splash", lowpass(noise(0.5, 8), sweep(2500, 500, 0.5)) * env(int(0.5 * RATE), 0.02, 0.2, 1.5), "Splash", "World", peak=0.6)
    save("whoosh", lowpass(noise(0.35, 9), np.concatenate([sweep(300, 2500, 0.175), sweep(2500, 300, 0.175)])) * np.sin(np.linspace(0, np.pi, int(0.35 * RATE))), "Whoosh", "World", peak=0.6)
    save("laser", osc(sweep(1800, 300, 0.18), 0.18, "saw") * env(int(0.18 * RATE), curve=1.5), "Zap", "Game", peak=0.5)
    save("hit", mix(lowpass(noise(0.1, 10), 1800) * env(int(0.1 * RATE), curve=3), note(140, 0.1, "sine", curve=3)), "Hit", "Game")
    # Seamless loops: every component completes whole cycles within the length.
    seconds = 1.0
    engine = lowpass(osc(55, seconds, "saw") * (0.7 + 0.3 * osc(110, seconds, "square")) + 0.3 * osc(165, seconds, "saw"), 900)
    save("engine", engine, "Engine Loop", "Vehicles", looping=True, peak=0.6)
    fire = lowpass(noise(2.0, 11), 700) * (0.6 + 0.4 * lowpass(np.abs(noise(2.0, 12)), 12))
    fade = int(0.2 * RATE)
    fire[:fade] = fire[:fade] * np.linspace(0, 1, fade) + fire[-fade:] * np.linspace(1, 0, fade)
    save("fire", fire[:-fade], "Fire Loop", "World", looping=True, peak=0.5)
    wind = lowpass(noise(4.0, 13), 350 + 250 * osc(0.25, 4.0))
    fade = int(0.5 * RATE)
    wind[:fade] = wind[:fade] * np.linspace(0, 1, fade) + wind[-fade:] * np.linspace(1, 0, fade)
    save("wind", wind[:-fade], "Wind Loop", "World", looping=True, peak=0.5)


SCALE = {"C": 261.63, "D": 293.66, "E": 329.63, "F": 349.23, "G": 392.0, "A": 440.0, "B": 493.88}


def pitch(name):
    """`C4`, `F#3`, `Bb5` → frequency."""
    letter, rest = name[0], name[1:]
    shift = rest.count("#") - rest.count("b")
    octave = int(rest.strip("#b"))
    return SCALE[letter] * 2 ** (octave - 4) * 2 ** (shift / 12)


def song(name, display, bpm, bars, chords, melody, bass_shape="tri", lead_shape="square", swing=0.0):
    """A loop: `chords` is one chord per bar (list of note names), `melody`
    is a list of (beat, note, beats) tuples spanning the whole loop."""
    beat = 60 / bpm
    total = bars * 4 * beat
    tracks = []
    for bar, chord in enumerate(chords):
        start = bar * 4 * beat
        root = pitch(chord[0]) / 2
        for step in range(8):
            f = root * (2 if step % 4 == 2 else 1)
            tracks.append(at(0.5 * note(f, beat * 0.45, bass_shape, curve=1.5), start + step * beat / 2, total))
        for i, n in enumerate(chord):
            tracks.append(at(0.16 * note(pitch(n), beat * 3.8, "sine", attack=0.08, curve=0.8), start, total))
        for step in range(8):
            n = chord[step % len(chord)]
            offset = swing * beat / 2 if step % 2 else 0
            tracks.append(at(0.14 * note(pitch(n) * 2, beat * 0.4, "tri", curve=3), start + step * beat / 2 + offset, total))
    for when, n, length in melody:
        tracks.append(at(0.3 * note(pitch(n), length * beat * 0.95, lead_shape, attack=0.01, curve=1.2), when * beat, total))
    for step in range(bars * 8):
        when = step * beat / 2
        if step % 4 == 0:
            tracks.append(at(0.7 * osc(sweep(140, 45, 0.12), 0.12) * env(int(0.12 * RATE), curve=2), when, total))
        if step % 4 == 2:
            tracks.append(at(0.3 * lowpass(noise(0.1, step), 5000) * env(int(0.1 * RATE), curve=3), when, total))
        tracks.append(at(0.08 * noise(0.03, step + 99) * env(int(0.03 * RATE), curve=3), when, total))
    save(name, mix(*tracks)[:int(total * RATE)], display, "Music", looping=True, peak=0.6)


def music():
    song("music_meadow", "Sunny Meadow", 104, 8,
         [["C4", "E4", "G4"], ["A3", "C4", "E4"], ["F3", "A3", "C4"], ["G3", "B3", "D4"]] * 2,
         [(0, "E5", 1), (1, "G5", 1), (2, "C6", 1.5), (3.5, "B5", .5), (4, "A5", 1), (5, "E5", 1), (6, "C5", 2), (8, "F5", 1), (9, "A5", 1), (10, "C6", 1), (11, "A5", 1), (12, "G5", 1.5), (13.5, "D5", .5), (14, "B4", 2),
          (16, "E5", .5), (16.5, "G5", .5), (17, "C6", 1), (18, "E6", 1.5), (19.5, "D6", .5), (20, "C6", 1), (21, "A5", 1), (22, "E5", 2), (24, "F5", 1), (25, "G5", .5), (25.5, "A5", .5), (26, "C6", 2), (28, "B5", 1), (29, "G5", 1), (30, "D5", 1), (31, "G5", 1)], swing=0.3)
    song("music_dash", "Neon Dash", 138, 8,
         [["A3", "C4", "E4"], ["F3", "A3", "C4"], ["C4", "E4", "G4"], ["G3", "B3", "D4"]] * 2,
         [(i * 4 + o, n, l) for i, phrase in enumerate([[(0, "A5", .5), (.5, "A5", .5), (1.5, "C6", .5), (2, "E6", 1), (3, "D6", .5), (3.5, "C6", .5)],
                                                        [(0, "A5", 1), (1.5, "F5", .5), (2, "A5", 1), (3, "C6", 1)], [(0, "G5", .5), (.5, "G5", .5), (1.5, "C6", .5), (2, "E6", 1), (3, "G6", 1)],
                                                        [(0, "D6", 1.5), (1.5, "B5", .5), (2, "G5", 1), (3, "B5", 1)]] * 2) for o, n, l in phrase],
         bass_shape="saw", lead_shape="saw")
    song("music_depths", "Quiet Depths", 76, 8,
         [["D3", "F3", "A3"], ["Bb2", "D3", "F3"], ["G3", "Bb3", "D4"], ["A2", "C#3", "E3"]] * 2,
         [(0, "A4", 2), (2, "F4", 1), (3, "D4", 1), (4, "F4", 3), (8, "G4", 2), (10, "Bb4", 1), (11, "D5", 1), (12, "C#5", 3), (16, "D5", 2), (18, "A4", 1), (19, "F4", 1), (20, "Bb4", 3), (24, "G4", 1.5), (25.5, "A4", .5), (26, "Bb4", 2), (28, "A4", 3)],
         lead_shape="tri")


if __name__ == "__main__":
    effects()
    music()
    data = {"version": 1, "models": {}, "audio": {}, "textures": {}}
    if os.path.exists(MANIFEST):
        with open(MANIFEST) as f:
            data.update(json.load(f))
    data["audio"] = dict(sorted(entries.items()))
    with open(MANIFEST, "w") as f:
        json.dump(data, f, indent=1)
        f.write("\n")
    print(f"manifest: {len(entries)} audio assets")
