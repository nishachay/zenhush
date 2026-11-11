"""Local noise and focus-sound synthesis for dither.

All audio is generated on the device with the Python standard library.
Nothing is downloaded and no network access is used, so every sound is
free of third-party licensing and safe to redistribute.
"""
import array
import math
import os
import random
import wave
import zlib

SAMPLE_RATE = 44100
LOOP_SECONDS = 20
SEAM_SECONDS = 0.25
TARGET_RMS = 0.12
PEAK = 0.75


def _white(n, rng):
    return [rng.uniform(-1.0, 1.0) for _ in range(n)]


def _brown(n, rng):
    out = []
    last = 0.0
    for _ in range(n):
        last = 0.998 * last + 0.02 * rng.uniform(-1.0, 1.0)
        out.append(last)
    return out


def _pink(n, rng):
    # Paul Kellett's three-pole approximation of pink noise.
    b0 = b1 = b2 = 0.0
    out = []
    for _ in range(n):
        w = rng.uniform(-1.0, 1.0)
        b0 = 0.99765 * b0 + w * 0.0990460
        b1 = 0.96300 * b1 + w * 0.2965164
        b2 = 0.57000 * b2 + w * 1.0526913
        out.append(b0 + b1 + b2 + w * 0.1848)
    return out


def _rain(n, rng):
    base = _pink(n, rng)
    drops = [0.0] * n
    for i in range(n):
        if rng.random() < 28.0 / SAMPLE_RATE:
            length = int(SAMPLE_RATE * rng.uniform(0.004, 0.015))
            amp = rng.uniform(0.3, 1.0)
            decay = length / 4.0
            for k in range(length):
                if i + k >= n:
                    break
                drops[i + k] += amp * rng.uniform(-1.0, 1.0) * math.exp(-k / decay)
    out = []
    prev = 0.0
    for i in range(n):
        hp = drops[i] - prev
        prev = drops[i]
        out.append(0.6 * base[i] + 0.45 * hp)
    return out


def _ocean(n, rng):
    base = _brown(n, rng)
    out = []
    for k in range(n):
        t = k / SAMPLE_RATE
        lfo = 0.55 + 0.30 * math.sin(2 * math.pi * 0.07 * t) + 0.15 * math.sin(2 * math.pi * 0.13 * t + 1.3)
        out.append(base[k] * max(lfo, 0.05))
    return out


SOUNDS = {
    "brown": ("Brown noise", _brown),
    "white": ("White noise", _white),
    "pink": ("Pink noise", _pink),
    "rain": ("Rain", _rain),
    "ocean": ("Ocean waves", _ocean),
}


def _normalize(sig):
    rms = math.sqrt(sum(x * x for x in sig) / len(sig)) or 1.0
    g = TARGET_RMS / rms
    out = [x * g for x in sig]
    m = max(abs(x) for x in out) or 1.0
    if m > PEAK:
        out = [x * PEAK / m for x in out]
    return out


def _seam(sig, n, fade):
    # sig has n + fade samples. Crossfade the extra tail onto the head so the
    # loop boundary (last sample -> first sample) is a continuous signal.
    out = sig[:n]
    for i in range(fade):
        t = i / fade
        out[i] = sig[i] * t + sig[n + i] * (1.0 - t)
    return out


def render_stereo(name, seconds=LOOP_SECONDS):
    """Return (left, right) float sample lists in [-1, 1] for a sound key."""
    if name not in SOUNDS:
        raise ValueError("unknown sound: %r" % name)
    n = int(SAMPLE_RATE * seconds)
    fade = int(SAMPLE_RATE * SEAM_SECONDS)
    gen = SOUNDS[name][1]
    base = zlib.crc32(name.encode("utf-8"))
    chans = []
    for ch in range(2):
        rng = random.Random(base * 2 + ch + 1)
        sig = gen(n + fade, rng)
        sig = _seam(sig, n, fade)
        sig = _normalize(sig)
        chans.append(sig)
    return chans[0], chans[1]


def render_wav(name, path, seconds=LOOP_SECONDS):
    """Render a sound key to a 16-bit stereo WAV file."""
    left, right = render_stereo(name, seconds)
    n = len(left)
    li = [int(max(-1.0, min(1.0, x)) * 32767) for x in left]
    ri = [int(max(-1.0, min(1.0, x)) * 32767) for x in right]
    inter = [0] * (2 * n)
    inter[0::2] = li
    inter[1::2] = ri
    pcm = array.array("h", inter)
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SAMPLE_RATE)
        w.writeframes(pcm.tobytes())
    return path


def ensure_wav(cache_dir, name):
    """Return the cached WAV path for a sound, rendering it if needed."""
    os.makedirs(cache_dir, exist_ok=True)
    path = os.path.join(cache_dir, "%s.wav" % name)
    if not os.path.exists(path):
        render_wav(name, path)
    return path
