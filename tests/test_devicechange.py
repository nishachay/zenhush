"""Device-change tests: the sound follows the system default output.

Two behaviors, both need a PulseAudio rig (pactl + null sinks); SKIP cleanly
without one:
  hotplug  - a NEW device appears and becomes default: sink rebuilds onto it
             (audioOutputsChanged fires).
  switch   - the user switches the default between two EXISTING devices:
             Qt fires no signal, so the next play/resume re-checks and moves.

Run: python3 tests/test_devicechange.py
"""
import os
import shutil
import struct
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
from PySide6.QtWidgets import QApplication

app = QApplication.instance() or QApplication([])
from zenhush.app import Player

FAILURES = []
SKIPS = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + f"{name:45s} {detail}")
    if not ok:
        FAILURES.append(name)


def sh(cmd):
    return subprocess.run(["bash", "-c", cmd], capture_output=True, text=True)


def pulse_ok():
    return shutil.which("pactl") and sh("pactl info").returncode == 0


def capture(monitor, seconds=1.2):
    raw = subprocess.run(
        ["bash", "-c", f"timeout {seconds} parec -d {monitor} "
         f"--format=float32le --rate=44100 --channels=1 2>/dev/null"],
        capture_output=True).stdout
    n = len(raw) // 4
    if not n:
        return 0.0
    vals = struct.unpack(f"<{n}f", raw[:n * 4])
    return (sum(v * v for v in vals) / n) ** 0.5


def pump(seconds):
    end = time.time() + seconds
    while time.time() < end:
        app.processEvents()
        time.sleep(0.05)


def cleanup():
    sh("pactl set-default-sink zh_dev1 2>/dev/null; "
       "pactl unload-module module-null-sink 2>/dev/null; true")


if not pulse_ok():
    SKIPS.append("pulse/pactl rig")
    print("SKIP device-change checks (no PulseAudio rig)")
    print("all device-change checks skipped")
    sys.exit(0)

sh("pactl unload-module module-null-sink 2>/dev/null; true")
sh("pactl load-module module-null-sink sink_name=zh_dev1 "
   "sink_properties=device.description=zh_one")
sh("pactl set-default-sink zh_dev1")
time.sleep(0.5)
try:
    p = Player()
    p.play_sound("brown")
    pump(1.0)
    base = capture("zh_dev1.monitor")
    check("baseline plays on first default", base > 0.003, f"rms={base:.4f}")

    # --- hotplug: new device appears and becomes default ---
    old_sink = p.sink
    sh("pactl load-module module-null-sink sink_name=zh_dev2 "
       "sink_properties=device.description=zh_two")
    sh("pactl set-default-sink zh_dev2")
    pump(2.0)
    moved = capture("zh_dev2.monitor")
    check("hotplug: sink object rebuilt", p.sink is not old_sink)
    check("hotplug: audio on new default", moved > 0.003, f"rms={moved:.4f}")
    silent = capture("zh_dev1.monitor", 0.8)
    check("hotplug: old device silent", silent < 0.002, f"rms={silent:.4f}")
    check("hotplug: still active, no error",
          str(p.sink.state()).endswith("ActiveState") and
          str(p.sink.error()).endswith("NoError"))

    # --- switch: default moves between two existing devices, no signal ---
    sh("pactl set-default-sink zh_dev1")
    pump(1.5)  # any list-change noise settles; no new device appears now
    p.restart()  # user action re-checks the default
    pump(1.0)
    back = capture("zh_dev1.monitor")
    check("switch: audio on switched default", back > 0.003,
          f"rms={back:.4f}")
    silent2 = capture("zh_dev2.monitor", 0.8)
    check("switch: previous device silent", silent2 < 0.002,
          f"rms={silent2:.4f}")

    p.pause()
    pump(0.3)
    check("pause still works after rebuilds",
          str(p.sink.state()).endswith("SuspendedState"))
finally:
    cleanup()

print()
if FAILURES:
    print(f"{len(FAILURES)} FAILURES: {', '.join(FAILURES)}")
    sys.exit(1)
print("all device-change checks passed")
