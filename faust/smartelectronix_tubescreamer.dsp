// smartelectronix tubescreamer (Mikko Apo) — clean-room Faust port.
//
// Derived only from info.json / describe.json and the buzz-machines-oracles
// measurements (no machine source was read). Attributes are fixed at their
// defaults (Algo = "Monday", no limiters, min drive 10/1000, tone 100..10000).
//
// Signal path (mono, applied to each channel):
//   u = pregain * x                       pregain dB = -110 + PreGain/2   (describe)
//   z = u + PZ * z'                        fixed one-pole, PZ = 0.81385 (fitted to the
//                                          zero crossings of the square term on ESS)
//   s = D * sign(z) * knee(K |z| / D)      D = 0.375 * drive, drive = 0.01 + 0.99 Drive/254
//                                          knee(w) = 1 - (1 - min(w, 2.5)/2.5)^2.5,
//                                          K = 4200 (small-signal slope on low-pregain noise)
//   v = u + s
//   y = postgain * LP1(v)                 postgain dB = -30 + 55 PostGain/240
//   LP1: y = (1-p) v + p y',  p = b - sqrt(b^2-1), b = 2 - cos(2 pi f / fs),
//        f = 100 + 9900 (Tone/254)^3   (pole measured on square-wave edges per Tone;
//        the cube fits 9 distinct Tone values; describe.json shows Hz linearly)
// Known differences: the machine stops output on silent blocks (Work returns false);
// masterblaster keeps running, so tails differ.
import("stdfaust.lib");

pregainP = rint(hslider("[0]PreGain", 220, 0, 240, 1));
driveP   = rint(hslider("[1]Drive", 0, 0, 254, 1));
toneP    = rint(hslider("[2]Tone", 254, 0, 254, 1));
postP    = rint(hslider("[3]PostGain", 131, 0, 240, 1));

db2lin(d) = pow(10.0, d / 20.0);
pregain   = db2lin(-110.0 + 0.5 * pregainP);
postgain  = db2lin(-30.0 + 55.0 * postP / 240.0);
drive     = 0.01 + 0.99 * driveP / 254.0;
squareAmp = 0.375 * drive;

PZ = 0.81385;
K  = 4200.0;
KN = 2.5;

toneHz   = 100.0 + 9900.0 * pow(toneP / 254.0, 3.0);
cosPole(f) = b - sqrt(b * b - 1.0)
with { b = 2.0 - cos(2.0 * ma.PI * f / ma.SR); };
lp1(p) = *(1.0 - p) : + ~ *(p);

knee(w) = 1.0 - pow(max(0.0, 1.0 - min(w, KN) / KN), KN);
sgn(v) = (v > 0) - (v < 0);
square(z) = squareAmp * sgn(z) * knee(K * abs(z) / squareAmp);
zfilt = + ~ *(PZ);

screamer(x) = u + square(zfilt(u)) : lp1(cosPole(toneHz)) : *(postgain)
with { u = pregain * x; };

process = screamer, screamer;
