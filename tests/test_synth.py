"""Engine tests: the sound is generated live, forever, never repeating.

Run: python3 tests/test_synth.py
"""
import math
import os
import struct
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
from PySide6.QtWidgets import QApplication

app = QApplication.instance() or QApplication([])
from dither_app import synth

FAILURES = []


def check(name, ok, detail=""):
    print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f" - {detail}" if detail else ""))
    if not ok:
        FAILURES.append(name)


def grab(source, seconds):
    data = b""
    while len(data) < seconds * synth.SAMPLE_RATE * 2:
        data += source.readData(8192)
    return data


def samples(data):
    return struct.unpack(f"{len(data)//2}h", data)


# 1. Never ends: 30 seconds of audio streams without exhaustion or decay
src = synth.LiveSource("brown")
t0 = time.time()
data = grab(src, 30)
dt = time.time() - t0
xs = samples(data)
rms = math.sqrt(sum(x * x for x in xs[:44100]) / 44100) / 32767
rms_end = math.sqrt(sum(x * x for x in xs[-44100:]) / 44100) / 32767
check("streams indefinitely", len(xs) >= 30 * synth.SAMPLE_RATE, f"{len(xs)} samples")
check("signal stays healthy over 30 s", 0.03 < rms < 0.5 and 0.03 < rms_end < 0.5,
      f"first {rms:.3f}, last {rms_end:.3f}")
check("generation is light on the CPU", dt < 1.5, f"30 s in {dt*1000:.0f} ms ({dt/30*100:.1f}% of one core)")

# 2. Never repeats: distant windows are uncorrelated, fresh sources differ
a = xs[:44100]
b = xs[20 * 44100: 21 * 44100]
dot = sum(x * y for x, y in zip(a, b)) / 44100
na = math.sqrt(sum(x * x for x in a) / 44100)
nb = math.sqrt(sum(x * x for x in b) / 44100)
corr = dot / (na * nb) if na and nb else 1.0
check("second 0 and second 20 are uncorrelated", abs(corr) < 0.2, f"corr {corr:.3f}")
other = synth.LiveSource("brown")
check("two sources generate different noise", grab(src, 1) != grab(other, 1))
src2 = synth.LiveSource("brown")
first = grab(src2, 1)
src2.restart()
check("restart makes fresh noise, not a replay", grab(src2, 1) != first)

# 3. Preview parameter mappings fit the handover's known points
check("volume 70 -> gain 0.44", abs(synth.volume_gain(70) - 0.441) < 0.005,
      f"{synth.volume_gain(70)}")
check("volume 0 -> silence", synth.volume_gain(0) == 0.0)
check("depth 65 -> 420 Hz", abs(synth.depth_cutoff_hz(65) - 420) < 5,
      f"{synth.depth_cutoff_hz(65):.0f} Hz")
check("rumble 58 -> +4 dB", abs(synth.rumble_gain_db(58) - 4.0) < 0.2,
      f"{synth.rumble_gain_db(58):.2f} dB")

# 4. Depth is a real lowpass: higher depth means less high-frequency energy
def hf_energy(depth):
    s = synth.LiveSource("brown")
    s.set_depth(depth)
    d = samples(grab(s, 2))
    return sum(abs(d[i] - d[i - 1]) for i in range(1, len(d))) / len(d)
check("depth darkens the sound", hf_energy(100) < hf_energy(0) * 0.3,
      f"{hf_energy(100):.0f} vs {hf_energy(0):.0f}")

# 5. Rumble is a real lowshelf: it tilts energy toward the lows
def high_residual(rumble):
    s = synth.LiveSource("white")
    s.set_depth(0)
    s.set_rumble(rumble)
    d = samples(grab(s, 2))
    lp = 0.0
    hi = 0.0
    for x in d:
        lp += 0.02 * (x - lp)
        hi += (x - lp) ** 2
    return math.sqrt(hi / len(d))
check("rumble boosts lows over highs", high_residual(100) < high_residual(0) * 0.45,
      f"{high_residual(100):.0f} vs {high_residual(0):.0f}")

# 6. Every sound generates distinct, healthy noise
for name in ("white", "pink", "rain", "ocean"):
    s = synth.LiveSource(name)
    d = samples(grab(s, 3))
    r = math.sqrt(sum(x * x for x in d) / len(d)) / 32767
    check(f"{name}: live, non-silent, stable", 0.03 < r < 0.5, f"rms {r:.3f}")
check("sounds differ from each other",
      grab(synth.LiveSource("brown"), 1) != grab(synth.LiveSource("white"), 1))

# 7. Gentle start: resume/switch ramps in, never clicks on at full level
s = synth.LiveSource("white")
s.set_depth(0)  # unfiltered white has even energy, so the ramp is measurable
grab(s, 1)
s.soft_start()
d = samples(grab(s, 1))
def seg_rms(seg):
    return math.sqrt(sum(x * x for x in seg) / len(seg)) if seg else 0
first, steady = seg_rms(d[:2205]), seg_rms(d[10000:18000])
check("gentle start ramps in", 0 < first < steady * 0.7,
      f"first 50 ms {first:.0f} vs steady {steady:.0f}")

# 8. The visualizer's energy feed is real output, one value per ~15 ms
s = synth.LiveSource("pink")
grab(s, 1)
ring = s.rms_values()
check("rms ring feeds the bars", 30 <= len(ring) <= 64 and all(0 <= v <= 1 for v in ring),
      f"{len(ring)} values")

print()
if FAILURES:
    print(f"{len(FAILURES)} FAILURES: {', '.join(FAILURES)}")
    sys.exit(1)
print("all synth checks passed")
