"""Continuous live sound synthesis for dither.

The app never plays a file. A LiveSource synthesizes fresh random samples
forever, shapes them with filters, and streams them straight to the audio
output: the sound is made right now, it never ends and it never repeats.

Parameter behaviour mirrors the original web preview (Muse handover,
Roma TASK-4): brown noise made live, defaults Volume 70 (gain 0.44),
Depth 65 (lowpass about 420 Hz), Rumble 58 (lowshelf about +4 dB).
The exact slider-to-Hz/dB curves were never shared, so the mappings here
are fitted to those three known points:

  volume_gain(v) = 0.9 * (v/100)^2          -> 70 maps to 0.441
  depth_cutoff(d) = 20000 * (52/20000)^(d/100) Hz  -> 65 maps to 420 Hz
  rumble_gain_db(r) = 12 * (r/100)^2        -> 58 maps to +4.0 dB
"""
import math
import os
import random
import threading
from array import array
from collections import deque

from PySide6.QtCore import QIODevice

SAMPLE_RATE = 44100

# Per-recipe loudness: measured output RMS at default Depth/Rumble varies
# widely, so each recipe gets a gain that lands it near TARGET_RMS, like the
# original offline renderer's normalization. The values are then trimmed by
# HEADROOM so the loudest corner (max rumble, default volume) stays below
# clipping; the volume slider's upper half easily makes the level back.
TARGET_RMS = 0.12
HEADROOM = 0.5
OUTPUT_GAIN = {"brown": 0.09 * HEADROOM, "white": 1.43 * HEADROOM,
               "pink": 0.41 * HEADROOM, "rain": 0.66 * HEADROOM,
               "ocean": 0.12 * HEADROOM}
# Visualizer: normalized output breathes around 0.7 on the bars.
RMS_SCALE = 12.0

# Gain staging, faithful to the preview's Web Audio chain: everything runs in
# float, the rumble lowshelf boosts absolutely (more rumble = more low end,
# never less volume), depth is a plain lowpass, and volume is the last gain
# before the output. Like a DAC, the only clip point is the final conversion;
# at the default Volume 70 (gain 0.44) even maxed Depth+Rumble stay clean.
CHANNELS = 1  # mono: lightest CPU, indistinguishable for shaped noise
_CHUNK = 661  # ~15 ms at 44.1 kHz, matches the visualizer window

DEFAULT_VOLUME = 70
DEFAULT_DEPTH = 65
DEFAULT_RUMBLE = 58

DEPTH_HI_HZ = 20000.0
DEPTH_LO_HZ = 52.0
RUMBLE_MAX_DB = 12.0
RUMBLE_SHELF_HZ = 160.0


def volume_gain(value):
    """Slider 0-100 -> output gain. 70 gives 0.441 (preview default 0.44)."""
    return 0.9 * (value / 100.0) ** 2


def depth_cutoff_hz(value):
    """Slider 0-100 -> lowpass cutoff in Hz. 65 gives 420 Hz."""
    return DEPTH_HI_HZ * (DEPTH_LO_HZ / DEPTH_HI_HZ) ** (value / 100.0)


def rumble_gain_db(value):
    """Slider 0-100 -> lowshelf boost in dB. 58 gives +4.0 dB."""
    return RUMBLE_MAX_DB * (value / 100.0) ** 2


# The one-pole shelf picks up ~60% of the sub-band, so the raw gain needs a
# drive factor to deliver the rated dB at the output (measured: 58 -> +4.0,
# 100 -> +12.2 dB on flat white noise).
RUMBLE_DRIVE = 1.7


class _Voice:
    """Stateful per-sample noise recipe. Each voice runs forever."""

    def __init__(self, rng):
        self.rng = rng
        # brown / pink filter state
        self.brown = 0.0
        self.p0 = self.p1 = self.p2 = 0.0
        # rain droplet state
        self.drop = 0.0
        self.drop_decay = 0.0
        self.drop_prev = 0.0
        # ocean wave phase
        self.t = 0.0

    def next(self, kind, uniform):
        r = uniform()
        if kind == "white":
            return r * 2.0 - 1.0
        if kind == "brown":
            self.brown = 0.998 * self.brown + 0.02 * (r * 2.0 - 1.0)
            return self.brown * 8.0
        if kind == "pink":
            w = r * 2.0 - 1.0
            self.p0 = 0.99765 * self.p0 + w * 0.0990460
            self.p1 = 0.96300 * self.p1 + w * 0.2965164
            self.p2 = 0.57000 * self.p2 + w * 1.0526913
            return (self.p0 + self.p1 + self.p2 + w * 0.1848) * 0.25
        if kind == "rain":
            w = r * 2.0 - 1.0
            self.p0 = 0.99765 * self.p0 + w * 0.0990460
            self.p1 = 0.96300 * self.p1 + w * 0.2965164
            self.p2 = 0.57000 * self.p2 + w * 1.0526913
            base = (self.p0 + self.p1 + self.p2 + w * 0.1848) * 0.25
            if self.rng.random() < 28.0 / SAMPLE_RATE:
                self.drop = self.rng.uniform(0.3, 1.0)
                self.drop_decay = math.exp(-1.0 / (SAMPLE_RATE * 0.002))
            self.drop *= self.drop_decay
            drop = self.drop * (self.rng.uniform(-1.0, 1.0))
            hp = drop - self.drop_prev
            self.drop_prev = drop
            return 0.6 * base + 0.45 * hp
        if kind == "ocean":
            self.brown = 0.998 * self.brown + 0.02 * (r * 2.0 - 1.0)
            self.t += 1.0 / SAMPLE_RATE
            lfo = (0.55 + 0.30 * math.sin(2 * math.pi * 0.07 * self.t)
                   + 0.15 * math.sin(2 * math.pi * 0.13 * self.t + 1.3))
            return self.brown * 8.0 * max(lfo, 0.05)
        raise ValueError("unknown sound: %r" % kind)


class LiveSource(QIODevice):
    """QIODevice that synthesizes the current sound on demand, forever.

    QAudioSink pulls bytes from this device. Every pull generates fresh
    random samples: no file, no buffer loop, no repetition. Depth and
    Rumble retune the filters live between pulls; Volume is applied by
    the sink itself so it stays click-free.
    """

    def __init__(self, sound="brown", parent=None):
        super().__init__(parent)
        self._lock = threading.Lock()
        self._sound = sound
        self._voice = _Voice(random.Random(self._seed()))
        # depth: one-pole lowpass coefficient
        self._depth_a = 1.0
        self._depth_state = 0.0
        # rumble: one-pole lowshelf, boosted low band added back
        self._rumble_a = 0.0
        self._rumble_gain = 1.0
        self._rumble_state = 0.0
        # volume: last gain stage, ramped to avoid zipper noise
        self._volume = volume_gain(DEFAULT_VOLUME)
        self._volume_target = self._volume
        self.set_depth(DEFAULT_DEPTH)
        self.set_rumble(DEFAULT_RUMBLE)
        # gentle fade-in so play/resume/switch never clicks or pops
        self._fade_left = 0
        # recent real output energy for the visualizer, ~15 ms per value
        self._rms_ring = deque(maxlen=64)
        self._rms_acc = 0.0
        self._rms_n = 0
        self.open(QIODevice.ReadOnly)

    @staticmethod
    def _seed():
        return int.from_bytes(os.urandom(8), "big")

    def isSequential(self):
        return True

    def bytesAvailable(self):
        return 1 << 20

    # -- live controls -------------------------------------------------

    FADE_SAMPLES = int(0.15 * SAMPLE_RATE)  # 150 ms gentle start

    def set_sound(self, name):
        with self._lock:
            self._sound = name
            self._voice = _Voice(random.Random(self._seed()))
            self._depth_state = 0.0
            self._rumble_state = 0.0
            self._fade_left = self.FADE_SAMPLES

    def restart(self):
        """Fresh noise from a new seed: restart never replays anything."""
        with self._lock:
            self._voice = _Voice(random.Random(self._seed()))
            self._depth_state = 0.0
            self._rumble_state = 0.0
            self._rms_ring.clear()
            self._fade_left = self.FADE_SAMPLES

    def set_volume(self, value):
        """Slider 0-100, preview-fitted curve: 70 -> gain 0.44."""
        self._volume_target = volume_gain(value)

    def set_depth(self, value):
        cutoff = depth_cutoff_hz(value)
        # one-pole lowpass: a = 1 - exp(-2*pi*fc/fs)
        self._depth_a = 1.0 - math.exp(-2.0 * math.pi * cutoff / SAMPLE_RATE)

    def set_rumble(self, value):
        self._rumble_gain = (10.0 ** (rumble_gain_db(value) / 20.0) - 1.0) * RUMBLE_DRIVE
        self._rumble_a = 1.0 - math.exp(-2.0 * math.pi * RUMBLE_SHELF_HZ / SAMPLE_RATE)

    def soft_start(self):
        """Resume from silence gently: ramp in instead of clicking on."""
        with self._lock:
            self._fade_left = self.FADE_SAMPLES

    def rms_values(self):
        """Recent real output energy, oldest first, one value per ~15 ms."""
        with self._lock:
            return tuple(self._rms_ring)

    # -- synthesis -----------------------------------------------------

    def readData(self, maxlen):
        n = maxlen // 2  # 16-bit mono
        if n <= 0:
            return b""
        with self._lock:
            kind = self._sound
            voice = self._voice
        gain = OUTPUT_GAIN.get(kind, 1.0)
        uniform = voice.rng.random
        depth_a = self._depth_a
        depth_s = self._depth_state
        rumble_a = self._rumble_a
        rumble_g = self._rumble_gain
        rumble_s = self._rumble_state
        nxt = voice.next
        vol = self._volume
        vol_t = self._volume_target
        vol_step = (vol_t - vol) / max(1, min(n, 882))  # ~20 ms ramp
        fade = self._fade_left
        fade_total = self.FADE_SAMPLES
        buf = array("h", bytes(2 * n))
        acc = self._rms_acc
        cnt = self._rms_n
        ring = self._rms_ring
        chunk = _CHUNK
        for i in range(n):
            x = nxt(kind, uniform)
            depth_s += depth_a * (x - depth_s)
            rumble_s += rumble_a * (x - rumble_s)
            v = (depth_s + rumble_s * rumble_g) * gain * vol
            if vol != vol_t:
                vol += vol_step
                if (vol_t - vol) * vol_step < 0:
                    vol = vol_t
            if fade > 0:
                v *= (fade_total - fade) / fade_total
                fade -= 1
            if v > 1.0:
                v = 1.0
            elif v < -1.0:
                v = -1.0
            buf[i] = int(v * 32767)
            acc += v * v
            cnt += 1
            if cnt >= chunk:
                ring.append(min(1.0, 0.15 + math.sqrt(acc / cnt) * RMS_SCALE))
                acc = 0.0
                cnt = 0
        self._depth_state = depth_s
        self._rumble_state = rumble_s
        self._volume = vol
        self._fade_left = fade
        self._rms_acc = acc
        self._rms_n = cnt
        return buf.tobytes()

    def writeData(self, data):
        return -1
