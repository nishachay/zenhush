//! Decode the user's local audio file (wav/mp3/flac/ogg/m4a) to mono
//! float PCM at 44.1 kHz, for the file-loop mode.
//!
//! Ported from the Python `zenhush/audiofile.py`: same 10-minute cap
//! (a longer file cannot loop), same mono 44.1 kHz output, same
//! "File is too long to loop" error. The cap is enforced incrementally
//! while decoding, so a 2-hour file is rejected fast instead of being
//! fully decoded into memory first.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::{Channels, SampleBuffer, SignalSpec};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::synth::SAMPLE_RATE;

/// Longest file that may loop, in seconds (matches the Python app).
pub const MAX_SECONDS: u64 = 600;

/// Decode `path` to mono f32 samples at 44.1 kHz.
/// Returns Err with a human message on failure or when too long.
pub fn decode(path: &Path) -> Result<Vec<f32>, String> {
    let file = File::open(path).map_err(|e| format!("Could not open file ({})", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Could not read file ({})", e))?;
    let mut format = probed.format;

    let track = format
        .default_track()
        .ok_or_else(|| "No audio track in this file".to_string())?;
    let track_id = track.id;
    let params = track.codec_params.clone();

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| format!("Could not decode file ({})", e))?;

    let spec = SignalSpec::new(
        params.sample_rate.unwrap_or(SAMPLE_RATE),
        params.channels.unwrap_or(Channels::FRONT_LEFT),
    );
    let max_frames = MAX_SECONDS * spec.rate as u64;

    let mut mono: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(Error::ResetRequired) => {
                // Mid-stream reset (rare): drop the decoder state and continue.
                decoder = symphonia::default::get_codecs()
                    .make(&params, &DecoderOptions::default())
                    .map_err(|e| format!("Could not decode file ({})", e))?;
                continue;
            }
            Err(e) => return Err(format!("Could not decode file ({})", e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue, // skip a bad packet
            Err(e) => return Err(format!("Could not decode file ({})", e)),
        };

        // Convert this packet to interleaved f32. Capture the spec
        // first: copy_interleaved_ref moves the decoded buffer.
        let spec = *decoded.spec();
        let channels = spec.channels.count();
        let capacity = decoded.capacity();
        let buf = sample_buf.get_or_insert_with(|| {
            SampleBuffer::<f32>::new(capacity as u64, spec)
        });
        buf.copy_interleaved_ref(decoded);
        let interleaved = buf.samples();

        // Downmix to mono (average channels) and enforce the cap.
        let frames = interleaved.len() / channels.max(1);
        if mono.len() as u64 + frames as u64 > max_frames {
            return Err("File is too long to loop (over 10 minutes)".to_string());
        }
        if channels == 1 {
            mono.extend_from_slice(interleaved);
        } else {
            for frame in interleaved.chunks_exact(channels) {
                let sum: f32 = frame.iter().sum();
                mono.push(sum / channels as f32);
            }
        }
    }

    if mono.is_empty() {
        return Err("File has no audio to play".to_string());
    }

    // Resample to 44.1 kHz if the file used another rate.
    // Linear interpolation: the file is a recording, not a tone, and the
    // depth lowpass already shapes the output, so the loss is inaudible.
    let rate = spec.rate;
    if rate == SAMPLE_RATE {
        return Ok(mono);
    }
    let ratio = rate as f64 / SAMPLE_RATE as f64;
    let out_len = (mono.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = mono[idx];
        let b = if idx + 1 < mono.len() {
            mono[idx + 1]
        } else {
            a
        };
        out.push(a + (b - a) * frac);
    }
    Ok(out)
}
