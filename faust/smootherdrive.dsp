// Muon SmootherDrive — clean-room port (no machine source read).
//
// Memoryless, odd-symmetric shaper; mono machine applied per channel.
// Derived from the buzz-machines-oracles stepped-sine measurements
// (output / input over 1 kHz steps, -60..0 dBFS):
//   t = Threshold / 32768,  k = min(Intensity + 1.6, 100)
//   c = k·t / (1 + (k-1)·t)                 (output level at the threshold)
//   |x| <  t : y = x · c/t                  (exactly linear: residual ~1e-7)
//   |x| >= t : y = ±(c + (1-c)·sin(π/2 · (|x|-t)/(1-t)))   (quarter-sine knee to 1)
//   Intensity = 0 : exact bypass.
// Evidence: the small-signal gain G gives 1/G = t + (1-t)/k with k = I + 1.6
// for every measured I in 7..88 and k = 100 at I = 100 (hence the clamp);
// Threshold 0 gives y = sin(π/2·x) for all intensities.
// The machine returns false on silent blocks; this shaper outputs 0 there anyway.
import("stdfaust.lib");

threshold = hslider("[0]Threshold", 32767, 0, 32767, 1);
intensity = hslider("[1]Intensity", 0, 0, 100, 1);

t = threshold / 32768.0;
k = min(intensity + 1.6, 100.0);
c = k * t / (1.0 + (k - 1.0) * t);
lin_gain = c / max(t, 1e-9);

knee(a) = c + (1.0 - c) * sin(ma.PI * 0.5 * (min(a, 1.0) - t) / (1.0 - t));
magnitude(a) = select2(a < t, knee(a), a * lin_gain);
shape(x) = select2(x < 0, magnitude(x), 0.0 - magnitude(0.0 - x));
drive(x) = select2(intensity < 0.5, shape(x), x);

process = drive, drive;
