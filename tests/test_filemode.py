"""Checks for local-file loop mode (audiofile.decode + synth.FileSource).

Run: python3 tests/test_filemode.py
Uses Qt's offscreen platform. Compressed-format fixtures are generated
with the ffmpeg CLI when present (skipped with a note when it is not);
the wav checks always run. All decoding happens locally, no network.
"""
import math
import os
import shutil
import struct
import subprocess
import sys
import wave
from array import array

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from PySide6.QtWidgets import QApplication

from zenhush import audiofile, synth
from zenhush.app import Player, Popup

FAILURES = []
TMP = "/tmp/zenhush-filemode-tests"
SR = synth.SAMPLE_RATE


def check(label, ok, detail=""):
    status = "PASS" if ok else "FAIL"
    print(f"[{status}] {label}" + (f" - {detail}" if detail else ""))
    if not ok:
        FAILURES.append(label)


def make_wav(path, seconds=2.0, freq=440.0, channels=1, amp=12000):
    with wave.open(path, "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(SR)
        n = int(seconds * SR)
        if channels == 1:
            frames = b"".join(struct.pack("<h", int(amp * math.sin(2 * math.pi * freq * i / SR)))
                              for i in range(n))
        else:
            frames = b"".join(struct.pack("<hh",
                                          int(amp * math.sin(2 * math.pi * freq * i / SR)),
                                          int(amp * math.sin(2 * math.pi * (freq * 1.5) * i / SR)))
                              for i in range(n))
        w.writeframes(frames)
    return n


def grab(source, seconds):
    return source.readData(int(seconds * SR) * 2)


def main():
    app = QApplication([])
    os.makedirs(TMP, exist_ok=True)

    # -- decode: wav always, compressed formats when ffmpeg CLI exists --
    wav_path = os.path.join(TMP, "tone.wav")
    n_src = make_wav(wav_path)
    samples = audiofile.decode(wav_path)
    check("wav decodes to mono float at engine rate",
          len(samples) == n_src and isinstance(samples, array)
          and samples.typecode == "f", f"{len(samples)} samples")
    peak = max(abs(x) for x in samples)
    check("wav decode preserves level", 0.30 < peak < 0.42, f"peak {peak:.3f}")

    stereo_path = os.path.join(TMP, "stereo.wav")
    make_wav(stereo_path, seconds=1.0, channels=2)
    stereo = audiofile.decode(stereo_path)
    check("stereo downmixes to mono", len(stereo) == SR, f"{len(stereo)} samples")
    check("downmix carries both channels",
          max(abs(x) for x in stereo) > 0.30, f"peak {max(abs(x) for x in stereo):.3f}")

    if shutil.which("ffmpeg"):
        for ext in ("flac", "mp3", "ogg", "m4a"):
            p = os.path.join(TMP, f"tone.{ext}")
            subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", wav_path, p],
                           check=True)
            try:
                s = audiofile.decode(p)
            except audiofile.AudioFileError as e:
                check(f"{ext} decodes", False, str(e))
                continue
            secs = len(s) / SR
            # Lossy containers can carry encoder padding (m4a observed +20 ms).
            check(f"{ext} decodes", 1.9 <= secs <= 2.2 and max(abs(x) for x in s) > 0.2,
                  f"{secs:.2f}s, peak {max(abs(x) for x in s):.3f}")
    else:
        print("[INFO] ffmpeg CLI not found: compressed-format decode checks skipped")

    bad_path = os.path.join(TMP, "notes.xyz")
    open(bad_path, "wb").write(b"not audio")
    try:
        audiofile.decode(bad_path)
        check("unsupported extension rejected", False)
    except audiofile.AudioFileError as e:
        check("unsupported extension rejected", "Unsupported type" in str(e), str(e))
    try:
        audiofile.decode(os.path.join(TMP, "missing.wav"))
        check("missing file rejected", False)
    except audiofile.AudioFileError as e:
        check("missing file rejected", "not exist" in str(e), str(e))

    # -- FileSource: sample-exact loop, rewind, honest output --
    tone = audiofile.decode(wav_path)
    fs = synth.FileSource(tone)
    # _produce wraps sample-exactly: sample n+k equals sample k
    k = 50
    seq = [fs._produce() for _ in range(len(tone) + k)]
    check("loop wraps sample-exactly",
          all(seq[len(tone) + i] == seq[i] for i in range(k)))

    fs2 = synth.FileSource(tone)
    fs2.set_volume(100)
    fs2.set_depth(0)
    fs2.set_rumble(0)
    for _ in range(3):
        fs2._produce()
    fs2.restart()
    check("restart rewinds to the file start", fs2._pos == 0 and fs2._fade_left > 0)

    fs3 = synth.FileSource(tone)
    fs3.set_volume(100)
    out = grab(fs3, 1.0)
    vals = struct.unpack("<%dh" % (len(out) // 2), out)
    check("file output is bounded int16", max(abs(v) for v in vals) <= 32767)
    check("file plays audibly at full volume",
          math.sqrt(sum(v * v for v in vals) / len(vals)) / 32767 > 0.05,
          f"rms {math.sqrt(sum(v * v for v in vals) / len(vals)) / 32767:.3f}")
    check("file output starts in the fade-in", abs(vals[0]) < abs(vals[-1]) * 0.5)

    fs4 = synth.FileSource(tone)
    fs4.set_volume(0)
    grab(fs4, 0.1)  # let the ramp settle
    out = grab(fs4, 0.5)
    vals = struct.unpack("<%dh" % (len(out) // 2), out)
    check("volume applies in file mode (0 = silence)", max(abs(v) for v in vals) == 0)

    fs5 = synth.FileSource(tone)
    fs5.set_volume(100)
    grab(fs5, 0.5)
    check("file mode feeds the visualizer (real output energy)",
          len(fs5.rms_values()) > 0 and all(0 <= v <= 1 for v in fs5.rms_values()))
    try:
        fs5.set_sound("brown")
        check("FileSource refuses noise recipes", False)
    except RuntimeError:
        check("FileSource refuses noise recipes", True)

    # -- app wiring: file mode through the Player and the status label --
    player = Player()
    popup = Popup(player)
    check("sound list ends with the file entry", popup.sounds[-1] == "file")
    check("file starts unchosen", popup.file_path is None)

    popup.file_path = wav_path
    popup.file_samples = tone
    popup._pick_sound("file")
    check("file mode plays through the player",
          player.current_kind == "file" and player.current == "file")
    check("file mode is never labelled Live",
          popup.eq.status_text == "File", popup.eq.status_text)
    check("subtitle shows the file name", popup.subtitle.text() == "tone.wav",
          popup.subtitle.text())
    popup._toggle_play()  # pause
    check("paused file reads Paused", popup.eq.status_text == "Paused")
    popup._toggle_play()  # resume
    check("resumed file reads File again", popup.eq.status_text == "File")

    popup._step_sound(-1)  # file -> ocean while playing: switches back to noise
    check("stepping back returns to live noise",
          player.current_kind == "noise" and popup.eq.status_text == "Live")

    popup._pick_sound("brown")
    for _ in range(popup.sounds.index("file") - popup.selected_index):
        popup._step_sound(1)
    check("stepping reaches the file entry", popup.selected_sound == "file")
    check("stepping onto a chosen file switches to it",
          player.current_kind == "file" and popup.eq.status_text == "File")

    player.sink.stop()

    print()
    if FAILURES:
        print(f"{len(FAILURES)} FAILURES: {', '.join(FAILURES)}")
        return 1
    print("all file-mode checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
