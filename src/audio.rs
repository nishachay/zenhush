//! Adaptive stereo audio output engine built on cpal.
//!
//! Runs synthesis and file-streaming in a real-time non-blocking callback
//! at the output device's native sample rate and stereo channel layout.
//! Supports smooth click-free pause fade-out followed by `stream.pause()`
//! so paused background usage consumes ~0% CPU.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream, StreamConfig};

use crate::filemode::{FileStreamHandle, FileStreamReader};
use crate::synth::{
    FileTone, Sound, Synth, DEFAULT_DEPTH, DEFAULT_LAYER_MIX, DEFAULT_RUMBLE,
    DEFAULT_SAMPLE_RATE, DEFAULT_VOLUME,
};

enum AudioCmd {
    SetSound(Sound),
    SetFile(FileStreamReader),
    SetLayer(Option<Sound>),
    SetLayerMix(u8),
    SetFileTone(FileTone),
    SetVolume(u8),
    SetDepth(u8),
    SetRumble(u8),
    SetMasterFade(f32),
    SoftStart,
    BeginPauseFade,
    Restart,
}

pub struct AudioEngine {
    stream: Option<Stream>,
    stream_paused: bool,
    device_name: String,
    sample_rate: u32,
    last_device_poll: Instant,
    rebuild_backoff_secs: u64,
    cmd_tx: Sender<AudioCmd>,
    rms_rx: rtrb::Consumer<f32>,
    pause_fade_done: Arc<AtomicBool>,

    // Authoritative settings (used to rebuild stream identically on device hotplug)
    sound: Sound,
    is_file: bool,
    file_handle: Option<FileStreamHandle>,
    file_name: Option<String>,
    layer_sound: Option<Sound>,
    layer_mix: u8,
    file_tone: FileTone,
    volume: u8,
    muted: bool,
    depth: u8,
    rumble: u8,
    playing: bool,

    error: Arc<Mutex<Option<String>>>,
}

impl AudioEngine {
    pub fn new() -> Self {
        let (rms_tx, rms_rx) = rtrb::RingBuffer::new(256);
        let (cmd_tx, cmd_rx) = channel::<AudioCmd>();
        let error = Arc::new(Mutex::new(None));
        let pause_fade_done = Arc::new(AtomicBool::new(false));

        let mut engine = AudioEngine {
            stream: None,
            stream_paused: false,
            device_name: String::new(),
            sample_rate: DEFAULT_SAMPLE_RATE,
            last_device_poll: Instant::now(),
            rebuild_backoff_secs: 4,
            cmd_tx,
            rms_rx,
            pause_fade_done,
            sound: Sound::Brown,
            is_file: false,
            file_handle: None,
            file_name: None,
            layer_sound: None,
            layer_mix: DEFAULT_LAYER_MIX,
            file_tone: FileTone::Original,
            volume: DEFAULT_VOLUME,
            muted: false,
            depth: DEFAULT_DEPTH,
            rumble: DEFAULT_RUMBLE,
            playing: false,
            error: error.clone(),
        };

        if let Err(e) = engine.build_stream(rms_tx, cmd_rx, error) {
            if let Ok(mut slot) = engine.error.lock() {
                *slot = Some(e);
            }
        }
        engine
    }

    fn default_device() -> Result<Device, String> {
        let host = cpal::default_host();
        host.default_output_device()
            .ok_or_else(|| "No audio — check output".to_string())
    }

    fn effective_volume(&self) -> u8 {
        if self.muted {
            0
        } else {
            self.volume
        }
    }

    fn build_stream(
        &mut self,
        mut rms_tx: rtrb::Producer<f32>,
        cmd_rx: Receiver<AudioCmd>,
        error: Arc<Mutex<Option<String>>>,
    ) -> Result<(), String> {
        let device = Self::default_device()?;
        self.device_name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "Default output".to_string());

        let (native_rate, native_channels) = match device.default_output_config() {
            Ok(cfg) => (cfg.sample_rate().clamp(22_050, 192_000), cfg.channels()),
            Err(_) => (DEFAULT_SAMPLE_RATE, 2),
        };
        let out_channels: u16 = if native_channels >= 2 { 2 } else { 1 };
        self.sample_rate = native_rate;

        let config = StreamConfig {
            channels: out_channels,
            sample_rate: native_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let mut synth = Synth::new(self.sound, native_rate);
        synth.set_volume(self.effective_volume());
        synth.set_depth(self.depth);
        synth.set_rumble(self.rumble);
        synth.set_layer(self.layer_sound);
        synth.set_layer_mix(self.layer_mix);
        synth.set_file_tone(self.file_tone);

        if self.is_file {
            if let Some(handle) = &self.file_handle {
                if let Ok(reader) = handle.open_reader(native_rate) {
                    synth.set_file_stream(reader);
                }
            }
        }
        if self.playing {
            synth.soft_start();
        }

        let pause_done_flag = Arc::new(AtomicBool::new(!self.playing));
        self.pause_fade_done = pause_done_flag.clone();

        let mut active = self.playing;
        let mut stereo_buf: Vec<f32> = Vec::new();
        let err_clone = error.clone();
        let ch_count = out_channels as usize;

        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    while let Ok(cmd) = cmd_rx.try_recv() {
                        match cmd {
                            AudioCmd::SetSound(s) => {
                                synth.set_sound(s);
                                active = true;
                                pause_done_flag.store(false, Ordering::Release);
                            }
                            AudioCmd::SetFile(reader) => {
                                synth.set_file_stream(reader);
                                active = true;
                                pause_done_flag.store(false, Ordering::Release);
                            }
                            AudioCmd::SetLayer(l) => synth.set_layer(l),
                            AudioCmd::SetLayerMix(m) => synth.set_layer_mix(m),
                            AudioCmd::SetFileTone(t) => synth.set_file_tone(t),
                            AudioCmd::SetVolume(v) => synth.set_volume(v),
                            AudioCmd::SetDepth(d) => synth.set_depth(d),
                            AudioCmd::SetRumble(r) => synth.set_rumble(r),
                            AudioCmd::SetMasterFade(f) => synth.set_master_fade(f),
                            AudioCmd::SoftStart => {
                                synth.soft_start();
                                active = true;
                                pause_done_flag.store(false, Ordering::Release);
                            }
                            AudioCmd::BeginPauseFade => {
                                synth.begin_pause_fade();
                            }
                            AudioCmd::Restart => {
                                synth.restart();
                                active = true;
                                pause_done_flag.store(false, Ordering::Release);
                            }
                        }
                    }

                    if !active {
                        data.fill(0.0);
                        return;
                    }

                    let frames = data.len() / ch_count.max(1);
                    if stereo_buf.len() != frames * 2 {
                        stereo_buf.resize(frames * 2, 0.0);
                    }
                    synth.render_stereo(&mut stereo_buf);

                    if ch_count == 2 {
                        data.copy_from_slice(&stereo_buf);
                    } else {
                        for (i, out_s) in data.iter_mut().enumerate() {
                            let l = stereo_buf[i * 2];
                            let r = stereo_buf[i * 2 + 1];
                            *out_s = 0.5 * (l + r);
                        }
                    }

                    if synth.is_pause_fade_done() {
                        active = false;
                        pause_done_flag.store(true, Ordering::Release);
                    }

                    for v in synth.drain_rms() {
                        let _ = rms_tx.push(v);
                    }
                },
                move |err| {
                    let _ = err;
                    if let Ok(mut slot) = err_clone.lock() {
                        *slot = Some("Audio lost — check output".to_string());
                    }
                },
                None,
            )
            .map_err(|_| "No audio — check output".to_string())?;

        if self.playing {
            stream
                .play()
                .map_err(|_| "No audio — check output".to_string())?;
            self.stream_paused = false;
        } else {
            self.stream_paused = true;
        }

        self.stream = Some(stream);
        if let Ok(mut slot) = self.error.lock() {
            *slot = None;
        }
        Ok(())
    }

    fn rebuild_stream(&mut self) -> bool {
        let (rms_tx, rms_rx) = rtrb::RingBuffer::new(256);
        let (cmd_tx, cmd_rx) = channel::<AudioCmd>();
        self.rms_rx = rms_rx;
        self.cmd_tx = cmd_tx;
        let err = self.error.clone();
        match self.build_stream(rms_tx, cmd_rx, err) {
            Ok(()) => true,
            Err(e) => {
                if let Ok(mut slot) = self.error.lock() {
                    *slot = Some(e);
                }
                false
            }
        }
    }

    /// Periodic maintenance: follow default output device hotplugs & suspend
    /// the audio stream once a pause fade-out finishes (0% idle CPU).
    pub fn maintain(&mut self) {
        if !self.playing
            && !self.stream_paused
            && self.pause_fade_done.load(Ordering::Acquire)
        {
            if let Some(stream) = &self.stream {
                let _ = stream.pause();
                self.stream_paused = true;
            }
        }

        // Poll default output device at a gentle cadence with backoff on errors
        // so missing/virtual devices never spin ALSA/PipeWire every second.
        let poll_interval = if self.stream.is_none() || self.audio_error().is_some() {
            Duration::from_secs(self.rebuild_backoff_secs)
        } else {
            Duration::from_secs(4)
        };
        if !self.playing || self.last_device_poll.elapsed() < poll_interval {
            return;
        }
        self.last_device_poll = Instant::now();

        let current = match Self::default_device() {
            Ok(d) => d
                .description()
                .map(|x| x.name().to_string())
                .unwrap_or_default(),
            Err(_) => {
                self.rebuild_backoff_secs = (self.rebuild_backoff_secs * 2).min(30);
                return;
            }
        };
        let needs_rebuild =
            self.stream.is_none() || (!current.is_empty() && current != self.device_name);
        if needs_rebuild {
            if self.rebuild_stream() {
                self.rebuild_backoff_secs = 4;
            } else {
                self.rebuild_backoff_secs = (self.rebuild_backoff_secs * 2).min(30);
            }
        } else {
            self.rebuild_backoff_secs = 4;
        }
    }

    fn ensure_playing_stream(&mut self) {
        let current_dev = Self::default_device()
            .ok()
            .and_then(|d| d.description().ok().map(|x| x.name().to_string()));
        let dev_changed = current_dev
            .as_deref()
            .map(|n| n != self.device_name)
            .unwrap_or(false);
        if dev_changed || self.stream.is_none() || self.audio_error().is_some() {
            self.rebuild_stream();
        }
        if self.stream_paused {
            if let Some(stream) = &self.stream {
                let _ = stream.play();
                self.stream_paused = false;
            }
        }
    }

    pub fn device_name(&self) -> &str {
        if self.device_name.is_empty() {
            "System Default"
        } else {
            &self.device_name
        }
    }

    pub fn audio_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|g| g.clone())
    }

    pub fn drain_rms(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        while let Ok(v) = self.rms_rx.pop() {
            out.push(v);
        }
        out
    }

    pub fn has_loaded_source(&self) -> bool {
        !self.is_file || self.file_handle.is_some()
    }

    pub fn is_file(&self) -> bool {
        self.is_file
    }

    pub fn volume(&self) -> u8 {
        self.volume
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn depth(&self) -> u8 {
        self.depth
    }

    pub fn rumble(&self) -> u8 {
        self.rumble
    }

    fn send(&self, cmd: AudioCmd) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn play_sound(&mut self, sound: Sound) {
        self.sound = sound;
        self.is_file = false;
        self.playing = true;
        self.ensure_playing_stream();
        self.send(AudioCmd::SetSound(sound));
        self.send(AudioCmd::SetMasterFade(1.0));
    }

    pub fn play_file(&mut self, handle: FileStreamHandle, name: String) -> Result<(), String> {
        let reader = handle.open_reader(self.sample_rate)?;
        self.is_file = true;
        self.file_handle = Some(handle);
        self.file_name = Some(name);
        self.playing = true;
        self.ensure_playing_stream();
        self.send(AudioCmd::SetFile(reader));
        self.send(AudioCmd::SetMasterFade(1.0));
        Ok(())
    }

    pub fn pause(&mut self) {
        if self.playing {
            self.playing = false;
            self.pause_fade_done.store(false, Ordering::Release);
            self.send(AudioCmd::BeginPauseFade);
        }
    }

    pub fn resume(&mut self) {
        if !self.playing {
            self.playing = true;
            self.ensure_playing_stream();
            self.send(AudioCmd::SoftStart);
        }
    }

    pub fn restart(&mut self) {
        self.playing = true;
        self.ensure_playing_stream();
        self.send(AudioCmd::Restart);
    }

    pub fn set_volume(&mut self, v: u8) {
        self.volume = v.min(100);
        if self.muted && self.volume > 0 {
            self.muted = false;
        }
        self.send(AudioCmd::SetVolume(self.effective_volume()));
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.send(AudioCmd::SetVolume(self.effective_volume()));
    }

    pub fn set_depth(&mut self, v: u8) {
        self.depth = v.min(100);
        self.send(AudioCmd::SetDepth(self.depth));
    }

    pub fn set_rumble(&mut self, v: u8) {
        self.rumble = v.min(100);
        self.send(AudioCmd::SetRumble(self.rumble));
    }

    pub fn set_layer(&mut self, layer: Option<Sound>) {
        self.layer_sound = layer;
        self.send(AudioCmd::SetLayer(layer));
    }

    pub fn set_layer_mix(&mut self, mix: u8) {
        self.layer_mix = mix.min(100);
        self.send(AudioCmd::SetLayerMix(self.layer_mix));
    }

    pub fn set_file_tone(&mut self, tone: FileTone) {
        self.file_tone = tone;
        self.send(AudioCmd::SetFileTone(tone));
    }

    pub fn set_crossfade(&mut self, enabled: bool) {
        if let Some(h) = &self.file_handle {
            h.set_crossfade(enabled);
        }
    }

    pub fn set_master_fade(&self, factor: f32) {
        self.send(AudioCmd::SetMasterFade(factor));
    }
}
