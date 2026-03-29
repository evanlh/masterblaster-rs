// Jeskola Filter 2 — multimode resonant filter (LP/HP/BP/Notch)
import("stdfaust.lib");

ftype = hslider("Type", 0, 0, 3, 1);
cutoff = hslider("Cutoff", 64, 0, 127, 1);
resonance = hslider("Resonance", 0, 0, 127, 1);

// Map cutoff 0-127 to frequency 20-20000 Hz (exponential)
freq = 20.0 * pow(1000.0, cutoff / 127.0);
// Map resonance 0-127 to Q 0.5-20
q = 0.5 + (resonance / 127.0) * 19.5;

// State variable filter returns (lp, bp, hp)
svf = fi.svf.lp(freq, q), fi.svf.bp(freq, q), fi.svf.hp(freq, q);

// Notch = lp + hp
lp(x) = x : fi.svf.lp(freq, q);
hp(x) = x : fi.svf.hp(freq, q);
bp(x) = x : fi.svf.bp(freq, q);
notch(x) = lp(x) + hp(x);

// Select filter type: 0=LP, 1=HP, 2=BP, 3=Notch
filter(x) = ba.selectn(4, int(ftype), lp(x), hp(x), bp(x), notch(x));

process = filter, filter;
