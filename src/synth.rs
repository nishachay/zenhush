//! Continuous stereo live sound synthesis for Zenhush.
//!
//! Generates 8 procedural focus soundscapes in decorrelated stereo at the
//! output device's native sample rate, plus supports layering a secondary
//! atmosphere texture (e.g. Rain or Fireplace) underneath any primary sound
//! or local audio file.
//!
//! All playback transitions (start, resume, switch, pause, sleep-timer
//! expiry) ramp smoothly to stay 100% click- and pop-free.

use std::collections::VecDeque;
use std::f32::consts::TAU;

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::filemode::FileStreamReader;

pub const DEFAULT_SAMPLE_RATE: u32 = 44_100;

/// Peak safety knee: untouched below this, softly rounded above.
pub const PEAK_KNEE: f32 = 0.89;

/// Headroom factor so loudest slider corners stay below clipping.
pub const HEADROOM: f32 = 0.5;

/// Visualizer normalization multiplier.
pub const RMS_SCALE: f32 = 12.0;

/// ~15 ms RMS window at 44.1 kHz (scales with sample_rate).
pub const RMS_WINDOW_MS: f32 = 15.0;

pub const DEFAULT_VOLUME: u8 = 70;
pub const DEFAULT_DEPTH: u8 = 65;
pub const DEFAULT_RUMBLE: u8 = 58;
pub const DEFAULT_LAYER_MIX: u8 = 30;

const DEPTH_HI_HZ: f32 = 20_000.0;
const DEPTH_LO_HZ: f32 = 52.0;
const RUMBLE_MAX_DB: f32 = 12.0;
const RUMBLE_SHELF_HZ: f32 = 160.0;
const RUMBLE_DRIVE: f32 = 1.7;

/// Slider 0-100 -> output gain. 70 gives 0.441.
pub fn volume_gain(value: u8) -> f32 {
    0.9 * (value.min(100) as f32 / 100.0).powi(2)
}

/// Slider 0-100 -> lowpass cutoff in Hz. 65 gives ~420 Hz.
pub fn depth_cutoff_hz(value: u8) -> f32 {
    DEPTH_HI_HZ * (DEPTH_LO_HZ / DEPTH_HI_HZ).powf(value.min(100) as f32 / 100.0)
}

/// Slider 0-100 -> lowshelf boost in dB. 58 gives +4.0 dB.
pub fn rumble_gain_db(value: u8) -> f32 {
    RUMBLE_MAX_DB * (value.min(100) as f32 / 100.0).powi(2)
}

/// How local audio files are EQ-processed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum FileTone {
    /// Studio pass-through: preserves original clarity & highs of user music.
    #[default]
    Original,
    /// Cozy lo-fi warmth: gentle ~4.5 kHz rolloff + subtle low warmth.
    Warm,
    /// Applies the user's Depth & Rumble sliders directly.
    Custom,
}

/// The procedural focus soundscapes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Sound {
    Brown,
    Pink,
    White,
    Rain,
    Ocean,
    Fireplace,
    Forest,
    Drone,
}

impl Sound {
    pub const ALL: [Sound; 8] = [
        Sound::Brown,
        Sound::Pink,
        Sound::White,
        Sound::Rain,
        Sound::Ocean,
        Sound::Fireplace,
        Sound::Forest,
        Sound::Drone,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Sound::Brown => "Brown Noise",
            Sound::Pink => "Pink Noise",
            Sound::White => "White Noise",
            Sound::Rain => "Rain",
            Sound::Ocean => "Ocean",
            Sound::Fireplace => "Fireplace",
            Sound::Forest => "Night Forest",
            Sound::Drone => "Deep Focus",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Sound::Brown => "Brown",
            Sound::Pink => "Pink",
            Sound::White => "White",
            Sound::Rain => "Rain",
            Sound::Ocean => "Ocean",
            Sound::Fireplace => "Fire",
            Sound::Forest => "Forest",
            Sound::Drone => "Focus",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Sound::Brown => "deep and warm",
            Sound::Pink => "soft and balanced",
            Sound::White => "bright and even",
            Sound::Rain => "steady rainfall",
            Sound::Ocean => "slow stereo waves",
            Sound::Fireplace => "warm hearth crackle",
            Sound::Forest => "wind & night breeze",
            Sound::Drone => "warm harmonic pad",
        }
    }
}

fn output_gain(kind: Sound) -> f32 {
    match kind {
        Sound::Brown => 0.09 * HEADROOM,
        Sound::White => 1.43 * HEADROOM,
        Sound::Pink => 0.41 * HEADROOM,
        Sound::Rain => 0.66 * HEADROOM,
        Sound::Ocean => 0.13 * HEADROOM,
        Sound::Fireplace => 0.36 * HEADROOM,
        Sound::Forest => 0.45 * HEADROOM,
        Sound::Drone => 0.42 * HEADROOM,
    }
}

/// Mono procedural voice; two independent voices produce rich decorrelated stereo.
struct MonoVoice {
    rng: SmallRng,
    sample_rate: f32,
    // Brown / pink / wind filter states
    brown: f32,
    p0: f32,
    p1: f32,
    p2: f32,
    // Rain droplet / crackle state
    drop: f32,
    drop_decay: f32,
    drop_prev: f32,
    // Fireplace crackle & pop state
    crackle_env: f32,
    crackle_decay: f32,
    crackle_lp: f32,
    pop_env: f32,
    pop_phase: f32,
    pop_freq: f32,
    // Forest cricket shimmer state
    cricket_env: f32,
    cricket_phase: f32,
    cricket_pulse_t: f32,
    // Ocean / Drone phase
    t: f32,
    phase_offset: f32,
    drone_lp: f32,
}

impl MonoVoice {
    fn new(sample_rate: u32, phase_offset: f32) -> Self {
        MonoVoice {
            rng: SmallRng::seed_from_u64(rand::random()),
            sample_rate: sample_rate.max(8000) as f32,
            brown: 0.0,
            p0: 0.0,
            p1: 0.0,
            p2: 0.0,
            drop: 0.0,
            drop_decay: 0.0,
            drop_prev: 0.0,
            crackle_env: 0.0,
            crackle_decay: 0.0,
            crackle_lp: 0.0,
            pop_env: 0.0,
            pop_phase: 0.0,
            pop_freq: 220.0,
            cricket_env: 0.0,
            cricket_phase: 0.0,
            cricket_pulse_t: 0.0,
            t: 0.0,
            phase_offset,
            drone_lp: 0.0,
        }
    }

    #[inline]
    fn pink_step(&mut self, w: f32) -> f32 {
        self.p0 = 0.99765 * self.p0 + w * 0.0990460;
        self.p1 = 0.96300 * self.p1 + w * 0.2965164;
        self.p2 = 0.57000 * self.p2 + w * 1.0526913;
        (self.p0 + self.p1 + self.p2 + w * 0.1848) * 0.25
    }

    #[inline]
    fn brown_step(&mut self, w: f32) -> f32 {
        // Scale integration rate gently relative to 44.1 kHz
        let dt_scale = 44_100.0 / self.sample_rate;
        let leak = 1.0 - 0.002 * dt_scale;
        self.brown = leak * self.brown + (0.02 * dt_scale.sqrt()) * w;
        self.brown * 8.0
    }

    fn next(&mut self, kind: Sound) -> f32 {
        let r = self.rng.random::<f32>();
        let w = r * 2.0 - 1.0;
        let sr = self.sample_rate;

        match kind {
            Sound::White => w,
            Sound::Brown => self.brown_step(w),
            Sound::Pink => self.pink_step(w),
            Sound::Rain => {
                let base = self.pink_step(w);
                if self.rng.random::<f32>() < 26.0 / sr {
                    self.drop = self.rng.random_range(0.28..1.0);
                    self.drop_decay = (-1.0 / (sr * 0.0022)).exp();
                }
                self.drop *= self.drop_decay;
                let drop = self.drop * self.rng.random_range(-1.0..1.0);
                let hp = drop - self.drop_prev;
                self.drop_prev = drop;
                0.62 * base + 0.44 * hp
            }
            Sound::Ocean => {
                let b = self.brown_step(w);
                self.t += 1.0 / sr;
                // Slow rolling wave with subtle L/R phase drift for spatial depth
                let lfo = 0.54
                    + 0.30 * (TAU * 0.07 * self.t + self.phase_offset).sin()
                    + 0.16 * (TAU * 0.125 * self.t + 1.3 + self.phase_offset * 0.5).sin();
                b * lfo.max(0.06)
            }
            Sound::Fireplace => {
                // Warm ember rumble + crisp stochastic crackles + occasional soft wood pops
                let ember = self.brown_step(w) * 0.42;
                let air = self.pink_step(w) * 0.35;

                // Fine high-frequency crackle bursts
                if self.rng.random::<f32>() < 16.0 / sr {
                    self.crackle_env = self.rng.random_range(0.35..1.0);
                    self.crackle_decay = (-1.0 / (sr * self.rng.random_range(0.001..0.0045))).exp();
                }
                self.crackle_env *= self.crackle_decay;
                let raw_crackle = self.crackle_env * self.rng.random_range(-1.0..1.0);
                let crackle_hp = raw_crackle - self.crackle_lp;
                self.crackle_lp = raw_crackle * 0.65 + self.crackle_lp * 0.35;

                // Occasional soft warm wood pop
                if self.rng.random::<f32>() < 1.8 / sr && self.pop_env < 0.02 {
                    self.pop_env = self.rng.random_range(0.35..0.75);
                    self.pop_freq = self.rng.random_range(160.0..420.0);
                    self.pop_phase = 0.0;
                }
                self.pop_env *= (-1.0 / (sr * 0.012)).exp();
                self.pop_phase += TAU * self.pop_freq / sr;
                let pop = self.pop_env * self.pop_phase.sin();

                ember + air + crackle_hp * 0.58 + pop * 0.35
            }
            Sound::Forest => {
                // Breathing wind through leaves + subtle nocturnal shimmer
                self.t += 1.0 / sr;
                let wind_lfo = 0.55
                    + 0.30 * (TAU * 0.05 * self.t + self.phase_offset).sin()
                    + 0.15 * (TAU * 0.11 * self.t + 0.7).sin();
                let wind = self.pink_step(w) * (0.45 + 0.55 * wind_lfo.max(0.0));
                let bed = self.brown_step(w) * 0.25;

                // Gentle distant cricket/night texture (very soft, non-intrusive)
                self.cricket_pulse_t += 1.0 / sr;
                if self.rng.random::<f32>() < 0.6 / sr && self.cricket_env < 0.01 {
                    self.cricket_env = self.rng.random_range(0.08..0.18);
                }
                self.cricket_env *= (-1.0 / (sr * 0.18)).exp();
                self.cricket_phase += TAU * (3800.0 + self.phase_offset * 90.0) / sr;
                let pulse = (TAU * 18.0 * self.cricket_pulse_t).sin().max(0.0);
                let cricket = self.cricket_env * pulse * self.cricket_phase.sin() * 0.35;

                wind + bed + cricket
            }
            Sound::Drone => {
                // Warm harmonic deep-focus pad (A2/E3/A3 detuned + breathing filter + brown bed)
                self.t += 1.0 / sr;
                let detune = 1.0 + self.phase_offset * 0.0022;
                let f1 = 110.0 * detune;
                let f2 = 164.81 * (2.0 - detune);
                let f3 = 220.0 * (1.0 + (TAU * 0.04 * self.t).sin() * 0.0015);
                let breath = 0.72 + 0.28 * (TAU * 0.065 * self.t + self.phase_offset * 0.4).sin();
                let pad = (0.50 * (TAU * f1 * self.t).sin()
                    + 0.30 * (TAU * f2 * self.t).sin()
                    + 0.20 * (TAU * f3 * self.t).sin())
                    * breath;
                // Soft one-pole warming filter on the pad
                let lp_a = 1.0 - (-TAU * 340.0 / sr).exp();
                self.drone_lp += lp_a * (pad - self.drone_lp);
                let bed = self.brown_step(w) * 0.26;
                self.drone_lp * 0.65 + bed
            }
        }
    }
}

/// Stereo voice pair with gentle cross-channel stereo width.
struct StereoVoice {
    left: MonoVoice,
    right: MonoVoice,
}

impl StereoVoice {
    fn new(sample_rate: u32) -> Self {
        StereoVoice {
            left: MonoVoice::new(sample_rate, 0.0),
            right: MonoVoice::new(sample_rate, 0.85),
        }
    }

    #[inline]
    fn next(&mut self, kind: Sound) -> [f32; 2] {
        let l = self.left.next(kind);
        let r = self.right.next(kind);
        // Subtle 15% center glue so headphones feel natural rather than disconnected
        [l * 0.85 + r * 0.15, r * 0.85 + l * 0.15]
    }
}

/// Primary source for the synth.
pub enum Source {
    Live(Sound),
    /// Streaming lock-free reader from `filemode`.
    File(FileStreamReader),
}

/// The real-time stereo synth + optional atmosphere layer.
pub struct Synth {
    sample_rate: u32,
    source: Source,
    voice: StereoVoice,
    out_gain: f32,

    // Optional atmosphere overlay (e.g. Rain over Locked Local Music)
    layer_sound: Option<Sound>,
    layer_voice: StereoVoice,
    layer_gain: f32,
    layer_mix: f32,

    // File clarity mode
    file_tone: FileTone,

    // Depth: stereo one-pole lowpass
    depth_a: f32,
    depth_state: [f32; 2],

    // Rumble: stereo one-pole lowshelf
    rumble_a: f32,
    rumble_gain: f32,
    rumble_state: [f32; 2],

    // Warm preset fixed filter (for FileTone::Warm)
    warm_lp_a: f32,
    warm_lp_state: [f32; 2],

    // Master volume (ramped) and external fade (sleep timer fade-out)
    volume: f32,
    volume_target: f32,
    master_fade: f32,
    master_fade_target: f32,

    // Soft start / pause fade
    fade_samples: usize,
    fade_in_left: usize,
    pause_fade_left: usize,
    pause_fade_total: usize,
    paused_silenced: bool,

    // RMS visualizer feed
    rms_chunk: usize,
    rms_ring: VecDeque<f32>,
    rms_acc: f32,
    rms_n: usize,
}

impl Synth {
    pub fn new(sound: Sound, sample_rate: u32) -> Self {
        let sr = sample_rate.max(8000);
        let fade_samples = (0.15 * sr as f32) as usize;
        let pause_fade_total = (0.065 * sr as f32) as usize;
        let rms_chunk = ((RMS_WINDOW_MS / 1000.0) * sr as f32).max(64.0) as usize;

        let mut s = Synth {
            sample_rate: sr,
            source: Source::Live(sound),
            voice: StereoVoice::new(sr),
            out_gain: output_gain(sound),
            layer_sound: None,
            layer_voice: StereoVoice::new(sr),
            layer_gain: 0.0,
            layer_mix: volume_gain(DEFAULT_LAYER_MIX),
            file_tone: FileTone::Original,
            depth_a: 1.0,
            depth_state: [0.0; 2],
            rumble_a: 0.0,
            rumble_gain: 1.0,
            rumble_state: [0.0; 2],
            warm_lp_a: 1.0 - (-TAU * 4200.0 / sr as f32).exp(),
            warm_lp_state: [0.0; 2],
            volume: volume_gain(DEFAULT_VOLUME),
            volume_target: volume_gain(DEFAULT_VOLUME),
            master_fade: 1.0,
            master_fade_target: 1.0,
            fade_samples,
            fade_in_left: 0,
            pause_fade_left: 0,
            pause_fade_total,
            paused_silenced: false,
            rms_chunk,
            rms_ring: VecDeque::with_capacity(64),
            rms_acc: 0.0,
            rms_n: 0,
        };
        s.set_depth(DEFAULT_DEPTH);
        s.set_rumble(DEFAULT_RUMBLE);
        s
    }

    pub fn set_sound(&mut self, sound: Sound) {
        self.source = Source::Live(sound);
        self.voice = StereoVoice::new(self.sample_rate);
        self.out_gain = output_gain(sound);
        self.depth_state = [0.0; 2];
        self.rumble_state = [0.0; 2];
        self.fade_in_left = self.fade_samples;
        self.paused_silenced = false;
    }

    pub fn set_file_stream(&mut self, reader: FileStreamReader) {
        self.source = Source::File(reader);
        self.out_gain = 1.0;
        self.depth_state = [0.0; 2];
        self.rumble_state = [0.0; 2];
        self.warm_lp_state = [0.0; 2];
        self.fade_in_left = self.fade_samples;
        self.paused_silenced = false;
    }

    pub fn set_layer(&mut self, sound: Option<Sound>) {
        if self.layer_sound != sound {
            self.layer_sound = sound;
            if let Some(s) = sound {
                self.layer_voice = StereoVoice::new(self.sample_rate);
                self.layer_gain = output_gain(s);
            } else {
                self.layer_gain = 0.0;
            }
        }
    }

    pub fn set_layer_mix(&mut self, mix: u8) {
        // Gentle curve up to 0.85 of normal layer level
        self.layer_mix = (mix.min(100) as f32 / 100.0).powf(1.5) * 0.9;
    }

    pub fn set_file_tone(&mut self, tone: FileTone) {
        self.file_tone = tone;
    }

    pub fn restart(&mut self) {
        match &mut self.source {
            Source::Live(_) => {
                self.voice = StereoVoice::new(self.sample_rate);
            }
            Source::File(reader) => {
                reader.rewind();
            }
        }
        self.depth_state = [0.0; 2];
        self.rumble_state = [0.0; 2];
        self.warm_lp_state = [0.0; 2];
        self.rms_ring.clear();
        self.fade_in_left = self.fade_samples;
        self.paused_silenced = false;
    }

    pub fn set_volume(&mut self, value: u8) {
        self.volume_target = volume_gain(value);
    }

    pub fn set_depth(&mut self, value: u8) {
        let cutoff = depth_cutoff_hz(value);
        self.depth_a = 1.0 - (-TAU * cutoff / self.sample_rate as f32).exp();
    }

    pub fn set_rumble(&mut self, value: u8) {
        self.rumble_gain = (10f32.powf(rumble_gain_db(value) / 20.0) - 1.0) * RUMBLE_DRIVE;
        self.rumble_a = 1.0 - (-TAU * RUMBLE_SHELF_HZ / self.sample_rate as f32).exp();
    }

    pub fn set_master_fade(&mut self, factor: f32) {
        self.master_fade_target = factor.clamp(0.0, 1.0);
    }

    pub fn soft_start(&mut self) {
        self.fade_in_left = self.fade_samples;
        self.pause_fade_left = 0;
        self.paused_silenced = false;
        self.master_fade = 1.0;
        self.master_fade_target = 1.0;
    }

    pub fn begin_pause_fade(&mut self) {
        if !self.paused_silenced && self.pause_fade_left == 0 {
            self.pause_fade_left = self.pause_fade_total;
        }
    }

    pub fn is_pause_fade_done(&self) -> bool {
        self.paused_silenced
    }

    pub fn drain_rms(&mut self) -> Vec<f32> {
        self.rms_ring.drain(..).collect()
    }

    #[cfg(test)]
    pub fn rms_values(&self) -> Vec<f32> {
        self.rms_ring.iter().copied().collect()
    }

    #[inline]
    fn produce_primary(&mut self) -> ([f32; 2], bool) {
        match &mut self.source {
            Source::Live(kind) => {
                let kind = *kind;
                (self.voice.next(kind), false)
            }
            Source::File(reader) => (reader.next_frame(), true),
        }
    }

    /// Render interleaved stereo `f32` frames (`out.len()` must be even: `[L, R, L, R, ...]`).
    pub fn render_stereo(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        if frames == 0 {
            return;
        }
        if self.paused_silenced {
            out.fill(0.0);
            return;
        }

        let gain = self.out_gain;
        let depth_a = self.depth_a;
        let mut depth_s = self.depth_state;
        let rumble_a = self.rumble_a;
        let rumble_g = self.rumble_gain;
        let mut rumble_s = self.rumble_state;
        let warm_a = self.warm_lp_a;
        let mut warm_s = self.warm_lp_state;
        let file_tone = self.file_tone;

        let mut vol = self.volume;
        let vol_t = self.volume_target;
        let ramp_len = frames.min((0.02 * self.sample_rate as f32) as usize).max(1) as f32;
        let vol_step = (vol_t - vol) / ramp_len;

        let mut mf = self.master_fade;
        let mf_t = self.master_fade_target;
        let mf_step = (mf_t - mf) / ramp_len;

        let mut fade_in = self.fade_in_left;
        let fade_total = self.fade_samples.max(1);

        let mut pause_left = self.pause_fade_left;
        let pause_total = self.pause_fade_total.max(1);

        let layer_kind = self.layer_sound;
        let layer_g = self.layer_gain * self.layer_mix;

        let mut acc = self.rms_acc;
        let mut cnt = self.rms_n;
        let rms_chunk = self.rms_chunk;

        for frame in out.as_chunks_mut::<2>().0 {
            let (raw, is_file) = self.produce_primary();

            // Shape primary channel
            let mut shaped = [0.0f32; 2];
            for ch in 0..2 {
                let x = raw[ch];
                if is_file {
                    match file_tone {
                        FileTone::Original => {
                            shaped[ch] = x * gain;
                        }
                        FileTone::Warm => {
                            warm_s[ch] += warm_a * (x - warm_s[ch]);
                            rumble_s[ch] += rumble_a * (x - rumble_s[ch]);
                            shaped[ch] = (warm_s[ch] + rumble_s[ch] * 0.35) * gain;
                        }
                        FileTone::Custom => {
                            depth_s[ch] += depth_a * (x - depth_s[ch]);
                            rumble_s[ch] += rumble_a * (x - rumble_s[ch]);
                            shaped[ch] = (depth_s[ch] + rumble_s[ch] * rumble_g) * gain;
                        }
                    }
                } else {
                    depth_s[ch] += depth_a * (x - depth_s[ch]);
                    rumble_s[ch] += rumble_a * (x - rumble_s[ch]);
                    shaped[ch] = (depth_s[ch] + rumble_s[ch] * rumble_g) * gain;
                }
            }

            // Optional atmosphere layer (gentle warm shaping)
            if let Some(lk) = layer_kind {
                let lx = self.layer_voice.next(lk);
                shaped[0] += lx[0] * layer_g;
                shaped[1] += lx[1] * layer_g;
            }

            // Apply volume & master fade
            let mut l = shaped[0] * vol * mf;
            let mut r = shaped[1] * vol * mf;

            if vol != vol_t {
                vol += vol_step;
                if (vol_t - vol) * vol_step <= 0.0 {
                    vol = vol_t;
                }
            }
            if mf != mf_t {
                mf += mf_step;
                if (mf_t - mf) * mf_step <= 0.0 {
                    mf = mf_t;
                }
            }

            if fade_in > 0 {
                let env = (fade_total - fade_in) as f32 / fade_total as f32;
                l *= env;
                r *= env;
                fade_in -= 1;
            }

            if pause_left > 0 {
                let env = pause_left as f32 / pause_total as f32;
                l *= env;
                r *= env;
                pause_left -= 1;
                if pause_left == 0 {
                    self.paused_silenced = true;
                }
            } else if self.paused_silenced {
                l = 0.0;
                r = 0.0;
            }

            // Soft peak safety knee
            l = soft_knee(l);
            r = soft_knee(r);

            frame[0] = l;
            frame[1] = r;

            let mono_sq = 0.5 * (l * l + r * r);
            acc += mono_sq;
            cnt += 1;
            if cnt >= rms_chunk {
                let rms = (acc / cnt as f32).sqrt() * RMS_SCALE;
                if self.rms_ring.len() == 64 {
                    self.rms_ring.pop_front();
                }
                self.rms_ring.push_back((0.12 + rms).min(1.0));
                acc = 0.0;
                cnt = 0;
            }
        }

        self.depth_state = depth_s;
        self.rumble_state = rumble_s;
        self.warm_lp_state = warm_s;
        self.volume = vol;
        self.master_fade = mf;
        self.fade_in_left = fade_in;
        self.pause_fade_left = pause_left;
        self.rms_acc = acc;
        self.rms_n = cnt;
    }
}

#[inline]
fn soft_knee(v: f32) -> f32 {
    if v > PEAK_KNEE {
        PEAK_KNEE + 0.11 * ((v - PEAK_KNEE) / 0.11).tanh()
    } else if v < -PEAK_KNEE {
        -PEAK_KNEE - 0.11 * ((-v - PEAK_KNEE) / 0.11).tanh()
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms_stereo(samples: &[f32]) -> f32 {
        let sum: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
        (sum / samples.len() as f64).sqrt() as f32
    }

    #[test]
    fn slider_curves_match_reference() {
        assert!((volume_gain(70) - 0.441).abs() < 0.002);
        assert!((depth_cutoff_hz(65) - 420.0).abs() < 15.0);
        assert!((rumble_gain_db(58) - 4.0).abs() < 0.15);
        assert_eq!(volume_gain(0), 0.0);
        assert!((volume_gain(100) - 0.9).abs() < 1e-6);
    }

    #[test]
    fn every_sound_renders_stereo_without_clipping() {
        for sound in Sound::ALL {
            let mut synth = Synth::new(sound, 48_000);
            let mut buf = vec![0.0f32; 48_000 * 2];
            synth.render_stereo(&mut buf);
            assert!(buf.iter().any(|&s| s != 0.0), "{:?} is silent", sound);
            assert!(buf.iter().all(|&s| s.abs() < 1.0), "{:?} clipped", sound);
            let r = rms_stereo(&buf);
            assert!(r > 0.01 && r < 0.55, "{:?} rms {} out of band", sound, r);
        }
    }

    #[test]
    fn all_sounds_land_in_similar_loudness_band() {
        let mut levels = Vec::new();
        for sound in Sound::ALL {
            let mut synth = Synth::new(sound, 44_100);
            let mut buf = vec![0.0f32; 44_100 * 2];
            synth.render_stereo(&mut buf);
            levels.push((sound, rms_stereo(&buf)));
        }
        let min = levels.iter().map(|(_, r)| *r).fold(f32::MAX, f32::min);
        let max = levels.iter().map(|(_, r)| *r).fold(f32::MIN, f32::max);
        assert!(max / min < 3.5, "loudness spread too wide: {:?}", levels);
    }

    #[test]
    fn pause_fade_ramps_smoothly_to_silence() {
        let mut synth = Synth::new(Sound::Brown, 44_100);
        let mut warmup = vec![0.0f32; 44_100 * 2];
        synth.render_stereo(&mut warmup);
        synth.begin_pause_fade();
        let mut tail = vec![0.0f32; 44_100 * 2];
        synth.render_stereo(&mut tail);
        assert!(synth.is_pause_fade_done());
        // Last 1000 samples must be exactly zero
        assert!(tail[tail.len() - 1000..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn atmosphere_layer_blends_cleanly() {
        let mut synth = Synth::new(Sound::Drone, 44_100);
        synth.set_layer(Some(Sound::Rain));
        synth.set_layer_mix(50);
        let mut buf = vec![0.0f32; 44_100 * 2];
        synth.render_stereo(&mut buf);
        assert!(buf.iter().all(|&s| s.abs() < 1.0));
        assert!(!synth.rms_values().is_empty());
    }
}
