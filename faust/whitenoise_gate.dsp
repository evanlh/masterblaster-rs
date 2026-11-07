// WhiteNoise Gate (David Wallin) — clean-room port.
//
// Derived only from info.json / describe.json and the buzz-machines-oracles
// measurements (no machine source was read). Evidence:
//
// * describe.json: Ratio v -> "v/10 : 1" (250 = "Noise Gate"),
//   Threshold v -> -50 + v/4 dB, Open Rate v -> v/4 ms, Close Rate v -> v ms.
// * Closed gain (sample-level output/input on steady-sine and noise-pink):
//   10/Ratio, and 1.52587890625e-9 (= 1e-4/65536) for Ratio 250.
// * Gain moves exponentially (linear in log gain) between the closed gain
//   and 1: a full swing takes exactly Open Rate/4 ms opening and Close Rate
//   ms closing (fitted per-sample log slopes: ln(Ratio/10)/(ms*44.1)).
//   The gain starts closed.
// * Detector: transition times on noise-pink (Open = Close = minimum, so
//   the gate state is visible sample by sample) sit at a constant
//   1024-sample rectangular-window RMS of the input (sd 0.03 dB, mean
//   -20.0 / -19.0 dB for Threshold 120 / 124); a one-pole RMS/abs follower
//   doesn't fit. Steady-sine onsets agree within 3 samples.
// * The probes never go silent, so the work-returns-false cut is not
//   modelled; with silent input the 1024-sample RMS falls to zero and the
//   gate closes anyway.
//
// Parameters arrive rescaled through 0..65535; rint() recovers the raw
// value (the truncation error is < 0.04 for every range here).
import("stdfaust.lib");

ratio = hslider("[0]Ratio", 200, 10, 250, 1) : rint;
threshold = hslider("[1]Threshold", 120, 0, 248, 1) : rint;
openRate = hslider("[2]Open Rate", 1, 1, 2000, 1) : rint;
closeRate = hslider("[3]Close Rate", 500, 1, 2000, 1) : rint;

srMs = ma.SR / 1000.0;
noiseGateGain = 1.52587890625e-9;

closedGain = select2(ratio >= 250, 10.0 / ratio, noiseGateGain);
logClosed = log(closedGain);
openStep = -logClosed / (openRate * 0.25 * srMs);
closeStep = logClosed / (closeRate * srMs);

windowLen = 1024;
meanSquare(x) = x * x : ba.slidingReduce(+, windowLen, windowLen, 0) : /(windowLen);
thresholdPower = pow(10.0, (threshold * 0.25 - 50.0) / 10.0);
isOpen(x) = meanSquare(x) > thresholdPower;

firstSample = 1 - 1';
clampLog(v) = min(0.0, max(logClosed, v));
logGain(open) = (+(select2(open, closeStep, openStep)) : clampLog)
              ~ (_ <: select2(firstSample, _, logClosed));

gate(l, r) = l * g, r * g
with { g = exp(logGain(isOpen((l + r) * 0.5))); };

process = gate;
