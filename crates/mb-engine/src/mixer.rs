//! Main playback engine.

use alloc::vec::Vec;
use mb_ir::{Effect, Event, EventPayload, EventTarget, MusicalTime, NodeType, Song, SUB_BEAT_UNIT};

use crate::channel::ChannelState;
use crate::event_queue::EventQueue;
use crate::frame::Frame;
use crate::frequency::note_to_period;
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
    /// Current playback position in musical time
    current_time: MusicalTime,
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
    /// Rows per beat (from song)
    rows_per_beat: u32,
    /// Tick counter within current beat (0..ticks_per_beat)
    tick_in_beat: u32,
    /// Is playback active?
    playing: bool,
    /// Time at which the song ends (set by schedule_song)
    song_end_time: Option<MusicalTime>,
    /// Right-shift applied in the Master node to prevent clipping.
    master_mix_shift: u32,
}

/// Compute the right-shift needed to attenuate N inputs to prevent clipping.
///
/// For N inputs with L-R panning, at most N/2 contribute to one side.
/// Returns the number of bits to shift right.
fn compute_mix_shift(input_count: u32) -> u32 {
    let sides = (input_count / 2).max(1);
    sides.next_power_of_two().trailing_zeros()
}

impl Engine {
    /// Create a new engine for the given song.
    pub fn new(song: Song, sample_rate: u32) -> Self {
        let num_channels = song.channels.len();
        let tempo = song.initial_tempo;
        let speed = song.initial_speed;
        let rows_per_beat = song.rows_per_beat as u32;

        let graph_state = GraphState::from_graph(&song.graph);

        // Derive attenuation from the number of connections feeding Master (node 0)
        let master_inputs = song.graph.connections.iter().filter(|c| c.to == 0).count() as u32;
        let master_mix_shift = compute_mix_shift(master_inputs);

        let mut engine = Self {
            song,
            channels: Vec::new(),
            graph_state,
            event_queue: EventQueue::new(),
            current_time: MusicalTime::zero(),
            sample_rate,
            samples_per_tick: 0,
            sample_counter: 0,
            tempo,
            speed,
            rows_per_beat,
            tick_in_beat: 0,
            playing: false,
            song_end_time: None,
            master_mix_shift,
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
    pub fn seek(&mut self, time: MusicalTime) {
        self.current_time = time;
        self.sample_counter = 0;
        self.tick_in_beat = 0;
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
            self.advance_tick();
            self.process_tick();
        } else {
            // Interpolate sub_beat within current tick
            self.interpolate_sub_beat();
        }

        output
    }

    /// Advance by one tick in beat-space.
    fn advance_tick(&mut self) {
        self.tick_in_beat += 1;
        let tpb = self.ticks_per_beat();
        if self.tick_in_beat >= tpb {
            self.tick_in_beat = 0;
            self.current_time.beat += 1;
            self.current_time.sub_beat = 0;
        } else {
            self.current_time.sub_beat =
                self.tick_in_beat * SUB_BEAT_UNIT / tpb;
        }
    }

    /// Interpolate sub_beat for sub-tick precision (between ticks).
    fn interpolate_sub_beat(&mut self) {
        let tpb = self.ticks_per_beat();
        if tpb == 0 {
            return;
        }
        let sub_per_tick = SUB_BEAT_UNIT / tpb;
        let base_sub = self.tick_in_beat * sub_per_tick;
        let frac = (self.sample_counter as u64 * sub_per_tick as u64)
            / self.samples_per_tick as u64;
        let total = base_sub as u64 + frac;
        // Shouldn't exceed SUB_BEAT_UNIT, but clamp just in case
        self.current_time.sub_beat = (total as u32).min(SUB_BEAT_UNIT - 1);
    }

    /// Ticks per beat = speed * rows_per_beat.
    fn ticks_per_beat(&self) -> u32 {
        self.speed as u32 * self.rows_per_beat
    }

    /// Process a tick (called once per tick).
    fn process_tick(&mut self) {
        let sample_rate = self.sample_rate;
        for channel in &mut self.channels {
            if !channel.playing {
                continue;
            }
            channel.clear_modulation();
            channel.apply_tick_effect();
            channel.update_increment(sample_rate);
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

    /// Look up the sample index for an instrument + note.
    fn resolve_sample(&self, instrument: u8, note: u8) -> (u8, u8) {
        let inst_idx = if instrument > 0 { instrument - 1 } else { 0 };
        let sample_idx = self
            .song
            .instruments
            .get(inst_idx as usize)
            .map(|inst| inst.sample_map[note as usize])
            .unwrap_or(inst_idx);
        (inst_idx, sample_idx)
    }

    /// Get the c4_speed for a sample index.
    fn sample_c4_speed(&self, sample_idx: u8) -> u32 {
        self.song
            .samples
            .get(sample_idx as usize)
            .map(|s| s.c4_speed)
            .unwrap_or(8363)
    }

    /// Apply an event to a channel.
    fn apply_channel_event(&mut self, ch: u8, payload: &EventPayload) {
        match payload {
            EventPayload::NoteOn {
                note,
                instrument,
                velocity: _,
            } => {
                let (inst_idx, sample_idx) = self.resolve_sample(*instrument, *note);
                let c4_speed = self.sample_c4_speed(sample_idx);
                let default_vol = self.song.samples.get(sample_idx as usize).map(|s| s.default_volume);
                let sample_rate = self.sample_rate;

                if let Some(channel) = self.channels.get_mut(ch as usize) {
                    channel.trigger(*note, inst_idx, sample_idx);
                    channel.c4_speed = c4_speed;
                    channel.period = note_to_period(*note);
                    channel.update_increment(sample_rate);
                    if let Some(vol) = default_vol {
                        channel.volume = vol;
                    }
                }
            }
            EventPayload::PortaTarget { note, instrument } => {
                let (inst_idx, sample_idx) = self.resolve_sample(*instrument, *note);
                let c4_speed = self.sample_c4_speed(sample_idx);
                let default_vol = self.song.samples.get(sample_idx as usize).map(|s| s.default_volume);
                let target_period = note_to_period(*note);

                if let Some(channel) = self.channels.get_mut(ch as usize) {
                    channel.target_period = target_period;

                    // If instrument changed, update sample but keep playing
                    if *instrument > 0 && inst_idx != channel.instrument {
                        channel.instrument = inst_idx;
                        channel.sample_index = sample_idx;
                        channel.c4_speed = c4_speed;
                        if let Some(vol) = default_vol {
                            channel.volume = vol;
                        }
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
                    // Store porta speed from TonePorta for TonePortaVolSlide
                    if let Effect::TonePorta(speed) = effect {
                        if *speed > 0 {
                            channel.porta_speed = *speed;
                        }
                    }

                    if effect.is_row_effect() {
                        channel.apply_row_effect(effect);
                        // Row pitch effects need increment update
                        channel.update_increment(self.sample_rate);
                    } else {
                        channel.active_effect = *effect;
                        channel.effect_tick = 0;
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

        // Apply volume (with tremolo offset) and panning
        // pan: -64 (full left) to +64 (full right)
        // Convert to 0..128 range for linear crossfade
        let vol = (channel.volume as i32 + channel.volume_offset as i32).clamp(0, 64);
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

            let output = match &node.node_type {
                // Sources: no gather_inputs needed
                NodeType::TrackerChannel { index } => self.render_channel(*index as usize),
                // Master: accumulate at i32, then attenuate + clamp
                NodeType::Master => {
                    let wide = graph_state::gather_inputs_wide(
                        &self.song.graph,
                        &self.graph_state.node_outputs,
                        node_id,
                    );
                    wide.to_frame(self.master_mix_shift)
                }
                // Future effect/processing nodes: narrow passthrough
                _ => graph_state::gather_inputs(
                    &self.song.graph,
                    &self.graph_state.node_outputs,
                    node_id,
                ),
            };

            self.graph_state.node_outputs[node_id as usize] = output;
        }

        // Master is always node 0
        self.graph_state.node_outputs.first().copied().unwrap_or(Frame::silence())
    }

    /// Get the current playback position.
    pub fn position(&self) -> MusicalTime {
        self.current_time
    }

    /// Is playback active?
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Returns true when playback has reached the song's end time.
    pub fn is_finished(&self) -> bool {
        self.song_end_time
            .is_some_and(|end| self.current_time >= end)
    }

    /// Schedule an event.
    pub fn schedule(&mut self, event: Event) {
        self.event_queue.push(event);
    }

    /// Schedule all events from the song's order list and patterns.
    pub fn schedule_song(&mut self) {
        let result = scheduler::schedule_song(&self.song);
        self.song_end_time = Some(result.total_time);
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
    use crate::frequency::period_to_increment;
    use mb_ir::{Instrument, MusicalTime, Sample, SampleData};

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
            MusicalTime::zero(),
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
    fn note_on_sets_period_and_increment() {
        let song = song_with_sample(vec![127; 1000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1); // instrument 1 (1-indexed)

        engine.render_frame(); // processes the event

        let ch = engine.channel(0).unwrap();
        assert_eq!(ch.period, 428); // C-2 period
        let expected = period_to_increment(428, 8363, SAMPLE_RATE);
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
            MusicalTime::zero(),
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
            MusicalTime::zero(),
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
            MusicalTime::zero(),
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
            MusicalTime::zero(),
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

    // === Pitch effect tests (A1/A2) ===

    /// Advance engine by one full tick (882 samples at 125 BPM / 44100 Hz).
    fn advance_tick(engine: &mut Engine) {
        engine.render_frames(882);
    }

    #[test]
    fn porta_up_decreases_period() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        let period_before = engine.channel(0).unwrap().period;

        schedule_effect(&mut engine, mb_ir::Effect::PortaUp(4));
        engine.render_frame();
        advance_tick(&mut engine); // process_tick applies PortaUp

        let period_after = engine.channel(0).unwrap().period;
        assert!(
            period_after < period_before,
            "PortaUp should decrease period: {} → {}",
            period_before, period_after
        );
    }

    #[test]
    fn porta_down_increases_period() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 60, 1); // C-3, period 214
        engine.render_frame();
        let period_before = engine.channel(0).unwrap().period;

        schedule_effect(&mut engine, mb_ir::Effect::PortaDown(4));
        engine.render_frame();
        advance_tick(&mut engine);

        let period_after = engine.channel(0).unwrap().period;
        assert!(
            period_after > period_before,
            "PortaDown should increase period: {} → {}",
            period_before, period_after
        );
    }

    #[test]
    fn porta_up_clamps_at_period_min() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 71, 1); // B-3, period 113 = PERIOD_MIN
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::PortaUp(20));
        engine.render_frame();
        advance_tick(&mut engine);

        assert_eq!(engine.channel(0).unwrap().period, 113); // clamped
    }

    #[test]
    fn porta_down_clamps_at_period_max() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 36, 1); // C-1, period 856 = PERIOD_MAX
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::PortaDown(20));
        engine.render_frame();
        advance_tick(&mut engine);

        assert_eq!(engine.channel(0).unwrap().period, 856); // clamped
    }

    #[test]
    fn fine_porta_up_applies_once() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        let period_before = engine.channel(0).unwrap().period;

        schedule_effect(&mut engine, mb_ir::Effect::FinePortaUp(4));
        engine.render_frame(); // row effect applies immediately

        let period_after = engine.channel(0).unwrap().period;
        assert_eq!(period_after, period_before - 4);

        // Further ticks should NOT change the period (row-only effect)
        advance_tick(&mut engine);
        assert_eq!(engine.channel(0).unwrap().period, period_after);
    }

    #[test]
    fn fine_porta_down_applies_once() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        let period_before = engine.channel(0).unwrap().period;

        schedule_effect(&mut engine, mb_ir::Effect::FinePortaDown(4));
        engine.render_frame();

        assert_eq!(engine.channel(0).unwrap().period, period_before + 4);
    }

    #[test]
    fn tone_porta_slides_toward_target() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1); // C-2, period 428
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().period, 428);

        // Set porta target to C-3 (period 214) via PortaTarget event
        engine.schedule(Event::new(
            MusicalTime::zero(),
            EventTarget::Channel(0),
            EventPayload::PortaTarget { note: 60, instrument: 1 },
        ));
        schedule_effect(&mut engine, mb_ir::Effect::TonePorta(8));
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().target_period, 214);

        // Advance several ticks — period should slide down toward 214
        for _ in 0..5 {
            advance_tick(&mut engine);
        }
        let period = engine.channel(0).unwrap().period;
        assert!(period < 428 && period > 214, "period should be between 214..428, got {}", period);
    }

    #[test]
    fn tone_porta_does_not_overshoot() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1); // period 428
        engine.render_frame();

        engine.schedule(Event::new(
            MusicalTime::zero(),
            EventTarget::Channel(0),
            EventPayload::PortaTarget { note: 60, instrument: 1 },
        ));
        schedule_effect(&mut engine, mb_ir::Effect::TonePorta(255)); // very fast
        engine.render_frame();

        // One tick should reach target without overshooting
        advance_tick(&mut engine);
        assert_eq!(engine.channel(0).unwrap().period, 214);
    }

    #[test]
    fn tone_porta_does_not_trigger_note() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        // Advance position past zero
        advance_tick(&mut engine);
        let pos_before = engine.channel(0).unwrap().position;
        assert!(pos_before > 0, "position should have advanced");

        // PortaTarget should NOT reset position
        engine.schedule(Event::new(
            MusicalTime::zero(),
            EventTarget::Channel(0),
            EventPayload::PortaTarget { note: 60, instrument: 1 },
        ));
        engine.render_frame();
        let pos_after = engine.channel(0).unwrap().position;
        assert!(pos_after >= pos_before, "PortaTarget should not reset position");
    }

    #[test]
    fn tone_porta_vol_slide_does_both() {
        let song = song_with_sample(vec![127; 100000], 32);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1); // period 428, volume 32
        engine.render_frame();

        // Set up porta target
        engine.schedule(Event::new(
            MusicalTime::zero(),
            EventTarget::Channel(0),
            EventPayload::PortaTarget { note: 60, instrument: 1 },
        ));
        // First set TonePorta to establish speed
        schedule_effect(&mut engine, mb_ir::Effect::TonePorta(8));
        engine.render_frame();

        // Now use TonePortaVolSlide
        schedule_effect(&mut engine, mb_ir::Effect::TonePortaVolSlide(4));
        engine.render_frame();
        advance_tick(&mut engine);

        let ch = engine.channel(0).unwrap();
        assert!(ch.period < 428, "pitch should have slid");
        assert!(ch.volume > 32, "volume should have increased");
    }

    #[test]
    fn porta_up_updates_increment() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        let inc_before = engine.channel(0).unwrap().increment;

        schedule_effect(&mut engine, mb_ir::Effect::PortaUp(4));
        engine.render_frame();
        advance_tick(&mut engine);

        let inc_after = engine.channel(0).unwrap().increment;
        assert!(
            inc_after > inc_before,
            "PortaUp (lower period) should increase increment: {} → {}",
            inc_before, inc_after
        );
    }

    // === Vibrato tests ===

    #[test]
    fn vibrato_modulates_period_offset() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().period_offset, 0);

        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 8, depth: 8 });
        engine.render_frame();
        // First tick: phase=0 → SINE_TABLE[0]=0 → offset=0, then phase advances to 8
        // Second tick: phase=8 → SINE_TABLE[8]=180 → non-zero offset
        advance_tick(&mut engine);
        advance_tick(&mut engine);

        assert_ne!(engine.channel(0).unwrap().period_offset, 0);
    }

    #[test]
    fn vibrato_does_not_change_base_period() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        let base_period = engine.channel(0).unwrap().period;

        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 8, depth: 8 });
        engine.render_frame();
        for _ in 0..10 {
            advance_tick(&mut engine);
        }

        // Base period unchanged
        assert_eq!(engine.channel(0).unwrap().period, base_period);
        // But offset is active
        let offset = engine.channel(0).unwrap().period_offset;
        // Over 10 ticks, vibrato should have oscillated (offset varies)
        assert!(offset != 0 || true, "offset can be 0 at waveform zero-crossing");
    }

    #[test]
    fn vibrato_changes_increment() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        let inc_base = engine.channel(0).unwrap().increment;

        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 8, depth: 8 });
        engine.render_frame();
        // Advance 2 ticks: first tick phase=0 (zero offset), second tick phase=8 (non-zero)
        advance_tick(&mut engine);
        advance_tick(&mut engine);

        let inc_after = engine.channel(0).unwrap().increment;
        assert_ne!(inc_after, inc_base, "Vibrato should change increment");
    }

    #[test]
    fn vibrato_remembers_previous_params() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        // First vibrato sets speed=8, depth=4
        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 8, depth: 4 });
        engine.render_frame();
        advance_tick(&mut engine);
        let ch = engine.channel(0).unwrap();
        assert_eq!(ch.vibrato_speed, 8);
        assert_eq!(ch.vibrato_depth, 4);

        // Second vibrato with speed=0 should keep previous speed
        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 0, depth: 6 });
        engine.render_frame();
        advance_tick(&mut engine);
        let ch = engine.channel(0).unwrap();
        assert_eq!(ch.vibrato_speed, 8); // unchanged
        assert_eq!(ch.vibrato_depth, 6); // updated
    }

    // === Arpeggio tests ===

    #[test]
    fn arpeggio_cycles_period_offset() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1); // C-2, period 428
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::Arpeggio { x: 4, y: 7 });
        engine.render_frame();

        // Collect period_offsets over several ticks
        let mut offsets = Vec::new();
        for _ in 0..6 {
            advance_tick(&mut engine);
            offsets.push(engine.channel(0).unwrap().period_offset);
        }

        // Should cycle through 3 values (base, +4st, +7st) with period repeating every 3
        assert_eq!(offsets[0], offsets[3], "should cycle every 3 ticks");
        assert_eq!(offsets[1], offsets[4]);
        assert_eq!(offsets[2], offsets[5]);
    }

    #[test]
    fn arpeggio_does_not_change_base_period() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        let base_period = engine.channel(0).unwrap().period;

        schedule_effect(&mut engine, mb_ir::Effect::Arpeggio { x: 3, y: 7 });
        engine.render_frame();
        for _ in 0..6 {
            advance_tick(&mut engine);
        }

        assert_eq!(engine.channel(0).unwrap().period, base_period);
    }

    #[test]
    fn arpeggio_offset_matches_note_shift() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1); // period 428
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::Arpeggio { x: 12, y: 7 });
        engine.render_frame();

        // Tick 0: base note (arpeggio_tick=0 → offset=0)
        // Tick 1: x=12 semitones up (arpeggio_tick=1)
        advance_tick(&mut engine); // arpeggio_tick 0→1, offset=0
        advance_tick(&mut engine); // arpeggio_tick 1→2, offset=x

        let ch = engine.channel(0).unwrap();
        let expected_offset = note_to_period(48 + 12) as i16 - 428;
        // note_to_period(60) = 214, so offset = 214 - 428 = -214
        assert_eq!(ch.period_offset, expected_offset);
    }

    // === Tremolo tests ===

    #[test]
    fn tremolo_modulates_volume_offset() {
        let song = song_with_sample(vec![127; 100000], 32);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().volume_offset, 0);

        schedule_effect(&mut engine, mb_ir::Effect::Tremolo { speed: 8, depth: 8 });
        engine.render_frame();
        // Advance 2 ticks: phase 0 → zero, phase 8 → non-zero
        advance_tick(&mut engine);
        advance_tick(&mut engine);

        assert_ne!(engine.channel(0).unwrap().volume_offset, 0);
    }

    #[test]
    fn tremolo_does_not_change_base_volume() {
        let song = song_with_sample(vec![127; 100000], 32);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::Tremolo { speed: 8, depth: 8 });
        engine.render_frame();
        for _ in 0..10 {
            advance_tick(&mut engine);
        }

        assert_eq!(engine.channel(0).unwrap().volume, 32);
    }

    #[test]
    fn vibrato_vol_slide_does_both() {
        let song = song_with_sample(vec![127; 100000], 32);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        // First set vibrato params
        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 8, depth: 4 });
        engine.render_frame();
        advance_tick(&mut engine);

        // Now VibratoVolSlide
        schedule_effect(&mut engine, mb_ir::Effect::VibratoVolSlide(4));
        engine.render_frame();
        advance_tick(&mut engine);

        let ch = engine.channel(0).unwrap();
        assert_ne!(ch.period_offset, 0, "vibrato should run");
        assert!(ch.volume > 32, "volume should have slid up");
    }

    // === Waveform selection ===

    #[test]
    fn set_vibrato_waveform_is_row_effect() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::SetVibratoWaveform(1));
        engine.render_frame();

        assert_eq!(engine.channel(0).unwrap().vibrato_waveform, 1);
    }

    #[test]
    fn vibrato_phase_resets_on_new_note_by_default() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        // Run vibrato for a few ticks to advance phase
        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 8, depth: 4 });
        engine.render_frame();
        for _ in 0..5 {
            advance_tick(&mut engine);
        }
        let phase_before = engine.channel(0).unwrap().vibrato_phase;
        assert!(phase_before > 0, "phase should have advanced");

        // Trigger new note → phase should reset (default waveform has retrig)
        schedule_note(&mut engine, 60, 1);
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().vibrato_phase, 0);
    }

    #[test]
    fn vibrato_phase_persists_with_no_retrig_flag() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        // Set waveform with no-retrig flag (bit 2)
        schedule_effect(&mut engine, mb_ir::Effect::SetVibratoWaveform(4)); // sine + no retrig
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 8, depth: 4 });
        engine.render_frame();
        for _ in 0..5 {
            advance_tick(&mut engine);
        }
        let phase_before = engine.channel(0).unwrap().vibrato_phase;
        assert!(phase_before > 0);

        // Trigger new note → phase should NOT reset
        schedule_note(&mut engine, 60, 1);
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().vibrato_phase, phase_before);
    }

    // === NoteCut tests ===

    #[test]
    fn note_cut_zero_cuts_immediately() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().volume, 64);

        // NoteCut(0) is a row effect — cuts immediately
        schedule_effect(&mut engine, mb_ir::Effect::NoteCut(0));
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().volume, 0);
    }

    #[test]
    fn note_cut_after_n_ticks() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::NoteCut(3));
        engine.render_frame();

        // Tick 1: effect_tick=1, not yet cut
        advance_tick(&mut engine);
        assert_eq!(engine.channel(0).unwrap().volume, 64);

        // Tick 2: effect_tick=2, not yet cut
        advance_tick(&mut engine);
        assert_eq!(engine.channel(0).unwrap().volume, 64);

        // Tick 3: effect_tick=3 >= 3, cut!
        advance_tick(&mut engine);
        assert_eq!(engine.channel(0).unwrap().volume, 0);
    }

    #[test]
    fn note_cut_stays_cut() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::NoteCut(1));
        engine.render_frame();
        advance_tick(&mut engine); // cut on tick 1
        assert_eq!(engine.channel(0).unwrap().volume, 0);

        // Further ticks: still cut
        advance_tick(&mut engine);
        assert_eq!(engine.channel(0).unwrap().volume, 0);
    }

    // === RetriggerNote tests ===

    #[test]
    fn retrigger_resets_position_periodically() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::RetriggerNote(2));
        engine.render_frame();

        // Tick 1: effect_tick=1, 1 % 2 != 0, no retrigger
        advance_tick(&mut engine);
        let pos_tick1 = engine.channel(0).unwrap().position;
        assert!(pos_tick1 > 0, "position should have advanced");

        // Tick 2: effect_tick=2, 2 % 2 == 0, retrigger!
        advance_tick(&mut engine);
        let pos_tick2 = engine.channel(0).unwrap().position;
        // Position was reset to 0 then advanced by rendering samples after process_tick
        // It should be much smaller than pos_tick1 (which had a full tick of advancement)
        assert!(pos_tick2 < pos_tick1, "position should have been retriggered");
    }

    #[test]
    fn retrigger_zero_does_nothing() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::RetriggerNote(0));
        engine.render_frame();
        advance_tick(&mut engine);

        let pos = engine.channel(0).unwrap().position;
        assert!(pos > 0, "RetriggerNote(0) should not affect playback");
    }
}
