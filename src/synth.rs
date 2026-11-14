//! Continuous live sound synthesis for Zenhush.
//!
//! Ported exactly from the Python `zenhush/synth.py` (same constants,
//! same filter math, same slider curves) so the Rust build sounds
//! identical to the Python app it replaces.
//!
//! The noise sources never play a file. A live source synthesizes fresh
//! random samples forever, shapes them with filters, and streams them
//! straight to the audio output: the sound is made right now, it never
//! ends and it never repeats. The file source is the one exception: it
//! loops the user's own local audio file through the same live filter
//! chain, and the app always labels that mode "File", never "Live".

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

pub const SAMPLE_RATE: u32 = 44_100;

/// Peak safety knee: untouched below this, softly rounded above.
pub const PEAK_KNEE: f32 = 0.89;

/// Per-recipe loudness: measured output RMS at default Depth/Rumble varies
/// widely, so each recipe gets a gain that lands it near TARGET_RMS.
/// Values are trimmed by HEADROOM so the loudest corner stays below clipping.
pub const TARGET_RMS: f32 = 0.12;
pub const HEADROOM: f32 = 0.5;

/// Visualizer: normalized output breathes around 0.7 on the bars.
pub const RMS_SCALE: f32 = 12.0;

/// Mono: lightest CPU, indistinguishable for shaped noise.
pub const CHANNELS: u16 = 1;
/// ~15 ms at 44.1 kHz, matches the visualizer window.
pub const CHUNK: usize = 661;

pub const DEFAULT_VOLUME: u8 = 70;
pub const DEFAULT_DEPTH: u8 = 65;
pub const DEFAULT_RUMBLE: u8 = 58;

const DEPTH_HI_HZ: f32 = 20_000.0;
const DEPTH_LO_HZ: f32 = 52.0;
const RUMBLE_MAX_DB: f32 = 12.0;
const RUMBLE_SHELF_HZ: f32 = 160.0;

/// The one-pole shelf picks up ~60% of the sub-band, so the raw gain needs
/// a drive factor to deliver the rated dB at the output.
const RUMBLE_DRIVE: f32 = 1.7;

/// Gentle fade-in so play/resume/switch never clicks or pops (150 ms).
pub const FADE_SAMPLES: usize = (0.15 * SAMPLE_RATE as f32) as usize;

/// Slider 0-100 -> output gain. 70 gives 0.441 (preview default 0.44).
pub fn volume_gain(value: u8) -> f32 {
    0.9 * (value as f32 / 100.0).powi(2)
}

/// Slider 0-100 -> lowpass cutoff in Hz. 65 gives 420 Hz.
pub fn depth_cutoff_hz(value: u8) -> f32 {
    DEPTH_HI_HZ * (DEPTH_LO_HZ / DEPTH_HI_HZ).powf(value as f32 / 100.0)
}

/// Slider 0-100 -> lowshelf boost in dB. 58 gives +4.0 dB.
pub fn rumble_gain_db(value: u8) -> f32 {
    RUMBLE_MAX_DB * (value as f32 / 100.0).powi(2)
}

fn output_gain(kind: Sound) -> f32 {
    match kind {
        Sound::Brown => 0.09 * HEADROOM,
        Sound::White => 1.43 * HEADROOM,
        Sound::Pink => 0.41 * HEADROOM,
        Sound::Rain => 0.66 * HEADROOM,
        Sound::Ocean => 0.12 * HEADROOM,
    }
}

/// The five live noise recipes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sound {
    Brown,
    White,
    Pink,
    Rain,
    Ocean,
}

impl Sound {
    pub fn title(self) -> &'static str {
        match self {
            Sound::Brown => "Brown Noise",
            Sound::White => "White Noise",
            Sound::Pink => "Pink Noise",
            Sound::Rain => "Rain",
            Sound::Ocean => "Ocean",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Sound::Brown => "deep and warm",
            Sound::White => "bright and even",
            Sound::Pink => "soft and balanced",
            Sound::Rain => "steady rainfall",
            Sound::Ocean => "slow waves",
        }
    }

    pub const ALL: [Sound; 5] = [
        Sound::Brown,
        Sound::White,
        Sound::Pink,
        Sound::Rain,
        Sound::Ocean,
    ];
}

/// Stateful per-sample noise recipe. Each voice runs forever.
struct Voice {
    rng: SmallRng,
    // brown / pink filter state
    brown: f32,
    p0: f32,
    p1: f32,
    p2: f32,
    // rain droplet state
    drop: f32,
    drop_decay: f32,
    drop_prev: f32,
    // ocean wave phase
    t: f32,
}

impl Voice {
    fn new() -> Self {
        Voice {
            // Fresh entropy each voice (thread RNG -> u64 seed).
            rng: SmallRng::seed_from_u64(rand::random()),
            brown: 0.0,
            p0: 0.0,
            p1: 0.0,
            p2: 0.0,
            drop: 0.0,
            drop_decay: 0.0,
            drop_prev: 0.0,
            t: 0.0,
        }
    }

    /// Next raw sample before the filter chain.
    fn next(&mut self, kind: Sound) -> f32 {
        let r = self.rng.random::<f32>();
        match kind {
            Sound::White => r * 2.0 - 1.0,
            Sound::Brown => {
                self.brown = 0.998 * self.brown + 0.02 * (r * 2.0 - 1.0);
                self.brown * 8.0
            }
            Sound::Pink => {
                let w = r * 2.0 - 1.0;
                self.p0 = 0.99765 * self.p0 + w * 0.0990460;
                self.p1 = 0.96300 * self.p1 + w * 0.2965164;
                self.p2 = 0.57000 * self.p2 + w * 1.0526913;
                (self.p0 + self.p1 + self.p2 + w * 0.1848) * 0.25
            }
            Sound::Rain => {
                let w = r * 2.0 - 1.0;
                self.p0 = 0.99765 * self.p0 + w * 0.0990460;
                self.p1 = 0.96300 * self.p1 + w * 0.2965164;
                self.p2 = 0.57000 * self.p2 + w * 1.0526913;
                let base = (self.p0 + self.p1 + self.p2 + w * 0.1848) * 0.25;
                if self.rng.random::<f32>() < 28.0 / SAMPLE_RATE as f32 {
                    self.drop = self.rng.random_range(0.3..1.0);
                    self.drop_decay = (-1.0 / (SAMPLE_RATE as f32 * 0.002)).exp();
                }
                self.drop *= self.drop_decay;
                let drop = self.drop * self.rng.random_range(-1.0..1.0);
                let hp = drop - self.drop_prev;
                self.drop_prev = drop;
                0.6 * base + 0.45 * hp
            }
            Sound::Ocean => {
                self.brown = 0.998 * self.brown + 0.02 * (r * 2.0 - 1.0);
                self.t += 1.0 / SAMPLE_RATE as f32;
                let lfo = 0.55
                    + 0.30 * (2.0 * std::f32::consts::PI * 0.07 * self.t).sin()
                    + 0.15 * (2.0 * std::f32::consts::PI * 0.13 * self.t + 1.3).sin();
                self.brown * 8.0 * lfo.max(0.05)
            }
        }
    }
}

/// What the synth plays: a live recipe, or the user's looped file.
pub enum Source {
    Live(Sound),
    /// Mono f32 samples at 44.1 kHz, looped sample-exactly.
    File { samples: Vec<f32>, pos: usize },
}

/// The live synth: generates 16-bit mono samples on demand, forever.
///
/// Depth and Rumble retune the filters live between calls; Volume is
/// ramped to stay click-free. The output is bounded strictly below
/// full scale by the soft peak knee, so int16 conversion can never
/// overflow or produce harsh digital clipping.
pub struct Synth {
    source: Source,
    voice: Voice,
    out_gain: f32,
    // depth: one-pole lowpass coefficient + state
    depth_a: f32,
    depth_state: f32,
    // rumble: one-pole lowshelf, boosted low band added back
    rumble_a: f32,
    rumble_gain: f32,
    rumble_state: f32,
    // volume: last gain stage, ramped to avoid zipper noise
    volume: f32,
    volume_target: f32,
    // gentle fade-in so play/resume/switch never clicks or pops
    fade_left: usize,
    // recent real output energy for the visualizer, ~15 ms per value
    rms_ring: std::collections::VecDeque<f32>,
    rms_acc: f32,
    rms_n: usize,
}

impl Synth {
    pub fn new(sound: Sound) -> Self {
        let mut s = Synth {
            source: Source::Live(sound),
            voice: Voice::new(),
            out_gain: output_gain(sound),
            depth_a: 1.0,
            depth_state: 0.0,
            rumble_a: 0.0,
            rumble_gain: 1.0,
            rumble_state: 0.0,
            volume: volume_gain(DEFAULT_VOLUME),
            volume_target: volume_gain(DEFAULT_VOLUME),
            fade_left: 0,
            rms_ring: std::collections::VecDeque::with_capacity(64),
            rms_acc: 0.0,
            rms_n: 0,
        };
        s.set_depth(DEFAULT_DEPTH);
        s.set_rumble(DEFAULT_RUMBLE);
        s
    }

    /// Switch to a live recipe. Fresh noise, fresh filters, gentle fade.
    pub fn set_sound(&mut self, sound: Sound) {
        self.source = Source::Live(sound);
        self.voice = Voice::new();
        self.out_gain = output_gain(sound);
        self.depth_state = 0.0;
        self.rumble_state = 0.0;
        self.fade_left = FADE_SAMPLES;
    }

    /// Switch to the user's file (decoded mono f32, 44.1 kHz).
    /// Respects the file's own level; volume is the user's control.
    pub fn set_file(&mut self, samples: Vec<f32>) {
        assert!(!samples.is_empty(), "file source needs at least one sample");
        self.source = Source::File { samples, pos: 0 };
        self.out_gain = 1.0;
        self.depth_state = 0.0;
        self.rumble_state = 0.0;
        self.fade_left = FADE_SAMPLES;
    }

    /// Fresh noise from a new seed (live) or rewind (file): never replays.
    pub fn restart(&mut self) {
        match &mut self.source {
            Source::Live(_) => {
                self.voice = Voice::new();
            }
            Source::File { pos, .. } => {
                *pos = 0;
            }
        }
        self.depth_state = 0.0;
        self.rumble_state = 0.0;
        self.rms_ring.clear();
        self.fade_left = FADE_SAMPLES;
    }

    /// Slider 0-100, preview-fitted curve: 70 -> gain 0.44.
    pub fn set_volume(&mut self, value: u8) {
        self.volume_target = volume_gain(value);
    }

    /// Slider 0-100 -> lowpass cutoff. 65 gives 420 Hz.
    pub fn set_depth(&mut self, value: u8) {
        let cutoff = depth_cutoff_hz(value);
        // one-pole lowpass: a = 1 - exp(-2*pi*fc/fs)
        self.depth_a = 1.0 - (-2.0 * std::f32::consts::PI * cutoff / SAMPLE_RATE as f32).exp();
    }

    /// Slider 0-100 -> lowshelf boost. 58 gives +4.0 dB.
    pub fn set_rumble(&mut self, value: u8) {
        self.rumble_gain =
            (10f32.powf(rumble_gain_db(value) / 20.0) - 1.0) * RUMBLE_DRIVE;
        self.rumble_a =
            1.0 - (-2.0 * std::f32::consts::PI * RUMBLE_SHELF_HZ / SAMPLE_RATE as f32).exp();
    }

    /// Resume from silence gently: ramp in instead of clicking on.
    pub fn soft_start(&mut self) {
        self.fade_left = FADE_SAMPLES;
    }

    /// Recent real output energy, oldest first, one value per ~15 ms.
    pub fn rms_values(&self) -> Vec<f32> {
        self.rms_ring.iter().copied().collect()
    }

    /// Take and clear the new RMS values since the last drain
    /// (used by the audio callback to feed the visualizer ring).
    pub fn drain_rms(&mut self) -> Vec<f32> {
        self.rms_ring.drain(..).collect()
    }

    /// Is the current source the user's file?
    pub fn is_file(&self) -> bool {
        matches!(self.source, Source::File { .. })
    }

    fn produce(&mut self) -> f32 {
        match &mut self.source {
            Source::Live(kind) => {
                let kind = *kind;
                self.voice.next(kind)
            }
            Source::File { samples, pos } => {
                let v = samples[*pos];
                *pos += 1;
                if *pos == samples.len() {
                    *pos = 0; // sample-exact wrap: gapless by construction
                }
                v
            }
        }
    }

    /// Fill `out` with 16-bit mono samples. `out` length is the frame count.
    pub fn render(&mut self, out: &mut [i16]) {        let n = out.len();
        if n == 0 {
            return;
        }
        let gain = self.out_gain;
        let depth_a = self.depth_a;
        let mut depth_s = self.depth_state;
        let rumble_a = self.rumble_a;
        let rumble_g = self.rumble_gain;
        let mut rumble_s = self.rumble_state;
        let mut vol = self.volume;
        let vol_t = self.volume_target;
        // ~20 ms ramp
        let vol_step = (vol_t - vol) / (n.min(882).max(1) as f32);
        let mut fade = self.fade_left;
        let fade_total = FADE_SAMPLES;
        let mut acc = self.rms_acc;
        let mut cnt = self.rms_n;

        for slot in out.iter_mut() {
            let x = self.produce();
            depth_s += depth_a * (x - depth_s);
            rumble_s += rumble_a * (x - rumble_s);
            let mut v = (depth_s + rumble_s * rumble_g) * gain * vol;
            if vol != vol_t {
                vol += vol_step;
                if (vol_t - vol) * vol_step < 0.0 {
                    vol = vol_t;
                }
            }
            if fade > 0 {
                v *= (fade_total - fade) as f32 / fade_total as f32;
                fade -= 1;
            }
            // Soft peak knee: below 0.89 samples pass untouched.
            if v > PEAK_KNEE {
                v = PEAK_KNEE + 0.11 * ((v - PEAK_KNEE) / 0.11).tanh();
            } else if v < -PEAK_KNEE {
                v = -PEAK_KNEE - 0.11 * ((-v - PEAK_KNEE) / 0.11).tanh();
            }
            *slot = (v * 32767.0) as i16;
            acc += v * v;
            cnt += 1;
            if cnt >= CHUNK {
                let rms = (acc / cnt as f32).sqrt() * RMS_SCALE;
                if self.rms_ring.len() == 64 {
                    self.rms_ring.pop_front();
                }
                self.rms_ring.push_back((0.15 + rms).min(1.0));
                acc = 0.0;
                cnt = 0;
            }
        }

        self.depth_state = depth_s;
        self.rumble_state = rumble_s;
        self.volume = vol;
        self.fade_left = fade;
        self.rms_acc = acc;
        self.rms_n = cnt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[i16]) -> f32 {
        let sum: f64 = samples.iter().map(|&s| (s as f64 / 32767.0).powi(2)).sum();
        (sum / samples.len() as f64).sqrt() as f32
    }

    #[test]
    fn slider_curves_match_the_python_app() {
        // The three known points from the Python docstring.
        assert!((volume_gain(70) - 0.441).abs() < 0.002, "volume 70 -> {}", volume_gain(70));
        assert!((depth_cutoff_hz(65) - 420.0).abs() < 15.0, "depth 65 -> {}", depth_cutoff_hz(65));
        assert!((rumble_gain_db(58) - 4.0).abs() < 0.15, "rumble 58 -> {}", rumble_gain_db(58));
        assert_eq!(volume_gain(0), 0.0);
        assert!((volume_gain(100) - 0.9).abs() < 1e-6);
    }

    #[test]
    fn every_sound_renders_without_clipping() {
        for sound in Sound::ALL {
            let mut synth = Synth::new(sound);
            let mut buf = vec![0i16; SAMPLE_RATE as usize]; // 1 second
            synth.render(&mut buf);
            // Not silent, not constant.
            assert!(buf.iter().any(|&s| s != 0), "{:?} is silent", sound);
            assert!(buf.iter().any(|&s| s != buf[0]), "{:?} is constant", sound);
            // The soft knee bounds the output below full scale.
            assert!(buf.iter().all(|&s| s.abs() < 32767), "{:?} clipped", sound);
            // Level is in a sane band (not whisper-quiet, not maxed).
            let r = rms(&buf);
            assert!(r > 0.01 && r < 0.6, "{:?} rms {} out of band", sound, r);
        }
    }

    #[test]
    fn all_sounds_land_in_a_similar_loudness_band() {
        // The per-recipe gains are calibrated so every sound lands near
        // the same target RMS at the same sliders (the Python app's
        // design goal). No sound should be more than about 3x another.
        let mut levels = Vec::new();
        for sound in Sound::ALL {
            let mut synth = Synth::new(sound);
            let mut buf = vec![0i16; SAMPLE_RATE as usize];
            synth.render(&mut buf);
            levels.push((sound, rms(&buf)));
        }
        let min = levels.iter().map(|(_, r)| *r).fold(f32::MAX, f32::min);
        let max = levels.iter().map(|(_, r)| *r).fold(f32::MIN, f32::max);
        assert!(
            max / min < 3.0,
            "loudness spread too wide: {:?}",
            levels
        );
    }

    #[test]
    fn file_source_loops_gaplessly() {
        // A 1-second file of a constant value must loop: the output
        // after the wrap is the same constant (no gap, no fade at seam).
        let samples = vec![0.5f32; SAMPLE_RATE as usize];
        let mut synth = Synth::new(Sound::Brown);
        synth.set_file(samples);
        // Render 1.5 seconds: the second half replays the file start.
        let mut buf = vec![0i16; (SAMPLE_RATE as usize * 3) / 2];
        synth.render(&mut buf);
        // After the 150 ms fade, the output is a steady filtered constant.
        // Compare the last 1000 samples of second 1 vs second 1.5's tail:
        // they should match closely (same looped input, steady filter).
        let tail1 = &buf[SAMPLE_RATE as usize - 1000..SAMPLE_RATE as usize];
        let tail2 = &buf[buf.len() - 1000..];
        let diff: i32 = tail1.iter().zip(tail2.iter()).map(|(a, b)| (a - b).abs() as i32).sum::<i32>() / 1000;
        assert!(diff < 200, "loop seam mismatch, mean abs diff {}", diff);
    }
}
