"""Zenhush - a tiny noise machine for your top bar.

Cross-platform desktop app (Windows, macOS, Linux): a system-tray icon
with a compact popup that plays locally synthesized noise. All audio is
generated on the device by noise_gen; nothing is downloaded.

UI direction follows the user's reference: a rounded card with an inset
equalizer panel (elapsed / playback status), sound title + subtitle, one large
filled play button, restart and settings beside it, in light and dark.
"""
import math
import os
import sys

from PySide6.QtCore import (QEasingCurve, QPoint, QPointF, QRectF, QSize, Qt,
                            QTimer, QPropertyAnimation, Property, Signal)
from PySide6.QtGui import (QAction, QBrush, QColor, QCursor, QFont,
                           QFontDatabase, QGuiApplication, QIcon, QPainter,
                           QPainterPath, QPen, QPixmap)
from PySide6.QtMultimedia import QAudio, QAudioFormat, QAudioSink
from PySide6.QtSvg import QSvgRenderer
from PySide6.QtWidgets import (QApplication, QFileDialog, QHBoxLayout, QLabel,
                               QMenu, QMessageBox, QPushButton, QSlider,
                               QSizePolicy, QSystemTrayIcon, QVBoxLayout,
                               QWidget)

from . import audiofile, noise_gen, synth

ASSETS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "assets")
CACHE_DIR = os.path.join(os.path.expanduser("~"), ".cache", "zenhush")

TIMER_CHOICES = [("Off", 0), ("15 min", 15), ("30 min", 30), ("45 min", 45), ("60 min", 60)]

_FONT_FAMILY = None


def app_font_family():
    """Load the bundled Schibsted Grotesk (SIL OFL 1.1) once; return its family.

    Falls back to the Qt default family only if the font file is missing, so a
    broken install degrades instead of crashing."""
    global _FONT_FAMILY
    if _FONT_FAMILY is None:
        path = os.path.join(ASSETS, "fonts", "SchibstedGrotesk.ttf")
        fid = QFontDatabase.addApplicationFont(path)
        fams = QFontDatabase.applicationFontFamilies(fid) if fid != -1 else []
        _FONT_FAMILY = fams[0] if fams else ""
    return _FONT_FAMILY


def _font_css():
    """font-family declaration for stylesheets that set font-size/weight.

    A Qt stylesheet with any font property re-resolves the widget font from
    its parent chain; without an explicit family the bundled font can be
    replaced by the system fallback (observed: DejaVu Sans after a theme
    switch). Pin the family wherever the stylesheet touches fonts."""
    fam = app_font_family()
    return f'font-family: "{fam}"; ' if fam else ""
THEME_CHOICES = ["Auto", "Light", "Dark"]

# Semantic color tokens per theme. Warm black + amber, the palette the user
# picked from the original build (sampled: #0B0A08, #C8823E, #3A332B, #F5F1EA);
# the reference blue remains in git history as an internal alternate.
# Text-accent is darkened in light mode so status text reaches 4.5:1.
THEMES = {
    "light": {
        "window": "#F3EFE8", "card": "#FDFBF7", "inset": "#F0EBE2",
        "text": "#1C1915", "subtext": "#6F6659",
        "bar": "#DDD5C7", "bar_strong": "#3B352C",
        "accent": "#C8823E", "accent_text": "#8F5B1E", "on_accent": "#FFFFFF",
        "outline": "#DCD4C6", "edge": (0, 0, 0, 18), "hover": "#EFE9DF",
        "field": "#F0EBE2", "pressed": "#A96E33", "switch_off": "#D9D0C0",
    },
    "dark": {
        "window": "#100E0C", "card": "#0B0A08", "inset": "#221E19",
        "text": "#F5F1EA", "subtext": "#A89F92",
        "bar": "#3A332B", "bar_strong": "#E8E1D4",
        "accent": "#C8823E", "accent_text": "#DDA266", "on_accent": "#FFFFFF",
        "outline": "#3A332B", "edge": (255, 255, 255, 16), "hover": "#1D1A15",
        "field": "#221E19", "pressed": "#A96E33", "switch_off": "#4A4238",
    },
}

SOUND_TITLES = {"brown": "Brown Noise", "white": "White Noise", "pink": "Pink Noise",
                "rain": "Rain", "ocean": "Ocean", "file": "Your File"}
SOUND_SUBTITLES = {"brown": "deep and warm", "white": "bright and even",
                   "pink": "soft and balanced", "rain": "steady rainfall",
                   "ocean": "slow waves"}
FILE_SUBTITLE_IDLE = "loop a local audio file"
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


_ICON_FILES = {
    "play": "play.svg", "pause": "pause.svg", "restart": "rotate-ccw.svg",
    "gear": "settings.svg", "chevron-left": "chevron-left.svg",
    "chevron-right": "chevron-right.svg",
}
_FILLED_KINDS = {"play", "pause"}  # solid glyphs on the accent button


def _icon_svg_bytes(kind, color):
    """Lucide icon (ISC license), recolored: its strokes use currentColor."""
    path = os.path.join(ASSETS, "icons", _ICON_FILES[kind])
    data = open(path, "rb").read()
    hexcol = QColor(color).name().encode()
    data = data.replace(b"currentColor", hexcol)
    if kind in _FILLED_KINDS:
        # The reference shows a solid play triangle / pause bars: fill the
        # library's path geometry, keeping its stroke in the same color so
        # the rounded joins and caps survive.
        for tag in (b"<polygon", b"<rect", b"<path"):
            data = data.replace(tag, tag + b' fill="%s"' % hexcol)
    return data


def _pixmap(kind, color, size=64, icon_ratio=0.42):
    """Render one Lucide icon centered on a transparent square pixmap."""
    pm = QPixmap(size, size)
    pm.fill(Qt.transparent)
    p = QPainter(pm)
    p.setRenderHint(QPainter.Antialiasing)
    icon_px = size * icon_ratio
    x = (size - icon_px) / 2.0
    QSvgRenderer(_icon_svg_bytes(kind, color)).render(
        p, QRectF(x, x, icon_px, icon_px))
    p.end()
    return pm



class Segmented(QWidget):
    """Pill segmented control with a sliding thumb (timer/appearance rows).

    QComboBox-compatible surface: addItem/itemText/itemData/currentText/
    currentIndex/setCurrentIndex/count + currentIndexChanged, so callers and
    tests read the same either way. Keyboard: arrows/Home/End; ring on focus.
    """

    currentIndexChanged = Signal(int)

    def __init__(self, theme, accessible_name):
        super().__init__()
        self.theme = theme
        self._items = []        # (label, data, display)
        self._index = -1
        self._pos = 0.0         # thumb position in segment units (animated)
        self._hover = -1
        self.reduce_motion = False
        self._anim = QPropertyAnimation(self, b"thumb_pos", self)
        self._anim.setDuration(160)
        self._anim.setEasingCurve(QEasingCurve.OutCubic)
        self.setAccessibleName(accessible_name)
        self.setFocusPolicy(Qt.StrongFocus)
        self.setFixedHeight(28)
        self.setSizePolicy(QSizePolicy.Expanding, QSizePolicy.Fixed)
        self.setCursor(Qt.PointingHandCursor)
        self.setAttribute(Qt.WA_Hover)

    # combo-compatible API
    def addItem(self, label, data=None, short=None):
        self._items.append((label, data, short or label))
        if self._index == -1:
            self.setCurrentIndex(0)

    def count(self):
        return len(self._items)

    def itemText(self, i):
        return self._items[i][0]

    def itemData(self, i):
        return self._items[i][1]

    def currentText(self):
        return self._items[self._index][0] if 0 <= self._index < len(self._items) else ""

    def currentIndex(self):
        return self._index

    def setCurrentIndex(self, i):
        if i == self._index or not (0 <= i < len(self._items)):
            return
        old = self._index
        self._index = i
        self._anim.stop()
        if self.reduce_motion or old == -1:
            self._pos = float(i)
            self.update()
        else:
            self._anim.setStartValue(self._pos)
            self._anim.setEndValue(float(i))
            self._anim.start()
        self.currentIndexChanged.emit(i)

    def get_thumb(self):
        return self._pos

    def set_thumb(self, v):
        self._pos = v
        self.update()

    thumb_pos = Property(float, get_thumb, set_thumb)

    def set_theme(self, theme):
        self.theme = theme
        self.update()

    # events
    def _seg_at(self, x):
        return min(max(int(x // (self.width() / max(1, len(self._items)))), 0),
                   len(self._items) - 1)

    def mousePressEvent(self, e):
        self.setFocus()
        self.setCurrentIndex(self._seg_at(e.position().x()))

    def keyPressEvent(self, e):
        if e.key() in (Qt.Key_Left, Qt.Key_Down):
            self.setCurrentIndex(max(0, self._index - 1))
        elif e.key() in (Qt.Key_Right, Qt.Key_Up):
            self.setCurrentIndex(min(len(self._items) - 1, self._index + 1))
        elif e.key() == Qt.Key_Home:
            self.setCurrentIndex(0)
        elif e.key() == Qt.Key_End:
            self.setCurrentIndex(len(self._items) - 1)
        else:
            super().keyPressEvent(e)

    def hoverMoveEvent(self, e):
        i = self._seg_at(e.position().x())
        if i != self._hover:
            self._hover = i
            self.update()

    def leaveEvent(self, e):
        self._hover = -1
        self.update()

    def event(self, e):
        if e.type() == e.Type.HoverMove:
            self.hoverMoveEvent(e)
        return super().event(e)

    def paintEvent(self, e):
        t = self.theme
        n = max(1, len(self._items))
        w, h = self.width(), self.height()
        seg = w / n
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        p.fillRect(self.rect(), Qt.transparent)
        # track
        p.setPen(Qt.NoPen)
        p.setBrush(QColor(t["field"]))
        p.drawRoundedRect(0, 0, w, h, h / 2, h / 2)
        # thumb: solid accent, one active language across the whole app
        inset = 3
        tx = self._pos * seg + inset
        p.setBrush(QColor(t["accent"]))
        p.setPen(Qt.NoPen)
        p.drawRoundedRect(QRectF(tx, inset, seg - 2 * inset, h - 2 * inset),
                          (h - 2 * inset) / 2, (h - 2 * inset) / 2)
        # labels
        f = QFont(self.font())
        f.setPixelSize(12)
        p.setFont(f)
        for i, (_, _, disp) in enumerate(self._items):
            if i == self._index:
                p.setPen(QColor(t["on_accent"]))
            elif i == self._hover:
                p.setPen(QColor(t["text"]))
            else:
                p.setPen(QColor(t["subtext"]))
            p.drawText(QRectF(i * seg, 0, seg, h), Qt.AlignCenter, disp)
        # focus ring
        if self.hasFocus():
            p.setBrush(Qt.NoBrush)
            p.setPen(QPen(QColor(t["accent_text"]), 2))
            p.drawRoundedRect(1, 1, w - 2, h - 2, h / 2, h / 2)
        p.end()


class Toggle(QWidget):
    """Slide switch (reduce motion row). Combo-compatible: Off=0, On=1."""

    currentIndexChanged = Signal(int)

    def __init__(self, theme, accessible_name):
        super().__init__()
        self.theme = theme
        self._labels = []
        self._index = 0
        self._pos = 0.0
        self.reduce_motion = False
        self._anim = QPropertyAnimation(self, b"knob_pos", self)
        self._anim.setDuration(140)
        self._anim.setEasingCurve(QEasingCurve.OutCubic)
        self.setAccessibleName(accessible_name)
        self.setFocusPolicy(Qt.StrongFocus)
        self.setFixedSize(44, 26)
        self.setCursor(Qt.PointingHandCursor)

    def addItem(self, label):
        self._labels.append(label)

    def currentText(self):
        return self._labels[self._index] if self._labels else ""

    def currentIndex(self):
        return self._index

    def setCurrentIndex(self, i):
        if i == self._index:
            return
        self._index = i
        self._anim.stop()
        if self.reduce_motion:
            self._pos = float(i)
            self.update()
        else:
            self._anim.setStartValue(self._pos)
            self._anim.setEndValue(float(i))
            self._anim.start()
        self.currentIndexChanged.emit(i)

    def get_knob(self):
        return self._pos

    def set_knob(self, v):
        self._pos = v
        self.update()

    knob_pos = Property(float, get_knob, set_knob)

    def set_theme(self, theme):
        self.theme = theme
        self.update()

    def mousePressEvent(self, e):
        self.setFocus()
        self.setCurrentIndex(1 - self._index)

    def keyPressEvent(self, e):
        if e.key() in (Qt.Key_Space, Qt.Key_Return, Qt.Key_Enter):
            self.setCurrentIndex(1 - self._index)
        elif e.key() == Qt.Key_Left:
            self.setCurrentIndex(0)
        elif e.key() == Qt.Key_Right:
            self.setCurrentIndex(1)
        else:
            super().keyPressEvent(e)

    def paintEvent(self, e):
        t = self.theme
        w, h = self.width(), self.height()
        on = self._pos
        p = QPainter(self)
        p.setRenderHint(QPainter.Antialiasing)
        # track: visible off-state -> accent blend
        off_c, on_c = QColor(t["switch_off"]), QColor(t["accent"])
        track = QColor(
            int(off_c.red() + (on_c.red() - off_c.red()) * on),
            int(off_c.green() + (on_c.green() - off_c.green()) * on),
            int(off_c.blue() + (on_c.blue() - off_c.blue()) * on))
        p.setPen(Qt.NoPen)
        p.setBrush(track)
        p.drawRoundedRect(2, 2, w - 4, h - 4, (h - 4) / 2, (h - 4) / 2)
        # knob
        knob_d = h - 10
        travel = (w - 4) - knob_d - 4
        kx = 2 + 2 + self._pos * travel
        p.setBrush(QColor("#FFFFFF"))
        p.drawEllipse(QRectF(kx, 5, knob_d, knob_d))
        # focus ring
        if self.hasFocus():
            p.setBrush(Qt.NoBrush)
            p.setPen(QPen(QColor(t["accent_text"]), 2))
            p.drawRoundedRect(0, 0, w - 1, h - 1, h / 2, h / 2)
        p.end()


class EqWidget(QWidget):
    """Inset panel: live bars, elapsed time, playback/sleep status.

    The bars are a real time-domain energy envelope of the sound being
    played, streamed straight from the synthesis engine's output energy
    (synth.LiveSource.rms_values). It is not a spectrum analysis and it is
    not decorative: paused freezes the actual recent energy. Under
    reduce-motion a single static frame is drawn instead of animating.
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
        self.source = None  # callable -> recent real output RMS values
        self.timer = QTimer(self)
        self.timer.setInterval(80)
        self.timer.timeout.connect(self._tick)

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
        self.timer.stop()
        if playing and not self.reduce_motion:
            self.timer.start()
        elif playing:
            self._tick()  # reduce motion: one static real frame
        # paused: keep the current bars - the frozen energy at the pause point
        self.update()

    def set_reduce_motion(self, on):
        self.reduce_motion = on
        self.set_playing(self.animating)

    def _tick(self):
        if self.source:
            vals = list(self.source())
            if vals:
                take = vals[-self.BAR_COUNT:]
                window = [take[0]] * (self.BAR_COUNT - len(take)) + take
                loudest, loudest_v = self.accent_index, -1.0
                for i, v in enumerate(window):
                    self.targets[i] = v
                    self.heights[i] += (v - self.heights[i]) * 0.45
                    if self.heights[i] > loudest_v:
                        loudest, loudest_v = i, self.heights[i]
                self.accent_index = loudest  # accent marks the loudest bar
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
        f = QFont(app_font_family() or self.font().family())
        f.setPixelSize(12)
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
        self._icons = {k: _pixmap(k, theme["on_accent"], 64, 26 / 64) for k in ("play", "pause")}
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
    """Plays the selected source: live synthesis, or the user's looped file.

    Noise mode: LiveSource generates fresh filtered random noise forever
    and QAudioSink pulls it in real time, so the sound is made right now,
    never ends and never repeats. File mode: FileSource loops the user's
    own decoded file through the same chain (the UI labels it "File",
    never "Live"). Volume, Depth and Rumble retune the active stream live
    while it plays.
    """

    def __init__(self):
        fmt = QAudioFormat()
        fmt.setSampleRate(synth.SAMPLE_RATE)
        fmt.setChannelCount(synth.CHANNELS)
        fmt.setSampleFormat(QAudioFormat.Int16)
        self.live = synth.LiveSource()
        self.file = None
        self.source = self.live
        self.sink = QAudioSink(fmt)
        self._started = False
        self.current = None
        self.current_kind = None  # "noise" or "file"

    def _drive(self, source):
        """Point the sink at source (starting it if needed) and resume."""
        if self.source is not source:
            if self._started:
                self.sink.stop()
            self.source = source
            self._started = False
        if not self._started:
            self.sink.start(self.source)
            self._started = True
        self.sink.resume()

    def play_sound(self, name):
        self.live.set_sound(name)
        self.current = name
        self.current_kind = "noise"
        self._drive(self.live)

    def play_file(self, samples):
        """Loop the user's decoded file (array('f') from audiofile.decode)."""
        self.file = synth.FileSource(samples)
        self.current = "file"
        self.current_kind = "file"
        self._drive(self.file)

    def pause(self):
        self.sink.suspend()

    def resume(self):
        if self.current:
            self.source.soft_start()
            self.sink.resume()

    def restart(self):
        """Noise: fresh stream from a new seed, never a replay.
        File: rewind the user's file to the start."""
        self.source.restart()
        self.sink.resume()

    def set_volume(self, value):
        """Slider 0-100, preview-fitted curve: 70 -> gain 0.44.

        Applied inside the float chain, before the output stage, so the
        order matches the preview (filters -> volume -> DAC)."""
        self.source.set_volume(value)

    def set_depth(self, value):
        """Slider 0-100: lowpass cutoff, higher is darker."""
        self.source.set_depth(value)

    def set_rumble(self, value):
        """Slider 0-100: sub-bass lowshelf boost."""
        self.source.set_rumble(value)

    @property
    def playing(self):
        return self.sink.state() == QAudio.State.ActiveState


class Popup(QWidget):
    """The compact control panel, following the user's reference design."""

    BASE_HEIGHT = 296
    SETTINGS_HEIGHT = 336

    def __init__(self, player):
        super().__init__(None, Qt.FramelessWindowHint | Qt.Tool | Qt.WindowStaysOnTopHint)
        self.setAttribute(Qt.WA_TranslucentBackground)
        fam = app_font_family()
        if fam:
            self.setFont(QFont(fam))
        self.player = player
        # What the user last asked for; the engine may lag behind it (device
        # stalls, no audio device), so button state tracks this, not the engine.
        self.is_playing = False
        self._settings_open = False
        self.sounds = list(noise_gen.SOUNDS) + ["file"]
        self.selected_index = 0
        self.file_path = None      # user's chosen loop file (this session)
        self.file_samples = None
        self.current_theme = "dark"
        self.theme_mode = "Auto"
        self.reduce_motion = False
        self.setWindowTitle("Zenhush")

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

        # Settings: secondary controls live behind the disclosure, grouped
        # into Sound / Sleep timer / Appearance sections.
        self.settings = QWidget()
        s_layout = QVBoxLayout(self.settings)
        s_layout.setContentsMargins(4, 12, 4, 0)
        s_layout.setSpacing(8)

        def _header(text):
            lab = QLabel(text)
            lab.setObjectName("section")
            return lab

        def _text_button(text, tip):
            b = QPushButton(text)
            b.setObjectName("textbtn")
            b.setCursor(Qt.PointingHandCursor)
            b.setToolTip(tip)
            return b

        def _slider_row(label, value, tip, changed):
            row = QHBoxLayout()
            row.setSpacing(8)
            lab = QLabel(label)
            lab.setFixedWidth(52)
            row.addWidget(lab)
            slider = QSlider(Qt.Horizontal)
            slider.setRange(0, 100)
            slider.setValue(value)
            slider.setAccessibleName(label)
            slider.setToolTip(tip)
            val = QLabel(str(value))
            val.setObjectName("svalue")
            val.setFixedWidth(24)
            val.setAlignment(Qt.AlignRight | Qt.AlignVCenter)
            slider.valueChanged.connect(lambda v, val=val: val.setText(str(v)))
            slider.valueChanged.connect(changed)
            row.addWidget(slider, 1)
            row.addWidget(val)
            s_layout.addLayout(row)
            return slider

        # --- Sound ---
        sound_head = QHBoxLayout()
        sound_head.addWidget(_header("SOUND"))
        sound_head.addStretch(1)
        self.reset_button = _text_button("Reset", "Back to Volume 70, Depth 65, Rumble 58")
        self.reset_button.setAccessibleName("Reset volume, depth and rumble to defaults")
        self.reset_button.clicked.connect(self._reset_sound_controls)
        sound_head.addWidget(self.reset_button)
        s_layout.addLayout(sound_head)

        self.volume = _slider_row("Volume", 70, "", self.player.set_volume)
        self.depth = _slider_row("Depth", synth.DEFAULT_DEPTH,
                                 "How dark the sound is: higher cuts more highs",
                                 self.player.set_depth)
        self.rumble = _slider_row("Rumble", synth.DEFAULT_RUMBLE,
                                  "Sub-bass boost", self.player.set_rumble)

        file_row = QHBoxLayout()
        file_row.setSpacing(8)
        file_lab = QLabel("File")
        file_lab.setFixedWidth(52)
        file_row.addWidget(file_lab)
        self.file_label = QLabel("None")
        self.file_label.setObjectName("svalue")
        self.file_label.setToolTip("No file chosen")
        file_row.addWidget(self.file_label, 1)
        self.file_button = _text_button(
            "Choose\u2026", "Loop a local audio file (wav, mp3, flac, ogg, m4a)")
        self.file_button.setAccessibleName("Choose an audio file to loop")
        self.file_button.clicked.connect(self._choose_and_play_file)
        file_row.addWidget(self.file_button)
        s_layout.addLayout(file_row)

        # --- Sleep timer ---
        s_layout.addSpacing(6)
        s_layout.addWidget(_header("SLEEP TIMER (MIN)"))
        self.timer_box = Segmented(THEMES[self._resolved_theme()], "Sleep timer")
        for label, minutes in TIMER_CHOICES:
            short = "Off" if minutes == 0 else str(minutes)
            self.timer_box.addItem(label, minutes, short)
            self.timer_box.setToolTip("Sleep timer: " + ", ".join(l for l, _ in TIMER_CHOICES))
        self.timer_box.currentIndexChanged.connect(self._timer_changed)
        s_layout.addWidget(self.timer_box)

        # --- Appearance ---
        s_layout.addSpacing(6)
        s_layout.addWidget(_header("APPEARANCE"))
        self.theme_box = Segmented(THEMES[self._resolved_theme()], "Appearance")
        for label in THEME_CHOICES:
            self.theme_box.addItem(label)
        self.theme_box.currentIndexChanged.connect(self._theme_changed)
        s_layout.addWidget(self.theme_box)

        motion_row = QHBoxLayout()
        motion_row.addWidget(QLabel("Reduce motion"))
        motion_row.addStretch(1)
        self.motion_box = Toggle(THEMES[self._resolved_theme()], "Reduce motion")
        self.motion_box.addItem("Off")
        self.motion_box.addItem("On")
        self.motion_box.currentIndexChanged.connect(self._motion_changed)
        motion_row.addWidget(self.motion_box)
        s_layout.addLayout(motion_row)
        self.settings.setVisible(False)
        layout.addWidget(self.settings)

        # Keyboard order: sound steppers, main controls, then settings fields.
        QWidget.setTabOrder(self.prev_button, self.next_button)
        QWidget.setTabOrder(self.next_button, self.restart_button)
        QWidget.setTabOrder(self.restart_button, self.play_button)
        QWidget.setTabOrder(self.play_button, self.settings_button)
        QWidget.setTabOrder(self.settings_button, self.volume)
        QWidget.setTabOrder(self.volume, self.depth)
        QWidget.setTabOrder(self.depth, self.rumble)
        QWidget.setTabOrder(self.rumble, self.file_button)
        QWidget.setTabOrder(self.file_button, self.reset_button)
        QWidget.setTabOrder(self.reset_button, self.timer_box)
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

        self.eq.source = self.player.source.rms_values  # real live energy
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
            f"{_font_css()}color: {t['text']}; font-size: 16px; font-weight: 700; background: transparent;")
        self.subtitle.setStyleSheet(
            f"{_font_css()}color: {t['subtext']}; font-size: 13px; background: transparent;")
        chevron = (f"QPushButton {{ color: {t['subtext']}; background: transparent;"
                   f" border: none; border-radius: 16px; }}"
                   f"QPushButton:hover {{ background: {t['hover']}; }}"
                   f"QPushButton:pressed {{ background: {t['field']}; }}"
                   f"QPushButton:focus {{ border: 2px solid {t['accent_text']}; }}")
        self.prev_button.setStyleSheet(chevron)
        self.next_button.setStyleSheet(chevron)
        self.prev_button.setIcon(QIcon(_pixmap("chevron-left", t["subtext"], 32, 18 / 32)))
        self.next_button.setIcon(QIcon(_pixmap("chevron-right", t["subtext"], 32, 18 / 32)))
        self.prev_button.setIconSize(QSize(32, 32))
        self.next_button.setIconSize(QSize(32, 32))
        # Bare icon buttons: play is the only filled circle (the hero),
        # restart/settings are quiet glyphs, like the reference.
        bare = (f"QPushButton {{ background: transparent; border: none;"
                f" border-radius: 20px; }}"
                f"QPushButton:hover {{ background: {t['hover']}; }}"
                f"QPushButton:pressed {{ background: {t['field']}; }}"
                f"QPushButton:focus {{ border: 2px solid {t['accent_text']}; }}")
        self.restart_button.setStyleSheet(bare)
        self.settings_button.setStyleSheet(bare)
        gear_color = t["accent"] if self.settings_button.isChecked() else t["subtext"]
        self.restart_button.setIcon(QIcon(_pixmap("restart", t["subtext"], 40, 20 / 40)))
        self.settings_button.setIcon(QIcon(_pixmap("gear", gear_color, 40, 20 / 40)))
        self.restart_button.setIconSize(QSize(40, 40))
        self.settings_button.setIconSize(QSize(40, 40))
        self.settings.setStyleSheet(
            f"QWidget {{ {_font_css()}background: transparent; color: {t['text']}; font-size: 13px; }}"
            f"QLabel {{ background: transparent; }}"
            f"QLabel#section {{ color: {t['subtext']}; font-size: 11px; font-weight: 600; }}"
            f"QLabel#svalue {{ color: {t['subtext']}; }}"
            f"QPushButton#textbtn {{ color: {t['accent_text']}; background: transparent;"
            f" border: none; font-size: 12px; padding: 2px 4px; }}"
            f"QPushButton#textbtn:hover {{ color: {t['accent']}; }}"
            f"QPushButton#textbtn:focus {{ border: 1px solid {t['accent_text']}; border-radius: 4px; }}"
            f"QSlider::groove:horizontal {{ height: 4px; background: {t['outline']}; border-radius: 2px; }}"
            f"QSlider::handle:horizontal {{ width: 16px; height: 16px; margin: -6px 0;"
            f" border-radius: 8px; background: {t['accent']}; }}"
            f"QSlider::handle:horizontal:hover {{ background: {t['pressed']}; }}"
            f"QSlider::handle:horizontal:pressed {{ background: {t['pressed']}; }}"
            f"QSlider::sub-page:horizontal {{ background: {t['accent']}; border-radius: 2px; }}")
        self.timer_box.set_theme(t)
        self.theme_box.set_theme(t)
        self.motion_box.set_theme(t)
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
        p.setPen(QPen(QColor(*t["edge"]), 1))  # structural edge, black/white at 8%
        p.drawPath(path)
        p.end()

    # ----- sound selection -----

    @property
    def selected_sound(self):
        return self.sounds[self.selected_index]

    def _subtitle_for(self, name):
        if name == "file":
            if self.file_path:
                base = os.path.basename(self.file_path)
                return base if len(base) <= 30 else base[:27] + "\u2026"
            return FILE_SUBTITLE_IDLE
        return SOUND_SUBTITLES[name]

    def _push_controls(self):
        """Re-apply the current slider values to a freshly created source."""
        self.player.set_volume(self.volume.value())
        self.player.set_depth(self.depth.value())
        self.player.set_rumble(self.rumble.value())
        self.eq.source = self.player.source.rms_values  # real output energy

    def _choose_file(self):
        """Pick and decode a local audio file. Returns True when loaded."""
        path, _ = QFileDialog.getOpenFileName(
            self, "Choose an audio file to loop", "", audiofile.dialog_filter())
        if not path:
            return False
        QApplication.setOverrideCursor(Qt.WaitCursor)
        try:
            samples = audiofile.decode(path)
        except audiofile.AudioFileError as e:
            QApplication.restoreOverrideCursor()
            QMessageBox.warning(self, "Cannot loop that file", str(e))
            return False
        QApplication.restoreOverrideCursor()
        self.file_path = path
        self.file_samples = samples
        base = os.path.basename(path)
        self.file_label.setText(base if len(base) <= 20 else base[:17] + "\u2026")
        self.file_label.setToolTip(path)
        return True

    def _choose_and_play_file(self):
        if self._choose_file():
            self._pick_sound("file")

    def _step_sound(self, delta):
        self.selected_index = (self.selected_index + delta) % len(self.sounds)
        self.title.setText(SOUND_TITLES[self.selected_sound])
        if self.is_playing:
            # Switching mid-play changes the sound seamlessly.
            if self.selected_sound == "file":
                if self.file_path is None:
                    # No file chosen yet: keep the noise that is playing;
                    # pressing play on "Your File" opens the picker.
                    self._render_status()
                    return
                self.player.play_file(self.file_samples)
                self._push_controls()
            else:
                self.player.play_sound(self.selected_sound)
            self.elapsed_seconds = 0
            self._render_elapsed()
            self._render_status()
        else:
            self.subtitle.setText(self._subtitle_for(self.selected_sound))

    def _pick_sound(self, name):
        if name == "file" and self.file_path is None:
            if not self._choose_file():
                # Dialog cancelled or undecodable: stay where we were.
                self.title.setText(SOUND_TITLES[self.selected_sound])
                self.subtitle.setText(self._subtitle_for(self.selected_sound))
                return
        self.selected_index = self.sounds.index(name)
        self.title.setText(SOUND_TITLES[name])
        if name == "file":
            self.player.play_file(self.file_samples)
            self._push_controls()
        else:
            self.player.play_sound(name)
        self._entered_playing()

    # ----- playback -----

    def _entered_playing(self):
        self.is_playing = True
        self.play_button.set_mode("pause")
        self.subtitle.setText(self._subtitle_for(self.selected_sound))
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
        self.player.restart()
        minutes = self.timer_box.itemData(self.timer_box.currentIndex())
        if minutes:
            # The preview's restart also restarts the sleep timer.
            self.sleep_timer.start(minutes * 60 * 1000)
            self.tick.start()
            self._update_countdown()
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
            if not self.is_playing:
                self.eq.status_text = "Paused"
            elif self.player.current_kind == "file":
                # A looped recording is never labelled "Live".
                self.eq.status_text = "File"
            else:
                self.eq.status_text = "Live"
            self.eq.update()

    def _reset_sound_controls(self):
        self.volume.setValue(synth.DEFAULT_VOLUME)
        self.depth.setValue(synth.DEFAULT_DEPTH)
        self.rumble.setValue(synth.DEFAULT_RUMBLE)

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
        self.settings_button.setIcon(QIcon(_pixmap(
            "gear", self.theme["accent"] if on else self.theme["subtext"], 40, 20 / 40)))
        self._set_fixed_height()

    def _motion_changed(self):
        self.reduce_motion = self.motion_box.currentText() == "On"
        self.eq.set_reduce_motion(self.reduce_motion)
        self.play_button.reduce_motion = self.reduce_motion
        self.timer_box.reduce_motion = self.reduce_motion
        self.theme_box.reduce_motion = self.reduce_motion
        self.motion_box.reduce_motion = self.reduce_motion
        self.timer_box.reduce_motion = self.reduce_motion
        self.theme_box.reduce_motion = self.reduce_motion
        self.motion_box.reduce_motion = self.reduce_motion

    def _set_fixed_height(self):
        h = self.BASE_HEIGHT + (self.SETTINGS_HEIGHT if self._settings_open else 0)
        self.setFixedSize(300, h)

    def showEvent(self, event):
        super().showEvent(event)
        if not getattr(self, "reduce_motion", False):
            self.setWindowOpacity(0.0)
            anim = QPropertyAnimation(self, b"windowOpacity", self)
            anim.setDuration(120)
            anim.setEasingCurve(QEasingCurve.OutCubic)
            anim.setStartValue(0.0)
            anim.setEndValue(1.0)
            anim.start(QPropertyAnimation.DeletionPolicy.DeleteWhenStopped)
            self._show_anim = anim

    def show_near_cursor(self):
        self.move(QCursor.pos().x() - self.width() // 2, QCursor.pos().y() + 12)
        self.show()
        self.raise_()
        self.activateWindow()


class TrayApp:
    def __init__(self):
        self.app = QApplication(sys.argv)
        fam = app_font_family()
        if fam:
            self.app.setFont(QFont(fam))
        self.app.setApplicationName("zenhush")
        self.app.setQuitOnLastWindowClosed(False)

        self.player = Player()
        self.popup = Popup(self.player)
        self.popup.setWindowIcon(QIcon(os.path.join(ASSETS, "icon.png")))
        QGuiApplication.styleHints().colorSchemeChanged.connect(
            lambda _: self.popup.system_color_scheme_changed())

        self.tray = None
        if QSystemTrayIcon.isSystemTrayAvailable():
            self.tray = QSystemTrayIcon(QIcon(os.path.join(ASSETS, "tray.png")), self.app)
            self.tray.setToolTip("Zenhush")
            menu = QMenu()
            show_action = QAction("Show Zenhush", menu)
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
        if os.environ.get("ZENHUSH_DEBUG"):
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
