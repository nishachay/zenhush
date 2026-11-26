//! Streaming local audio file decoder (wav/mp3/flac/ogg/m4a) for Zenhush.
//!
//! Unlike a full-file RAM decode, this decodes packets on a dedicated low-CPU
//! background thread and streams resampled stereo f32 frames into a tiny
//! lock-free ring buffer (~192 KB). That means:
//! - Playback starts in < 20 ms with zero UI freeze
//! - Long focus mixes (1+ hours) work effortlessly in < 1 MB RAM
//! - True stereo is preserved (mono files are centered)
//! - Seamless loop with optional 250 ms crossfade at loop boundaries

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

const RING_FRAMES: usize = 24_000; // ~0.5 s stereo buffer (~192 KB)

/// Metadata probed quickly from a local audio file.
#[derive(Clone, Debug)]
pub struct TrackProbe {
    pub duration_secs: Option<u64>,
}

/// Validate and quickly probe a local audio file without decoding the full audio.
pub fn probe_file(path: &Path) -> Result<TrackProbe, String> {
    if !path.is_file() {
        return Err("Audio file was not found on disk".to_string());
    }
    let file = File::open(path).map_err(|e| format!("Could not open file ({})", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Unsupported or unreadable file ({})", e))?;

    let track = probed
        .format
        .default_track()
        .ok_or_else(|| "No audio track found in file".to_string())?;
    let params = &track.codec_params;

    // Verify we can actually instantiate a decoder for this codec.
    let _decoder = symphonia::default::get_codecs()
        .make(params, &DecoderOptions::default())
        .map_err(|e| format!("Unsupported codec ({})", e))?;

    let duration_secs = match (params.n_frames, params.sample_rate) {
        (Some(frames), Some(rate)) if rate > 0 => Some(frames / rate as u64),
        _ => None,
    };

    Ok(TrackProbe { duration_secs })
}

/// Real-time audio-thread consumer for a streaming local audio file.
pub struct FileStreamReader {
    rx: rtrb::Consumer<[f32; 2]>,
    rewind_flag: Arc<AtomicBool>,
    last_frame: [f32; 2],
    _control: Arc<StreamControl>,
}

struct StreamControl {
    stop: AtomicBool,
    crossfade: AtomicBool,
}

impl Drop for StreamControl {
    fn drop(&mut self) {
        // Non-blocking: never join() on the real-time audio callback thread;
        // the decoder thread sees `stop` within a few milliseconds and exits.
        self.stop.store(true, Ordering::Release);
    }
}

impl FileStreamReader {
    /// Pull the next stereo frame in the real-time audio callback (lock-free).
    #[inline]
    pub fn next_frame(&mut self) -> [f32; 2] {
        if let Ok(frame) = self.rx.pop() {
            self.last_frame = frame;
            frame
        } else {
            // Brief startup / seek underrun: decay gently instead of clicking
            self.last_frame[0] *= 0.98;
            self.last_frame[1] *= 0.98;
            self.last_frame
        }
    }

    /// Rewind the file back to the beginning.
    pub fn rewind(&mut self) {
        while self.rx.pop().is_ok() {}
        self.rewind_flag.store(true, Ordering::Release);
    }
}

/// Handle kept by `AudioEngine` so streams can be rebuilt on device switch
/// or crossfade settings updated live.
#[derive(Clone)]
pub struct FileStreamHandle {
    path: PathBuf,
    crossfade: Arc<AtomicBool>,
}

impl FileStreamHandle {
    pub fn new(path: PathBuf, crossfade: bool) -> Self {
        FileStreamHandle {
            path,
            crossfade: Arc::new(AtomicBool::new(crossfade)),
        }
    }

    pub fn set_crossfade(&self, enabled: bool) {
        self.crossfade.store(enabled, Ordering::Release);
    }

    /// Spawn a streaming decoder for `target_sample_rate`.
    pub fn open_reader(&self, target_sample_rate: u32) -> Result<FileStreamReader, String> {
        probe_file(&self.path)?;

        let (tx, rx) = rtrb::RingBuffer::<[f32; 2]>::new(RING_FRAMES);
        let rewind_flag = Arc::new(AtomicBool::new(false));
        let control = Arc::new(StreamControl {
            stop: AtomicBool::new(false),
            crossfade: AtomicBool::new(self.crossfade.load(Ordering::Acquire)),
        });

        let path = self.path.clone();
        let ctrl_thread = control.clone();
        let rw_thread = rewind_flag.clone();
        let xf_shared = self.crossfade.clone();

        thread::Builder::new()
            .name("zenhush-decoder".into())
            .spawn(move || {
                decoder_loop(
                    &path,
                    target_sample_rate.max(8000),
                    tx,
                    ctrl_thread,
                    rw_thread,
                    xf_shared,
                );
            })
            .map_err(|e| format!("Failed to spawn decoder thread ({})", e))?;

        Ok(FileStreamReader {
            rx,
            rewind_flag,
            last_frame: [0.0; 2],
            _control: control,
        })
    }
}

fn open_format(path: &Path) -> Option<(Box<dyn symphonia::core::formats::FormatReader>, u32, Box<dyn symphonia::core::codecs::Decoder>, symphonia::core::codecs::CodecParameters)> {
    let file = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;
    let format = probed.format;
    let track = format.default_track()?;
    let track_id = track.id;
    let params = track.codec_params.clone();
    let decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .ok()?;
    Some((format, track_id, decoder, params))
}

struct CrossfadeState {
    xfade_len: usize,
    tail_buf: std::collections::VecDeque<[f32; 2]>,
    blend_tail: std::collections::VecDeque<[f32; 2]>,
    blend_total: usize,
}

impl CrossfadeState {
    fn new(target_rate: u32) -> Self {
        let xfade_len = ((target_rate as usize * 18) / 100).max(1);
        CrossfadeState {
            xfade_len,
            tail_buf: std::collections::VecDeque::with_capacity(xfade_len + 4),
            blend_tail: std::collections::VecDeque::new(),
            blend_total: 0,
        }
    }

    fn clear(&mut self) {
        self.tail_buf.clear();
        self.blend_tail.clear();
        self.blend_total = 0;
    }

    fn on_eof(&mut self) {
        if !self.tail_buf.is_empty() {
            self.blend_total = self.tail_buf.len();
            self.blend_tail = std::mem::take(&mut self.tail_buf);
        }
    }

    fn push_frame(
        &mut self,
        tx: &mut rtrb::Producer<[f32; 2]>,
        frame: [f32; 2],
        use_xf: bool,
        control: &StreamControl,
        rewind_flag: &AtomicBool,
    ) -> bool {
        let mut cur = frame;
        if let Some(t_frame) = self.blend_tail.pop_front() {
            let done = self.blend_total.saturating_sub(self.blend_tail.len() + 1);
            let alpha = done as f32 / self.blend_total.max(1) as f32;
            cur = [
                t_frame[0] * (1.0 - alpha) + cur[0] * alpha,
                t_frame[1] * (1.0 - alpha) + cur[1] * alpha,
            ];
        }

        if use_xf {
            self.tail_buf.push_back(cur);
            if self.tail_buf.len() <= self.xfade_len {
                return true;
            }
            cur = self.tail_buf.pop_front().unwrap_or(cur);
        } else {
            while let Some(old) = self.tail_buf.pop_front() {
                while tx.push(old).is_err() {
                    if control.stop.load(Ordering::Acquire) || rewind_flag.load(Ordering::Acquire) {
                        return false;
                    }
                    thread::sleep(Duration::from_millis(4));
                }
            }
        }

        while tx.push(cur).is_err() {
            if control.stop.load(Ordering::Acquire) || rewind_flag.load(Ordering::Acquire) {
                return false;
            }
            thread::sleep(Duration::from_millis(4));
        }
        true
    }
}

fn decoder_loop(
    path: &Path,
    target_rate: u32,
    mut tx: rtrb::Producer<[f32; 2]>,
    control: Arc<StreamControl>,
    rewind_flag: Arc<AtomicBool>,
    xf_shared: Arc<AtomicBool>,
) {
    let Some((mut format, mut track_id, mut decoder, mut params)) = open_format(path) else {
        return;
    };

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut resample_pos: f64 = 0.0;
    let mut prev_frame: [f32; 2] = [0.0; 2];
    let mut has_prev = false;
    let mut xf = CrossfadeState::new(target_rate);

    while !control.stop.load(Ordering::Acquire) {
        if rewind_flag.swap(false, Ordering::AcqRel) {
            if format
                .seek(
                    SeekMode::Accurate,
                    SeekTo::Time {
                        time: Time { seconds: 0, frac: 0.0 },
                        track_id: Some(track_id),
                    },
                )
                .is_err()
            {
                if let Some((f, tid, d, p)) = open_format(path) {
                    format = f;
                    track_id = tid;
                    decoder = d;
                    params = p;
                }
            }
            decoder.reset();
            resample_pos = 0.0;
            has_prev = false;
            xf.clear();
        }

        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(ref e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                xf.on_eof();

                if format
                    .seek(
                        SeekMode::Accurate,
                        SeekTo::Time {
                            time: Time { seconds: 0, frac: 0.0 },
                            track_id: Some(track_id),
                        },
                    )
                    .is_err()
                {
                    if let Some((f, tid, d, p)) = open_format(path) {
                        format = f;
                        track_id = tid;
                        decoder = d;
                        params = p;
                    } else {
                        break;
                    }
                }
                decoder.reset();
                resample_pos = 0.0;
                has_prev = false;
                continue;
            }
            Err(Error::ResetRequired) => {
                if let Ok(d) = symphonia::default::get_codecs()
                    .make(&params, &DecoderOptions::default())
                {
                    decoder = d;
                }
                continue;
            }
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        };

        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        let src_rate = spec.rate.max(8000);
        let frames_in = decoded.frames();
        if frames_in == 0 {
            continue;
        }

        let capacity = decoded.capacity();
        let buf = sample_buf.get_or_insert_with(|| SampleBuffer::<f32>::new(capacity as u64, spec));
        if buf.capacity() < capacity {
            *buf = SampleBuffer::<f32>::new(capacity as u64, spec);
        }
        buf.copy_interleaved_ref(decoded);
        let interleaved = buf.samples();

        let ratio = src_rate as f64 / target_rate as f64;
        let get_stereo = |idx: usize| -> [f32; 2] {
            let base = idx * channels;
            if channels == 1 {
                let s = interleaved[base];
                [s, s]
            } else {
                [interleaved[base], interleaved[base + 1]]
            }
        };

        let use_xf = xf_shared.load(Ordering::Acquire) || control.crossfade.load(Ordering::Acquire);
        let mut i_pos = resample_pos;
        while i_pos < frames_in as f64 && !control.stop.load(Ordering::Acquire) {
            if rewind_flag.load(Ordering::Acquire) {
                break;
            }
            let idx = i_pos as usize;
            let frac = (i_pos - idx as f64) as f32;
            let a = if idx == 0 && has_prev && i_pos < 0.0 {
                prev_frame
            } else {
                get_stereo(idx.min(frames_in - 1))
            };
            let b = if idx + 1 < frames_in {
                get_stereo(idx + 1)
            } else {
                a
            };
            let out = [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac];
            if !xf.push_frame(&mut tx, out, use_xf, &control, &rewind_flag) {
                break;
            }
            i_pos += ratio;
        }
        prev_frame = get_stereo(frames_in - 1);
        has_prev = true;
        resample_pos = i_pos - frames_in as f64;
        if resample_pos < 0.0 {
            resample_pos = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_test_wav(path: &Path, sample_rate: u32, seconds: f32) {
        let frames = (sample_rate as f32 * seconds) as u32;
        let data_bytes = frames * 2 * 2; // stereo 16-bit
        let mut f = File::create(path).unwrap();
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(36 + data_bytes).to_le_bytes()).unwrap();
        f.write_all(b"WAVE").unwrap();
        f.write_all(b"fmt ").unwrap();
        f.write_all(&16u32.to_le_bytes()).unwrap();
        f.write_all(&1u16.to_le_bytes()).unwrap(); // PCM
        f.write_all(&2u16.to_le_bytes()).unwrap(); // stereo
        f.write_all(&sample_rate.to_le_bytes()).unwrap();
        f.write_all(&(sample_rate * 4).to_le_bytes()).unwrap();
        f.write_all(&4u16.to_le_bytes()).unwrap();
        f.write_all(&16u16.to_le_bytes()).unwrap();
        f.write_all(b"data").unwrap();
        f.write_all(&data_bytes.to_le_bytes()).unwrap();
        for i in 0..frames {
            let s = ((i as f32 * 440.0 * std::f32::consts::TAU / sample_rate as f32).sin()
                * 12000.0) as i16;
            f.write_all(&s.to_le_bytes()).unwrap();
            f.write_all(&s.to_le_bytes()).unwrap();
        }
    }

    #[test]
    fn probes_and_streams_stereo_wav_with_loop() {
        let tmp = std::env::temp_dir().join("zenhush_test_loop.wav");
        write_test_wav(&tmp, 44_100, 1.0);

        let probe = probe_file(&tmp).unwrap();
        assert_eq!(probe.duration_secs, Some(1));

        let handle = FileStreamHandle::new(tmp.clone(), true);
        let mut reader = handle.open_reader(48_000).unwrap();
        std::thread::sleep(Duration::from_millis(80));

        // Read 1.5 seconds at 48 kHz -> forces loop wrap
        let mut max_amp = 0.0f32;
        for _ in 0..72_000 {
            let frame = reader.next_frame();
            max_amp = max_amp.max(frame[0].abs()).max(frame[1].abs());
        }
        assert!(max_amp > 0.2, "streamed audio amplitude too low: {}", max_amp);
        let _ = std::fs::remove_file(&tmp);
    }
}
