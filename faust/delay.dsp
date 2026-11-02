// Jeskola Delay — mono feedback delay (clean-room port).
//
// Derived only from info.json/describe.json and the buzz-machines-oracles
// measurements (impulse probe, development configs); no machine source read.
//
// Signal path (impulse responses are exact single-sample taps, no filtering):
//   d   = delayline(in + fb * d, L)        integer delay, no interpolation
//   out = dry * in + wet * d
//   fb  = (Feedback - 64) / 64             0x00 = -100%, 0x40 = 0, 0x80 = +100%
//   wet = Wet out / 128
//   L   = Length in units, truncated to whole samples, clamped to the
//         "Max Delay (ms)" attribute default (1000 ms = 44100 samples):
//           tick   : Length * samples-per-tick (5250 at 126 BPM / 4 TPB)
//           ms     : floor(Length * SR / 1000)      (3 ms -> 132)
//           sample : Length
//           1/256  : floor(Length * tick / 256)     (3 -> 61, 1430 -> 29326)
//
// Silence gate (work_returns_false): when the input goes silent the
// machine keeps running for N * L samples, N = number of round trips for
// |fb|^N to fall below 1/65536 (N = 16 at fb 0.5, 5 at fb 5/64, 1 at
// fb 0 and |fb| = 1), then reports silence. Measured from where the
// impulse-probe tails stop: at |fb| = 1 the oracle gives exactly one echo.
// Modelled as a sample counter since the last non-zero input; Buzz checks
// it per Work() chunk (<= 256 samples), so the cut can land up to one
// chunk later than here.
//
// Known masterblaster limitations:
//   * "Dry thru" is a switch declared -1..-1 (min == max); masterblaster
//     always delivers it as 0, so it is pinned to its default (1 = dry on).
//     Faust rejects an init outside [min, max], so the slider is declared
//     -1/-1 with init -1 (schema check: default 1 vs -1) and ignored.
//     Configurations with Dry thru = 0 therefore keep the dry signal.
//   * masterblaster gives Faust no tempo; a tick is fixed at 126 BPM / 4 TPB.
import("stdfaust.lib");

dryThru = hslider("[0]Dry thru", -1, -1, -1, 1) : pin(1);
lengthRaw = hslider("[1]Length", 3, 1, 32768, 1) : rint;
unitRaw = hslider("[2]Length unit", 0, 0, 3, 1) : rint;
feedbackRaw = hslider("[3]Feedback", 96, 0, 128, 1) : rint;
wetRaw = hslider("[4]Wet out", 48, 0, 128, 1) : rint;

// Returns v but still reads x, so Faust keeps the slider in the UI (an
// unused slider would be dropped and shift every later param index).
pin(v, x) = ba.if(x == 12345.0, x, v);

maxDelay = 65536;
maxSamples = int(ma.SR);                       // Max Delay attribute: 1000 ms
tickSamples = ma.SR * 60.0 / (126.0 * 4.0);

unitSamples(len) = ba.selectn(4, unitRaw,
    len * tickSamples,
    len * ma.SR / 1000.0,
    len,
    len * tickSamples / 256.0);

delaySamples = int(unitSamples(lengthRaw)) : min(maxSamples) : max(1);

feedback = (feedbackRaw - 64.0) / 64.0;
wet = wetRaw / 128.0;

// Round trips until |fb|^N < 1/65536; 1 when |fb| is 0 or 1.
roundTrips(g) = ba.if((g > 0.0) & (g < 1.0), ceil(log(1.0 / 65536.0) / log(g)), 1.0);
tailSamples = roundTrips(abs(feedback)) * delaySamples;

// Samples since the last non-zero input sample.
silentRun(x) = (x == 0) : (+ : *(x == 0)) ~ _;
// Buzz decrements the countdown per Work() chunk (<= 256 samples) and only
// stops at a chunk boundary; one chunk of slack approximates that.
chunkSlack = 256;
gate(x) = silentRun(x) <= tailSamples + chunkSlack;

// w = x + fb * w[n-L];  output the line's tap w[n-L].
recirculate = + ~ (de.delay(maxDelay, delaySamples - 1) : *(feedback));
fbDelay = recirculate : de.delay(maxDelay, delaySamples);

delayChannel(x) = dryThru * x + wet * fbDelay(x) * gate(x);

// Buzz runs the machine mono; masterblaster's FaustMachine copies only the
// outputs a DSP has, so each stereo channel gets its own identical delay
// (the comparator feeds the mono probe to both and scores L).
process = delayChannel, delayChannel;
