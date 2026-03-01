//! Main playback engine.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use mb_ir::{Edit, Event, EventPayload, EventTarget, MusicalTime, NodeType, Song, SUB_BEAT_UNIT};

use crate::event_queue::EventQueue;
use crate::graph_state::{self, GraphState};
use crate::machine::Machine;
use crate::machines;
use crate::scheduler;

/// The main playback engine.
pub struct Engine {
    /// The song being played
    song: Song,
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
            if machine_name == "Tracker" {
                let mix_gain = tracker_mix_gain(song.channels.len() as u32);
                let mut machine = machines::tracker::TrackerMachine::new(
                    &song.channels,
                    song.samples.clone(),
                    song.instruments.clone(),
                    song.initial_speed,
                    song.rows_per_beat,
                    sample_rate,
                    mix_gain,
                );
                machine.init(sample_rate);
                return Some(Box::new(machine) as Box<dyn Machine>);
            }
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

/// Compute the mix gain for the TrackerMachine.
///
/// Previously N TrackerChannel nodes fed into AmigaFilter, which applied
/// `1.0 / 2^compute_mix_shift(N)`. Now the TrackerMachine applies this
/// attenuation internally when summing channels.
fn tracker_mix_gain(num_channels: u32) -> f32 {
    let shift = compute_mix_shift(num_channels);
    1.0 / (1u32 << shift) as f32
}

impl Engine {
    /// Create a new engine for the given song.
    pub fn new(song: Song, sample_rate: u32) -> Self {
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

    /// Process a tick (called once per tick).
    fn process_tick(&mut self) {
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
            EventTarget::NodeChannel(node_id, ch) => {
                if let Some(Some(machine)) = self.machines.get_mut(node_id as usize) {
                    machine.apply_event(ch, &event.payload);
                }
            }
            EventTarget::Global => {
                self.apply_global_event(&event.payload);
            }
            EventTarget::Node(_id) => {
                // TODO: Route to graph node
            }
        }
    }

    /// Apply an event to a legacy Channel target (forwards to TrackerMachine if available).
    fn apply_channel_event(&mut self, _ch: u8, _payload: &EventPayload) {
        // Legacy Channel events are no longer used — all tracker events
        // now route via NodeChannel to TrackerMachine.
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
                for machine in self.machines.iter_mut().flatten() {
                    machine.set_speed(*speed);
                }
            }
            _ => {}
        }
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

    /// Get a reference to a machine by node ID (for testing).
    pub fn machine(&self, node_id: u16) -> Option<&dyn Machine> {
        self.machines.get(node_id as usize)?.as_deref()
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

        // 2. Resolve event target for this track + column
        let track = &self.song.tracks[track_idx as usize];
        let target = scheduler::target_for_track_column(track, column);

        // 3. Find times and update events
        let track = &self.song.tracks[track_idx as usize];
        let times = scheduler::time_for_track_clip_row(track, clip_idx, row, self.song.rows_per_beat);
        let rpb = self.song.rows_per_beat as u32;
        let speed = self.speed as u32;
        let pat_rpb = track.clips.get(clip_idx as usize)
            .and_then(|c| c.pattern())
            .and_then(|p| p.rows_per_beat)
            .map_or(rpb, |r| r as u32);

        for time in &times {
            let t = *time;
            self.event_queue.retain(|e| !(e.time == t && e.target == target));
            let mut new_events = Vec::new();
            scheduler::schedule_cell(&cell, t, target, speed, pat_rpb, &mut new_events);
            for event in new_events {
                self.event_queue.push(event);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_ir::{find_tracker_node, Instrument, Sample, SampleData};

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

    /// Find the Tracker node ID in a song's graph.
    fn tracker_node(song: &Song) -> u16 {
        find_tracker_node(&song.graph).expect("no Tracker node")
    }

    /// Schedule a NoteOn at the engine's current time via NodeChannel to Tracker.
    fn schedule_note(engine: &mut Engine, song: &Song, note: u8, instrument: u8) {
        let node_id = tracker_node(song);
        engine.schedule(Event::new(
            engine.position(),
            EventTarget::NodeChannel(node_id, 0),
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
    fn note_on_produces_nonsilent_output() {
        let song = song_with_sample(vec![127; 1000], 64);
        let mut engine = Engine::new(song.clone(), SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, &song, 48, 1);
        let frame = engine.render_frame();
        assert!(is_nonsilent(&frame), "Expected non-silent output");
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
        let mut engine = Engine::new(song.clone(), SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, &song, 48, 1);
        let frame = engine.render_frame();
        assert_eq!(frame, [0.0, 0.0]);
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
        let frame = engine.render_frame();
        assert!(is_nonsilent(&frame), "SetCell should produce audio output");
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
        let frame = engine.render_frame();
        assert_eq!(frame, [0.0, 0.0], "cleared cell should be silent");
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
