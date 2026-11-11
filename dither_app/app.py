"""dither - a tiny noise machine for your top bar.

Cross-platform desktop app (Windows, macOS, Linux): a system-tray icon
with a compact popup that plays locally synthesized noise. All audio is
generated on the device by noise_gen; nothing is downloaded.

UI direction follows the user's reference: a rounded card with an inset
equalizer panel (elapsed / Live), sound title + subtitle, one large
filled play button, restart and settings beside it, in light and dark.
"""
import math
import os
import sys

from PySide6.QtCore import (QEasingCurve, QPoint, QPointF, Qt, QTimer, QUrl,
                            QPropertyAnimation, Property)
from PySide6.QtGui import (QAction, QBrush, QColor, QCursor, QFont,
                           QGuiApplication, QIcon, QPainter, QPainterPath,
                           QPen, QPixmap)
from PySide6.QtMultimedia import QAudioOutput, QMediaPlayer
from PySide6.QtWidgets import (QApplication, QComboBox, QHBoxLayout, QLabel,
                               QMenu, QPushButton, QSlider, QSystemTrayIcon,
                               QVBoxLayout, QWidget)

from . import noise_gen

ASSETS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "assets")
CACHE_DIR = os.path.join(os.path.expanduser("~"), ".cache", "dither")

TIMER_CHOICES = [("Off", 0), ("15 min", 15), ("30 min", 30), ("45 min", 45), ("60 min", 60)]
THEME_CHOICES = ["Auto", "Light", "Dark"]

# Semantic color tokens per theme (one accent ramp = the dither amber).
# Text-accent is darkened in light mode so status text reaches 4.5:1.
THEMES = {
    "light": {
        "window": "#F2F2F5", "card": "#FFFFFF", "inset": "#F0F0F3",
        "text": "#151517", "subtext": "#6E6E74",
        "bar": "#D9D9DE", "bar_strong": "#3C3C41",
        "accent": "#C8823E", "accent_text": "#8F5B1E", "on_accent": "#FFFFFF",
        "outline": "#D8D8DD", "edge": "#00000014", "hover": "#E8E8EC",
        "field": "#F0F0F3", "pressed": "#A96E33",
    },
    "dark": {
        "window": "#1B1B1E", "card": "#0C0C0F", "inset": "#232327",
        "text": "#F4F4F6", "subtext": "#9B9BA2",
        "bar": "#3A3A40", "bar_strong": "#D6D6DB",
        "accent": "#C8823E", "accent_text": "#DDA266", "on_accent": "#FFFFFF",
        "outline": "#3A3A40", "edge": "#FFFFFF14", "hover": "#26262B",
        "field": "#232327", "pressed": "#A96E33",
    },
}

SOUND_TITLES = {"brown": "Brown Noise", "white": "White Noise", "pink": "Pink Noise",
                "rain": "Rain", "ocean": "Ocean"}
SOUND_SUBTITLES = {"brown": "deep and warm", "white": "bright and even",
                   "pink": "soft and balanced", "rain": "steady rainfall",
                   "ocean": "slow waves"}
IDLE_SUBTITLE = "press play to focus"


def contrast_ratio(fg, bg):
    """WCAG relative-luminance contrast ratio for two hex colors."""
    def lum(hexcolor):
        c = QColor(hexcolor)
        def ch(v):
            v /= 255.0
            return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4
        return 0.2126 * ch(c.red()) + 0.7152 * ch(c.green()) + 0.0722 * ch(c.blue())
    a, b = lum(fg), lum(bg)
    hi, lo = max(a, b), min(a, b)
    return (hi + 0.05) / (lo + 0.05)


def _pixmap(kind, color, size=64):
    """Paint a line icon. One drawing, recolored per theme/state."""
    pm = QPixmap(size, size)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    pen = QPen(QColor(color), size / 13.0, Qt.SolidLine, Qt.RoundCap, Qt.RoundJoin)
    p.setPen(pen)
    p.setBrush(Qt.NoBrush)
    c = size / 2.0
    if kind == "play":
        path = QPainterPath()
        path.moveTo(c - 7, c - 12)
        path.lineTo(c - 7, c + 12)
        path.lineTo(c + 13, c)  # shifted toward its point (optical centering)
        path.closeSubpath()
        p.setBrush(QColor(color))
        p.setPen(Qt.NoPen)
        p.drawPath(path)
    elif kind == "pause":
        p.setBrush(QColor(color))
        p.setPen(Qt.NoPen)
        p.drawRoundedRect(int(c - 11), int(c - 11), 8, 22, 2, 2)
        p.drawRoundedRect(int(c + 3), int(c - 11), 8, 22, 2, 2)
    elif kind == "restart":
        r = size * 0.30
        p.drawArc(int(c - r), int(c - r), int(2 * r), int(2 * r), -50 * 16, -270 * 16)
        ang = math.radians(-50)
        tip = QPointF(c + r * math.cos(ang), c + r * math.sin(ang))
        path = QPainterPath()
        path.moveTo(tip)
        path.lineTo(tip + QPointF(-2, -10))
        path.lineTo(tip + QPointF(9, -2))
        path.closeSubpath()
        p.setBrush(QColor(color))
        p.drawPath(path)
    elif kind == "gear":
        r1, r2 = size * 0.20, size * 0.31
        for i in range(8):
            a = math.radians(i * 45)
            p.drawLine(QPointF(c + r1 * math.cos(a), c + r1 * math.sin(a)),
                       QPointF(c + r2 * math.cos(a), c + r2 * math.sin(a)))
        p.drawEllipse(QPointF(c, c), size * 0.10, size * 0.10)
    elif kind == "chevron-left":
        path = QPainterPath()
        path.moveTo(c + 6, c - 11)
        path.lineTo(c - 6, c)
        path.lineTo(c + 6, c + 11)
        p.drawPath(path)
    elif kind == "chevron-right":
        path = QPainterPath()
        path.moveTo(c - 6, c - 11)
        path.lineTo(c + 6, c)
        path.lineTo(c - 6, c + 11)
        p.drawPath(path)
    p.end()
    return pm


class EqWidget(QWidget):
    """Inset panel: animated bars, elapsed time, live/sleep status.

    The bar animation is decorative (a smooth random walk, not an audio
    spectrum measurement); it only runs while playing, and never under
    reduce-motion. Elapsed and status text are always static truth.
    """

    BAR_COUNT = 12
    IDLE_HEIGHTS = [0.35, 0.55, 0.8, 0.6, 0.45, 0.9, 1.0, 0.5, 0.7, 0.4, 0.55, 0.3]

    def __init__(self, theme):
        super().__init__()
        self.theme = theme
        self._cache_colors()
        self.setFixedHeight(118)
        self.heights = list(self.IDLE_HEIGHTS)
        self.targets = list(self.IDLE_HEIGHTS)
        self.accent_index = 6
        self.elapsed_text = "0:00"
        self.status_text = ""
        self.animating = False
        self.reduce_motion = False
        self._tick_count = 0
        self.timer = QTimer(self)
        self.timer.setInterval(80)
        self.timer.timeout.connect(self._tick)
        import random as _random
        self._rng = _random.Random(7)

    def _cache_colors(self):
        # Prebuild paint objects: hot-path enum/color churn under CPython 3.10
        # + PySide6 can crash (none_dealloc), so paints reuse these.
        t = self.theme
        self._c_accent = QColor(t["accent"])
        self._c_strong = QColor(t["bar_strong"])
        self._c_bar = QColor(t["bar"])
        self._c_inset = QColor(t["inset"])
        self._c_subtext = QColor(t["subtext"])
        self._c_status = QColor(t["accent_text"])
        self._nopen = QPen(Qt.NoPen)

    def set_playing(self, playing):
        self.animating = playing
        if playing and not self.reduce_motion:
            self.timer.start()
        else:
            self.timer.stop()
            if not playing:
                self.heights = list(self.IDLE_HEIGHTS)
        self.update()

    def set_reduce_motion(self, on):
        self.reduce_motion = on
        self.set_playing(self.animating)

    def _tick(self):
        self._tick_count += 1
        for i in range(self.BAR_COUNT):
            if self._rng.random() < 0.25:
                self.targets[i] = 0.25 + self._rng.random() * 0.75
            self.heights[i] += (self.targets[i] - self.heights[i]) * 0.25
        if self._tick_count % 6 == 0:
            self.accent_index = (self.accent_index + 1) % self.BAR_COUNT
        self.update()

    def paintEvent(self, event):
        t = self.theme
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        rect = self.rect()
        path = QPainterPath()
        path.addRoundedRect(0, 0, rect.width() - 1, rect.height() - 1, 12, 12)
        p.fillPath(path, self._c_inset)
        margin, bottom = 14, 30
        top = 12
        bar_h = rect.height() - margin - bottom - top
        n = self.BAR_COUNT
        slot = (rect.width() - 2 * margin) / n
        bw = slot * 0.55
        p.setPen(self._nopen)
        for i in range(n):
            x = margin + i * slot + (slot - bw) / 2
            h = max(3.0, self.heights[i] * bar_h)
            color = self._c_accent if i == self.accent_index else (
                self._c_strong if self.heights[i] > 0.62 else self._c_bar)
            p.setBrush(color)
            p.drawRoundedRect(int(x), int(top + bar_h - h), int(bw), int(h), bw / 2, bw / 2)
        p.setPen(self._c_subtext)
        f = QFont()
        f.setPixelSize(12)
        f.setStyleHint(QFont.TypeWriter)  # tabular digits for changing values
        p.setFont(f)
        p.drawText(margin, rect.height() - 10, self.elapsed_text)
        if self.status_text:
            p.setPen(self._c_status)
            p.drawText(rect.adjusted(0, 0, -margin, -10), Qt.AlignRight | Qt.AlignBottom,
                       self.status_text)
        p.end()


class PlayButton(QPushButton):
    """The one filled action. Cross-fades play/pause icons on state change."""

    def __init__(self, theme):
        super().__init__()
        self._nopen = QPen(Qt.NoPen)
        self.set_theme(theme)
        self.mode = "play"
        self.setFixedSize(64, 64)
        self.setCursor(Qt.PointingHandCursor)
        self.setFocusPolicy(Qt.StrongFocus)
        self._blend = 1.0  # 1 = current icon fully shown
        self._old_kind = None
        self._anim = QPropertyAnimation(self, b"blend", self)
        self._anim.setDuration(300)
        self._anim.setEasingCurve(QEasingCurve.OutCubic)
        self.reduce_motion = False
        self._update_accessible_name()

    def get_blend(self):
        return self._blend

    def set_blend(self, value):
        self._blend = value
        self.update()

    blend = Property(float, get_blend, set_blend)

    def set_mode(self, mode, animate=True):
        if mode == self.mode:
            return
        self._old_kind = self.mode
        self.mode = mode
        self._update_accessible_name()
        if animate and not self.reduce_motion:
            self._blend = 0.0
            self._anim.stop()
            self._anim.setStartValue(0.0)
            self._anim.setEndValue(1.0)
            self._anim.start()
        else:
            self._blend = 1.0
            self._old_kind = None
        self.update()

    def _update_accessible_name(self):
        name = "Pause playback" if self.mode == "pause" else "Play sound"
        self.setAccessibleName(name)
        self.setToolTip(name)

    def set_theme(self, theme):
        self.theme = theme
        self._c_fill = QColor(theme["accent"])
        self._c_pressed = QColor(theme["pressed"])
        self._c_ring = QPen(QColor(theme["accent_text"]), 2)
        self._icons = {k: _pixmap(k, theme["on_accent"]) for k in ("play", "pause")}
        self.update()

    def paintEvent(self, event):
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        p.setPen(self._nopen)
        p.setBrush(self._c_pressed if self.isDown() else self._c_fill)
        p.drawEllipse(1, 1, 62, 62)
        if self.hasFocus():
            p.setPen(self._c_ring)
            p.setBrush(Qt.NoBrush)
            p.drawEllipse(3, 3, 58, 58)
        if self._old_kind and self._blend < 1.0:
            p.setOpacity(1.0 - self._blend)
            p.drawPixmap(0, 0, self._icons[self._old_kind])
            p.setOpacity(self._blend)
        p.drawPixmap(0, 0, self._icons[self.mode])
        p.end()


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
    """The compact control panel, following the user's reference design."""

    BASE_HEIGHT = 296
    SETTINGS_HEIGHT = 128

    def __init__(self, player):
        super().__init__(None, Qt.FramelessWindowHint | Qt.Tool | Qt.WindowStaysOnTopHint)
        self.setAttribute(Qt.WA_TranslucentBackground)
        self.player = player
        # What the user last asked for; the engine may lag behind it (device
        # stalls, no audio device), so button state tracks this, not the engine.
        self.is_playing = False
        self._settings_open = False
        self.sounds = list(noise_gen.SOUNDS)
        self.selected_index = 0
        self.current_theme = "dark"
        self.theme_mode = "Auto"
        self.reduce_motion = False
        self.setWindowTitle("dither")

        layout = QVBoxLayout(self)
        layout.setContentsMargins(12, 12, 12, 12)
        layout.setSpacing(0)

        self.eq = EqWidget(THEMES[self.current_theme])
        layout.addWidget(self.eq)
        layout.addSpacing(14)

        title_row = QHBoxLayout()
        title_row.setSpacing(4)
        self.prev_button = QPushButton()
        self.prev_button.setFixedSize(32, 32)
        self.prev_button.setAccessibleName("Previous sound")
        self.prev_button.setToolTip("Previous sound")
        self.prev_button.clicked.connect(lambda: self._step_sound(-1))
        title_row.addWidget(self.prev_button)
        title_row.addStretch(1)
        title_col = QVBoxLayout()
        title_col.setSpacing(2)
        self.title = QLabel("Brown Noise")
        self.title.setAlignment(Qt.AlignCenter)
        self.subtitle = QLabel(IDLE_SUBTITLE)
        self.subtitle.setAlignment(Qt.AlignCenter)
        title_col.addWidget(self.title)
        title_col.addWidget(self.subtitle)
        title_row.addLayout(title_col, 0)
        title_row.addStretch(1)
        self.next_button = QPushButton()
        self.next_button.setFixedSize(32, 32)
        self.next_button.setAccessibleName("Next sound")
        self.next_button.setToolTip("Next sound")
        self.next_button.clicked.connect(lambda: self._step_sound(1))
        title_row.addWidget(self.next_button)
        layout.addLayout(title_row)
        layout.addSpacing(14)

        button_row = QHBoxLayout()
        button_row.setSpacing(28)  # borderless controls get extra clearance
        self.restart_button = QPushButton()
        self.restart_button.setFixedSize(40, 40)
        self.restart_button.setAccessibleName("Restart playback")
        self.restart_button.setToolTip("Restart playback")
        self.restart_button.clicked.connect(self._restart)
        self.play_button = PlayButton(THEMES[self.current_theme])
        self.play_button.clicked.connect(self._toggle_play)
        self.settings_button = QPushButton()
        self.settings_button.setFixedSize(40, 40)
        self.settings_button.setCheckable(True)
        self.settings_button.setAccessibleName("Open settings")
        self.settings_button.setToolTip("Settings")
        self.settings_button.toggled.connect(self._settings_toggled)
        button_row.addStretch(1)
        button_row.addWidget(self.restart_button)
        button_row.addWidget(self.play_button)
        button_row.addWidget(self.settings_button)
        button_row.addStretch(1)
        layout.addLayout(button_row)

        # Settings: secondary controls live behind the disclosure.
        self.settings = QWidget()
        s_layout = QVBoxLayout(self.settings)
        s_layout.setContentsMargins(4, 12, 4, 0)
        s_layout.setSpacing(10)
        vol_row = QHBoxLayout()
        vol_label = QLabel("Volume")
        vol_row.addWidget(vol_label)
        self.volume = QSlider(Qt.Horizontal)
        self.volume.setRange(0, 100)
        self.volume.setValue(70)
        self.volume.setAccessibleName("Volume")
        self.volume.valueChanged.connect(lambda v: self.player.set_volume(v / 100.0))
        vol_row.addWidget(self.volume)
        s_layout.addLayout(vol_row)
        timer_row = QHBoxLayout()
        timer_row.addWidget(QLabel("Sleep timer"))
        self.timer_box = QComboBox()
        for label, minutes in TIMER_CHOICES:
            self.timer_box.addItem(label, minutes)
        self.timer_box.currentIndexChanged.connect(self._timer_changed)
        timer_row.addWidget(self.timer_box, 1)
        s_layout.addLayout(timer_row)
        look_row = QHBoxLayout()
        look_row.addWidget(QLabel("Appearance"))
        self.theme_box = QComboBox()
        for label in THEME_CHOICES:
            self.theme_box.addItem(label)
        self.theme_box.currentIndexChanged.connect(self._theme_changed)
        look_row.addWidget(self.theme_box, 1)
        s_layout.addLayout(look_row)
        motion_row = QHBoxLayout()
        motion_row.addWidget(QLabel("Reduce motion"))
        self.motion_box = QComboBox()
        self.motion_box.addItem("Off")
        self.motion_box.addItem("On")
        self.motion_box.currentIndexChanged.connect(self._motion_changed)
        motion_row.addWidget(self.motion_box, 1)
        s_layout.addLayout(motion_row)
        self.settings.setVisible(False)
        layout.addWidget(self.settings)

        # Keyboard order: sound steppers, main controls, then settings fields.
        QWidget.setTabOrder(self.prev_button, self.next_button)
        QWidget.setTabOrder(self.next_button, self.restart_button)
        QWidget.setTabOrder(self.restart_button, self.play_button)
        QWidget.setTabOrder(self.play_button, self.settings_button)
        QWidget.setTabOrder(self.settings_button, self.volume)
        QWidget.setTabOrder(self.volume, self.timer_box)
        QWidget.setTabOrder(self.timer_box, self.theme_box)
        QWidget.setTabOrder(self.theme_box, self.motion_box)

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

        self.apply_theme(self._system_theme())
        self._set_fixed_height()

    # ----- theme -----

    def _system_theme(self):
        scheme = QGuiApplication.styleHints().colorScheme()
        return "dark" if scheme == Qt.ColorScheme.Dark else "light"

    def _resolved_theme(self):
        return self._system_theme() if self.theme_mode == "Auto" else self.theme_mode.lower()

    def _theme_changed(self):
        self.theme_mode = self.theme_box.currentText()
        self.apply_theme(self._resolved_theme())

    def system_color_scheme_changed(self):
        if self.theme_mode == "Auto":
            self.apply_theme(self._system_theme())

    def apply_theme(self, name):
        self.current_theme = name
        t = THEMES[name]
        self.theme = t
        self.eq.theme = t
        self.eq._cache_colors()
        self.play_button.set_theme(t)
        self.title.setStyleSheet(
            f"color: {t['text']}; font-size: 16px; font-weight: 600; background: transparent;")
        self.subtitle.setStyleSheet(
            f"color: {t['subtext']}; font-size: 13px; background: transparent;")
        chevron = (f"QPushButton {{ color: {t['subtext']}; background: transparent;"
                   f" border: none; border-radius: 16px; }}"
                   f"QPushButton:hover {{ background: {t['hover']}; }}"
                   f"QPushButton:focus {{ border: 2px solid {t['accent_text']}; }}")
        self.prev_button.setStyleSheet(chevron)
        self.next_button.setStyleSheet(chevron)
        self.prev_button.setIcon(QIcon(_pixmap("chevron-left", t["subtext"], 32)))
        self.next_button.setIcon(QIcon(_pixmap("chevron-right", t["subtext"], 32)))
        circle = (f"QPushButton {{ background: transparent; border: 1.5px solid {t['outline']};"
                  f" border-radius: 20px; }}"
                  f"QPushButton:hover {{ background: {t['hover']}; }}"
                  f"QPushButton:focus {{ border: 2px solid {t['accent_text']}; }}"
                  f"QPushButton:checked {{ border-color: {t['accent']}; }}")
        self.restart_button.setStyleSheet(circle)
        self.settings_button.setStyleSheet(circle)
        self.restart_button.setIcon(QIcon(_pixmap("restart", t["text"], 40)))
        self.settings_button.setIcon(QIcon(_pixmap("gear", t["text"], 40)))
        self.settings.setStyleSheet(
            f"QWidget {{ background: transparent; color: {t['text']}; font-size: 13px; }}"
            f"QLabel {{ background: transparent; }}"
            f"QSlider::groove:horizontal {{ height: 4px; background: {t['outline']}; border-radius: 2px; }}"
            f"QSlider::handle:horizontal {{ width: 14px; height: 14px; margin: -5px 0;"
            f" border-radius: 7px; background: {t['accent']}; }}"
            f"QSlider::sub-page:horizontal {{ background: {t['accent']}; border-radius: 2px; }}"
            f"QComboBox {{ background: {t['field']}; color: {t['text']}; border: none;"
            f" border-radius: 6px; padding: 5px 8px; }}"
            f"QComboBox QAbstractItemView {{ background: {t['field']}; color: {t['text']};"
            f" border: 1px solid {t['outline']}; }}"
            f"QComboBox:focus {{ border: 2px solid {t['accent_text']}; border-radius: 6px; }}")
        self.eq.update()
        self.play_button.update()
        self.update()

    def paintEvent(self, event):
        t = self.theme
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        path = QPainterPath()
        # Outer radius (24) = inset radius (12) + padding (12).
        path.addRoundedRect(0, 0, self.width() - 1, self.height() - 1, 24, 24)
        p.fillPath(path, QColor(t["card"]))
        p.setPen(QPen(QColor(t["edge"]), 1))  # structural edge, black/white at 8%
        p.drawPath(path)
        p.end()

    # ----- sound selection -----

    @property
    def selected_sound(self):
        return self.sounds[self.selected_index]

    def _step_sound(self, delta):
        self.selected_index = (self.selected_index + delta) % len(self.sounds)
        self.title.setText(SOUND_TITLES[self.selected_sound])
        if self.is_playing:
            # Switching mid-play changes the sound seamlessly.
            self.player.play_sound(self.selected_sound)
            self.elapsed_seconds = 0
            self._render_elapsed()
        else:
            self.subtitle.setText(SOUND_SUBTITLES[self.selected_sound])

    def _pick_sound(self, name):
        self.selected_index = self.sounds.index(name)
        self.title.setText(SOUND_TITLES[name])
        self.player.play_sound(name)
        self._entered_playing()

    # ----- playback -----

    def _entered_playing(self):
        self.is_playing = True
        self.play_button.set_mode("pause")
        self.subtitle.setText(SOUND_SUBTITLES[self.selected_sound])
        self.eq.set_playing(True)
        self._render_status()
        self.elapsed_seconds = 0
        self._render_elapsed()
        self.elapsed_timer.start()

    def _entered_stopped(self):
        self.is_playing = False
        self.play_button.set_mode("play")
        self.eq.set_playing(False)
        self._render_status()
        self.elapsed_timer.stop()

    def _toggle_play(self):
        if self.is_playing:
            self.player.pause()
            self._entered_stopped()
        else:
            if self.player.current and self.player.current == self.selected_sound:
                self.player.resume()
                self.is_playing = True
                self.play_button.set_mode("pause")
                self.eq.set_playing(True)
                self._render_status()
                self.elapsed_timer.start()
            else:
                self._pick_sound(self.selected_sound)

    def _restart(self):
        if not self.player.current:
            return
        self.player.media.setPosition(0)
        self.player.resume()
        self._entered_playing()

    def _elapsed_tick(self):
        if self.is_playing:
            self.elapsed_seconds += 1
            self._render_elapsed()

    def _render_elapsed(self):
        mins, secs = divmod(self.elapsed_seconds, 60)
        self.eq.elapsed_text = f"{mins}:{secs:02d}"
        self.eq.update()

    def _render_status(self):
        if self.sleep_timer.isActive():
            self._update_countdown()
        else:
            self.eq.status_text = "Live" if self.is_playing else ""
            self.eq.update()

    # ----- sleep timer -----

    def _timer_changed(self, index):
        minutes = self.timer_box.itemData(index)
        self.sleep_timer.stop()
        self.tick.stop()
        if minutes:
            self.sleep_timer.start(minutes * 60 * 1000)
            self.tick.start()
            self._update_countdown()
        else:
            self._render_status()

    def _update_countdown(self):
        remaining = self.sleep_timer.remainingTime()
        if remaining >= 0:
            mins, secs = divmod(remaining // 1000, 60)
            self.eq.status_text = f"{mins}:{secs:02d}"
            self.eq.update()

    def _sleep_expired(self):
        self.player.pause()
        self._entered_stopped()
        self.timer_box.setCurrentIndex(0)
        self._render_status()

    # ----- settings -----

    def _settings_toggled(self, on):
        self._settings_open = on
        self.settings.setVisible(on)
        self.settings_button.setAccessibleName("Close settings" if on else "Open settings")
        self._set_fixed_height()

    def _motion_changed(self):
        self.reduce_motion = self.motion_box.currentText() == "On"
        self.eq.set_reduce_motion(self.reduce_motion)
        self.play_button.reduce_motion = self.reduce_motion

    def _set_fixed_height(self):
        h = self.BASE_HEIGHT + (self.SETTINGS_HEIGHT if self._settings_open else 0)
        self.setFixedSize(300, h)

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

        self.player = Player()
        self.popup = Popup(self.player)
        self.popup.setWindowIcon(QIcon(os.path.join(ASSETS, "icon.png")))
        QGuiApplication.styleHints().colorSchemeChanged.connect(
            lambda _: self.popup.system_color_scheme_changed())

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
