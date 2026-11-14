//! Audio output engine: cpal stream + the live synth.
//!
//! The synth lives in the audio callback (real-time thread) so samples
//! are generated on demand with no extra copies. The UI thread controls
//! it by sending commands over a channel; the callback drains the
//! channel at the start of each buffer and applies them. The callback
//! never blocks: it only reads the channel (try_recv) and writes samples.
//!
//! Pause uses the stream's own pause/play so the audio device can sleep
//! when the user pauses. Resume fades in gently (soft start).

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream, StreamConfig};

use crate::synth::{Sound, Synth, SAMPLE_RATE};

/// Commands the UI thread sends to the audio callback.
enum AudioCmd {
    SetSound(Sound),
    SetFile(Arc<Vec<f32>>),
    SetVolume(u8),
    SetDepth(u8),
    SetRumble(u8),
    SetPlaying(bool),
    SoftStart,
    Restart,
}

/// The audio engine. Owns the cpal stream and the authoritative settings
/// (so a device-change rebuild can recreate the synth identically).
pub struct AudioEngine {
    stream: Option<Stream>,
    device_name: String,
    cmd_tx: Sender<AudioCmd>,
    /// RMS values from the callback, for the visualizer (lock-free).
    rms_rx: rtrb::Consumer<f32>,
    // Authoritative settings, kept here for rebuilds.
    sound: Sound,
    is_file: bool,
    file_samples: Option<Arc<Vec<f32>>>,
    file_name: Option<String>,
    volume: u8,
    depth: u8,
    rumble: u8,
    playing: bool,
    /// Last audio error, if the device failed (shown in the UI).
    error: Arc<Mutex<Option<String>>>,
}

impl AudioEngine {
    /// Build the engine on the default output device (44.1 kHz mono f32).
    /// Never fails: if there is no audio device, the error is recorded
    /// and the UI shows it instead of "Live" over silence. Resume will
    /// retry building the stream.
    pub fn new() -> Self {
        let (rms_tx, rms_rx) = rtrb::RingBuffer::new(256);
        let (cmd_tx, cmd_rx) = channel::<AudioCmd>();
        let error = Arc::new(Mutex::new(None));
        let mut engine = AudioEngine {
            stream: None,
            device_name: String::new(),
            cmd_tx,
            rms_rx,
            sound: Sound::Brown,
            is_file: false,
            file_samples: None,
            file_name: None,
            volume: crate::synth::DEFAULT_VOLUME,
            depth: crate::synth::DEFAULT_DEPTH,
            rumble: crate::synth::DEFAULT_RUMBLE,
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
            .ok_or_else(|| "No audio device".to_string())
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
            .unwrap_or_else(|_| "Unknown device".to_string());

        let config = StreamConfig {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            buffer_size: cpal::BufferSize::Default,
        };

        let mut synth = Synth::new(self.sound);
        synth.set_volume(self.volume);
        synth.set_depth(self.depth);
        synth.set_rumble(self.rumble);
        if let Some(samples) = &self.file_samples {
            synth.set_file((**samples).clone());
        }
        if self.playing {
            synth.soft_start();
        }

        let mut playing = self.playing;
        let mut buf: Vec<i16> = Vec::new();
        let err_clone = error.clone();

        let stream = device.build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Apply any pending commands first.
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        AudioCmd::SetSound(s) => synth.set_sound(s),
                        AudioCmd::SetFile(samples) => synth.set_file((*samples).clone()),
                        AudioCmd::SetVolume(v) => synth.set_volume(v),
                        AudioCmd::SetDepth(d) => synth.set_depth(d),
                        AudioCmd::SetRumble(r) => synth.set_rumble(r),
                        AudioCmd::SetPlaying(p) => playing = p,
                        AudioCmd::SoftStart => synth.soft_start(),
                        AudioCmd::Restart => synth.restart(),
                    }
                }
                if !playing {
                    for s in data.iter_mut() {
                        *s = 0.0;
                    }
                    return;
                }
                if buf.len() != data.len() {
                    buf.resize(data.len(), 0);
                }
                synth.render(&mut buf);
                for (i, s) in data.iter_mut().enumerate() {
                    *s = buf[i] as f32 / 32768.0;
                }
                // Drain new RMS values to the UI (lock-free ring).
                for v in synth.drain_rms() {
                    let _ = rms_tx.push(v);
                }
            },
            move |err| {
                // The device failed: record it so the UI can say so
                // instead of showing "Live" over silence.
                if let Ok(mut slot) = err_clone.lock() {
                    *slot = Some(format!("No audio device ({})", err));
                }
            },
            None,
        );

        let stream = match stream {
            Ok(s) => s,
            Err(e) => return Err(format!("No audio device ({})", e)),
        };
        if let Err(e) = stream.play() {
            return Err(format!("No audio device ({})", e));
        }
        self.stream = Some(stream);
        Ok(())
    }

    /// Rebuild the stream if the default output device changed.
    /// Call this periodically from the UI. Returns true if it rebuilt.
    pub fn rebuild_if_device_changed(&mut self) -> bool {
        let current = match Self::default_device() {
            Ok(d) => d
                .description()
                .map(|x| x.name().to_string())
                .unwrap_or_default(),
            Err(_) => return false,
        };
        if current == self.device_name && self.stream.is_some() {
            return false;
        }
        // Rebuild: fresh channel + ring so the old callback drops cleanly.
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

    /// The current default device name (for display / debugging).
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// Last audio error, if the device failed.
    pub fn audio_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|g| g.clone())
    }

    /// Drain new RMS values (oldest first) for the visualizer.
    pub fn drain_rms(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        while let Ok(v) = self.rms_rx.pop() {
            out.push(v);
        }
        out
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn is_file(&self) -> bool {
        self.is_file
    }

    pub fn sound(&self) -> Sound {
        self.sound
    }

    pub fn file_name(&self) -> Option<&str> {
        self.file_name.as_deref()
    }

    pub fn volume(&self) -> u8 {
        self.volume
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
        self.file_samples = None;
        self.file_name = None;
        self.send(AudioCmd::SetSound(sound));
        if !self.playing {
            self.playing = true;
            self.send(AudioCmd::SetPlaying(true));
            self.send(AudioCmd::SoftStart);
        }
    }

    pub fn play_file(&mut self, samples: Arc<Vec<f32>>, name: String) {
        self.is_file = true;
        self.file_samples = Some(samples.clone());
        self.file_name = Some(name);
        self.send(AudioCmd::SetFile(samples));
        if !self.playing {
            self.playing = true;
            self.send(AudioCmd::SetPlaying(true));
            self.send(AudioCmd::SoftStart);
        }
    }

    pub fn pause(&mut self) {
        if self.playing {
            self.playing = false;
            self.send(AudioCmd::SetPlaying(false));
        }
    }

    pub fn resume(&mut self) {
        if !self.playing {
            // If the device died while paused, rebuild the stream so
            // resume actually restarts it instead of doing nothing.
            if self.stream.is_none() || self.audio_error().is_some() {
                let (rms_tx, rms_rx) = rtrb::RingBuffer::new(256);
                let (cmd_tx, cmd_rx) = channel::<AudioCmd>();
                self.rms_rx = rms_rx;
                self.cmd_tx = cmd_tx;
                if let Ok(mut slot) = self.error.lock() {
                    *slot = None;
                }
                let err = self.error.clone();
                let _ = self.build_stream(rms_tx, cmd_rx, err);
            }
            self.playing = true;
            self.send(AudioCmd::SetPlaying(true));
            self.send(AudioCmd::SoftStart);
        }
    }

    pub fn restart(&mut self) {
        self.send(AudioCmd::Restart);
    }

    pub fn set_volume(&mut self, v: u8) {
        self.volume = v;
        self.send(AudioCmd::SetVolume(v));
    }

    pub fn set_depth(&mut self, v: u8) {
        self.depth = v;
        self.send(AudioCmd::SetDepth(v));
    }

    pub fn set_rumble(&mut self, v: u8) {
        self.rumble = v;
        self.send(AudioCmd::SetRumble(v));
    }
}
