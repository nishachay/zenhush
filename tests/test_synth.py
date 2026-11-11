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

# 2. Never repeats: not a loop, distant windows differ, fresh sources differ
a = xs[:44100]
b = xs[20 * 44100: 21 * 44100]
dot = sum(x * y for x, y in zip(a, b)) / 44100
na = math.sqrt(sum(x * x for x in a) / 44100)
nb = math.sqrt(sum(x * x for x in b) / 44100)
corr = dot / (na * nb) if na and nb else 1.0
# A 20 s loop replays exactly (corr 1.0); brown noise's natural long memory
# wanders anywhere in between, so the bar is "not a replay".
check("second 20 is not a replay of second 0", abs(corr) < 0.9, f"corr {corr:.3f}")
wsrc = synth.LiveSource("white")
w = samples(grab(wsrc, 22))
wa, wb = w[:44100], w[21 * 44100: 22 * 44100]
dot = sum(x * y for x, y in zip(wa, wb)) / 44100
na = math.sqrt(sum(x * x for x in wa) / 44100)
nb = math.sqrt(sum(x * x for x in wb) / 44100)
wcorr = dot / (na * nb) if na and nb else 1.0
check("white noise windows are truly independent", abs(wcorr) < 0.1, f"corr {wcorr:.3f}")
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

# 5. Rumble is a real lowshelf: it boosts the low band absolutely, matching
# the preview's rated dB at the known points (58 -> +4, 100 -> +12)
def low_band_db(rumble):
    s = synth.LiveSource("white")
    s.set_depth(0)  # flat source so the shelf is isolated
    s.set_rumble(rumble)
    s.set_volume(100)
    d = samples(grab(s, 2))
    lp = 0.0
    lo = 0.0
    a = 1 - math.exp(-2 * math.pi * 160 / synth.SAMPLE_RATE)
    for x in d:
        lp += a * (x - lp)
        lo += lp * lp
    return math.sqrt(lo / len(d))
base = low_band_db(0)
db58 = 20 * math.log10(low_band_db(58) / base)
db100 = 20 * math.log10(low_band_db(100) / base)
check("rumble 58 boosts lows by ~+4 dB", 3.0 < db58 < 5.5, f"{db58:+.1f} dB")
check("rumble 100 boosts lows by ~+12 dB", 10.5 < db100 < 15.0, f"{db100:+.1f} dB")

# 5b. Gain staging: transparent below the knee at defaults, and NO harsh
# clipping at ANY valid slider combination - the peak safety rounds peaks
# softly, so samples never flatline at full scale even at max everything.
def matrix_run(sound, vol, seeds=3):
    """(max |sample|, knee engagements, would-have-clipped, longest full-scale run)"""
    peak = knee = limited = longest = 0
    for _ in range(seeds):
        for depth in (0, 65, 100):
            for rumble in (0, 58, 100):
                s = synth.LiveSource(sound)
                s.set_depth(depth); s.set_rumble(rumble); s.set_volume(vol)
                d = samples(grab(s, 1))
                peak = max(peak, max(abs(x) for x in d) / 32767)
                knee += s.knee_samples
                limited += s.limited_samples
                run = best = 0
                for x in d:
                    run = run + 1 if abs(x) >= 32767 else 0
                    best = max(best, run)
                longest = max(longest, best)
    return peak, knee, limited, longest

for name in ("brown", "white", "pink", "rain", "ocean"):
    peak, knee, limited, longest = matrix_run(name, 70)
    check(f"{name}: peak safety never engages at or below default volume",
          peak < 0.96 and knee == 0 and limited == 0, f"peak {peak:.2f}, knee {knee}")
for name in ("brown", "white", "pink", "rain", "ocean"):
    peak, knee, limited, longest = matrix_run(name, 100)
    check(f"{name}: no harsh clipping even at max volume",
          peak <= 1.0 and longest <= 3,
          f"peak {peak:.2f}, knee {knee} samples, would-clip {limited}, longest flatline {longest}")

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
