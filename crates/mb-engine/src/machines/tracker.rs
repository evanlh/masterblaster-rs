//! TrackerMachine — encapsulates all tracker channel logic as a Machine.
//!
//! Replaces the per-channel handling previously embedded in the Engine.
//! One TrackerMachine holds N channels and renders them into a single
//! stereo AudioBuffer with mix_gain attenuation.

use alloc::vec::Vec;

use mb_ir::{
    AudioBuffer, AudioStream, ChannelConfig, ChannelSettings, Effect,
    EventPayload, Instrument, Sample, sub_beats_per_tick,
};

use crate::channel::ChannelState;
use crate::frequency::note_to_period;
use crate::machine::{Machine, MachineInfo, MachineType};

static INFO: MachineInfo = MachineInfo {
    name: "Tracker",
    short_name: "Tracker",
    author: "masterblaster",
    machine_type: MachineType::Generator,
    params: &[],
};

/// A Machine that drives N tracker channels, rendering and mixing them.
pub struct TrackerMachine {
    channels: Vec<ChannelState>,
    samples: Vec<Sample>,
    instruments: Vec<Instrument>,
    speed: u8,
    rows_per_beat: u8,
    sample_rate: u32,
    mix_gain: f32,
}

impl TrackerMachine {
    /// Create a new TrackerMachine.
    pub fn new(
        channel_settings: &[ChannelSettings],
        samples: Vec<Sample>,
        instruments: Vec<Instrument>,
        speed: u8,
        rows_per_beat: u8,
        sample_rate: u32,
        mix_gain: f32,
    ) -> Self {
        let channels = channel_settings
            .iter()
            .map(|s| {
                let mut ch = ChannelState::new();
                ch.panning = s.initial_pan;
                ch
            })
            .collect();

        Self {
            channels,
            samples,
            instruments,
            speed,
            rows_per_beat,
            sample_rate,
            mix_gain,
        }
    }

    /// Access a channel (for testing).
    #[cfg(test)]
    pub(crate) fn channel(&self, index: usize) -> Option<&ChannelState> {
        self.channels.get(index)
    }

    /// Sub-beat units per tick (for modulator timing).
    fn spt(&self) -> u32 {
        sub_beats_per_tick(self.speed, self.rows_per_beat)
    }

    /// Look up the sample index for an instrument + note.
    fn resolve_sample(&self, instrument: u8, note: u8) -> (u8, u8) {
        let inst_idx = if instrument > 0 { instrument - 1 } else { 0 };
        let sample_idx = self
            .instruments
            .get(inst_idx as usize)
            .map(|inst| inst.sample_map[note as usize])
            .unwrap_or(inst_idx);
        (inst_idx, sample_idx)
    }

    /// Get the c4_speed for a sample index.
    fn sample_c4_speed(&self, sample_idx: u8) -> u32 {
        self.samples
            .get(sample_idx as usize)
            .map(|s| s.c4_speed)
            .unwrap_or(8363)
    }

    /// Resolve instrument/sample for NoteOn, falling back to channel's current.
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

    /// Apply an event payload to a specific channel.
    fn apply_channel_event(&mut self, ch: u8, payload: &EventPayload) {
        match payload {
            EventPayload::NoteOn { note, instrument, velocity: _ } => {
                let (inst_idx, sample_idx) = self.resolve_note_on(ch, *instrument, *note);
                let c4_speed = self.sample_c4_speed(sample_idx);
                let default_vol = self.samples.get(sample_idx as usize).map(|s| s.default_volume);
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
                let default_vol = self.samples.get(sample_idx as usize).map(|s| s.default_volume);
                let target_period = note_to_period(*note);

                if let Some(channel) = self.channels.get_mut(ch as usize) {
                    channel.target_period = target_period;
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
                    if let Effect::TonePorta(speed) = effect {
                        if *speed > 0 {
                            channel.porta_speed = *speed;
                        }
                    }

                    if effect.is_row_effect() {
                        channel.apply_row_effect(effect);
                        channel.update_increment(self.sample_rate);
                    } else {
                        channel.setup_modulator(effect, spt);
                    }
                }
            }
            _ => {}
        }
    }

    /// Process one tick for all channels (advance modulators, update increments).
    fn process_channels_tick(&mut self) {
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
    }
}

impl AudioStream for TrackerMachine {
    fn channel_config(&self) -> ChannelConfig {
        ChannelConfig { inputs: 0, outputs: 2 }
    }

    fn render(&mut self, output: &mut AudioBuffer) {
        // Render all channels and sum into output with mix_gain
        for channel in &mut self.channels {
            if !channel.playing {
                continue;
            }
            let sample = match self.samples.get(channel.sample_index as usize) {
                Some(s) => s,
                None => continue,
            };
            let frame = channel.render(sample);
            let left = frame.left as f32 / 32768.0;
            let right = frame.right as f32 / 32768.0;
            output.channel_mut(0)[0] += left * self.mix_gain;
            output.channel_mut(1)[0] += right * self.mix_gain;
        }
    }
}

impl Machine for TrackerMachine {
    fn info(&self) -> &MachineInfo { &INFO }

    fn init(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
    }

    fn tick(&mut self) {
        self.process_channels_tick();
    }

    fn stop(&mut self) {
        for channel in &mut self.channels {
            channel.stop();
        }
    }

    fn set_param(&mut self, _param: u16, _value: i32) {}

    fn apply_event(&mut self, channel: u8, payload: &EventPayload) {
        self.apply_channel_event(channel, payload);
    }

    fn set_speed(&mut self, speed: u8) {
        self.speed = speed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frequency::period_to_increment;
    use mb_ir::{ChannelSettings, Instrument, Sample, SampleData};

    const SR: u32 = 44100;

    fn make_machine(data: Vec<i8>, volume: u8) -> TrackerMachine {
        let settings = [ChannelSettings { initial_pan: -64, initial_vol: 64, muted: false }];
        let mut sample = Sample::new("test");
        sample.data = SampleData::Mono8(data);
        sample.default_volume = volume;
        sample.c4_speed = 8363;
        let mut inst = Instrument::new("test");
        inst.set_single_sample(0);

        // mix_gain = 1.0 (1 channel → shift=0)
        TrackerMachine::new(&settings, vec![sample], vec![inst], 6, 4, SR, 1.0)
    }

    fn note_on(machine: &mut TrackerMachine, note: u8, instrument: u8) {
        machine.apply_event(0, &EventPayload::NoteOn { note, velocity: 64, instrument });
    }

    fn effect(machine: &mut TrackerMachine, eff: Effect) {
        machine.apply_event(0, &EventPayload::Effect(eff));
    }

    #[test]
    fn note_on_sets_period_and_increment() {
        let mut m = make_machine(vec![127; 1000], 64);
        note_on(&mut m, 48, 1);
        let ch = m.channel(0).unwrap();
        assert_eq!(ch.period, 428);
        assert_eq!(ch.increment, period_to_increment(428, 8363, SR));
        assert!(ch.increment > 0);
    }

    #[test]
    fn note_on_sets_volume_from_sample() {
        let mut m = make_machine(vec![127; 1000], 48);
        note_on(&mut m, 48, 1);
        assert_eq!(m.channel(0).unwrap().volume, 48);
    }

    #[test]
    fn note_off_stops_channel() {
        let mut m = make_machine(vec![127; 1000], 64);
        note_on(&mut m, 48, 1);
        assert!(m.channel(0).unwrap().playing);
        m.apply_event(0, &EventPayload::NoteOff { note: 0 });
        assert!(!m.channel(0).unwrap().playing);
    }

    #[test]
    fn higher_note_gives_higher_increment() {
        let mut m = make_machine(vec![127; 1000], 64);
        note_on(&mut m, 48, 1);
        let inc_48 = m.channel(0).unwrap().increment;
        note_on(&mut m, 60, 1);
        let inc_60 = m.channel(0).unwrap().increment;
        assert!(inc_60 > inc_48);
        assert_eq!(inc_60, inc_48 * 2);
    }

    #[test]
    fn set_volume_effect_changes_volume() {
        let mut m = make_machine(vec![127; 1000], 64);
        note_on(&mut m, 48, 1);
        assert_eq!(m.channel(0).unwrap().volume, 64);
        effect(&mut m, Effect::SetVolume(32));
        assert_eq!(m.channel(0).unwrap().volume, 32);
    }

    #[test]
    fn set_volume_clamps_to_64() {
        let mut m = make_machine(vec![127; 1000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::SetVolume(100));
        assert_eq!(m.channel(0).unwrap().volume, 64);
    }

    #[test]
    fn fine_volume_slide_up() {
        let mut m = make_machine(vec![127; 1000], 32);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::FineVolumeSlideUp(4));
        assert_eq!(m.channel(0).unwrap().volume, 36);
    }

    #[test]
    fn fine_volume_slide_down() {
        let mut m = make_machine(vec![127; 1000], 32);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::FineVolumeSlideDown(4));
        assert_eq!(m.channel(0).unwrap().volume, 28);
    }

    #[test]
    fn fine_volume_slide_down_clamps_to_zero() {
        let mut m = make_machine(vec![127; 1000], 2);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::FineVolumeSlideDown(15));
        assert_eq!(m.channel(0).unwrap().volume, 0);
    }

    #[test]
    fn fine_porta_up_applies_once() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        let before = m.channel(0).unwrap().period;
        effect(&mut m, Effect::FinePortaUp(4));
        assert_eq!(m.channel(0).unwrap().period, before - 4);
        m.tick();
        assert_eq!(m.channel(0).unwrap().period, before - 4);
    }

    #[test]
    fn fine_porta_down_applies_once() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        let before = m.channel(0).unwrap().period;
        effect(&mut m, Effect::FinePortaDown(4));
        assert_eq!(m.channel(0).unwrap().period, before + 4);
    }

    #[test]
    fn volume_slide_modulator_advances() {
        let mut m = make_machine(vec![127; 100000], 32);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::VolumeSlide(4));
        let vol_before = m.channel(0).unwrap().volume;
        m.tick();
        let vol_after = m.channel(0).unwrap().volume;
        assert!(vol_after > vol_before);
    }

    #[test]
    fn new_note_clears_modulators() {
        let mut m = make_machine(vec![127; 100000], 32);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::VolumeSlide(4));
        assert!(m.channel(0).unwrap().volume_mod.is_some());
        note_on(&mut m, 60, 1);
        assert!(m.channel(0).unwrap().volume_mod.is_none());
        assert!(m.channel(0).unwrap().period_mod.is_none());
    }

    #[test]
    fn porta_up_decreases_period() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        let before = m.channel(0).unwrap().period;
        effect(&mut m, Effect::PortaUp(4));
        m.tick();
        assert!(m.channel(0).unwrap().period < before);
    }

    #[test]
    fn porta_down_increases_period() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 60, 1);
        let before = m.channel(0).unwrap().period;
        effect(&mut m, Effect::PortaDown(4));
        m.tick();
        assert!(m.channel(0).unwrap().period > before);
    }

    #[test]
    fn porta_up_clamps_at_period_min() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 71, 1);
        effect(&mut m, Effect::PortaUp(20));
        m.tick();
        assert_eq!(m.channel(0).unwrap().period, 113);
    }

    #[test]
    fn porta_down_clamps_at_period_max() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 36, 1);
        effect(&mut m, Effect::PortaDown(20));
        m.tick();
        assert_eq!(m.channel(0).unwrap().period, 856);
    }

    #[test]
    fn tone_porta_slides_toward_target() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        assert_eq!(m.channel(0).unwrap().period, 428);
        m.apply_event(0, &EventPayload::PortaTarget { note: 60, instrument: 1 });
        effect(&mut m, Effect::TonePorta(8));
        assert_eq!(m.channel(0).unwrap().target_period, 214);
        for _ in 0..5 { m.tick(); }
        let period = m.channel(0).unwrap().period;
        assert!(period < 428 && period > 214);
    }

    #[test]
    fn tone_porta_does_not_overshoot() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        m.apply_event(0, &EventPayload::PortaTarget { note: 60, instrument: 1 });
        effect(&mut m, Effect::TonePorta(255));
        m.tick();
        assert_eq!(m.channel(0).unwrap().period, 214);
    }

    #[test]
    fn tone_porta_does_not_trigger_note() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        // Render a few frames to advance position
        let mut buf = AudioBuffer::new(2, 1);
        for _ in 0..882 {
            buf.silence();
            m.render(&mut buf);
        }
        let pos_before = m.channel(0).unwrap().position;
        assert!(pos_before > 0);
        m.apply_event(0, &EventPayload::PortaTarget { note: 60, instrument: 1 });
        assert!(m.channel(0).unwrap().position >= pos_before);
    }

    #[test]
    fn tone_porta_vol_slide_does_both() {
        let mut m = make_machine(vec![127; 100000], 32);
        note_on(&mut m, 48, 1);
        m.apply_event(0, &EventPayload::PortaTarget { note: 60, instrument: 1 });
        effect(&mut m, Effect::TonePorta(8));
        effect(&mut m, Effect::TonePortaVolSlide(4));
        m.tick();
        let ch = m.channel(0).unwrap();
        assert!(ch.period < 428);
        assert!(ch.volume > 32);
    }

    #[test]
    fn porta_up_updates_increment() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        let inc_before = m.channel(0).unwrap().increment;
        effect(&mut m, Effect::PortaUp(4));
        m.tick();
        assert!(m.channel(0).unwrap().increment > inc_before);
    }

    #[test]
    fn vibrato_modulates_period_offset() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        assert_eq!(m.channel(0).unwrap().period_offset, 0);
        effect(&mut m, Effect::Vibrato { speed: 8, depth: 8 });
        m.tick();
        m.tick();
        assert_ne!(m.channel(0).unwrap().period_offset, 0);
    }

    #[test]
    fn vibrato_does_not_change_base_period() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        let base_period = m.channel(0).unwrap().period;
        effect(&mut m, Effect::Vibrato { speed: 8, depth: 8 });
        for _ in 0..10 { m.tick(); }
        assert_eq!(m.channel(0).unwrap().period, base_period);
    }

    #[test]
    fn vibrato_changes_increment() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        let inc_base = m.channel(0).unwrap().increment;
        effect(&mut m, Effect::Vibrato { speed: 8, depth: 8 });
        m.tick();
        m.tick();
        assert_ne!(m.channel(0).unwrap().increment, inc_base);
    }

    #[test]
    fn vibrato_remembers_previous_params() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::Vibrato { speed: 8, depth: 4 });
        m.tick();
        assert_eq!(m.channel(0).unwrap().vibrato_speed, 8);
        assert_eq!(m.channel(0).unwrap().vibrato_depth, 4);
        effect(&mut m, Effect::Vibrato { speed: 0, depth: 6 });
        m.tick();
        assert_eq!(m.channel(0).unwrap().vibrato_speed, 8);
        assert_eq!(m.channel(0).unwrap().vibrato_depth, 6);
    }

    #[test]
    fn arpeggio_cycles_period_offset() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::Arpeggio { x: 4, y: 7 });
        let mut offsets = Vec::new();
        for _ in 0..6 {
            m.tick();
            offsets.push(m.channel(0).unwrap().period_offset);
        }
        assert_eq!(offsets[0], offsets[3]);
        assert_eq!(offsets[1], offsets[4]);
        assert_eq!(offsets[2], offsets[5]);
    }

    #[test]
    fn arpeggio_does_not_change_base_period() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        let base_period = m.channel(0).unwrap().period;
        effect(&mut m, Effect::Arpeggio { x: 3, y: 7 });
        for _ in 0..6 { m.tick(); }
        assert_eq!(m.channel(0).unwrap().period, base_period);
    }

    #[test]
    fn arpeggio_offset_matches_note_shift() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::Arpeggio { x: 12, y: 7 });
        m.tick();
        let expected_x = note_to_period(48 + 12) as i16 - 428;
        assert_eq!(m.channel(0).unwrap().period_offset, expected_x);
        m.tick();
        let expected_y = note_to_period(48 + 7) as i16 - 428;
        assert_eq!(m.channel(0).unwrap().period_offset, expected_y);
    }

    #[test]
    fn tremolo_modulates_volume_offset() {
        let mut m = make_machine(vec![127; 100000], 32);
        note_on(&mut m, 48, 1);
        assert_eq!(m.channel(0).unwrap().volume_offset, 0);
        effect(&mut m, Effect::Tremolo { speed: 8, depth: 8 });
        m.tick();
        m.tick();
        assert_ne!(m.channel(0).unwrap().volume_offset, 0);
    }

    #[test]
    fn tremolo_does_not_change_base_volume() {
        let mut m = make_machine(vec![127; 100000], 32);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::Tremolo { speed: 8, depth: 8 });
        for _ in 0..10 { m.tick(); }
        assert_eq!(m.channel(0).unwrap().volume, 32);
    }

    #[test]
    fn vibrato_vol_slide_does_both() {
        let mut m = make_machine(vec![127; 100000], 32);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::Vibrato { speed: 8, depth: 4 });
        m.tick();
        effect(&mut m, Effect::VibratoVolSlide(4));
        m.tick();
        let ch = m.channel(0).unwrap();
        assert_ne!(ch.period_offset, 0);
        assert!(ch.volume > 32);
    }

    #[test]
    fn set_vibrato_waveform_is_row_effect() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::SetVibratoWaveform(1));
        assert_eq!(m.channel(0).unwrap().vibrato_waveform, 1);
    }

    #[test]
    fn vibrato_mod_resets_on_new_note_by_default() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::Vibrato { speed: 8, depth: 4 });
        for _ in 0..5 { m.tick(); }
        assert!(m.channel(0).unwrap().period_mod.is_some());
        note_on(&mut m, 60, 1);
        assert!(m.channel(0).unwrap().period_mod.is_none());
    }

    #[test]
    fn vibrato_mod_persists_with_no_retrig_flag() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::SetVibratoWaveform(4));
        effect(&mut m, Effect::Vibrato { speed: 8, depth: 4 });
        for _ in 0..5 { m.tick(); }
        assert!(m.channel(0).unwrap().period_mod.is_some());
        note_on(&mut m, 60, 1);
        assert!(m.channel(0).unwrap().period_mod.is_some());
    }

    #[test]
    fn note_cut_zero_cuts_immediately() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::NoteCut(0));
        assert_eq!(m.channel(0).unwrap().volume, 0);
    }

    #[test]
    fn note_cut_after_n_ticks() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::NoteCut(3));
        m.tick();
        assert_eq!(m.channel(0).unwrap().volume, 64);
        m.tick();
        assert_eq!(m.channel(0).unwrap().volume, 64);
        m.tick();
        assert_eq!(m.channel(0).unwrap().volume, 0);
    }

    #[test]
    fn note_cut_stays_cut() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::NoteCut(1));
        m.tick();
        assert_eq!(m.channel(0).unwrap().volume, 0);
        m.tick();
        assert_eq!(m.channel(0).unwrap().volume, 0);
    }

    #[test]
    fn retrigger_resets_position_periodically() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::RetriggerNote(2));
        // Render some frames to advance position
        let mut buf = AudioBuffer::new(2, 1);
        for _ in 0..882 {
            buf.silence();
            m.render(&mut buf);
        }
        m.tick();
        let pos1 = m.channel(0).unwrap().position;
        assert!(pos1 > 0);
        // Render more and tick again
        for _ in 0..882 {
            buf.silence();
            m.render(&mut buf);
        }
        m.tick();
        let pos2 = m.channel(0).unwrap().position;
        assert!(pos2 < pos1);
    }

    #[test]
    fn retrigger_zero_does_nothing() {
        let mut m = make_machine(vec![127; 100000], 64);
        note_on(&mut m, 48, 1);
        effect(&mut m, Effect::RetriggerNote(0));
        let mut buf = AudioBuffer::new(2, 1);
        for _ in 0..882 {
            buf.silence();
            m.render(&mut buf);
        }
        m.tick();
        assert!(m.channel(0).unwrap().position > 0);
    }

    #[test]
    fn set_speed_updates_internal_speed() {
        let mut m = make_machine(vec![127; 1000], 64);
        m.set_speed(3);
        assert_eq!(m.speed, 3);
    }
}
