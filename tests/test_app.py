"""Automated UI checks for the Zenhush Qt app.

Run: python3 tests/test_app.py
Uses Qt's offscreen platform - no display needed. Checks widget structure,
control behavior and theme contrast; actual audio output to a device is
NOT covered here.
"""
import os
import sys

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from PySide6.QtWidgets import QApplication, QSystemTrayIcon

from zenhush import noise_gen
from zenhush import synth
from zenhush.app import (CACHE_DIR, THEMES, TIMER_CHOICES, EqWidget, PlayButton,
                            Popup, Player, app_font_family, contrast_ratio)

FAILURES = []


def check(label, ok, detail=""):
    status = "PASS" if ok else "FAIL"
    print(f"[{status}] {label}" + (f" - {detail}" if detail else ""))
    if not ok:
        FAILURES.append(label)


def main():
    app = QApplication([])
    player = Player()
    popup = Popup(player)

    # Structure
    check("sound order matches engine plus file entry", popup.sounds == list(noise_gen.SOUNDS) + ["file"])
    check("play button is a 64px filled circle", popup.play_button.width() == 64
          and isinstance(popup.play_button, PlayButton))
    check("play mode starts as 'play'", popup.play_button.mode == "play")
    check("timer choices", [popup.timer_box.itemText(i) for i in range(popup.timer_box.count())] == [t[0] for t in TIMER_CHOICES])
    check("volume starts at 70", popup.volume.value() == 70)
    check("settings hidden by default", not popup.settings.isVisible())
    check("icon-only buttons have accessible names",
          all(b.accessibleName() for b in (popup.prev_button, popup.next_button,
              popup.restart_button, popup.settings_button, popup.play_button)))
    check("main buttons keyboard-focusable",
          popup.play_button.focusPolicy() != 0 and popup.restart_button.focusPolicy() != 0)

    # Selection without playback: chevrons change sound, nothing plays
    popup._step_sound(1)  # brown -> white
    check("chevron changes selected sound", popup.selected_sound == "white"
          and popup.title.text() == "White Noise")
    check("chevron while stopped does not play", not popup.is_playing and player.current is None)
    popup._step_sound(-1)
    check("chevron back", popup.selected_sound == "brown")
    popup._step_sound(-1)
    check("chevron wraps around", popup.selected_sound == "file")
    popup._step_sound(-1)
    check("chevron wraps past file to ocean", popup.selected_sound == "ocean")
    popup._step_sound(1)
    popup._step_sound(1)  # back to brown

    # Play the selected sound
    popup._toggle_play()
    check("play starts selected sound", player.current == "brown" and popup.is_playing)
    check("no audio file is played: live synthesis drives the sink",
          not hasattr(player, "media") and player._started and player.source is not None)
    check("play button shows pause mode", popup.play_button.mode == "pause")
    check("status shows Live while playing", popup.eq.status_text == "Live")
    check("eq animates while playing", popup.eq.timer.isActive())

    # Switch sound mid-play: immediate switch, keeps playing
    popup._step_sound(1)  # -> white
    check("chevron while playing switches audio", player.current == "white" and popup.is_playing)

    # Toggle pause/resume
    popup._toggle_play()
    check("toggle pauses", popup.play_button.mode == "play" and not player.playing)
    check("eq stops animating when paused", not popup.eq.timer.isActive())
    check("status shows Paused when paused", popup.eq.status_text == "Paused")
    popup._toggle_play()
    check("toggle resumes", popup.play_button.mode == "pause")

    # Restart with nothing played is a no-op (fresh popup)
    fresh = Popup(Player())
    fresh._restart()
    check("restart no-op before first play", not fresh.is_playing and fresh.eq.elapsed_text == "0:00")

    # Volume
    popup.volume.setValue(35)
    expected = synth.volume_gain(35)
    check("volume applied to the float chain", abs(player.source._volume_target - expected) < 1e-6,
          f"{player.source._volume_target} vs {expected}")
    check("preview default volume 70 maps to gain 0.44",
          abs(synth.volume_gain(70) - 0.441) < 0.005, f"{synth.volume_gain(70)}")

    # Sleep timer arming and countdown
    idx = next(i for i, (_, m) in enumerate(TIMER_CHOICES) if m == 15)
    popup.timer_box.setCurrentIndex(idx)
    app.processEvents()
    check("sleep timer active after choosing 15 min", popup.sleep_timer.isActive())
    popup._update_countdown()
    text = popup.eq.status_text
    check("countdown shows m:ss", len(text) >= 4 and ":" in text, text)

    # Timer expiry pauses and resets
    popup._sleep_expired()
    check("expiry pauses and resets", popup.play_button.mode == "play"
          and popup.timer_box.currentIndex() == 0)

    # Choosing Off stops the timer
    popup.timer_box.setCurrentIndex(idx)
    popup.timer_box.setCurrentIndex(0)
    check("Off stops timer", not popup.sleep_timer.isActive())

    # Elapsed
    check("elapsed starts 0:00", popup.eq.elapsed_text == "0:00")
    popup._toggle_play()
    check("elapsed timer runs while playing", popup.elapsed_timer.isActive())
    popup._elapsed_tick(); popup._elapsed_tick()
    check("elapsed counts", popup.eq.elapsed_text == "0:02", popup.eq.elapsed_text)
    popup._toggle_play()
    check("pause stops elapsed timer", not popup.elapsed_timer.isActive())
    popup._restart()
    check("restart resets elapsed and resumes",
          popup.eq.elapsed_text == "0:00" and popup.play_button.mode == "pause" and popup.elapsed_timer.isActive())
    popup._sleep_expired()
    check("sleep expiry stops elapsed timer", not popup.elapsed_timer.isActive())

    # EQ animation ticks change bars only while playing: the bars are fed
    # by the synthesis engine's real recent output energy.
    popup._toggle_play()
    vals = [0.3] * 12
    def fake_source():
        vals[0] = 0.9 - vals[0]  # alternate loud/quiet so ticks visibly move
        return tuple(vals)
    popup.eq.source = fake_source
    before = list(popup.eq.heights)
    for _ in range(4):
        popup.eq._tick()
    check("eq bars move while playing", popup.eq.heights != before)
    popup._toggle_play()
    popup.eq.set_reduce_motion(True)
    check("reduce motion stops eq timer", not popup.eq.timer.isActive())
    popup.eq.set_reduce_motion(False)

    # Depth/Rumble sliders drive the engine live; Reset restores preview defaults
    recorded = {}
    player.source.set_depth = lambda v: recorded.__setitem__("depth", v)
    player.source.set_rumble = lambda v: recorded.__setitem__("rumble", v)
    popup.depth.setValue(90)
    popup.rumble.setValue(12)
    check("depth slider reaches the engine", recorded.get("depth") == 90, str(recorded))
    check("rumble slider reaches the engine", recorded.get("rumble") == 12, str(recorded))
    popup.volume.setValue(55)
    popup._reset_sound_controls()
    check("reset restores preview defaults 70/65/58",
          popup.volume.value() == 70 and popup.depth.value() == 65 and popup.rumble.value() == 58,
          f"{popup.volume.value()}/{popup.depth.value()}/{popup.rumble.value()}")
    check("reset pushes defaults to the engine",
          recorded.get("depth") == 65 and recorded.get("rumble") == 58, str(recorded))

    # Settings disclosure grows the popup
    h0 = popup.height()
    popup.settings_button.setChecked(True)
    check("settings disclosure expands popup", popup.height() == h0 + Popup.SETTINGS_HEIGHT)
    popup.settings_button.setChecked(False)
    check("settings collapse restores height", popup.height() == h0)

    # Live bars: real energy from the synthesis engine, not decoration
    live = synth.LiveSource("brown")
    live.readData(44100 * 2)  # one second of fresh noise
    ring = live.rms_values()
    check("engine reports real output energy", len(ring) > 12 and any(v > 0.2 for v in ring),
          f"{len(ring)} values")
    popup.eq.source = live.rms_values
    popup.eq.set_playing(True)
    popup.eq.heights = [0.0] * popup.eq.BAR_COUNT
    for _ in range(30):
        popup.eq._tick()
    engine_heights = list(popup.eq.heights)
    window_mean = sum(live.rms_values()[-12:]) / 12
    check("bars track real engine energy",
          sum(engine_heights) > 3 and abs(sum(engine_heights) / 12 - window_mean) < 0.3,
          f"bars {sum(engine_heights)/12:.2f} vs signal {window_mean:.2f}")
    quiet = synth.LiveSource("white")
    quiet.readData(44100 * 2)
    popup.eq.source = lambda: tuple(v * 0.1 for v in quiet.rms_values())
    for _ in range(40):
        popup.eq._tick()
    check("bars fall when the signal is quiet",
          sum(popup.eq.heights) < sum(engine_heights) * 0.7,
          f"{sum(popup.eq.heights):.2f} vs {sum(engine_heights):.2f}")
    frozen = list(popup.eq.heights)
    popup.eq.set_playing(False)
    check("pause freezes the real energy", popup.eq.heights == frozen
          and popup.eq.heights != list(EqWidget.IDLE_HEIGHTS))
    popup.eq.source = None
    popup.eq.set_playing(True); popup.eq.set_playing(False)

    # Custom settings controls: segmented + toggle behave like the combos they replaced
    from zenhush.app import Segmented, Toggle
    check("sleep timer is a segmented control", isinstance(popup.timer_box, Segmented))
    check("appearance is a segmented control", isinstance(popup.theme_box, Segmented))
    check("reduce motion is a toggle", isinstance(popup.motion_box, Toggle))
    popup.timer_box.setCurrentIndex(2)
    check("segmented selection lands", popup.timer_box.currentIndex() == 2
          and popup.timer_box.currentText() == "30 min")
    for _ in range(20):
        app.processEvents()
    import time as _t
    _t.sleep(0.25)
    for _ in range(20):
        app.processEvents()
    check("segmented thumb settles under selection",
          abs(popup.timer_box._pos - 2.0) < 1e-6, f"{popup.timer_box._pos}")
    popup.timer_box.setCurrentIndex(0)
    popup.motion_box.setCurrentIndex(1)
    check("toggle turns on", popup.motion_box.currentIndex() == 1
          and popup.motion_box.currentText() == "On" and popup.reduce_motion)
    popup.motion_box.setCurrentIndex(0)
    check("toggle turns off", not popup.reduce_motion)

    # Themes: both exist, switching applies, measured contrast passes
    check("both themes defined", set(THEMES) == {"light", "dark"})
    popup.apply_theme("light")
    fam = app_font_family()
    check("bundled font loads", fam == "Schibsted Grotesk", fam or "no family")
    from PySide6.QtGui import QFontInfo
    check("title uses bundled font, not fallback",
          QFontInfo(popup.title.font()).family() == "Schibsted Grotesk",
          QFontInfo(popup.title.font()).family())

    check("light theme applied", THEMES["light"]["text"] in popup.title.styleSheet())
    popup.apply_theme("dark")
    check("dark theme applied", THEMES["dark"]["text"].lstrip("#") in popup.title.styleSheet())
    for name, t in THEMES.items():
        r_title = contrast_ratio(t["text"], t["card"])
        check(f"{name}: title on card >= 4.5:1", r_title >= 4.5, f"{r_title:.2f}:1")
        r_sub = contrast_ratio(t["subtext"], t["card"])
        check(f"{name}: subtitle on card >= 4.5:1", r_sub >= 4.5, f"{r_sub:.2f}:1")
        r_status = contrast_ratio(t["accent_text"], t["inset"])
        check(f"{name}: status on inset >= 4.5:1", r_status >= 4.5, f"{r_status:.2f}:1")
        r_icon = contrast_ratio(t["on_accent"], t["accent"])
        check(f"{name}: play icon on accent >= 3:1", r_icon >= 3.0, f"{r_icon:.2f}:1")

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
          not popup.is_playing and popup.play_button.mode == "play" and popup.timer_box.currentIndex() == 0)

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
