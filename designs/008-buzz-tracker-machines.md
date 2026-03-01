# Buzz Tracker Machine Support

Created: 20260216
Updated: 20260216


Feature analysis for Jeskola Tracker, Matilde Tracker, and Matilde Tracker 2.
Covers what our engine currently supports and what gaps remain.

Reference sources:
- [Buzztrax/buzzmachines](https://github.com/Buzztrax/buzzmachines) (Matilde source)
- [Matilde Tracker 2 docs](https://github.com/Buzztrax/buzzmachines/blob/master/Matilde/Tracker/Matilde_Tracker2.html)
- [Buzz QuickStart](http://jeskola.net/archive/buzz/1.1/Help/QuickStart.html)
- [Hackey-Trackey](https://github.com/JoepVanlier/Hackey-Trackey) (Jeskola Tracker reimplementation)
- [Buzz Wiki: Matilde Tracker](https://buzzwiki.robotplanet.dk/index.php/Matilde_Tracker)

---

## Machine Overview

### Jeskola Tracker

The built-in Buzz tracker. Closed-source (lost in 2000 crash). Minimal feature
set compared to Matilde. 5 bytes/track/tick: Note, Wave, Volume, Effect, EffectArg.
One global param (Mute/subdivide). No filter, no envelopes, no virtual channels.

### Matilde Tracker (v1.0-v1.1)

Open-source. "More like ProTracker than Jeskola Tracker." 5 bytes/track/tick
(same layout as Jeskola). Added probability (0x10), volume/pitch/panning
envelopes, sustain points.

### Matilde Tracker 2 (v1.2+)

7 bytes/track/tick: Note, Wave, Volume, Effect1, Arg1, **Effect2, Arg2**.
Second effect column is the major structural addition. Added: stereo waves,
MIDI input, virtual channels (64-note polyphony), per-channel two-pole filter,
shuffle, loop fit, spline interpolation. Source in `Matilde/Tracker/`.

---

## Track Column Layout

| Column | Jeskola | Matilde 1 | Matilde 2 | Our Cell | Status |
|--------|---------|-----------|-----------|----------|--------|
| Note | C-0..B-9, off | same | same | `note: Note` | OK |
| Wave | 1-200 | same | same | `instrument: u8` | OK |
| Volume | 0x00-0x80 normal, 0x81-0xFE amp | same | same | `volume: VolumeCommand` | OK |
| Effect 1 | yes | yes | yes | `effect: Effect` | OK |
| Arg 1 | yes | yes | yes | (in Effect enum) | OK |
| **Effect 2** | no | no | **yes** | **missing** | UNSUPPORTED |
| **Arg 2** | no | no | **yes** | **missing** | UNSUPPORTED |
| Subdivide | Jeskola only (separate column) | via 0x0F | via 0x0F | **missing** | UNSUPPORTED |

### Volume Column Differences

Buzz trackers use 0x00-0xFE range (0x80 = full, 0xFE = ~2x gain).
ProTracker uses 0-64. Our BMX parser scales: `(vol * 64) / 0xFE`.
Amplification above 0x80 (>100%) is clamped to 64 — we lose the overdrive range.

### Second Effect Column

Matilde Tracker 2's second effect column replaces ProTracker's combined effects
(0x05 TonePorta+VolSlide, 0x06 Vibrato+VolSlide). These combined effects are
explicitly removed from Matilde 2 since you can just use both slots.

Our `Cell` struct has one `effect: Effect` field. To support the second column:
- Option A: Add `effect2: Effect` to Cell (simple, wastes memory for non-MT2)
- Option B: Use a separate MT2-specific cell type
- Option C: Store as auxiliary data outside the pattern

Currently our BMX parser reads the second effect column but discards it.

---

## Effect Comparison

### Effects We Support (in engine with tests)

| Code | Effect | Jeskola | Matilde | Notes |
|------|--------|---------|---------|-------|
| 0x00 | Arpeggio | 0x0A* | 0x00 | *Jeskola uses different numbering |
| 0x01 | Porta Up | yes | yes | |
| 0x02 | Porta Down | yes | yes | |
| 0x03 | Tone Portamento | yes | yes | |
| 0x04 | Vibrato | yes** | yes | **Jeskola may lack vibrato |
| 0x07 | Tremolo | no | yes | |
| 0x08 | Set Panning | yes | yes | |
| 0x09 | Sample Offset | yes | yes | See "offset scaling" below |
| 0x0A | Volume Slide | no | yes | |
| 0x0B | Position Jump | no | no*** | ***Handled by Buzz sequencer |
| 0x0C | Set Volume | no | no*** | ***Replaced by volume column |
| 0x0D | Pattern Break | no | no*** | |
| 0x0F | Set Speed/Tempo | no | no*** | |
| 0xE1 | Fine Porta Up | no | yes | |
| 0xE2 | Fine Porta Down | no | yes | |
| 0xE4 | Set Vibrato Wave | no | yes | |
| 0xE7 | Set Tremolo Wave | no | yes | |
| 0xE9 | Retrigger Note | 0x0B* | yes | |
| 0xEA | Fine Vol Up | no | yes | |
| 0xEB | Fine Vol Down | no | yes | |
| 0xEC | Note Cut | 0x0C* | yes | |
| 0xED | Note Delay | no | yes | |

### Jeskola Tracker Effect Numbering

Jeskola Tracker uses **different effect numbers** from ProTracker:

| Jeskola Code | ProTracker Equiv | Effect |
|-------------|-----------------|--------|
| 0x01 | 0x01 | Porta Up |
| 0x02 | 0x02 | Porta Down |
| 0x03 | 0x03 | Tone Portamento |
| 0x04 | 0x04 | Vibrato |
| 0x08 | 0x08 | Set Panning |
| 0x09 | 0x09 | Sample Offset |
| 0x0A | 0x00 | **Arpeggio** (0x00 in PT) |
| 0x0B | 0xE9 | **Retrigger** (0xE9 in PT) |
| 0x0C | 0xEC | **Note Cut / Probability** (0xEC in PT) |
| 0x60 | — | **Reverse** (unique to Jeskola) |

This means our `parse_effect()` (which assumes ProTracker encoding) **misparses
Jeskola Tracker patterns**. We need a separate `parse_jeskola_effect()`.

### Sample Offset Scaling

| Tracker | 0x09 Behavior |
|---------|---------------|
| ProTracker | Offset = param * 256 bytes |
| Jeskola/Matilde | Offset = param / 256 * sample_length (fractional) |

0x80 means "start at 50%" in Buzz trackers. Our engine uses ProTracker's
256-byte-unit interpretation. Buzz tracker offsets need conversion at parse time:
`offset_bytes = (param * sample_length) / 256`.

---

## Unsupported Effects (Matilde Tracker 2)

### Panning Effects

| Code | Effect | Priority |
|------|--------|----------|
| 0x05 xy | Panning Slide (x=left, y=right) | Medium |
| 0x06 xy | Autopan LFO (x=speed, y=depth) | Low |
| 0xE6 0x | Set Autopan Waveform | Low |
| 0xEE xx | Fine Panning Slide Left | Medium |
| 0xEF xx | Fine Panning Slide Right | Medium |

### Filter Effects (per-channel two-pole filter)

| Code | Effect | Priority |
|------|--------|----------|
| 0x20 xx | Set Filter Cutoff | Medium |
| 0x21 xx | Slide Cutoff Up | Low |
| 0x22 xx | Slide Cutoff Down | Low |
| 0x23 xx | Set Cutoff LFO Type | Low |
| 0x24 xy | Cutoff LFO (speed/depth) | Low |
| 0x25 xx | Fine Slide Cutoff Up | Low |
| 0x26 xx | Fine Slide Cutoff Down | Low |
| 0x28 xx | Set Filter Resonance | Medium |
| 0x29 xx | Slide Resonance Up | Low |
| 0x2A xx | Slide Resonance Down | Low |
| 0x2B xx | Set Resonance LFO Type | Low |
| 0x2C xy | Resonance LFO (speed/depth) | Low |
| 0x2D xx | Fine Slide Resonance Up | Low |
| 0x2E xx | Fine Slide Resonance Down | Low |
| 0xE0 xx | Set Filter Type | Low |

Note: Our AmigaFilter is a graph-level node, not a per-channel effect. To support
these, we'd need either per-channel filter state in `ChannelState` or per-channel
Machine instances.

### Timing & Probability Effects

| Code | Effect | Priority |
|------|--------|----------|
| 0x0F xx | Subdivide (per-row effect rate) | Medium |
| 0x10 xx | Probability with note-off (xx/255 chance) | Medium |
| 0x30 xx | Probability without note-off | Medium |
| 0x13 xy | Auto Shuffle (x=step, y=amount) | Low |
| 0x15 xx | Random Delay (up to xx subdivisions) | Low |
| 0xDC xx | Note Release (envelope release at subdivision xx) | Medium |

### Pitch & Playback Effects

| Code | Effect | Priority |
|------|--------|----------|
| 0x11 xx | Loop Fit (adjust freq to complete loop in xx ticks) | Medium |
| 0x12 xx | Loop Fit with speed tracking | Low |
| 0x2F xx | Long Loop Fit (xx * 128) | Low |
| 0x14 xx | Randomize Volume (max variance xx) | Low |
| 0x16 xx | Randomize Pitch (max variance xx notches) | Low |
| 0x17 xx | Harmonic Play (multiply frequency by xx) | Low |
| 0x18 xy | Combined Note Delay+Cut (x=trigger, y=release) | Medium |
| 0x19 xy | Sustain Pedal (x=1 depress, x=2 release) | Low |
| 0xE5 xx | Set Finetune (0x00=-half, 0x80=center, 0xFF=+half) | Medium |
| 0xE8 01 | Reverse Sample Playback | Medium |
| 0x60 00 | Reverse (Jeskola only) | Medium |

---

## Unsupported Structural Features

### Envelope System

Matilde reads three envelope types from the Buzz wavetable:

| Envelope | Effect |
|----------|--------|
| Volume | Amplitude shape over time. Sustain points supported. |
| Pitch | Pitch modulation (depth from attribute, default 12 semitones) |
| Panning | Stereo movement over time |

Envelopes complete in 64 ticks by default (configurable via Volume Envelope
Span attribute). Our engine has no per-channel envelope processing.

### Virtual Channels (NNA-style)

When enabled: up to 64 simultaneous voices per machine. New notes on a track
don't cut previous notes — they continue until their envelope finishes. Looping
samples without volume envelopes are auto-cut. This is equivalent to Impulse
Tracker's New Note Action system.

Our engine: one voice per channel, new notes always cut.

### Global Parameters

| Parameter | Range | Description |
|-----------|-------|-------------|
| Ampl.Decay | 0x00-0xFE | Volume decay per tick (for percussion loops) |
| Offset | 0x00-0xFE | Global sample offset slider |
| Quantize | 1-64 | How offset maps to sample positions |
| Tuning | 0x00-0xFE | Global pitch adjustment (0x7F = reset) |

These appear as tweakable sliders in Buzz. Our BMX parser reads them from MACH
init state but doesn't apply them.

### Machine Attributes

| Attribute | Description |
|-----------|-------------|
| Volume Ramp | Anti-click micro volume ramp (ms) |
| Volume Envelope Span | Ticks for envelope completion (default 64) |
| MIDI Channel | MIDI input channel (0 = disabled) |
| MIDI Velocity Sensitivity | 0-256 |
| MIDI Wave | Waveform for MIDI notes |
| MIDI Uses Free Tracks | Only use unoccupied tracks for MIDI |
| Filter Mode | 0=none, 1=linear, 2=spline interpolation |
| Pitch Envelope Depth | Semitone range for pitch envelope |
| Enable Virtual Channels | 64-voice polyphony toggle |
| Long Loop Fit Factor | Multiplier for 0x2F command |
| Offset Volume Gain | Compensate volume when using offset slider |
| Tuning Range | Range for tuning parameter |

---

## Implementation Priorities

### Phase 1: Fix Jeskola effect numbering
- Add `parse_jeskola_effect()` for Jeskola Tracker's different encoding
- Map 0x0A→Arpeggio, 0x0B→Retrigger, 0x0C→NoteCut
- Fix sample offset to use fractional scaling

### Phase 2: Core Matilde compatibility
- Panning slide (0x05), fine panning slides (0xEE/0xEF)
- Finetune (0xE5)
- Note Release vs Note Cut distinction (0xDC)
- Reverse sample playback (0xE8 / 0x60)
- Fractional sample offset for all Buzz trackers

### Phase 3: Matilde 2 extensions
- Second effect column (Cell struct change)
- Subdivide (0x0F) — per-track effect rate
- Probability (0x10, 0x30)
- Combined note delay+cut (0x18)
- Loop fit (0x11, 0x12)

### Phase 4: Filter system
- Per-channel filter state in ChannelState
- Set cutoff/resonance (0x20, 0x28)
- Filter slides and LFOs
- Filter type selection

### Phase 5: Advanced features
- Volume/pitch/panning envelopes from wavetable
- Virtual channels (NNA polyphony)
- Auto shuffle / randomize effects
- Harmonic play
- Machine attributes and global parameters
