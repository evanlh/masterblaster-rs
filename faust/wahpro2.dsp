// FSM WahMan Pro 2 — LFO-modulated multimode resonant filter
// 8 filter types (incl. vocal), 17 LFO shapes, 2x output gain
import("stdfaust.lib");

// === Parameters (matching Buzz ranges) ===
cutoff = hslider("[0]Cutoff", 120, 0, 240, 1);
reso = hslider("[1]Resonance", 120, 0, 240, 1);
ftype = int(hslider("[2]Filter", 6, 0, 7, 1));
lfo_rate = hslider("[3]LFO rate", 30, 0, 254, 1);
lfo_depth = hslider("[4]LFO mod", 60, 0, 240, 1);
lfo_shape = int(hslider("[5]LFO Shape", 0, 0, 16, 1));
inertia_p = hslider("[6]Inertia", 10, 0, 60, 1);
phase_p = hslider("[7]Set phase", 0, 0, 127, 1);

// === LFO ===
lfo_freq = 0.05 * pow(800.0, min(lfo_rate, 239.0) / 255.0);
phs = ma.frac((os.phasor(1.0, lfo_freq) + phase_p / 128.0)) * 2.0 * ma.PI;

// Pre-compute reused sub-expressions (avoids Faust negation routing issue)
saw = (phs / ma.PI - 1.5) * 2.0;
tri = (select2(phs < ma.PI, 2.0 - phs / ma.PI, phs / ma.PI) - 0.5) * 2;
step4 = floor(((phs / ma.PI - 1.0) - 0.5) * 8.0) * 0.25;
phs_frac = ma.decimal(phs / (ma.PI / 4.0));
phs_frac_rev = ma.decimal((2.0 * ma.PI - phs) / (ma.PI / 4.0));

lfo_val = ba.selectn(17, lfo_shape,
    sin(phs),                                  // 0: sine
    saw,                                       // 1: saw up
    0 - saw,                                   // 2: saw down
    select2(phs < ma.PI, -1.0, 1.0),          // 3: square
    tri,                                       // 4: triangle
    sin(phs + ma.PI/4.0 * sin(phs)),          // 5: weird 1
    sin(phs + ma.PI/6.0 * sin(2.0*phs)),      // 6: weird 2
    sin(2.0*phs + ma.PI * cos(3.0*phs)),      // 7: weird 3
    0.5*sin(2.0*phs) + 0.5*cos(3.0*phs),      // 8: weird 4
    step4,                                     // 9: steps up
    0 - step4,                                 // 10: steps down
    step4 + 0.5 * phs_frac,                   // 11: upsaws up
    (0 - step4) + 0.5 * phs_frac,             // 12: upsaws down
    step4 + 0.5 * phs_frac_rev,               // 13: dnsaws up
    (0 - step4) + 0.5 * phs_frac_rev,         // 14: dnsaws down
    // S&H (pseudo-random stepped)
    0.5*sin(19123*floor(phs*4/ma.PI)+40*sin(12*floor(phs*4/ma.PI))),
    0.5*sin(1239543*floor(phs*2/ma.PI)+40*sin(15*floor(phs*8/ma.PI)))
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

// Vocal formant interpolation: 0-120 -> a..b, 120-240 -> b..c
threesel(sel, a, b, c) = select2(sel < 120.0,
    b + (c - b) * (sel - 120.0) / 120.0,
    a + (b - a) * sel / 120.0);

v1_f1 = threesel(smooth_cutoff, 270, 400, 800);
v1_f2 = threesel(smooth_cutoff, 2140, 800, 1150);
v2_f1 = threesel(smooth_cutoff, 270, 400, 650);
v2_f2 = threesel(smooth_cutoff, 2140, 1700, 1080);
v_q = 6.0 + reso / 24.0;
v_bw = 2.0 + reso / 48.0;
v2_q = 6.0 + reso / 16.0;
v2_bw = 2.0 + reso / 56.0;

// === Filter types (each 1-in-1-out, with gain normalization) ===
// fi.peak_eq_cq(Lfx_dB, fx_Hz, Q)
type0 = fi.resonlp(freq, q_lp, 1.0 / sqrt(q_lp));
type1 = fi.peak_eq_cq(10.0, freq_capped, q_peak) : *(1.0 / sqrt(q_peak));
type2 = fi.peak_eq_cq(32.0, freq_capped, gain_peak2)
      : *(0.2 / (1.0 + (240.0 - reso) / 180.0));
type3 = fi.resonlp(freq, sqrt(q_lp), 1.0)
      : fi.resonlp(freq * 0.7, sqrt(q_lp), 0.35);
type4 = fi.peak_eq_cq(g4, freq_capped, q4)
      : *(0.4 / (1.0 + (240.0 - reso) / 120.0))
      : fi.peak_eq_cq(g4, freq_capped / (1.0 + reso / 240.0), q4)
      : *(0.4);
type5 = fi.peak_eq_cq(9.0, freq_capped, 8.0) : *(0.5)
      : fi.peak_eq_cq(9.0, freq_capped / (3.5 - 2.0 * reso / 240.0), 8.0)
      : *(0.4);
type6 = _ <: (fi.peak_eq_cq(v_bw, v1_f1, v_q) : *(0.3)),
             (fi.peak_eq_cq(v_bw, v1_f2, v_q) : *(0.3)) :> _;
type7 = _ <: (fi.peak_eq_cq(v2_bw, v2_f1, v2_q) : *(0.3)),
             (fi.peak_eq_cq(v2_bw, v2_f2, v2_q) : *(0.3)) :> _;

// Split input to all filters, select one, apply 2x output gain
wahpro2 = _ <: type0, type1, type2, type3, type4, type5, type6, type7
             : ba.selectn(8, ftype) : *(2.0);

process = wahpro2, wahpro2;
