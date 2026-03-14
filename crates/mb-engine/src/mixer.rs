//! Main playback engine.

use alloc::boxed::Box;
use alloc::vec::Vec;
use mb_ir::{Edit, Event, EventPayload, EventTarget, MusicalTime, NodeType, Song, SUB_BEAT_UNIT};

use crate::clip_source::ClipSourceState;
use crate::event_source::EventSource;
use crate::graph_state::{self, GraphState};
use crate::machine::Machine;
use crate::machines;

/// The main playback engine.
pub struct Engine {
    /// The song being played
    song: Song,
    /// Runtime graph state (node outputs + topological order)
    graph_state: GraphState,
    /// Lazy event sources (one per tracker track)
    sources: Vec<ClipSourceState>,
    /// Scratch buffer for drained events (reused each frame)
    event_buf: Vec<Event>,
    /// Manually scheduled events (via `schedule()`)
    pending_events: Vec<Event>,
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
    /// Machine instances (indexed by NodeId; `Some` only for BuzzMachine nodes).
    machines: Vec<Option<Box<dyn Machine>>>,
    /// Per-node bypass flags (indexed by NodeId).
    node_bypass: Vec<bool>,
}

/// Find the channel settings slice for a tracker node from the song's tracks.
fn channels_for_node(song: &Song, node_id: u16) -> &[mb_ir::ChannelSettings] {
    song.tracks.iter()
        .find(|t| t.machine_node == Some(node_id) && t.num_channels > 0)
        .map(|t| {
            let base = t.base_channel as usize;
            let end = base + t.num_channels as usize;
            &song.channels[base..end.min(song.channels.len())]
        })
        .unwrap_or(&[])
}

/// Instantiate machines for all BuzzMachine nodes in the graph.
fn init_machines(song: &Song, sample_rate: u32) -> Vec<Option<Box<dyn Machine>>> {
    song.graph.nodes.iter().map(|node| {
        if let NodeType::Machine { is_tracker, machine_name } = &node.node_type {
            if *is_tracker {
                let ch_settings = channels_for_node(song, node.id);
                let mix_gain = tracker_mix_gain(ch_settings.len() as u32);
                let mut machine = machines::tracker::TrackerMachine::new(
                    ch_settings,
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

/// Compute the right-shift needed to attenuate N inputs to prevent clipping.
///
/// For N inputs with L-R panning, at most N/2 contribute to one side.
/// Returns the number of bits to shift right.
fn compute_mix_shift(input_count: u32) -> u32 {
    let sides = (input_count / 2).max(1);
    sides.next_power_of_two().trailing_zeros()
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

/// Copy N frames from scratch buffer to node output buffer.
fn copy_scratch_to_output(scratch: &mb_ir::AudioBuffer, output: &mut mb_ir::AudioBuffer, frames: usize) {
    let src_l = scratch.channel(0);
    let src_r = scratch.channel(1);
    let (dst_l, dst_r) = output.channels_mut_2(0, 1);
    dst_l[..frames].copy_from_slice(&src_l[..frames]);
    dst_r[..frames].copy_from_slice(&src_r[..frames]);
}

impl Engine {
    /// Create a new engine for the given song.
    pub fn new(song: Song, sample_rate: u32) -> Self {
        let tempo = song.initial_tempo;
        let speed = song.initial_speed;
        let rows_per_beat = song.rows_per_beat as u32;

        let graph_state = GraphState::from_graph(&song.graph);

        // Instantiate machines for BuzzMachine nodes
        let machines_vec = init_machines(&song, sample_rate);
        let node_bypass = vec![false; song.graph.nodes.len()];

        let mut engine = Self {
            song,
            graph_state,
            sources: Vec::new(),
            event_buf: Vec::new(),
            pending_events: Vec::new(),
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
            machines: machines_vec,
            node_bypass,
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
        let mut buf = [[0.0f32; 2]];
        self.render_block(&mut buf);
        buf[0]
    }

    /// Render multiple frames, returning a new Vec (offline rendering).
    pub fn render_frames(&mut self, count: usize) -> Vec<[f32; 2]> {
        let mut buf = vec![[0.0f32; 2]; count];
        self.render_block(&mut buf);
        buf
    }

    /// Drain events from all sources + pending into `event_buf`.
    fn drain_all_sources(&mut self, time: MusicalTime) {
        self.event_buf.clear();
        // Include manually scheduled events at or before current time
        let mut i = 0;
        while i < self.pending_events.len() {
            if self.pending_events[i].time <= time {
                self.event_buf.push(self.pending_events.swap_remove(i));
            } else {
                i += 1;
            }
        }
        for source in &mut self.sources {
            source.drain_until(time, &self.song, &mut self.event_buf);
        }
        self.event_buf.sort_unstable_by(|a, b| a.time.cmp(&b.time));

        // Once all sources are exhausted, lock in the end time so is_finished()
        // triggers on the same frame (no 1-frame lag).
        if self.song_end_time.is_none()
            && !self.sources.is_empty()
            && self.sources.iter().all(|s| s.end_time().is_some())
        {
            self.song_end_time = self.sources.iter()
                .filter_map(|s| s.end_time())
                .max();
        }
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
            EventTarget::Channel(_) => {}
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
                for source in &mut self.sources {
                    source.set_speed(*speed);
                }
            }
            _ => {}
        }
    }

    /// Render the audio graph for `frames` frames.
    fn render_graph_block(&mut self, frames: usize) {
        // Set active frame count on all buffers
        let f = frames as u16;
        for buf in &mut self.graph_state.node_outputs {
            buf.set_frames(f);
        }
        self.graph_state.scratch.set_frames(f);

        self.graph_state.clear_outputs();

        for i in 0..self.graph_state.topo_order.len() {
            let node_id = self.graph_state.topo_order[i];
            let node = match self.song.graph.node(node_id) {
                Some(n) => n,
                None => continue,
            };

            match &node.node_type {
                NodeType::Machine { .. } => {
                    self.render_machine_block(node_id, frames);
                }
                NodeType::Master => {
                    graph_state::gather_inputs(
                        &self.graph_state.conn_by_dest,
                        &self.graph_state.node_outputs,
                        node_id,
                        &mut self.graph_state.scratch,
                    );
                    copy_scratch_to_output(&self.graph_state.scratch, &mut self.graph_state.node_outputs[node_id as usize], frames);
                }
            }
        }
    }

    /// Render a BuzzMachine node for N frames.
    fn render_machine_block(&mut self, node_id: u16, frames: usize) {
        if self.node_bypass.get(node_id as usize).copied().unwrap_or(false) {
            return;
        }

        graph_state::gather_inputs(
            &self.graph_state.conn_by_dest,
            &self.graph_state.node_outputs,
            node_id,
            &mut self.graph_state.scratch,
        );

        if let Some(Some(machine)) = self.machines.get_mut(node_id as usize) {
            machine.render(&mut self.graph_state.scratch);
        }

        copy_scratch_to_output(&self.graph_state.scratch, &mut self.graph_state.node_outputs[node_id as usize], frames);
    }

    /// Render a block of audio into the output buffer.
    ///
    /// Sub-block splitting: drains events, finds tick boundaries, renders
    /// sub-blocks between boundaries, dispatches events and advances time.
    pub fn render_block(&mut self, output: &mut [[f32; 2]]) {
        #[cfg(feature = "alloc_check")]
        {
            assert_no_alloc::assert_no_alloc(|| self.render_block_inner(output));
        }
        #[cfg(not(feature = "alloc_check"))]
        {
            self.render_block_inner(output);
        }
    }

    fn render_block_inner(&mut self, output: &mut [[f32; 2]]) {
        if !self.playing {
            for frame in output.iter_mut() { *frame = [0.0, 0.0]; }
            return;
        }

        let total_frames = output.len();
        let mut offset = 0;

        while offset < total_frames {
            // Drain events at current time
            self.drain_all_sources(self.current_time);
            for i in 0..self.event_buf.len() {
                let event = self.event_buf[i].clone();
                self.dispatch_event(&event);
            }

            // Find sub-block size: frames until next tick boundary, capped by buffer capacity
            let remaining = total_frames - offset;
            let frames_to_tick = (self.samples_per_tick - self.sample_counter) as usize;
            let sub_block = remaining.min(frames_to_tick).min(mb_ir::BLOCK_SIZE);

            // Render graph for sub-block
            self.render_graph_block(sub_block);

            // Copy master output to caller's buffer
            let master = &self.graph_state.node_outputs[0];
            let left = master.channel(0);
            let right = master.channel(1);
            for i in 0..sub_block {
                output[offset + i] = [left[i], right[i]];
            }

            // Advance time by sub_block samples
            self.sample_counter += sub_block as u32;
            offset += sub_block;

            if self.sample_counter >= self.samples_per_tick {
                self.sample_counter = 0;
                self.advance_tick();
                self.process_tick();
            }
        }
    }

    /// Get the current playback position.
    pub fn position(&self) -> MusicalTime {
        self.current_time
    }

    /// Returns true when playback has reached the song's end time.
    ///
    /// End time is determined from source exhaustion (accounts for PatternBreak/
    /// PositionJump shortening a pattern) or from an explicit `song_end_time`.
    pub fn is_finished(&self) -> bool {
        if let Some(end) = self.song_end_time {
            return self.current_time >= end;
        }
        // All sources must be exhausted and we must be past all their end times
        if self.sources.is_empty() {
            return false;
        }
        self.sources.iter().all(|s| {
            s.end_time()
                .is_some_and(|end| self.current_time >= end)
        })
    }

    /// Schedule an event for dispatch on the next render call.
    pub fn schedule(&mut self, event: Event) {
        self.pending_events.push(event);
    }

    /// Build lazy event sources from the song's tracks.
    pub fn schedule_song(&mut self) {
        self.song_end_time = None; // Determined lazily from source exhaustion
        self.sources = (0..self.song.tracks.len())
            .map(|i| ClipSourceState::new(&self.song, i))
            .collect();
        // Pre-allocate event buffer to avoid allocations in the hot path.
        // Worst case: every column on every track produces ~3 events per row.
        let total_columns: usize = self.song.tracks.iter()
            .map(|t| t.num_channels as usize)
            .sum();
        self.event_buf.reserve(total_columns * 3 + 16);
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
            Edit::SetNodeBypass { node, bypassed } => {
                if let Some(slot) = self.node_bypass.get_mut(*node as usize) {
                    *slot = *bypassed;
                }
            }
            Edit::SetSeqEntry { .. } => {} // Sequence edits handled by Controller only
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
        // Mutate track clip data. ClipSources read lazily, so edits
        // ahead of the cursor are picked up automatically.
        let Some(track) = self.song.tracks.get_mut(track_idx as usize) else { return };
        let Some(c) = track.clips.get_mut(clip_idx as usize) else { return };
        let Some(pat) = c.pattern_mut() else { return };
        if row >= pat.rows || column >= pat.channels { return; }
        *pat.cell_mut(row, column) = cell;
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

        // Add a Track linking the Tracker node to channels (required by init_machines)
        let tracker_id = find_tracker_node(&song.graph);
        song.tracks.push(mb_ir::Track::new(tracker_id, 0, 1));

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

    // === Node bypass tests ===

    fn engine_with_note(song: &Song) -> Engine {
        let mut engine = Engine::new(song.clone(), SAMPLE_RATE);
        engine.play();
        schedule_note(&mut engine, song, 48, 1);
        engine
    }

    #[test]
    fn bypass_silences_machine() {
        let song = song_with_sample(vec![127; 1000], 64);
        let node_id = tracker_node(&song);
        let mut engine = engine_with_note(&song);

        engine.apply_edits(&[Edit::SetNodeBypass { node: node_id, bypassed: true }]);
        let frame = engine.render_frame();
        assert_eq!(frame, [0.0, 0.0], "bypassed node should produce silence");
    }

    #[test]
    fn unbypass_restores_audio() {
        let song = song_with_sample(vec![127; 1000], 64);
        let node_id = tracker_node(&song);
        let mut engine = engine_with_note(&song);

        engine.apply_edits(&[Edit::SetNodeBypass { node: node_id, bypassed: true }]);
        engine.render_frame();

        engine.apply_edits(&[Edit::SetNodeBypass { node: node_id, bypassed: false }]);
        let frame = engine.render_frame();
        assert!(is_nonsilent(&frame), "unbypassed node should produce audio");
    }

    #[test]
    fn bypass_invalid_node_is_noop() {
        let song = song_with_sample(vec![127; 1000], 64);
        let mut engine = Engine::new(song, SAMPLE_RATE);
        engine.apply_edits(&[Edit::SetNodeBypass { node: 999, bypassed: true }]);
    }
}
