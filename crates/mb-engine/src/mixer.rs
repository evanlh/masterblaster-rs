//! Main playback engine.

use alloc::vec::Vec;
use mb_ir::{Event, EventPayload, EventTarget, Song, Timestamp};

use crate::channel::ChannelState;
use crate::event_queue::EventQueue;
use crate::frame::Frame;

/// The main playback engine.
pub struct Engine {
    /// The song being played
    song: Song,
    /// Channel states
    channels: Vec<ChannelState>,
    /// Event queue
    event_queue: EventQueue,
    /// Current playback position
    current_time: Timestamp,
    /// Audio sample rate (e.g., 44100)
    sample_rate: u32,
    /// Samples per tick at current tempo
    samples_per_tick: u32,
    /// Sample counter within current tick
    sample_counter: u32,
    /// Current tempo (BPM)
    tempo: u8,
    /// Current speed (ticks per row)
    speed: u8,
    /// Is playback active?
    playing: bool,
}

impl Engine {
    /// Create a new engine for the given song.
    pub fn new(song: Song, sample_rate: u32) -> Self {
        let num_channels = song.channels.len();
        let tempo = song.initial_tempo;
        let speed = song.initial_speed;

        let mut engine = Self {
            song,
            channels: Vec::new(),
            event_queue: EventQueue::new(),
            current_time: Timestamp::from_ticks(0),
            sample_rate,
            samples_per_tick: 0,
            sample_counter: 0,
            tempo,
            speed,
            playing: false,
        };

        // Initialize channels
        for _ in 0..num_channels {
            engine.channels.push(ChannelState::new());
        }

        engine.update_samples_per_tick();
        engine
    }

    /// Update samples_per_tick based on current tempo.
    fn update_samples_per_tick(&mut self) {
        // BPM = tempo, ticks per beat = speed * rows_per_beat (assume 4)
        // samples_per_tick = sample_rate * 60 / (tempo * 24)
        // Standard: 2500 / tempo * sample_rate / 1000 (approx)
        self.samples_per_tick = (self.sample_rate * 5) / (self.tempo as u32 * 2);
    }

    /// Start playback.
    pub fn play(&mut self) {
        self.playing = true;
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.playing = false;
    }

    /// Seek to a position.
    pub fn seek(&mut self, time: Timestamp) {
        self.current_time = time;
        self.sample_counter = 0;
        self.event_queue.clear();
        // TODO: Re-schedule events from patterns
    }

    /// Generate one frame of audio.
    pub fn render_frame(&mut self) -> Frame {
        if !self.playing {
            return Frame::silence();
        }

        // 1. Process events at current time
        for event in self.event_queue.pop_until(self.current_time) {
            self.dispatch_event(&event);
        }

        // 2. Mix all channels
        let output = self.mix_channels();

        // 3. Advance time
        self.sample_counter += 1;
        if self.sample_counter >= self.samples_per_tick {
            self.sample_counter = 0;
            self.current_time.tick += 1;
            self.current_time.subtick = 0;
            self.process_tick();
        } else {
            // Interpolate subtick
            self.current_time.subtick =
                ((self.sample_counter as u64 * 65536) / self.samples_per_tick as u64) as u16;
        }

        output
    }

    /// Process a tick (called once per tick).
    fn process_tick(&mut self) {
        // Update effect state for each channel
        for channel in &mut self.channels {
            if !channel.playing {
                continue;
            }
            // TODO: Process per-tick effects (vibrato, volume slide, etc.)
        }
    }

    /// Dispatch an event to its target.
    fn dispatch_event(&mut self, event: &Event) {
        match event.target {
            EventTarget::Channel(ch) => {
                self.apply_channel_event(ch, &event.payload);
            }
            EventTarget::Global => {
                self.apply_global_event(&event.payload);
            }
            EventTarget::Node(_id) => {
                // TODO: Route to graph node
            }
        }
    }

    /// Apply an event to a channel.
    fn apply_channel_event(&mut self, ch: u8, payload: &EventPayload) {
        match payload {
            EventPayload::NoteOn {
                note,
                instrument,
                velocity: _,
            } => {
                // Look up sample from instrument
                let sample_idx = self
                    .song
                    .instruments
                    .get(*instrument as usize)
                    .map(|inst| inst.sample_map[*note as usize])
                    .unwrap_or(*instrument);

                if let Some(channel) = self.channels.get_mut(ch as usize) {
                    channel.trigger(*note, *instrument, sample_idx);
                }
            }
            EventPayload::NoteOff { note: _ } => {
                if let Some(channel) = self.channels.get_mut(ch as usize) {
                    channel.stop();
                }
            }
            EventPayload::Effect(_effect) => {
                // TODO: Apply effect
            }
            _ => {}
        }
    }

    /// Apply a global event.
    fn apply_global_event(&mut self, payload: &EventPayload) {
        match payload {
            EventPayload::SetTempo(tempo) => {
                self.tempo = (*tempo / 100) as u8;
                self.update_samples_per_tick();
            }
            EventPayload::SetSpeed(speed) => {
                self.speed = *speed;
            }
            _ => {}
        }
    }

    /// Mix all channels into a single frame.
    fn mix_channels(&mut self) -> Frame {
        let mut left: i32 = 0;
        let mut right: i32 = 0;

        for (i, channel) in self.channels.iter_mut().enumerate() {
            if !channel.playing {
                continue;
            }

            // Get sample data
            let sample = match self.song.samples.get(channel.sample_index as usize) {
                Some(s) => s,
                None => continue,
            };

            // Read sample value at current position
            let pos = (channel.position >> 16) as usize;
            let sample_value = sample.data.get_mono(pos);

            // Apply volume and panning
            let vol = channel.volume as i32;
            let pan = channel.panning as i32; // -64 to +64

            // Calculate left/right volumes
            let left_vol = ((64 - pan) * vol) >> 6;
            let right_vol = ((64 + pan) * vol) >> 6;

            left += (sample_value as i32 * left_vol) >> 6;
            right += (sample_value as i32 * right_vol) >> 6;

            // Advance position
            channel.position += channel.increment;

            // Handle looping
            let pos_samples = (channel.position >> 16) as u32;
            if sample.has_loop() && pos_samples >= sample.loop_end {
                let loop_len = sample.loop_end - sample.loop_start;
                channel.position -= loop_len << 16;
            } else if pos_samples >= sample.len() as u32 {
                channel.playing = false;
            }
        }

        Frame {
            left: left.clamp(-32768, 32767) as i16,
            right: right.clamp(-32768, 32767) as i16,
        }
    }

    /// Get the current playback position.
    pub fn position(&self) -> Timestamp {
        self.current_time
    }

    /// Is playback active?
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Schedule an event.
    pub fn schedule(&mut self, event: Event) {
        self.event_queue.push(event);
    }
}
