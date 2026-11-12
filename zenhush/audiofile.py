"""Decode a local audio file for looping.

The file never leaves the device: Qt Multimedia's FFmpeg backend decodes
it in-process to mono float PCM at the engine's sample rate. No upload,
no cloud conversion, no extra dependencies.

Format support is what was actually verified by decoding test files in
the Linux build environment (Qt 6.11.3 / FFmpeg 7.1.5): wav, flac, mp3,
ogg/vorbis and m4a/aac all decode. Lossy files can carry encoder padding
(observed: an aac test file decoded ~20 ms longer than its source), so a
loop seam can be audible on some files. That edge lives in the encoded
file itself; the loop code wraps sample-exactly and adds no gap of its
own.
"""
import os
from array import array

from PySide6.QtCore import QCoreApplication, QElapsedTimer, QUrl
from PySide6.QtMultimedia import QAudioDecoder, QAudioFormat

from . import synth

SUPPORTED_EXTENSIONS = (".wav", ".mp3", ".flac", ".ogg", ".m4a")
MAX_SECONDS = 10 * 60  # decoded cap: 10 min mono float is ~106 MB of RAM
DECODE_TIMEOUT_MS = 60000


class AudioFileError(Exception):
    """User-presentable decode failure (missing / unsupported / too long)."""


def dialog_filter():
    """QFileDialog name filter listing exactly the verified extensions."""
    exts = " ".join("*" + e for e in SUPPORTED_EXTENSIONS)
    return f"Audio files ({exts})"


def decode(path, pump=None):
    """Decode path -> array('f') of mono float samples at synth.SAMPLE_RATE.

    pump: optional callable run while decoding (defaults to processing Qt
    events so the UI stays alive). Raises AudioFileError on any failure.
    """
    if not os.path.isfile(path):
        raise AudioFileError("That file does not exist.")
    if os.path.splitext(path)[1].lower() not in SUPPORTED_EXTENSIONS:
        raise AudioFileError(
            "Unsupported type. Zenhush can loop: "
            + ", ".join(SUPPORTED_EXTENSIONS))

    app = QCoreApplication.instance()

    def _pump():
        if pump is not None:
            pump()
        elif app is not None:
            app.processEvents()

    fmt = QAudioFormat()
    fmt.setSampleRate(synth.SAMPLE_RATE)
    fmt.setChannelCount(1)
    fmt.setSampleFormat(QAudioFormat.Float)

    dec = QAudioDecoder()
    dec.setAudioFormat(fmt)
    dec.setSource(QUrl.fromLocalFile(os.path.abspath(path)))

    chunks = []
    errors = []
    finished = []
    dec.bufferReady.connect(lambda: chunks.append(dec.read()))
    err_sig = getattr(dec, "errorOccurred", None) or getattr(dec, "error", None)
    if err_sig is not None:
        try:
            err_sig.connect(lambda *a: errors.append(str(a[-1])))
        except TypeError:
            err_sig.connect(lambda e: errors.append(str(e)))
    dec.finished.connect(lambda: finished.append(1))

    dec.start()
    t = QElapsedTimer()
    t.start()
    while not finished and not errors and t.elapsed() < DECODE_TIMEOUT_MS:
        _pump()

    if errors:
        raise AudioFileError("Could not decode this file: " + errors[0])
    if not finished:
        raise AudioFileError("Decoding took too long; try a shorter file.")

    samples = array("f")
    total = 0
    for buf in chunks:
        data = buf.constData()
        if data is None:
            continue
        n = len(data) // 4
        total += n
        if total > MAX_SECONDS * synth.SAMPLE_RATE:
            raise AudioFileError(
                "File is too long to loop (over %d minutes)." % (MAX_SECONDS // 60))
        samples.frombytes(bytes(data[: n * 4]))
    if not samples:
        raise AudioFileError("No audio stream found in this file.")
    return samples
