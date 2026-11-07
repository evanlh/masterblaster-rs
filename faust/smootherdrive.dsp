// Dave's SmootherDrive: clean-room port (no machine source was read).
//
// Derived only from info.json and the buzz-machines-oracles measurements
// (stepped-sine transfer curves, checked on ess / noise-pink / two-tone):
//
//   * The machine is a memoryless, odd-symmetric shaper on the normalised
//     sample |x| (Buzz +-32768 scale / 32768). Threshold t = T / 32768.
//   * Intensity 0 is an exact bypass (output == input on every probe).
//   * Otherwise, with a = 1 / min(I + 0.6, 99)  (measured: 1/a = I + 0.6 for
//     every tested I <= 88, and 99 at I = 100):
//         g = (1 + a) / (t + a),   K = g * t
//         |x| <  t : y = g * |x|                          (linear boost)
//         |x| >= t : y = K + (1 - K) * sin(pi/2 * (|x| - t) / (1 - t))
//     where |x| is truncated to a whole Buzz sample (floor(|x|*32768)) in
//     the sine branch only: the T = 0 curve is a staircase with 1-LSB steps
//     (residual 1.57 LSB -> 0.001 LSB with truncation, all Thresholds), while
//     the linear branch is not quantised (y/x constant to 1e-7).
//     The small-signal gains fit this to ~1e-7 (e.g. T=4096, I=25 -> 19/3),
//     and the normalised above-threshold curve matches the quarter sine to
//     4 decimals for every Threshold. T = 0 degenerates to a pure
//     sin(pi/2 * x) shaper whatever the Intensity.
//   * Whole-model residual on all development jobs: max |err| ~ 0.001 LSB (3e-8).
//   * work_returns_false: silence in gives exactly 0 out from a memoryless
//     odd shaper, so no block-cut emulation is needed.
//
// Mono machine; applied identically to both channels (2-in/2-out).
import("stdfaust.lib");

// [N] prefixes: Buzz parameter order; ranges/defaults = Buzz raw values.
threshold_s = hslider("[0]Threshold", 32767, 0, 32767, 1);
intensity_s = hslider("[1]Intensity", 0, 0, 100, 1);

// masterblaster delivers min + (floor(raw*65535/range)/65535)*range.
// Recover the 0..65535 step exactly, then the raw integer by ceiling division.
scaled_step(s, range) = int(rint(s * 65535.0 / range));
ceil_div_65535(n) = int(rint(float(n - n % 65535) / 65535.0));
raw_word(s, range) = ceil_div_65535(scaled_step(s, range) * range + 65534);
raw_byte(s) = floor(s + 0.01);

T = raw_word(threshold_s, 32767);
I = raw_byte(intensity_s);

t = float(T) / 32768.0;
a = 1.0 / min(I + 0.6, 99.0);
g = (1.0 + a) / (t + a);
K = g * t;

below(ax) = g * ax;
sample_floor(ax) = floor(ax * 32768.0) / 32768.0;
knee_pos(ax) = min((sample_floor(ax) - t) / max(1.0 - t, 1e-9), 1.0);
above(ax) = K + (1.0 - K) * sin(ma.PI / 2.0 * max(knee_pos(ax), 0.0));
shape(ax) = select2(ax < t, above(ax), below(ax));
drive(x) = ma.signum(x) * shape(abs(x));

smootherdrive(x) = select2(I > 0, x, drive(x));

process = smootherdrive, smootherdrive;
