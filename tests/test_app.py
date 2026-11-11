"""Automated UI checks for the dither Qt app.

Run: python3 tests/test_app.py
Uses Qt's offscreen platform - no display needed. Checks widget structure
and control behavior; actual audio output to a device is NOT covered here.
"""
import os
import sys

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from PySide6.QtWidgets import QApplication, QSystemTrayIcon
from PySide6.QtMultimedia import QMediaPlayer

from dither_app import noise_gen
from dither_app.app import CACHE_DIR, Popup, Player, STYLE, TIMER_CHOICES

FAILURES = []


def check(label, ok, detail=""):
    status = "PASS" if ok else "FAIL"
    print(f"[{status}] {label}" + (f" - {detail}" if detail else ""))
    if not ok:
        FAILURES.append(label)


def main():
    app = QApplication([])
    app.setStyleSheet(STYLE)
    player = Player()
    popup = Popup(player)

    # Structure
    check("five sound buttons", len(popup.sound_buttons) == 5)
    check("sound names match engine", set(popup.sound_buttons) == set(noise_gen.SOUNDS))
    check("play button starts as 'play'", popup.play_button.text() == "play")
    check("timer choices", [popup.timer_box.itemText(i) for i in range(popup.timer_box.count())] == [t[0] for t in TIMER_CHOICES])
    check("volume starts at 70", popup.volume.value() == 70)

    # Pick a sound: checked state, engine render, playback requested
    popup._pick_sound("brown")
    check("brown checked, others not", popup.sound_buttons["brown"].isChecked()
          and not any(b.isChecked() for n, b in popup.sound_buttons.items() if n != "brown"))
    check("player current is brown", player.current == "brown")
    wav = os.path.join(CACHE_DIR, "brown.wav")
    check("brown.wav rendered to cache", os.path.exists(wav) and os.path.getsize(wav) > 100000,
          f"{os.path.getsize(wav) if os.path.exists(wav) else 0} bytes")
    app.processEvents()
    state = player.media.playbackState()
    check("playback requested (playing or loading)",
          state in (QMediaPlayer.PlaybackState.PlayingState,) or player.media.mediaStatus() != QMediaPlayer.MediaStatus.NoMedia,
          f"state={state}")
    check("play button shows 'pause'", popup.play_button.text() == "pause")

    # Switch sound mid-play
    popup._pick_sound("pink")
    check("switch to pink", player.current == "pink" and popup.sound_buttons["pink"].isChecked())

    # Toggle pause/resume
    popup._toggle_play()
    check("toggle pauses", popup.play_button.text() == "play" and not player.playing)
    popup._toggle_play()
    check("toggle resumes", popup.play_button.text() == "pause")

    # Restart with nothing played is a no-op (fresh popup)
    fresh = Popup(Player())
    fresh._restart()
    check("restart no-op before first play", not fresh.is_playing and fresh.elapsed.text() == "00:00")

    # Volume
    popup.volume.setValue(35)
    check("volume applied to output", abs(player.output.volume() - 0.35) < 1e-6, f"{player.output.volume()}")

    # Sleep timer arming and countdown
    idx = next(i for i, (_, m) in enumerate(TIMER_CHOICES) if m == 15)
    popup.timer_box.setCurrentIndex(idx)
    app.processEvents()
    check("sleep timer active after choosing 15 min", popup.sleep_timer.isActive())
    popup._update_countdown()
    text = popup.countdown.text()
    check("countdown shows mm:ss", len(text) == 5 and text[2] == ":", text)

    # Timer expiry pauses and resets
    popup._sleep_expired()
    check("expiry pauses and resets", popup.play_button.text() == "play"
          and popup.timer_box.currentIndex() == 0 and popup.countdown.text() == "")

    # Choosing Off stops the timer
    popup.timer_box.setCurrentIndex(idx)
    popup.timer_box.setCurrentIndex(0)
    check("Off stops timer", not popup.sleep_timer.isActive() and popup.countdown.text() == "")

    # Restart and elapsed
    check("restart button exists", popup.restart_button.text() == "restart")
    check("elapsed starts 00:00", popup.elapsed.text() == "00:00")
    popup._pick_sound("brown")
    check("elapsed timer runs while playing", popup.elapsed_timer.isActive())
    popup._elapsed_tick(); popup._elapsed_tick()
    check("elapsed counts", popup.elapsed.text() == "00:02", popup.elapsed.text())
    popup._toggle_play()
    check("pause stops elapsed timer", not popup.elapsed_timer.isActive())
    popup._restart()
    check("restart resets elapsed and resumes",
          popup.elapsed.text() == "00:00" and popup.play_button.text() == "pause" and popup.elapsed_timer.isActive())
    popup._sleep_expired()
    check("sleep expiry stops elapsed timer", not popup.elapsed_timer.isActive())

    # Real QTimer expiry path (short interval, real timeout signal)
    popup._pick_sound("brown")
    popup.timer_box.setCurrentIndex(idx)
    popup.sleep_timer.start(300)  # override interval only; wiring is real
    import time
    t0 = time.time()
    while time.time() - t0 < 3 and popup.is_playing:
        app.processEvents()
        time.sleep(0.05)
    check("real QTimer expiry pauses and resets",
          not popup.is_playing and popup.play_button.text() == "play" and popup.timer_box.currentIndex() == 0)

    # Tray availability is environmental - report, don't assert
    available = QSystemTrayIcon.isSystemTrayAvailable()
    print(f"[INFO] system tray available in this environment: {available}")

    print()
    if FAILURES:
        print(f"{len(FAILURES)} FAILURES: {', '.join(FAILURES)}")
        return 1
    print("all app checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
