// Jeskola Reverb 2 (Oskari Tammelin), mono effect.
//
// No source exists. Everything below was measured from the buzz-machines-oracles
// renders of the original DLL (impulse, pink-noise and burst probes, 44.1 kHz):
//
// * The machine is exactly LTI while input keeps arriving: convolving the pink-noise
//   input with the measured impulse response reproduces the oracle to 136 dB SNR.
// * Wet path = High Cutoff lowpass applied to (Rev Out * tank + ER Out * early taps).
//   The lowpass is an RBJ/Butterworth biquad (Q = 1/sqrt 2, f0 = High Cutoff in Hz,
//   bilinear): a biquad fit to the transient recovers f0 to 0.01 Hz and Q = 0.70711
//   at every tested cutoff. Inverse-filtering it leaves the same tank response for
//   every cutoff (-130 dB), so it sits outside the tank.
// * Levels are linear, raw/4096 (describe.json: 2048 = -6.0 dB). Dry is unfiltered.
// * Early reflections: six taps (Moorer's pattern at 44.1 kHz), read off the
//   inverse-filtered response: 877 0.868, 1561 0.696, 1716 0.540, 1826 0.612,
//   3083 0.628, 3510 0.204 (samples, gain).
// * Tank: Gardner's "medium room" nested-allpass reverb. Every coefficient and delay
//   was read off sparse pulses of the inverse-filtered impulse response, and the
//   model reproduces the whole 3 s response to -117..-143 dB:
//     a = NAP(g 0.3; 207 + AP(366, 0.7) + AP(970, 0.5))   (35 ms double-nested)
//     c = a -> AP(1323, 0.5) -> 3175 samples               (5 ms + 30 ms AP + 67 ms)
//     b = NAP(g 0.3; 1288 + AP(432, 0.6)) of (x + 0.5 * c delayed 662)   (39 ms)
//     feedback into a: 0.7 * LP2500(b delayed 4763)   (108 ms; Butterworth 2.5 kHz)
//     tank = 0.5 * (a + c + b)
//   AP(D, g) = (g + z^-D) / (1 + g z^-D); NAP(g; G) = (g + G) / (1 + g G).
// * Rev Time does NOT change the tank (identical responses for 1..64). It only sets
//   how long the machine keeps running after its input goes silent: Work is called
//   in <= 256-sample chunks that restart at each tick, and at the start of a silent
//   chunk, once the silent chunks since the last non-silent one add up to
//   Rev Time * 0.2 s (8820 samples per step), the wet path is cleared. The output
//   lowpass rings out for the rest of that chunk, then the output is zero (Work
//   returns false). The cut positions for Rev Time 1 and 10 bracket 8820/step
//   within (8810, 8834]. Here the chunk grid assumes 126 BPM / 4 TPB (5250 samples
//   per tick); clearing is emulated by masking every tank delay read until the
//   delay has refilled since the gate last closed.
// * Pre-Delay has no effect at all (bit-identical output for 0..100); it is kept
//   only so parameter indices line up.
//
// Delays are given in samples at 44.1 kHz and scaled to the running rate.
import("stdfaust.lib");

dry_out = hslider("[0]Dry Out", 4096, 0, 4096, 1);
rev_out = hslider("[1]Rev Out", 768, 0, 4096, 1);
er_out = hslider("[2]ER Out", 0, 0, 4096, 1);
rev_time = hslider("[3]Rev Time", 10, 1, 64, 1);
predelay = hslider("[4]Pre-Delay", 0, 0, 100, 1); // dead in the original
high_cutoff = hslider("[5]High Cutoff", 5000, 100, 16384, 1);

// ---- sample-rate scaling ---------------------------------------------------
sr_scale = ma.SR / 44100.0;
samples(n) = int(rint(n * sr_scale));
max_samples(n) = n * 5; // room for rates up to 220 kHz

// ---- filters ---------------------------------------------------------------
// RBJ lowpass, Q = 1/sqrt(2).
butter_lp(f) = fi.tf2(b0, 2 * b0, b0, a1, a2)
with {
    w = 2 * ma.PI * min(f, 0.49 * ma.SR) / ma.SR;
    alpha = sin(w) / sqrt(2);
    norm = 1 + alpha;
    b0 = (1 - cos(w)) / 2 / norm;
    a1 = -2 * cos(w) / norm;
    a2 = (1 - alpha) / norm;
};

// ---- silence gate (per 256-sample chunk, chunks restart at each tick) --------
// Returns (gate_in, gate_out, since_open) for input x.
silence_gate(x) = gate_in, gate_out, since_open
with {
    n = ba.time;
    tick_len = samples(5250);
    chunk_start = ((n % tick_len) % 256) == 0;
    heard = x != 0;
    // any non-zero input so far in the current chunk
    chunk_heard = (*(1 - chunk_start) : max(heard)) ~ _;
    prev_heard = chunk_heard';
    // start of the chunk following the last non-silent chunk
    last_sound_end = ba.sAndH(chunk_start * prev_heard, n);
    limit = int(rint(rev_time)) * samples(8820);
    killed = ba.sAndH(chunk_start, (n - last_sound_end) >= limit);
    closed = killed * (1 - chunk_heard);
    prev_closed = ba.sAndH(chunk_start, closed');
    gate_in = 1 - closed;
    gate_out = 1 - closed * prev_closed;
    since_open = (+(1) : *(gate_in)) ~ _;
};

// ---- tank primitives ---------------------------------------------------------
// Delay of n samples (at 44.1 kHz) minus `k` (for use inside `~`), reading zero
// until the line has refilled since the gate last opened (emulates clearing).
cleared_delay(since_open, n, k) = de.delay(max_samples(n), samples(n) - k) * (since_open > samples(n));

// Schroeder allpass (g + z^-D) / (1 + g z^-D).
allpass(so, d, g) = (+ ~ (cleared_delay(so, d, 1) : *(0 - g))) <: *(g), cleared_delay(so, d, 0) :> _;

// Nested allpass (g + G) / (1 + g G); `inner(k)` is G with its leading delay shortened by k.
nested_allpass(g, inner) = _ <: *(g), (*(1 - g * g) : (+ ~ (inner(1) : *(0 - g))) : inner(0)) :> _;

tank(so, x) = (loop ~ _) : !, _
with {
    room_a(k) = cleared_delay(so, 207, k) : allpass(so, 366, 0.7) : allpass(so, 970, 0.5);
    room_b(k) = cleared_delay(so, 1288, k) : allpass(so, 432, 0.6);
    loop(fb) = feedback, 0.5 * (a + c + b)
    with {
        a = (x + fb) : nested_allpass(0.3, room_a);
        c = a : allpass(so, 1323, 0.5) : cleared_delay(so, 3175, 0);
        b = (x + 0.5 * (c : cleared_delay(so, 662, 0))) : nested_allpass(0.3, room_b);
        feedback = b : cleared_delay(so, 4763, 1) : butter_lp(2500) : *(0.7);
    };
};

early(x) = sum(i, 6, x : de.delay(max_samples(3510), samples(ba.take(i + 1, taps))) * ba.take(i + 1, gains))
with {
    taps = (877, 1561, 1716, 1826, 3083, 3510);
    gains = (0.868, 0.696, 0.540, 0.612, 0.628, 0.204);
};

channel(x) = attach(x * dry_out / 4096 + wet * gate_out, predelay)
with {
    gates = silence_gate(x);
    gate_in = gates : _, !, !;
    gate_out = gates : !, _, !;
    since_open = gates : !, !, _;
    wet = (tank(since_open, x) * rev_out / 4096 + early(x) * er_out / 4096) * gate_in
        : butter_lp(high_cutoff);
};

process = channel, channel;
