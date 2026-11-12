"""Automated checks for the Noise Notch audio engine.

Run: python3 tests/test_noise.py
Needs numpy (test-only dependency; the app itself is stdlib-only).
"""
import math
import os
import sys
import wave

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "zenhush"))
import noise_gen

import numpy as np

SECONDS = 6
FAILURES = []


def check(label, ok, detail=""):
    print("%s %-46s %s" % ("PASS" if ok else "FAIL", label, detail))
    if not ok:
        FAILURES.append(label)


def octave_slope_db(samples, rate):
    """Slope of log-power across octave bands (dB per octave)."""
    x = np.asarray(samples) * np.hanning(len(samples))
    power = np.abs(np.fft.rfft(x)) ** 2
    freqs = np.fft.rfftfreq(len(x), 1.0 / rate)
    band_centers, band_power = [], []
    f = 100.0
    while f * 2 <= 6400.0:
        mask = (freqs >= f) & (freqs < f * 2)
        band_centers.append(math.log2(f * math.sqrt(2)))
        band_power.append(math.log10(power[mask].mean() + 1e-20))
        f *= 2
    slope, _intercept = np.polyfit(band_centers, band_power, 1)
    return slope * 10.0  # dB per octave


for name in noise_gen.SOUNDS:
    left, right = noise_gen.render_stereo(name, seconds=SECONDS)
    n = int(noise_gen.SAMPLE_RATE * SECONDS)
    peak = max(max(abs(x) for x in left), max(abs(x) for x in right))
    rms = math.sqrt(sum(x * x for x in left) / len(left))
    check("%s: length" % name, len(left) == n == len(right), str(len(left)))
    check("%s: no clipping (peak <= %.2f)" % (name, noise_gen.PEAK), peak <= noise_gen.PEAK + 1e-9, "peak=%.3f" % peak)
    check("%s: loudness in range" % name, 0.05 <= rms <= 0.25, "rms=%.3f" % rms)
    check("%s: stereo channels decorrelated" % name,
          abs(float(np.corrcoef(np.asarray(left), np.asarray(right))[0, 1])) < 0.2)
    seam_jump = abs(left[0] - left[-1])
    if name in ("brown", "pink", "ocean"):
        check("%s: loop seam smooth" % name, seam_jump < 0.15, "jump=%.4f" % seam_jump)

slope_white = octave_slope_db(noise_gen.render_stereo("white", SECONDS)[0], noise_gen.SAMPLE_RATE)
slope_pink = octave_slope_db(noise_gen.render_stereo("pink", SECONDS)[0], noise_gen.SAMPLE_RATE)
slope_brown = octave_slope_db(noise_gen.render_stereo("brown", SECONDS)[0], noise_gen.SAMPLE_RATE)
check("white: flat spectrum (-2..+2 dB/oct)", -2.0 <= slope_white <= 2.0, "%.2f" % slope_white)
check("pink: ~-3 dB/oct (-5..-1)", -5.0 <= slope_pink <= -1.0, "%.2f" % slope_pink)
check("brown: ~-6 dB/oct (-8.5..-4)", -8.5 <= slope_brown <= -4.0, "%.2f" % slope_brown)
check("ordering: white > pink > brown", slope_white > slope_pink > slope_brown)

path = "/tmp/noise-notch-test.wav"
noise_gen.render_wav("brown", path, seconds=2)
with wave.open(path, "rb") as w:
    check("wav: 2 ch / 16-bit / 44.1kHz",
          (w.getnchannels(), w.getsampwidth(), w.getframerate()) == (2, 2, 44100),
          "%dch %dbit %dHz" % (w.getnchannels(), w.getsampwidth() * 8, w.getframerate()))
    check("wav: frame count", w.getnframes() == 2 * 44100, str(w.getnframes()))
os.remove(path)

print()
if FAILURES:
    print("FAILED: %d check(s)" % len(FAILURES))
    sys.exit(1)
print("ALL CHECKS PASSED")
