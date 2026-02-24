//! VoicePool: centralized voice allocation and lifecycle management.

use alloc::vec::Vec;
use mb_ir::{AudioBuffer, Sample, SampleKey};
use slotmap::SlotMap;

use crate::voice::{Voice, VoiceState};

/// Identifier for a voice slot in the pool.
pub type VoiceId = usize;

/// Maximum number of simultaneous voices.
pub const MAX_VOICES: usize = 128;

/// Centralized pool of voices with sample bank.
pub struct VoicePool {
    /// Voice slots (None = free).
    pub(crate) slots: Vec<Option<Voice>>,
    /// Sample bank (owns all sample data).
    pub sample_bank: SlotMap<SampleKey, Sample>,
}

impl VoicePool {
    /// Create a new empty voice pool.
    pub fn new() -> Self {
        Self {
            slots: (0..MAX_VOICES).map(|_| None).collect(),
            sample_bank: SlotMap::with_key(),
        }
    }

    /// Allocate a voice slot, returning its ID.
    /// Steals a slot if pool is full (priority: Fading > Released > Background > Active).
    pub fn allocate(&mut self, voice: Voice) -> VoiceId {
        // First try a free slot
        if let Some(id) = self.slots.iter().position(|s| s.is_none()) {
            self.slots[id] = Some(voice);
            return id;
        }
        // Steal: find best victim
        let id = self.find_steal_candidate();
        self.slots[id] = Some(voice);
        id
    }

    /// Find the best slot to steal (Fading > Released > Background > Active).
    fn find_steal_candidate(&self) -> VoiceId {
        let priority = |state: VoiceState| match state {
            VoiceState::Fading => 0,
            VoiceState::Released => 1,
            VoiceState::Background => 2,
            VoiceState::Active => 3,
        };
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_ref().map(|v| (i, priority(v.state))))
            .min_by_key(|(_, p)| *p)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// Get a reference to a voice.
    pub fn get(&self, id: VoiceId) -> Option<&Voice> {
        self.slots.get(id).and_then(|s| s.as_ref())
    }

    /// Get a mutable reference to a voice.
    pub fn get_mut(&mut self, id: VoiceId) -> Option<&mut Voice> {
        self.slots.get_mut(id).and_then(|s| s.as_mut())
    }

    /// Kill (remove) a voice immediately.
    pub fn kill(&mut self, id: VoiceId) {
        if let Some(slot) = self.slots.get_mut(id) {
            *slot = None;
        }
    }

    /// Set a voice to Released state.
    pub fn release(&mut self, id: VoiceId) {
        if let Some(voice) = self.get_mut(id) {
            voice.state = VoiceState::Released;
        }
    }

    /// Set a voice to Fading state.
    pub fn fade(&mut self, id: VoiceId) {
        if let Some(voice) = self.get_mut(id) {
            voice.state = VoiceState::Fading;
        }
    }

    /// Remove voices that have stopped playing.
    pub fn reap_finished(&mut self) {
        for slot in &mut self.slots {
            if let Some(voice) = slot {
                if !voice.playing {
                    *slot = None;
                }
            }
        }
    }

    /// Count of active (occupied) voice slots.
    pub fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Render a single voice by ID into the output buffer (split-borrow safe).
    pub fn render_voice(&mut self, voice_id: VoiceId, output: &mut AudioBuffer) {
        let bank = &self.sample_bank;
        if let Some(Some(voice)) = self.slots.get_mut(voice_id) {
            if let Some(sample) = bank.get(voice.sample_key) {
                voice.render_with_source(sample, output);
            } else {
                voice.playing = false;
            }
        }
    }

    /// Render all active voices into the output buffer.
    pub fn render_all(&mut self, output: &mut AudioBuffer) {
        let bank = &self.sample_bank;
        for slot in &mut self.slots {
            if let Some(voice) = slot {
                if let Some(sample) = bank.get(voice.sample_key) {
                    voice.render_with_source(sample, output);
                } else {
                    voice.playing = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mb_ir::{Sample, SampleData, SampleKey};
    use crate::frequency::period_to_increment;
    use crate::voice::Voice;

    fn test_sample(data: Vec<i8>, volume: u8) -> Sample {
        let mut s = Sample::new("test");
        s.data = SampleData::Mono8(data);
        s.default_volume = volume;
        s.c4_speed = 8363;
        s
    }

    fn make_voice(key: SampleKey) -> Voice {
        let mut v = Voice::new(key, 0);
        v.increment = period_to_increment(428, 8363, 44100);
        v
    }

    // === Allocation tests ===

    #[test]
    fn pool_new_is_empty() {
        let pool = VoicePool::new();
        assert_eq!(pool.active_count(), 0);
    }

    #[test]
    fn pool_allocate_returns_valid_id() {
        let mut pool = VoicePool::new();
        let key = pool.sample_bank.insert(test_sample(vec![127; 100], 64));
        let id = pool.allocate(make_voice(key));
        assert!(pool.get(id).is_some());
    }

    #[test]
    fn pool_allocate_multiple() {
        let mut pool = VoicePool::new();
        let key = pool.sample_bank.insert(test_sample(vec![127; 100], 64));
        let id1 = pool.allocate(make_voice(key));
        let id2 = pool.allocate(make_voice(key));
        assert_ne!(id1, id2);
        assert_eq!(pool.active_count(), 2);
    }

    #[test]
    fn pool_get_mut_modifies_voice() {
        let mut pool = VoicePool::new();
        let key = pool.sample_bank.insert(test_sample(vec![127; 100], 64));
        let id = pool.allocate(make_voice(key));
        pool.get_mut(id).unwrap().volume = 32;
        assert_eq!(pool.get(id).unwrap().volume, 32);
    }

    #[test]
    fn pool_kill_frees_slot() {
        let mut pool = VoicePool::new();
        let key = pool.sample_bank.insert(test_sample(vec![127; 100], 64));
        let id = pool.allocate(make_voice(key));
        pool.kill(id);
        assert!(pool.get(id).is_none());
        assert_eq!(pool.active_count(), 0);
    }

    #[test]
    fn pool_release_sets_state() {
        let mut pool = VoicePool::new();
        let key = pool.sample_bank.insert(test_sample(vec![127; 100], 64));
        let id = pool.allocate(make_voice(key));
        pool.release(id);
        assert_eq!(pool.get(id).unwrap().state, VoiceState::Released);
    }

    #[test]
    fn pool_fade_sets_state() {
        let mut pool = VoicePool::new();
        let key = pool.sample_bank.insert(test_sample(vec![127; 100], 64));
        let id = pool.allocate(make_voice(key));
        pool.fade(id);
        assert_eq!(pool.get(id).unwrap().state, VoiceState::Fading);
    }

    #[test]
    fn pool_reap_removes_stopped() {
        let mut pool = VoicePool::new();
        let key = pool.sample_bank.insert(test_sample(vec![127; 100], 64));
        let id = pool.allocate(make_voice(key));
        pool.get_mut(id).unwrap().playing = false;
        pool.reap_finished();
        assert_eq!(pool.active_count(), 0);
    }

    #[test]
    fn pool_steal_fading_first() {
        let mut pool = VoicePool::new();
        let key = pool.sample_bank.insert(test_sample(vec![127; 100], 64));
        // Fill all slots
        for _ in 0..MAX_VOICES {
            pool.allocate(make_voice(key));
        }
        // Set one to Fading
        pool.get_mut(50).unwrap().state = VoiceState::Fading;
        // Allocate should steal slot 50
        let id = pool.allocate(make_voice(key));
        assert_eq!(id, 50);
        assert_eq!(pool.get(id).unwrap().state, VoiceState::Active);
    }

    #[test]
    fn pool_steal_released_second() {
        let mut pool = VoicePool::new();
        let key = pool.sample_bank.insert(test_sample(vec![127; 100], 64));
        for _ in 0..MAX_VOICES {
            pool.allocate(make_voice(key));
        }
        pool.get_mut(30).unwrap().state = VoiceState::Released;
        let id = pool.allocate(make_voice(key));
        assert_eq!(id, 30);
    }

    #[test]
    fn pool_steal_background_third() {
        let mut pool = VoicePool::new();
        let key = pool.sample_bank.insert(test_sample(vec![127; 100], 64));
        for _ in 0..MAX_VOICES {
            pool.allocate(make_voice(key));
        }
        pool.get_mut(20).unwrap().state = VoiceState::Background;
        let id = pool.allocate(make_voice(key));
        assert_eq!(id, 20);
    }

    // === Render tests ===

    #[test]
    fn pool_render_silent_when_empty() {
        let mut pool = VoicePool::new();
        let mut buf = AudioBuffer::new(2, 1);
        pool.render_all(&mut buf);
        assert_eq!(buf.channel(0)[0], 0.0);
        assert_eq!(buf.channel(1)[0], 0.0);
    }

    #[test]
    fn pool_render_single_voice() {
        let mut pool = VoicePool::new();
        let sample = test_sample(vec![127; 100], 64);
        let key = pool.sample_bank.insert(sample.clone());
        let voice = make_voice(key);

        // Render directly for reference
        let mut ref_voice = voice.clone();
        let mut ref_buf = AudioBuffer::new(2, 1);
        ref_voice.render_with_source(&sample, &mut ref_buf);

        let _id = pool.allocate(voice);
        let mut buf = AudioBuffer::new(2, 1);
        pool.render_all(&mut buf);

        assert_eq!(buf.channel(0)[0], ref_buf.channel(0)[0]);
        assert_eq!(buf.channel(1)[0], ref_buf.channel(1)[0]);
    }

    #[test]
    fn pool_render_sums_voices() {
        let mut pool = VoicePool::new();
        let sample = test_sample(vec![127; 100], 64);
        let key = pool.sample_bank.insert(sample.clone());

        // Render one voice for reference
        let mut ref_voice = make_voice(key);
        let mut ref_buf = AudioBuffer::new(2, 1);
        ref_voice.render_with_source(&sample, &mut ref_buf);

        pool.allocate(make_voice(key));
        pool.allocate(make_voice(key));
        let mut buf = AudioBuffer::new(2, 1);
        pool.render_all(&mut buf);

        // Two identical voices → double the amplitude
        let tolerance = 1e-6;
        assert!((buf.channel(0)[0] - ref_buf.channel(0)[0] * 2.0).abs() < tolerance);
        assert!((buf.channel(1)[0] - ref_buf.channel(1)[0] * 2.0).abs() < tolerance);
    }

    #[test]
    fn pool_render_stops_voice_with_missing_sample() {
        let mut pool = VoicePool::new();
        let sample = test_sample(vec![127; 100], 64);
        let key = pool.sample_bank.insert(sample);
        let id = pool.allocate(make_voice(key));
        // Remove the sample
        pool.sample_bank.remove(key);
        let mut buf = AudioBuffer::new(2, 1);
        pool.render_all(&mut buf);
        assert!(!pool.get(id).unwrap().playing);
    }
}
