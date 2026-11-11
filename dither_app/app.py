"""dither - a tiny noise machine for your top bar.

Cross-platform desktop app (Windows, macOS, Linux): a system-tray icon
with a compact popup that plays locally synthesized noise. All audio is
generated on the device by noise_gen; nothing is downloaded.
"""
import os
import sys

from PySide6.QtCore import Qt, QTimer, QUrl
from PySide6.QtGui import QAction, QCursor, QIcon
from PySide6.QtMultimedia import QAudioOutput, QMediaPlayer
from PySide6.QtWidgets import (QApplication, QComboBox, QHBoxLayout, QLabel,
                               QMenu, QPushButton, QSlider, QSystemTrayIcon,
                               QVBoxLayout, QWidget)

from . import noise_gen

ASSETS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "assets")
CACHE_DIR = os.path.join(os.path.expanduser("~"), ".cache", "dither")

TIMER_CHOICES = [("Off", 0), ("15 min", 15), ("30 min", 30), ("45 min", 45), ("60 min", 60)]

STYLE = """
QWidget { background: #0B0A08; color: #F5F1EA; font-family: monospace; font-size: 13px; }
QLabel#wordmark { font-size: 22px; color: #F5F1EA; }
QLabel#countdown { color: #C8823E; }
QPushButton {
    background: transparent; border: 1px solid #3A332B; border-radius: 6px;
    padding: 6px 4px; color: #F5F1EA;
}
QPushButton:checked { background: #C8823E; border-color: #C8823E; color: #0B0A08; }
QPushButton#play { font-size: 15px; padding: 8px; }
QSlider::groove:horizontal { height: 4px; background: #3A332B; border-radius: 2px; }
QSlider::handle:horizontal {
    width: 14px; height: 14px; margin: -5px 0; border-radius: 7px; background: #C8823E;
}
QSlider::sub-page:horizontal { background: #C8823E; border-radius: 2px; }
QComboBox {
    background: transparent; border: 1px solid #3A332B; border-radius: 6px; padding: 5px 8px;
}
QComboBox QAbstractItemView { background: #171512; color: #F5F1EA; border: 1px solid #3A332B; }
QMenu { background: #171512; color: #F5F1EA; border: 1px solid #3A332B; }
QMenu::item:selected { background: #C8823E; color: #0B0A08; }
"""


class Player:
    """Gapless looping playback of locally synthesized sounds."""

    def __init__(self):
        self.output = QAudioOutput()
        self.media = QMediaPlayer()
        self.media.setAudioOutput(self.output)
        self.media.setLoops(-1)
        self.output.setVolume(0.7)
        self.current = None

    def play_sound(self, name):
        path = noise_gen.ensure_wav(CACHE_DIR, name)
        self.current = name
        self.media.setSource(QUrl.fromLocalFile(path))
        self.media.play()

    def pause(self):
        self.media.pause()

    def resume(self):
        if self.current:
            self.media.play()

    def set_volume(self, value):
        self.output.setVolume(value)

    @property
    def playing(self):
        return self.media.playbackState() == QMediaPlayer.PlaybackState.PlayingState


class Popup(QWidget):
    """The compact control panel, styled on the dither brand."""

    def __init__(self, player):
        super().__init__(None, Qt.FramelessWindowHint | Qt.Tool | Qt.WindowStaysOnTopHint)
        self.player = player
        # What the user last asked for; the engine may lag behind it (device
        # stalls, no audio device), so button state tracks this, not the engine.
        self.is_playing = False
        self.setFixedSize(300, 330)
        self.setWindowTitle("dither")

        layout = QVBoxLayout(self)
        layout.setContentsMargins(16, 14, 16, 14)
        layout.setSpacing(10)

        wordmark = QLabel("dither")
        wordmark.setObjectName("wordmark")
        layout.addWidget(wordmark)

        row = QHBoxLayout()
        row.setSpacing(6)
        self.sound_buttons = {}
        for name in noise_gen.SOUNDS:
            btn = QPushButton(name)
            btn.setCheckable(True)
            btn.clicked.connect(lambda checked=False, n=name: self._pick_sound(n))
            self.sound_buttons[name] = btn
            row.addWidget(btn)
        layout.addLayout(row)

        vol_row = QHBoxLayout()
        vol_row.addWidget(QLabel("volume"))
        self.volume = QSlider(Qt.Horizontal)
        self.volume.setRange(0, 100)
        self.volume.setValue(70)
        self.volume.valueChanged.connect(lambda v: self.player.set_volume(v / 100.0))
        vol_row.addWidget(self.volume)
        layout.addLayout(vol_row)

        timer_row = QHBoxLayout()
        timer_row.addWidget(QLabel("sleep"))
        self.timer_box = QComboBox()
        for label, minutes in TIMER_CHOICES:
            self.timer_box.addItem(label, minutes)
        self.timer_box.currentIndexChanged.connect(self._timer_changed)
        timer_row.addWidget(self.timer_box)
        self.countdown = QLabel("")
        self.countdown.setObjectName("countdown")
        timer_row.addWidget(self.countdown)
        layout.addLayout(timer_row)

        play_row = QHBoxLayout()
        self.play_button = QPushButton("play")
        self.play_button.setObjectName("play")
        self.play_button.clicked.connect(self._toggle_play)
        play_row.addWidget(self.play_button)
        self.restart_button = QPushButton("restart")
        self.restart_button.clicked.connect(self._restart)
        play_row.addWidget(self.restart_button)
        layout.addLayout(play_row)

        self.elapsed = QLabel("00:00")
        self.elapsed.setObjectName("countdown")
        layout.addWidget(self.elapsed)

        self.sleep_timer = QTimer(self)
        self.sleep_timer.setSingleShot(True)
        self.sleep_timer.timeout.connect(self._sleep_expired)
        self.tick = QTimer(self)
        self.tick.setInterval(1000)
        self.tick.timeout.connect(self._update_countdown)

        self.elapsed_seconds = 0
        self.elapsed_timer = QTimer(self)
        self.elapsed_timer.setInterval(1000)
        self.elapsed_timer.timeout.connect(self._elapsed_tick)

    def _pick_sound(self, name):
        for n, btn in self.sound_buttons.items():
            btn.setChecked(n == name)
        self.player.play_sound(name)
        self.is_playing = True
        self.play_button.setText("pause")
        self.elapsed_seconds = 0
        self._render_elapsed()
        self.elapsed_timer.start()

    def _toggle_play(self):
        if self.is_playing:
            self.player.pause()
            self.is_playing = False
            self.play_button.setText("play")
            self.elapsed_timer.stop()
        else:
            if self.player.current:
                self.player.resume()
                self.is_playing = True
                self.play_button.setText("pause")
                self.elapsed_timer.start()
            else:
                first = next(iter(noise_gen.SOUNDS))
                self._pick_sound(first)

    def _restart(self):
        if not self.player.current:
            return
        self.player.media.setPosition(0)
        self.player.resume()
        self.is_playing = True
        self.play_button.setText("pause")
        self.elapsed_seconds = 0
        self._render_elapsed()
        self.elapsed_timer.start()

    def _elapsed_tick(self):
        if self.is_playing:
            self.elapsed_seconds += 1
            self._render_elapsed()

    def _render_elapsed(self):
        mins, secs = divmod(self.elapsed_seconds, 60)
        self.elapsed.setText(f"{mins:02d}:{secs:02d}")

    def _timer_changed(self, index):
        minutes = self.timer_box.itemData(index)
        self.sleep_timer.stop()
        self.tick.stop()
        self.countdown.setText("")
        if minutes:
            self.sleep_timer.start(minutes * 60 * 1000)
            self.tick.start()
            self._update_countdown()

    def _update_countdown(self):
        remaining = self.sleep_timer.remainingTime()
        if remaining >= 0:
            mins, secs = divmod(remaining // 1000, 60)
            self.countdown.setText(f"{mins:02d}:{secs:02d}")

    def _sleep_expired(self):
        self.player.pause()
        self.is_playing = False
        self.play_button.setText("play")
        self.elapsed_timer.stop()
        self.timer_box.setCurrentIndex(0)
        self.countdown.setText("")

    def show_near_cursor(self):
        self.move(QCursor.pos().x() - self.width() // 2, QCursor.pos().y() + 12)
        self.show()
        self.raise_()
        self.activateWindow()


class TrayApp:
    def __init__(self):
        self.app = QApplication(sys.argv)
        self.app.setApplicationName("dither")
        self.app.setQuitOnLastWindowClosed(False)
        self.app.setStyleSheet(STYLE)

        self.player = Player()
        self.popup = Popup(self.player)
        self.popup.setWindowIcon(QIcon(os.path.join(ASSETS, "icon.png")))

        self.tray = None
        if QSystemTrayIcon.isSystemTrayAvailable():
            self.tray = QSystemTrayIcon(QIcon(os.path.join(ASSETS, "tray.png")), self.app)
            self.tray.setToolTip("dither")
            menu = QMenu()
            show_action = QAction("show dither", menu)
            show_action.triggered.connect(self.popup.show_near_cursor)
            quit_action = QAction("quit", menu)
            quit_action.triggered.connect(self.app.quit)
            menu.addAction(show_action)
            menu.addAction(quit_action)
            self.tray.setContextMenu(menu)
            self.tray.activated.connect(self._tray_activated)
            self.tray.show()
        else:
            # No tray host (rare desktop setups): run as a normal small window.
            self.popup.show()

    def _tray_activated(self, reason):
        if os.environ.get("DITHER_DEBUG"):
            print(f"tray activated reason={reason} visible={self.popup.isVisible()}", file=sys.stderr)
        if reason == QSystemTrayIcon.ActivationReason.Trigger:
            if self.popup.isVisible():
                self.popup.hide()
            else:
                self.popup.show_near_cursor()

    def run(self):
        return self.app.exec()


def main():
    return TrayApp().run()


if __name__ == "__main__":
    sys.exit(main())
