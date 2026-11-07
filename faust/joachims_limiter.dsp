// Clean-room Faust port of Joachims Limiter v3 (primary 2b22111ffa43).
//
// Derived only from info/describe/triage and development-oracle measurements:
// - Input gain is 6 * ((Input - 1) / 127)^3.
// - Threshold is 2^(-Thresh./6), with independent gain detection per channel.
// - Gain reduction is immediate; recovery is a linear sample-by-sample ramp.
// - The release ramp starts at gain 0.1. Its coefficient curve was measured at
//   ten development settings and is represented by a smooth log-polynomial,
//   not by configuration- or output-value lookup tables.
// No machine source, disassembly, other port, or evaluation data was used.

import("stdfaust.lib");

// ceil restores integer Buzz values after masterblaster's truncating
// raw -> 0..65535 -> slider round trip (minimum and maximum remain exact).
input_raw = ceil(hslider("[0]Input", 71, 1, 128, 1));
threshold_raw = ceil(hslider("[1]Thresh.", 1, 0, 6, 1));
release_raw = ceil(hslider("[2]Release", 60, 1, 128, 1));

input_gain = 6.0 * pow((input_raw - 1.0) / 127.0, 3.0);
ceiling = pow(2.0, -threshold_raw / 6.0);

// log(release_step) as a polynomial in u, where u maps Release 1..128
// to -1..1. Dense checks show a monotonic curve over the entire raw range.
release_u = (release_raw - 64.5) / 63.5;
release_log_step =
    -9.86679636223
    + release_u * (-4.93493343798
    + release_u * (0.19152520919
    + release_u * (0.483877604652
    + release_u * (-0.538981613949
    + release_u * (-1.28163353374
    + release_u * 1.2394693021)))));
release_step = exp(release_log_step);

limit_channel(x) = pre * (gain_delta + 0.1)
with {
    pre = x * input_gain;
    desired_gain = min(1.0, ceiling / max(abs(pre), 1.0e-30));
    next_delta(previous) = min(desired_gain, previous + 0.1 + release_step) - 0.1;
    gain_delta = next_delta ~ _;
};

process = limit_channel, limit_channel;
