//! Voice: audio generation unit for sample playback.

use mb_ir::{AudioBuffer, Sample, SampleKey};

/// Voice lifecycle state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VoiceState {
    /// Actively owned by a channel.
    #[default]
    Active,
    /// Note-off received; envelope releasing.
    Released,
    /// Fading out (NNA Fade).
    Fading,
    /// Background voice (NNA Continue/Off).
    Background,
}

/// A single voice producing audio from a sample.
#[derive(Clone, Debug)]
pub struct Voice {
    /// Which sample this voice plays.
    pub sample_key: SampleKey,
    /// Current position in sample (16.16 fixed-point).
    pub position: u32,
    /// Playback increment (16.16 fixed-point).
    pub increment: u32,
    /// Is the voice currently producing audio?
    pub playing: bool,
    /// Current volume (0-64).
    pub volume: u8,
    /// Current panning (-64 to +64).
    pub panning: i8,
    /// Volume offset from tremolo.
    pub volume_offset: i8,
    /// Loop direction for ping-pong (true = forward).
    pub loop_forward: bool,
    /// Voice lifecycle state.
    pub state: VoiceState,
    /// Owning channel index (for NNA tracking).
    pub channel: u8,
    /// Fade volume (0-1024, for NNA fade).
    pub fade_volume: u16,
}

impl Voice {
    /// Create a new voice for the given sample key.
    pub fn new(sample_key: SampleKey, channel: u8) -> Self {
        Self {
            sample_key,
            position: 0,
            increment: 0,
            playing: true,
            volume: 64,
            panning: 0,
            volume_offset: 0,
            loop_forward: true,
            state: VoiceState::Active,
            channel,
            fade_volume: 1024,
        }
    }

    /// Render one frame into the output buffer, reading from the given sample.
    /// Adds (sums) into the buffer for multi-voice mixing.
    pub fn render_with_source(&mut self, sample: &Sample, output: &mut AudioBuffer) {
        if !self.playing {
            return;
        }

        let sample_value = sample.data.get_mono_interpolated(self.position);
        let (left, right) = apply_volume_and_pan(sample_value, self.volume, self.volume_offset, self.panning);

        output.channel_mut(0)[0] += left;
        output.channel_mut(1)[0] += right;

        self.position += self.increment;
        self.advance_loop(sample);
    }

    /// Handle loop/end-of-sample logic after position advance.
    fn advance_loop(&mut self, sample: &Sample) {
        let pos_samples = self.position >> 16;
        if sample.has_loop() && pos_samples >= sample.loop_end {
            let loop_len = sample.loop_end - sample.loop_start;
            self.position -= loop_len << 16;
        } else if pos_samples >= sample.len() as u32 {
            self.playing = false;
        }
    }
}

/// Compute stereo f32 output from a sample value, volume, volume_offset, and panning.
/// Returns (left, right) in f32.
fn apply_volume_and_pan(sample_value: i16, volume: u8, volume_offset: i8, panning: i8) -> (f32, f32) {
    let vol = (volume as i32 + volume_offset as i32).clamp(0, 64);
    let pan_right = panning as i32 + 64; // 0..128
    let left_vol = ((128 - pan_right) * vol) >> 7;
    let right_vol = (pan_right * vol) >> 7;

    let left = (sample_value as i32 * left_vol) >> 6;
    let right = (sample_value as i32 * right_vol) >> 6;

    let left_f32 = left.clamp(-32768, 32767) as f32 / 32768.0;
    let right_f32 = right.clamp(-32768, 32767) as f32 / 32768.0;
    (left_f32, right_f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_ir::{AudioBuffer, Sample, SampleData, SampleKey};
    use slotmap::SlotMap;

    use crate::frequency::period_to_increment;

    fn test_sample(data: Vec<i8>, volume: u8) -> Sample {
        let mut s = Sample::new("test");
        s.data = SampleData::Mono8(data);
        s.default_volume = volume;
        s.c4_speed = 8363;
        s
    }

    fn looping_sample(data: Vec<i8>, loop_start: u32, loop_end: u32) -> Sample {
        let mut s = test_sample(data, 64);
        s.loop_start = loop_start;
        s.loop_end = loop_end;
        s.loop_type = mb_ir::LoopType::Forward;
        s
    }

    fn voice_with_increment(key: SampleKey, increment: u32, volume: u8, panning: i8) -> Voice {
        let mut v = Voice::new(key, 0);
        v.increment = increment;
        v.volume = volume;
        v.panning = panning;
        v
    }

    fn render_one(voice: &mut Voice, sample: &Sample) -> AudioBuffer {
        let mut buf = AudioBuffer::new(2, 1);
        voice.render_with_source(sample, &mut buf);
        buf
    }

    #[test]
    fn voice_render_produces_nonsilent_output() {
        let sample = test_sample(vec![127; 100], 64);
        let mut bank: SlotMap<SampleKey, Sample> = SlotMap::with_key();
        let key = bank.insert(sample.clone());
        let inc = period_to_increment(428, 8363, 44100);
        let mut voice = voice_with_increment(key, inc, 64, 0);
        let buf = render_one(&mut voice, &sample);
        assert!(buf.channel(0)[0] != 0.0 || buf.channel(1)[0] != 0.0);
    }

    #[test]
    fn voice_render_silent_when_not_playing() {
        let sample = test_sample(vec![127; 100], 64);
        let mut bank: SlotMap<SampleKey, Sample> = SlotMap::with_key();
        let key = bank.insert(sample.clone());
        let mut voice = voice_with_increment(key, 1 << 16, 64, 0);
        voice.playing = false;
        let buf = render_one(&mut voice, &sample);
        assert_eq!(buf.channel(0)[0], 0.0);
        assert_eq!(buf.channel(1)[0], 0.0);
    }

    #[test]
    fn voice_render_volume_zero_is_silent() {
        let sample = test_sample(vec![127; 100], 64);
        let mut bank: SlotMap<SampleKey, Sample> = SlotMap::with_key();
        let key = bank.insert(sample.clone());
        let mut voice = voice_with_increment(key, 1 << 16, 0, 0);
        let buf = render_one(&mut voice, &sample);
        assert_eq!(buf.channel(0)[0], 0.0);
        assert_eq!(buf.channel(1)[0], 0.0);
    }

    #[test]
    fn voice_render_panning_center_equal_lr() {
        let sample = test_sample(vec![127; 100], 64);
        let mut bank: SlotMap<SampleKey, Sample> = SlotMap::with_key();
        let key = bank.insert(sample.clone());
        let mut voice = voice_with_increment(key, 1 << 16, 64, 0);
        let buf = render_one(&mut voice, &sample);
        assert_eq!(buf.channel(0)[0], buf.channel(1)[0]);
    }

    #[test]
    fn voice_render_panning_hard_left() {
        let sample = test_sample(vec![127; 100], 64);
        let mut bank: SlotMap<SampleKey, Sample> = SlotMap::with_key();
        let key = bank.insert(sample.clone());
        let mut voice = voice_with_increment(key, 1 << 16, 64, -64);
        let buf = render_one(&mut voice, &sample);
        assert_eq!(buf.channel(1)[0], 0.0);
        assert!(buf.channel(0)[0] != 0.0);
    }

    #[test]
    fn voice_render_panning_hard_right() {
        let sample = test_sample(vec![127; 100], 64);
        let mut bank: SlotMap<SampleKey, Sample> = SlotMap::with_key();
        let key = bank.insert(sample.clone());
        let mut voice = voice_with_increment(key, 1 << 16, 64, 64);
        let buf = render_one(&mut voice, &sample);
        assert_eq!(buf.channel(0)[0], 0.0);
        assert!(buf.channel(1)[0] != 0.0);
    }

    #[test]
    fn voice_render_advances_position() {
        let sample = test_sample(vec![127; 100], 64);
        let mut bank: SlotMap<SampleKey, Sample> = SlotMap::with_key();
        let key = bank.insert(sample.clone());
        let inc = 1 << 16;
        let mut voice = voice_with_increment(key, inc, 64, 0);
        let pos_before = voice.position;
        render_one(&mut voice, &sample);
        assert_eq!(voice.position, pos_before + inc);
    }

    #[test]
    fn voice_render_stops_at_sample_end() {
        let sample = test_sample(vec![127; 2], 64);
        let mut bank: SlotMap<SampleKey, Sample> = SlotMap::with_key();
        let key = bank.insert(sample.clone());
        let mut voice = voice_with_increment(key, 1 << 16, 64, 0);
        for _ in 0..10 {
            render_one(&mut voice, &sample);
        }
        assert!(!voice.playing);
    }

    #[test]
    fn voice_render_loops_forward() {
        let sample = looping_sample(vec![100, 50, 25, 10], 1, 3);
        let mut bank: SlotMap<SampleKey, Sample> = SlotMap::with_key();
        let key = bank.insert(sample.clone());
        let mut voice = voice_with_increment(key, 1 << 16, 64, 0);
        // Render 10 frames — should loop and stay playing
        for _ in 0..10 {
            render_one(&mut voice, &sample);
        }
        assert!(voice.playing);
        let pos_samples = voice.position >> 16;
        assert!(pos_samples >= 1 && pos_samples < 3, "position should be within loop: {}", pos_samples);
    }

    #[test]
    fn voice_render_matches_channel_render() {
        use crate::channel::ChannelState;

        let sample = test_sample([127i8, 64, -32, 100, -100, 50].iter().copied().cycle().take(100).collect(), 48);
        let mut bank: SlotMap<SampleKey, Sample> = SlotMap::with_key();
        let key = bank.insert(sample.clone());
        let inc = period_to_increment(428, 8363, 44100);

        // Set up voice
        let mut voice = voice_with_increment(key, inc, 48, -32);

        // Set up channel with identical state
        let mut channel = ChannelState::new();
        channel.increment = inc;
        channel.volume = 48;
        channel.panning = -32;
        channel.playing = true;

        // Render 50 frames and compare
        for _ in 0..50 {
            let frame = channel.render(&sample);
            let mut buf = AudioBuffer::new(2, 1);
            voice.render_with_source(&sample, &mut buf);

            let expected_left = frame.left as f32 / 32768.0;
            let expected_right = frame.right as f32 / 32768.0;
            let actual_left = buf.channel(0)[0];
            let actual_right = buf.channel(1)[0];

            assert!(
                (actual_left - expected_left).abs() < 1e-6,
                "left mismatch at frame: {} vs {}", actual_left, expected_left
            );
            assert!(
                (actual_right - expected_right).abs() < 1e-6,
                "right mismatch at frame: {} vs {}", actual_right, expected_right
            );
        }
    }
}
