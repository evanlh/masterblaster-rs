// Geonik's Saturator -- memoryless tanh-shaped saturator with a drive control.
//
// Derived from oracle measurements only (no machine source was read):
//  * describe.json: Drive raw 48..128 prints "-16 dB".."64 dB" (raw - 64 dB).
//  * Memoryless (triage class nonlinear-memoryless, noise-pink transfer curves
//    are single-valued): out = f(gain * in).
//  * Small-signal slope / 10^(dB/20) rises ~0.00036 per dB, i.e. the gain is
//    2^(dB/6) (6.0 dB per octave, not 6.0206), with f'(0) = 0.908.
//  * f(u) = tanh(0.9*u) / tanh(2.7) with |u| clamped at 3 (output reaches
//    exactly +-1 at u = 3). A fit of min(1, A*tanh(k*u)) to the noise-pink
//    transfer gave A = 1.00904, k = 0.90005 (rms error 2.6e-5), and
//    1/tanh(2.7) = 1.00908.
//  * Silent input gives zero output (f(0) = 0), which matches the
//    work-returns-false cut without extra emulation.
// The machine is mono; both Faust channels run the same curve.
import("stdfaust.lib");

drive = hslider("Drive", 64, 48, 128, 1);

gain = pow(2.0, (drive - 64.0) / 6.0);
shape(u) = ma.tanh(0.9 * max(-3.0, min(3.0, u))) / ma.tanh(2.7);
saturate(x) = shape(gain * x);

process = saturate, saturate;
