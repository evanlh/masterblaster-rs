// WhiteNoise's Harmonic Delay -- clean-room port (no machine source read).
//
// Derived only from info.json and the buzz-machines-oracles measurements
// (impulse / burst-then-silence / noise-pink development renders):
//
// * Signal path (bit-exact against the burst probe, max error ~1e-7):
//     buf[n] = x[n] + fb * buf[n-d]
//     y[n]   = dry * x[n] + wet * buf[n-d]
//   with exact single-sample taps (no interpolation).
// * d = floor(44100 / f), f = 440 * 2^((n - 57) / 12), n = 12*octave + nibble - 1
//   from the Buzz note byte (octave = high nibble, nibble = low nibble; C-0 =
//   16.35 Hz -> 2696 samples, B-9 -> 2). Invalid nibbles (13..15, e.g. 0x4F)
//   continue the same linear note numbering, which the oracle confirms.
// * fb = (raw - 64) / 64 (raw 0 = -100 %, 64 = 0 %, 128 = +100 %), wet = raw / 128,
//   dry = 1 or 0 ("Dry thru" switch).
// * Tail cut (the machine's Work returns false): Buzz calls Work per 256-sample
//   block on a grid restarting every tick (5250 samples at 126 BPM / 4 TPB;
//   masterblaster gives Faust no tempo, so the tick length is fixed here).
//   Counting from the first input-silent block, output stops at the first
//   block start where the silent count >= L, with
//     L = d * (1 + ln(32768) / -ln|fb|)   for 0 < |fb| < 1,
//     L = d                               for fb = 0 or |fb| = 1,
//   i.e. one delay period plus the periods needed for a full-scale echo to
//   decay below one 16-bit LSB. Fitted from cut positions at fb = 1/16,
//   0.297, 0.5, 0.72, 0.906, 0.953 and +-1 across delays 2..2696.
//   Causal approximation: a block in which input reappears re-opens the gate
//   from that sample (Buzz would process the whole block).
// * Max Delay attribute (1000 ms) never limits: the longest note delay is 2696.
// * Schema: Buzz's Note default is 0, its "no value" (outside 1..156); the
//   slider starts at 1 (C-0), which is what the oracle's default config uses.
// * The machine is mono; both channels share the gate and have their own comb.
import("stdfaust.lib");

dryThru  = hslider("[0]Dry thru", 1, 0, 1, 1) : rint;
note     = hslider("[1]Note", 1, 1, 156, 1) : rint;
feedback = hslider("[2]Feedback", 96, 0, 128, 1) : rint;
wetOut   = hslider("[3]Wet out", 48, 0, 128, 1) : rint;

// --- parameter -> coefficient ---------------------------------------------
noteNumber(v) = 12 * int(v / 16) + (int(v) % 16) - 1;
noteFreq(v)   = 440.0 * pow(2.0, (noteNumber(v) - 57) / 12.0);
delayLen      = int(floor(44100.0 / noteFreq(note)));

fb  = (feedback - 64) / 64.0;
wet = wetOut / 128.0;
dry = dryThru;

decayPeriods(g) = select2((g > 0) & (g < 1), 0.0, log(32768.0) / (0.0 - log(max(g, 1e-30))));
tailLen = delayLen * (1.0 + decayPeriods(abs(fb)));

// --- comb -----------------------------------------------------------------
maxDelay = 4096;
echo(x) = x : ((+ : de.delay(maxDelay, delayLen - 1)) ~ *(fb)) : mem;
voice(g, x) = g * (dry * x + wet * echo(x));

// --- tail gate (Work-returns-false emulation) -----------------------------
tickLen  = 5250;
blockLen = 256;
t = ba.time;
blockStart = ((t % tickLen) % blockLen) == 0;

seenIn(nz) = (\(prev).(nz | (prev & (1 - blockStart)))) ~ _;
silentFrom(prevSeen) = (\(prev).(select2(blockStart & prevSeen, prev, t))) ~ _;
blockGate(from) = (\(prev).(select2(blockStart, prev, (t - from) < tailLen))) ~ _;

gate(l, r) = blockGate(silentFrom(s')) | s
with { s = seenIn((abs(l) > 0) | (abs(r) > 0)); };

process(l, r) = voice(g, l), voice(g, r) with { g = gate(l, r); };
