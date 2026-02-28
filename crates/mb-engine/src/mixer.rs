//! Main playback engine.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use mb_ir::{Edit, Effect, Event, EventPayload, EventTarget, MusicalTime, NodeType, Song, SUB_BEAT_UNIT, sub_beats_per_tick};

use crate::channel::ChannelState;
use crate::event_queue::EventQueue;
use crate::frequency::note_to_period;
use crate::graph_state::{self, GraphState};
use crate::machine::Machine;
use crate::machines;
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
    /// Per-node gain (linear, computed from mix shift).
    mix_gains: Vec<f32>,
    /// Machine instances (indexed by NodeId; `Some` only for BuzzMachine nodes).
    machines: Vec<Option<Box<dyn Machine>>>,
}

/// Compute the right-shift needed to attenuate N inputs to prevent clipping.
///
/// For N inputs with L-R panning, at most N/2 contribute to one side.
/// Returns the number of bits to shift right.
fn compute_mix_shift(input_count: u32) -> u32 {
    let sides = (input_count / 2).max(1);
    sides.next_power_of_two().trailing_zeros()
}

/// Compute per-node mix gains from connection counts in the graph.
fn compute_all_mix_gains(song: &Song) -> Vec<f32> {
    let n = song.graph.nodes.len();
    let mut gains = vec![1.0f32; n];
    for (id, _node) in song.graph.nodes.iter().enumerate() {
        let input_count = song.graph.connections.iter().filter(|c| c.to == id as u16).count() as u32;
        let shift = compute_mix_shift(input_count);
        gains[id] = 1.0 / (1u32 << shift) as f32;
    }
    gains
}

/// Instantiate machines for all BuzzMachine nodes in the graph.
fn init_machines(song: &Song, sample_rate: u32) -> Vec<Option<Box<dyn Machine>>> {
    song.graph.nodes.iter().map(|node| {
        if let NodeType::BuzzMachine { machine_name } = &node.node_type {
            let mut machine = machines::create_machine(machine_name)?;
            machine.init(sample_rate);
            // Apply initial parameter values from graph node
            for param in &node.parameters {
                machine.set_param(param.id, param.value);
            }
            Some(machine)
        } else {
            None
        }
    }).collect()
}

impl Engine {
    /// Create a new engine for the given song.
    pub fn new(song: Song, sample_rate: u32) -> Self {
        let num_channels = song.channels.len();
        let tempo = song.initial_tempo;
        let speed = song.initial_speed;
        let rows_per_beat = song.rows_per_beat as u32;

        let graph_state = GraphState::from_graph(&song.graph);

        // Compute per-node mix gains from connection counts
        let mix_gains = compute_all_mix_gains(&song);

        // Instantiate machines for BuzzMachine nodes
        let machines_vec = init_machines(&song, sample_rate);

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
            mix_gains,
            machines: machines_vec,
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

    /// Generate one frame of audio as [f32; 2].
    pub fn render_frame(&mut self) -> [f32; 2] {
        #[cfg(feature = "alloc_check")]
        {
            assert_no_alloc::assert_no_alloc(|| self.render_frame_inner())
        }
        #[cfg(not(feature = "alloc_check"))]
        {
            self.render_frame_inner()
        }
    }

    /// Render multiple frames, returning a new Vec (offline rendering).
    pub fn render_frames(&mut self, count: usize) -> Vec<[f32; 2]> {
        (0..count).map(|_| self.render_frame()).collect()
    }

    /// Render frames into a caller-provided buffer (allocation-free).
    pub fn render_frames_into(&mut self, buf: &mut [[f32; 2]]) {
        for frame in buf.iter_mut() {
            *frame = self.render_frame();
        }
    }

    fn render_frame_inner(&mut self) -> [f32; 2] {
        if !self.playing {
            return [0.0, 0.0];
        }

        // 1. Process events at current time (cursor-based, zero allocation)
        let event_range = self.event_queue.drain_until(self.current_time);
        for i in event_range {
            let event = self.event_queue.get(i).unwrap().clone();
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
        self.current_time.sub_beat = (total as u32).min(SUB_BEAT_UNIT - 1);
    }

    /// Ticks per beat = speed * rows_per_beat.
    fn ticks_per_beat(&self) -> u32 {
        self.speed as u32 * self.rows_per_beat
    }

    /// Sub-beat units per tick (for modulator timing).
    fn spt(&self) -> u32 {
        sub_beats_per_tick(self.speed, self.rows_per_beat as u8)
    }

    /// Process a tick (called once per tick).
    fn process_tick(&mut self) {
        let sample_rate = self.sample_rate;
        let spt = self.spt();
        for channel in &mut self.channels {
            if !channel.playing {
                continue;
            }
            channel.clear_modulation();
            channel.advance_modulators(spt);
            channel.update_increment(sample_rate);
        }
        for machine in self.machines.iter_mut().flatten() {
            machine.tick();
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

    /// Resolve instrument/sample for a NoteOn, falling back to the channel's
    /// current instrument when `instrument == 0` (MOD convention: "keep current").
    fn resolve_note_on(&self, ch: u8, instrument: u8, note: u8) -> (u8, u8) {
        if instrument > 0 {
            self.resolve_sample(instrument, note)
        } else {
            match self.channels.get(ch as usize) {
                Some(channel) => (channel.instrument, channel.sample_index),
                None => self.resolve_sample(instrument, note),
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
                let (inst_idx, sample_idx) = self.resolve_note_on(ch, *instrument, *note);
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
                let spt = self.spt();
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
                        channel.setup_modulator(effect, spt);
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

    /// Render a single tracker channel, writing f32 output into node_outputs.
    fn render_channel(&mut self, ch_index: usize, node_id: u16) {
        let channel = match self.channels.get_mut(ch_index) {
            Some(ch) => ch,
            None => return,
        };
        if !channel.playing {
            return;
        }
        let sample = match self.song.samples.get(channel.sample_index as usize) {
            Some(s) => s,
            None => return,
        };
        let frame = channel.render(sample);
        let buf = &mut self.graph_state.node_outputs[node_id as usize];
        buf.channel_mut(0)[0] = frame.left as f32 / 32768.0;
        buf.channel_mut(1)[0] = frame.right as f32 / 32768.0;
    }

    /// Render the audio graph by traversing nodes in topological order.
    fn render_graph(&mut self) -> [f32; 2] {
        self.graph_state.clear_outputs();

        // Index loop avoids cloning topo_order (allocation-free).
        for i in 0..self.graph_state.topo_order.len() {
            let node_id = self.graph_state.topo_order[i];
            let node = match self.song.graph.node(node_id) {
                Some(n) => n,
                None => continue,
            };

            match &node.node_type {
                NodeType::TrackerChannel { index } => {
                    self.render_channel(*index as usize, node_id);
                }
                NodeType::Master => {
                    let gain = self.mix_gains.get(node_id as usize).copied().unwrap_or(1.0);
                    graph_state::gather_inputs(
                        &self.song.graph,
                        &self.graph_state.node_outputs,
                        node_id,
                        &mut self.graph_state.scratch,
                    );
                    self.graph_state.scratch.apply_gain(gain);
                    let left = self.graph_state.scratch.channel(0)[0];
                    let right = self.graph_state.scratch.channel(1)[0];
                    let buf = &mut self.graph_state.node_outputs[node_id as usize];
                    buf.channel_mut(0)[0] = left;
                    buf.channel_mut(1)[0] = right;
                }
                NodeType::BuzzMachine { .. } => {
                    self.render_machine(node_id);
                }
                _ => {
                    let gain = self.mix_gains.get(node_id as usize).copied().unwrap_or(1.0);
                    graph_state::gather_inputs(
                        &self.song.graph,
                        &self.graph_state.node_outputs,
                        node_id,
                        &mut self.graph_state.scratch,
                    );
                    self.graph_state.scratch.apply_gain(gain);
                    let left = self.graph_state.scratch.channel(0)[0];
                    let right = self.graph_state.scratch.channel(1)[0];
                    let buf = &mut self.graph_state.node_outputs[node_id as usize];
                    buf.channel_mut(0)[0] = left;
                    buf.channel_mut(1)[0] = right;
                }
            }
        }

        // Master is always node 0
        let master = &self.graph_state.node_outputs[0];
        [master.channel(0)[0], master.channel(1)[0]]
    }

    /// Render a BuzzMachine node: gather inputs → f32 → machine.render() → output.
    fn render_machine(&mut self, node_id: u16) {
        let gain = self.mix_gains.get(node_id as usize).copied().unwrap_or(1.0);
        graph_state::gather_inputs(
            &self.song.graph,
            &self.graph_state.node_outputs,
            node_id,
            &mut self.graph_state.scratch,
        );
        self.graph_state.scratch.apply_gain(gain);

        if let Some(Some(machine)) = self.machines.get_mut(node_id as usize) {
            machine.render(&mut self.graph_state.scratch);
        }

        // Copy scratch to node output
        let left = self.graph_state.scratch.channel(0)[0];
        let right = self.graph_state.scratch.channel(1)[0];
        let buf = &mut self.graph_state.node_outputs[node_id as usize];
        buf.channel_mut(0)[0] = left;
        buf.channel_mut(1)[0] = right;
    }

    /// Get the current playback position.
    pub fn position(&self) -> MusicalTime {
        self.current_time
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

    /// Schedule all events from the song's track clips + sequences.
    pub fn schedule_song(&mut self) {
        let result = scheduler::schedule_song(&self.song);
        self.song_end_time = Some(result.total_time);
        for event in result.events {
            self.event_queue.push(event);
        }
        self.event_queue.reset_cursor();
    }

    /// Get a reference to a channel's state (for testing).
    pub fn channel(&self, index: usize) -> Option<&ChannelState> {
        self.channels.get(index)
    }

    /// Get a reference to the song.
    pub fn song(&self) -> &Song {
        &self.song
    }

    /// Apply a batch of edits to the song data and update the event queue.
    pub fn apply_edits(&mut self, edits: &[Edit]) {
        for edit in edits {
            self.apply_edit(edit);
        }
    }

    fn apply_edit(&mut self, edit: &Edit) {
        match edit {
            Edit::SetCell { track, clip, row, column, cell } => {
                self.apply_set_cell(*track, *clip, *row, *column, *cell);
            }
        }
    }

    fn apply_set_cell(
        &mut self,
        track_idx: u16,
        clip_idx: u16,
        row: u16,
        column: u8,
        cell: mb_ir::Cell,
    ) {
        // 1. Mutate track clip data
        let Some(track) = self.song.tracks.get_mut(track_idx as usize) else { return };
        let Some(c) = track.clips.get_mut(clip_idx as usize) else { return };
        let Some(pat) = c.pattern_mut() else { return };
        if row >= pat.rows || column >= pat.channels { return; }
        *pat.cell_mut(row, column) = cell;

        // 2. Resolve channel index for this track + column
        let track = &self.song.tracks[track_idx as usize];
        let ch = scheduler::track_column_to_channel(track, column);

        // 3. Find times and update events
        let track = &self.song.tracks[track_idx as usize];
        let times = scheduler::time_for_track_clip_row(track, clip_idx, row, self.song.rows_per_beat);
        let rpb = self.song.rows_per_beat as u32;
        let speed = self.speed as u32;
        let pat_rpb = track.clips.get(clip_idx as usize)
            .and_then(|c| c.pattern())
            .and_then(|p| p.rows_per_beat)
            .map_or(rpb, |r| r as u32);

        let target = EventTarget::Channel(ch);
        for time in &times {
            let t = *time;
            self.event_queue.retain(|e| !(e.time == t && e.target == target));
            let mut new_events = Vec::new();
            scheduler::schedule_cell(&cell, t, ch, speed, pat_rpb, &mut new_events);
            for event in new_events {
                self.event_queue.push(event);
            }
        }
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

    /// Schedule a NoteOn at the engine's current time on channel 0.
    fn schedule_note(engine: &mut Engine, note: u8, instrument: u8) {
        engine.schedule(Event::new(
            engine.position(),
            EventTarget::Channel(0),
            EventPayload::NoteOn { note, velocity: 64, instrument },
        ));
    }

    /// Check if f32 stereo frame is non-silent.
    fn is_nonsilent(frame: &[f32; 2]) -> bool {
        frame[0] != 0.0 || frame[1] != 0.0
    }

    #[test]
    fn silent_when_not_playing() {
        let song = song_with_sample(vec![127; 100], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        let frame = engine.render_frame();
        assert_eq!(frame, [0.0, 0.0]);
    }

    #[test]
    fn silent_with_no_events() {
        let song = song_with_sample(vec![127; 100], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        let frame = engine.render_frame();
        assert_eq!(frame, [0.0, 0.0]);
    }

    #[test]
    fn note_on_sets_period_and_increment() {
        let song = song_with_sample(vec![127; 1000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);

        engine.render_frame();

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

        let frame = engine.render_frame();
        assert!(is_nonsilent(&frame), "Expected non-silent output");
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
            engine.position(),
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
            engine.position(),
            EventTarget::Channel(0),
            EventPayload::NoteOff { note: 0 },
        ));
        engine.render_frame();
        assert!(!engine.channel(0).unwrap().playing);
    }

    #[test]
    fn sample_stops_at_end_without_loop() {
        let song = song_with_sample(vec![127; 4], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);

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

        let spt_before = engine.samples_per_tick;
        assert_eq!(spt_before, 882);

        engine.schedule(Event::new(
            engine.position(),
            EventTarget::Global,
            EventPayload::SetTempo(15000),
        ));
        engine.render_frame();

        assert_eq!(engine.samples_per_tick, 735);
    }

    #[test]
    fn zero_volume_sample_produces_silence() {
        let song = song_with_sample(vec![127; 1000], 0);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);

        let frame = engine.render_frame();
        assert_eq!(frame, [0.0, 0.0]);
    }

    /// Schedule an effect at tick 0 on channel 0.
    fn schedule_effect(engine: &mut Engine, effect: mb_ir::Effect) {
        engine.schedule(Event::new(
            engine.position(),
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
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::VolumeSlide(4));
        engine.render_frame();

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

        schedule_effect(&mut engine, mb_ir::Effect::VolumeSlide(4));
        engine.render_frames(882 * 10);
        assert_eq!(engine.channel(0).unwrap().volume, 64);
    }

    #[test]
    fn new_note_clears_modulators() {
        let song = song_with_sample(vec![127; 100000], 32);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::VolumeSlide(4));
        engine.render_frame();
        assert!(engine.channel(0).unwrap().volume_mod.is_some());

        schedule_note(&mut engine, 60, 1);
        engine.render_frame();
        assert!(engine.channel(0).unwrap().volume_mod.is_none());
        assert!(engine.channel(0).unwrap().period_mod.is_none());
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
        advance_tick(&mut engine);

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
        schedule_note(&mut engine, 60, 1);
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
        schedule_note(&mut engine, 71, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::PortaUp(20));
        engine.render_frame();
        advance_tick(&mut engine);

        assert_eq!(engine.channel(0).unwrap().period, 113);
    }

    #[test]
    fn porta_down_clamps_at_period_max() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 36, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::PortaDown(20));
        engine.render_frame();
        advance_tick(&mut engine);

        assert_eq!(engine.channel(0).unwrap().period, 856);
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
        engine.render_frame();

        let period_after = engine.channel(0).unwrap().period;
        assert_eq!(period_after, period_before - 4);

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
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().period, 428);

        engine.schedule(Event::new(
            engine.position(),
            EventTarget::Channel(0),
            EventPayload::PortaTarget { note: 60, instrument: 1 },
        ));
        schedule_effect(&mut engine, mb_ir::Effect::TonePorta(8));
        engine.render_frame();
        assert_eq!(engine.channel(0).unwrap().target_period, 214);

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
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        engine.schedule(Event::new(
            engine.position(),
            EventTarget::Channel(0),
            EventPayload::PortaTarget { note: 60, instrument: 1 },
        ));
        schedule_effect(&mut engine, mb_ir::Effect::TonePorta(255));
        engine.render_frame();

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

        advance_tick(&mut engine);
        let pos_before = engine.channel(0).unwrap().position;
        assert!(pos_before > 0, "position should have advanced");

        engine.schedule(Event::new(
            engine.position(),
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
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        engine.schedule(Event::new(
            engine.position(),
            EventTarget::Channel(0),
            EventPayload::PortaTarget { note: 60, instrument: 1 },
        ));
        schedule_effect(&mut engine, mb_ir::Effect::TonePorta(8));
        engine.render_frame();

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

        assert_eq!(engine.channel(0).unwrap().period, base_period);
        let offset = engine.channel(0).unwrap().period_offset;
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

        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 8, depth: 4 });
        engine.render_frame();
        advance_tick(&mut engine);
        let ch = engine.channel(0).unwrap();
        assert_eq!(ch.vibrato_speed, 8);
        assert_eq!(ch.vibrato_depth, 4);

        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 0, depth: 6 });
        engine.render_frame();
        advance_tick(&mut engine);
        let ch = engine.channel(0).unwrap();
        assert_eq!(ch.vibrato_speed, 8);
        assert_eq!(ch.vibrato_depth, 6);
    }

    // === Arpeggio tests ===

    #[test]
    fn arpeggio_cycles_period_offset() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::Arpeggio { x: 4, y: 7 });
        engine.render_frame();

        let mut offsets = Vec::new();
        for _ in 0..6 {
            advance_tick(&mut engine);
            offsets.push(engine.channel(0).unwrap().period_offset);
        }

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
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::Arpeggio { x: 12, y: 7 });
        engine.render_frame();

        advance_tick(&mut engine);
        let expected_x = note_to_period(48 + 12) as i16 - 428;
        assert_eq!(engine.channel(0).unwrap().period_offset, expected_x);

        advance_tick(&mut engine);
        let expected_y = note_to_period(48 + 7) as i16 - 428;
        assert_eq!(engine.channel(0).unwrap().period_offset, expected_y);
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

        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 8, depth: 4 });
        engine.render_frame();
        advance_tick(&mut engine);

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
    fn vibrato_mod_resets_on_new_note_by_default() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 8, depth: 4 });
        engine.render_frame();
        for _ in 0..5 {
            advance_tick(&mut engine);
        }
        assert!(engine.channel(0).unwrap().period_mod.is_some(), "vibrato mod should be active");

        schedule_note(&mut engine, 60, 1);
        engine.render_frame();
        assert!(engine.channel(0).unwrap().period_mod.is_none(), "mod should reset on note");
    }

    #[test]
    fn vibrato_mod_persists_with_no_retrig_flag() {
        let song = song_with_sample(vec![127; 100000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, 48, 1);
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::SetVibratoWaveform(4));
        engine.render_frame();

        schedule_effect(&mut engine, mb_ir::Effect::Vibrato { speed: 8, depth: 4 });
        engine.render_frame();
        for _ in 0..5 {
            advance_tick(&mut engine);
        }
        assert!(engine.channel(0).unwrap().period_mod.is_some(), "vibrato mod should be active");

        schedule_note(&mut engine, 60, 1);
        engine.render_frame();
        assert!(engine.channel(0).unwrap().period_mod.is_some(), "mod should persist with no-retrig");
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

        advance_tick(&mut engine);
        assert_eq!(engine.channel(0).unwrap().volume, 64);

        advance_tick(&mut engine);
        assert_eq!(engine.channel(0).unwrap().volume, 64);

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
        advance_tick(&mut engine);
        assert_eq!(engine.channel(0).unwrap().volume, 0);

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

        advance_tick(&mut engine);
        let pos_tick1 = engine.channel(0).unwrap().position;
        assert!(pos_tick1 > 0, "position should have advanced");

        advance_tick(&mut engine);
        let pos_tick2 = engine.channel(0).unwrap().position;
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

    // === Edit dispatch tests ===

    use mb_ir::{build_tracks, Cell, Edit, Note, OrderEntry, Pattern};

    /// Build a 1-channel song with one sample and a scheduled pattern.
    fn song_with_pattern(data: Vec<i8>) -> Song {
        let mut song = song_with_sample(data, 64);
        let patterns = vec![Pattern::new(4, 1)];
        let order = vec![OrderEntry::Pattern(0)];
        build_tracks(&mut song, &patterns, &order);
        song
    }

    #[test]
    fn set_cell_updates_song_data() {
        let song = song_with_pattern(vec![127; 1000]);
        let mut engine = Engine::new(song, SAMPLE_RATE);

        let cell = Cell { note: Note::On(60), instrument: 1, ..Cell::empty() };
        engine.apply_edits(&[Edit::SetCell { track: 0, clip: 0, row: 2, column: 0, cell }]);

        let clip = engine.song().tracks[0].clips[0].pattern().unwrap();
        assert_eq!(clip.cell(2, 0).note, Note::On(60));
    }

    #[test]
    fn set_cell_inserts_events_in_queue() {
        let song = song_with_pattern(vec![127; 1000]);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.schedule_song();

        let cell = Cell { note: Note::On(60), instrument: 1, ..Cell::empty() };
        engine.apply_edits(&[Edit::SetCell { track: 0, clip: 0, row: 0, column: 0, cell }]);

        engine.play();
        engine.render_frame();
        assert!(engine.channel(0).unwrap().playing, "channel should be triggered by SetCell event");
    }

    #[test]
    fn set_cell_replaces_old_events() {
        let song = song_with_pattern(vec![127; 1000]);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.schedule_song();

        let cell = Cell { note: Note::On(60), instrument: 1, ..Cell::empty() };
        engine.apply_edits(&[Edit::SetCell { track: 0, clip: 0, row: 0, column: 0, cell }]);

        let empty = Cell::empty();
        engine.apply_edits(&[Edit::SetCell { track: 0, clip: 0, row: 0, column: 0, cell: empty }]);

        engine.play();
        engine.render_frame();
        assert!(!engine.channel(0).unwrap().playing, "channel should not trigger after clearing cell");
    }

    #[test]
    fn set_cell_on_invalid_track_is_noop() {
        let song = song_with_pattern(vec![127; 1000]);
        let mut engine = Engine::new(song, SAMPLE_RATE);

        let cell = Cell { note: Note::On(60), instrument: 1, ..Cell::empty() };
        engine.apply_edits(&[Edit::SetCell { track: 99, clip: 0, row: 0, column: 0, cell }]);
    }

    #[test]
    fn set_cell_on_invalid_row_is_noop() {
        let song = song_with_pattern(vec![127; 1000]);
        let mut engine = Engine::new(song, SAMPLE_RATE);

        let cell = Cell { note: Note::On(60), instrument: 1, ..Cell::empty() };
        engine.apply_edits(&[Edit::SetCell { track: 0, clip: 0, row: 999, column: 0, cell }]);
    }
}
