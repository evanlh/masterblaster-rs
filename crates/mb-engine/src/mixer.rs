//! Main playback engine.

use alloc::vec::Vec;
use mb_ir::{Event, EventPayload, EventTarget, NodeType, Song, Timestamp};

use crate::channel::ChannelState;
use crate::event_queue::EventQueue;
use crate::frame::Frame;
use crate::frequency::note_to_increment;
use crate::graph_state::{self, GraphState};
use crate::scheduler;

/// The main playback engine.
pub struct Engine {
    /// The song being played
    song: Song,
    /// Channel states
    channels: Vec<ChannelState>,
    /// Runtime graph state (node outputs + topological order)
    graph_state: GraphState,
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
    /// Tick at which the song ends (set by schedule_song)
    song_end_tick: u64,
}

impl Engine {
    /// Create a new engine for the given song.
    pub fn new(song: Song, sample_rate: u32) -> Self {
        let num_channels = song.channels.len();
        let tempo = song.initial_tempo;
        let speed = song.initial_speed;

        let graph_state = GraphState::from_graph(&song.graph);

        let mut engine = Self {
            song,
            channels: Vec::new(),
            graph_state,
            event_queue: EventQueue::new(),
            current_time: Timestamp::from_ticks(0),
            sample_rate,
            samples_per_tick: 0,
            sample_counter: 0,
            tempo,
            speed,
            playing: false,
            song_end_tick: 0,
        };

        // Initialize channels with panning from song settings
        for i in 0..num_channels {
            let mut ch = ChannelState::new();
            if let Some(settings) = engine.song.channels.get(i) {
                ch.panning = settings.initial_pan;
            }
            engine.channels.push(ch);
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

        // 2. Render audio graph
        let output = self.render_graph();

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
        for channel in &mut self.channels {
            if !channel.playing {
                continue;
            }
            channel.apply_tick_effect();
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
                // MOD instruments are 1-indexed: instrument 1 = index 0
                let inst_idx = if *instrument > 0 { *instrument - 1 } else { 0 };
                let sample_idx = self
                    .song
                    .instruments
                    .get(inst_idx as usize)
                    .map(|inst| inst.sample_map[*note as usize])
                    .unwrap_or(inst_idx);

                if let Some(channel) = self.channels.get_mut(ch as usize) {
                    channel.trigger(*note, inst_idx, sample_idx);

                    // Compute playback increment from sample's c4_speed
                    let c4_speed = self
                        .song
                        .samples
                        .get(sample_idx as usize)
                        .map(|s| s.c4_speed)
                        .unwrap_or(8363);
                    channel.increment = note_to_increment(*note, c4_speed, self.sample_rate);

                    // Set volume from sample default
                    if let Some(sample) = self.song.samples.get(sample_idx as usize) {
                        channel.volume = sample.default_volume;
                    }
                }
            }
            EventPayload::NoteOff { note: _ } => {
                if let Some(channel) = self.channels.get_mut(ch as usize) {
                    channel.stop();
                }
            }
            EventPayload::Effect(effect) => {
                if let Some(channel) = self.channels.get_mut(ch as usize) {
                    if effect.is_row_effect() {
                        channel.apply_row_effect(effect);
                    } else {
                        // Per-tick effects: store as active, processed in process_tick
                        channel.active_effect = *effect;
                    }
                }
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

    /// Render a single tracker channel into a stereo frame.
    fn render_channel(&mut self, ch_index: usize) -> Frame {
        let channel = match self.channels.get_mut(ch_index) {
            Some(ch) => ch,
            None => return Frame::silence(),
        };
        if !channel.playing {
            return Frame::silence();
        }

        let sample = match self.song.samples.get(channel.sample_index as usize) {
            Some(s) => s,
            None => return Frame::silence(),
        };

        // Read sample value with linear interpolation
        let sample_value = sample.data.get_mono_interpolated(channel.position);

        // Apply volume and panning
        // pan: -64 (full left) to +64 (full right)
        // Convert to 0..128 range for linear crossfade
        let vol = channel.volume as i32;
        let pan_right = (channel.panning as i32 + 64) as i32; // 0..128
        let left_vol = ((128 - pan_right) * vol) >> 7;
        let right_vol = (pan_right * vol) >> 7;

        let left = (sample_value as i32 * left_vol) >> 6;
        let right = (sample_value as i32 * right_vol) >> 6;

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

        Frame {
            left: left.clamp(-32768, 32767) as i16,
            right: right.clamp(-32768, 32767) as i16,
        }
    }

    /// Render the audio graph by traversing nodes in topological order.
    fn render_graph(&mut self) -> Frame {
        self.graph_state.clear_outputs();

        // Clone topo_order to avoid borrow conflict with &mut self
        let topo_order = self.graph_state.topo_order.clone();

        for &node_id in &topo_order {
            let node = match self.song.graph.node(node_id) {
                Some(n) => n,
                None => continue,
            };

            // Gather inputs from all connections feeding this node
            let input = graph_state::gather_inputs(
                &self.song.graph,
                &self.graph_state.node_outputs,
                node_id,
            );

            // Process node based on type
            let output = match &node.node_type {
                NodeType::TrackerChannel { index } => self.render_channel(*index as usize),
                NodeType::Master => input,
                _ => Frame::silence(),
            };

            self.graph_state.node_outputs[node_id as usize] = output;
        }

        // Master is always node 0
        self.graph_state.node_outputs.first().copied().unwrap_or(Frame::silence())
    }

    /// Get the current playback position.
    pub fn position(&self) -> Timestamp {
        self.current_time
    }

    /// Is playback active?
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Returns true when playback has reached the song's end tick.
    pub fn is_finished(&self) -> bool {
        self.song_end_tick > 0 && self.current_time.tick >= self.song_end_tick
    }

    /// Schedule an event.
    pub fn schedule(&mut self, event: Event) {
        self.event_queue.push(event);
    }

    /// Schedule all events from the song's order list and patterns.
    pub fn schedule_song(&mut self) {
        let result = scheduler::schedule_song(&self.song);
        self.song_end_tick = result.total_ticks;
        for event in result.events {
            self.event_queue.push(event);
        }
    }

    /// Render multiple frames into a buffer.
    pub fn render_frames(&mut self, count: usize) -> Vec<Frame> {
        (0..count).map(|_| self.render_frame()).collect()
    }

    /// Get a reference to a channel's state (for testing).
    pub fn channel(&self, index: usize) -> Option<&ChannelState> {
        self.channels.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frequency::note_to_increment;
    use mb_ir::{Instrument, Sample, SampleData};

    const SAMPLE_RATE: u32 = 44100;

    /// Build a 1-channel song with one sample containing `data`.
    fn song_with_sample(data: Vec<i8>, volume: u8) -> Song {
        let mut song = Song::with_channels("test", 1);

        let mut sample = Sample::new("test sample");
        sample.data = SampleData::Mono8(data);
        sample.default_volume = volume;
        sample.c4_speed = 8363;
        song.samples.push(sample);

        let mut inst = Instrument::new("test inst");
        inst.set_single_sample(0);
        song.instruments.push(inst);

        song
    }

    /// Schedule a NoteOn at tick 0 on channel 0.
    fn schedule_note(engine: &mut Engine, note: u8, instrument: u8) {
        engine.schedule(Event::new(
            Timestamp::from_ticks(0),
            EventTarget::Channel(0),
            EventPayload::NoteOn { note, velocity: 64, instrument },
        ));
    }

    #[test]
    fn silent_when_not_playing() {
        let song = song_with_sample(vec![127; 100], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        // Don't call play()
        let frame = engine.render_frame();
        assert_eq!(frame, Frame::silence());
    }

    #[test]
    fn silent_with_no_events() {
        let song = song_with_sample(vec![127; 100], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        let frame = engine.render_frame();
        assert_eq!(frame, Frame::silence());
    }

    #[test]
    fn note_on_sets_increment() {
        let song = song_with_sample(vec![127; 1000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1); // instrument 1 (1-indexed)

        engine.render_frame(); // processes the event

        let ch = engine.channel(0).unwrap();
        let expected = note_to_increment(48, 8363, SAMPLE_RATE);
        assert_eq!(ch.increment, expected);
        assert!(ch.increment > 0);
    }

    #[test]
    fn note_on_sets_volume_from_sample() {
        let song = song_with_sample(vec![127; 1000], 48);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);

        engine.render_frame();

        assert_eq!(engine.channel(0).unwrap().volume, 48);
    }

    #[test]
    fn note_on_produces_nonsilent_output() {
        let song = song_with_sample(vec![127; 1000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);

        // First frame processes the event and mixes
        let frame = engine.render_frame();
        assert!(frame.left != 0 || frame.right != 0, "Expected non-silent output");
    }

    #[test]
    fn higher_note_gives_higher_increment() {
        let song = song_with_sample(vec![127; 1000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        let inc_48 = engine.channel(0).unwrap().increment;

        // Re-trigger at higher note
        engine.schedule(Event::new(
            Timestamp::from_ticks(0),
            EventTarget::Channel(0),
            EventPayload::NoteOn { note: 60, velocity: 64, instrument: 1 },
        ));
        engine.render_frame();
        let inc_60 = engine.channel(0).unwrap().increment;

        assert!(inc_60 > inc_48);
        assert_eq!(inc_60, inc_48 * 2); // octave up = double
    }

    #[test]
    fn note_off_stops_channel() {
        let song = song_with_sample(vec![127; 1000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        assert!(engine.channel(0).unwrap().playing);

        engine.schedule(Event::new(
            Timestamp::from_ticks(0),
            EventTarget::Channel(0),
            EventPayload::NoteOff { note: 0 },
        ));
        engine.render_frame();
        assert!(!engine.channel(0).unwrap().playing);
    }

    #[test]
    fn sample_stops_at_end_without_loop() {
        // Very short sample, high note = fast increment
        let song = song_with_sample(vec![127; 4], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);

        // Render enough frames to exhaust the 4-sample data
        for _ in 0..10000 {
            engine.render_frame();
        }

        assert!(!engine.channel(0).unwrap().playing);
    }

    #[test]
    fn set_tempo_changes_samples_per_tick() {
        let song = song_with_sample(vec![127; 100], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();

        // Default tempo 125: samples_per_tick = 44100 * 5 / (125 * 2) = 882
        let spt_before = engine.samples_per_tick;
        assert_eq!(spt_before, 882);

        engine.schedule(Event::new(
            Timestamp::from_ticks(0),
            EventTarget::Global,
            EventPayload::SetTempo(15000), // 150 BPM * 100
        ));
        engine.render_frame();

        // 150 BPM: 44100 * 5 / (150 * 2) = 735
        assert_eq!(engine.samples_per_tick, 735);
    }

    #[test]
    fn zero_volume_sample_produces_silence() {
        let song = song_with_sample(vec![127; 1000], 0);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);

        let frame = engine.render_frame();
        assert_eq!(frame, Frame::silence());
    }

    /// Schedule an effect at tick 0 on channel 0.
    fn schedule_effect(engine: &mut Engine, effect: mb_ir::Effect) {
        engine.schedule(Event::new(
            Timestamp::from_ticks(0),
            EventTarget::Channel(0),
            EventPayload::Effect(effect),
        ));
    }

    #[test]
    fn set_volume_effect_changes_channel_volume() {
        let song = song_with_sample(vec![127; 1000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().volume, 64);

        schedule_effect(&mut engine, mb_ir::Effect::SetVolume(32));
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().volume, 32);
    }

    #[test]
    fn set_volume_clamps_to_64() {
        let song = song_with_sample(vec![127; 1000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::SetVolume(100));
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().volume, 64);
    }

    #[test]
    fn fine_volume_slide_up() {
        let song = song_with_sample(vec![127; 1000], 32);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().volume, 32);

        schedule_effect(&mut engine, mb_ir::Effect::FineVolumeSlideUp(4));
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().volume, 36);
    }

    #[test]
    fn fine_volume_slide_down() {
        let song = song_with_sample(vec![127; 1000], 32);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::FineVolumeSlideDown(4));
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().volume, 28);
    }

    #[test]
    fn fine_volume_slide_down_clamps_to_zero() {
        let song = song_with_sample(vec![127; 1000], 2);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::FineVolumeSlideDown(15));
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().volume, 0);
    }

    #[test]
    fn volume_slide_applied_per_tick() {
        let song = song_with_sample(vec![127; 100000], 32);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame(); // tick 0: note triggered, volume = 32

        // Schedule volume slide +4 per tick
        schedule_effect(&mut engine, mb_ir::Effect::VolumeSlide(4));
        engine.render_frame(); // stores active effect

        // Render until the next tick boundary to trigger process_tick
        // samples_per_tick at 125 BPM = 882
        let vol_before = engine.channel(0).unwrap().volume;
        for _ in 0..882 {
            engine.render_frame();
        }
        let vol_after = engine.channel(0).unwrap().volume;
        assert!(
            vol_after > vol_before,
            "Volume should increase: before={}, after={}",
            vol_before, vol_after
        );
    }

    #[test]
    fn volume_slide_clamps_at_bounds() {
        let song = song_with_sample(vec![127; 100000], 62);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        // Slide up by 4 per tick — should clamp at 64
        schedule_effect(&mut engine, mb_ir::Effect::VolumeSlide(4));
        // Render many ticks
        engine.render_frames(882 * 10);
        assert_eq!(engine.channel(0).unwrap().volume, 64);
    }

    #[test]
    fn new_note_clears_active_effect() {
        let song = song_with_sample(vec![127; 100000], 32);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::VolumeSlide(4));
        engine.render_frame();
        assert_ne!(engine.channel(0).unwrap().active_effect, mb_ir::Effect::None);

        // New note should clear the active effect
        schedule_note(&mut engine, 60, 1);
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().active_effect, mb_ir::Effect::None);
    }
}
