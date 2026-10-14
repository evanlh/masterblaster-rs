// FSM WahMan Pro — LFO-modulated multimode resonant filter
// 5 filter types, 9 LFO shapes, mono effect applied per stereo channel
import("stdfaust.lib");

// === Parameters (matching Buzz ranges) ===
cutoff = hslider("[0]Cutoff", 120, 0, 240, 1);
reso = hslider("[1]Resonance", 120, 0, 240, 1);
ftype = int(hslider("[2]Filter", 1, 0, 4, 1));
lfo_rate = hslider("[3]LFO rate", 30, 0, 240, 1);
lfo_depth = hslider("[4]LFO mod", 60, 0, 240, 1);
lfo_shape = int(hslider("[5]LFO Shape", 1, 0, 8, 1));
inertia_p = hslider("[6]Inertia", 10, 0, 60, 1);
phase_p = hslider("[7]Set phase", 0, 0, 127, 1);

// === LFO ===
lfo_freq = 0.05 * pow(800.0, lfo_rate / 255.0);
phs = ma.frac((os.phasor(1.0, lfo_freq) + phase_p / 128.0)) * 2.0 * ma.PI;

// Pre-compute shapes that are reused (avoids Faust negation routing issue)
saw = (phs / ma.PI - 1.5) * 2.0;
tri = (select2(phs < ma.PI, 2.0 - phs / ma.PI, phs / ma.PI) - 0.5) * 2;

lfo_val = ba.selectn(9, lfo_shape,
    sin(phs),                                  // 0: sine
    saw,                                       // 1: saw up
    0 - saw,                                   // 2: saw down
    select2(phs < ma.PI, -1.0, 1.0),          // 3: square
    tri,                                       // 4: triangle
    sin(phs + ma.PI/4.0 * sin(phs)),          // 5: weird 1
    sin(phs + ma.PI/6.0 * sin(2.0*phs)),      // 6: weird 2
    sin(2.0*phs + ma.PI * cos(3.0*phs)),      // 7: weird 3
    0.5*sin(2.0*phs) + 0.5*cos(3.0*phs)       // 8: weird 4
);

// === Cutoff with LFO modulation + inertia smoothing ===
mod_cutoff = cutoff + lfo_depth * lfo_val / 2.0;
tau = max(0.001, inertia_p / 60.0 * 0.3);
smooth_cutoff = mod_cutoff : si.smooth(ba.tau2pole(tau));

// === Frequency and Q mapping ===
freq = max(33.0, min(20000.0, 264.0 * pow(32.0, smooth_cutoff / 240.0)));
freq_capped = min(8000.0, freq);

q_lp = 1.01 + 5.0 * reso / 240.0;
q_peak = 1.01 + 30.0 * reso / 240.0;
gain_peak2 = 1.0 + reso / 6.0;
g4 = 6.0 + reso / 30.0;
q4 = 1.0 + reso / 12.0;

// === Filter types (each is 1-in-1-out) ===
// fi.peak_eq_cq(Lfx_dB, fx_Hz, Q)
type0 = fi.resonlp(freq, q_lp, 1.0);
type1 = fi.peak_eq_cq(10.0, freq_capped, q_peak);
type2 = fi.peak_eq_cq(32.0, freq_capped, gain_peak2);
type3 = fi.resonlp(freq, sqrt(q_lp), 1.0)
      : fi.resonlp(freq * 0.7, sqrt(q_lp), 1.0);
type4 = fi.peak_eq_cq(g4, freq_capped, q4)
      : fi.peak_eq_cq(g4, freq_capped / (1.0 + reso / 240.0), q4);

// Split input to all filters, select one output
wahpro = _ <: type0, type1, type2, type3, type4
            : ba.selectn(5, ftype);

process = wahpro, wahpro;
