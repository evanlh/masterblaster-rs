// FSM ScrapMan -- granular "scrap" cloud (clean-room, statistical model)
// Derived only from info/describe/triage and oracle measurements (no machine source read):
//  - Dry/Wet: gain 2^((raw-240)/60), raw 0 = off.
//  - Grains start on a 64-sample grid; the first at ~6272 + f(Density) samples, then one every
//    ~(grain length + idle wait(Density))/Richness samples (random multiples of 64), assigned
//    round-robin to a 24-voice pool. Each grain has gain 1/sqrt(Richness) (measured peaks).
//  - Each grain reads the input at a random delay uniform in [~90, Dmax(Scattering)] samples
//    (Dmax ~ 374 + 104*3.12^(scat/16)), drifting at a random slope up to +-2.8e-4*Fatness
//    samples/sample; sine attack / flat sustain / cosine release, lengths fitted to measured
//    per-grain gain curves (2^(raw/60) laws).
//  - Fullness: probability a grain plays an octave down. Feedback: wet sum fed back (scaled by
//    0.2*sqrt(raw)/10, tanh-limited) so the wet level rise matches the oracle.
//  - The oracle's grain sequence is deterministic but could not be recovered (its delay draws
//    change non-monotonically with Scattering): hashes replace it, so statistics match but
//    not the phase-exact interference. This caps steady-sine (envelope/mod-rate metrics).
//  - Dummy slider kept alive (dead parameter).
import("stdfaust.lib");

dry_p  = hslider("[00]Dry out", 240, 0, 240, 1);
fb_p   = hslider("[01]Feedback", 0, 0, 99, 1);
full_p = hslider("[02]Fullness", 0, 0, 100, 1);
rich_p = hslider("[03]Richness", 12, 4, 24, 1);
dens_p = hslider("[04]Density", 48, 0, 64, 1);
scat_p = hslider("[05]Scattering", 48, 0, 64, 1);
fat_p  = hslider("[06]Fatness", 16, 0, 64, 1);
att_p  = hslider("[07]Attack", 120, 0, 240, 1);
sus_p  = hslider("[08]Sustain", 32, 0, 240, 1);
rel_p  = hslider("[09]Release", 64, 0, 240, 1);
wet_p  = hslider("[10]Wet out", 240, 0, 240, 1);
dummy  = hslider("[11]Dummy", 240, 0, 240, 1);
keep   = 1e-30 * dummy * (ba.time < 0);

gain(r) = select2(r > 0, 0.0, pow(2.0, (r - 240.0) / 60.0));
fbk     = 0.2 * sqrt(fb_p) / 10.0;
nvoice  = 24;
vgain   = 1.0 / sqrt(rich_p);

// envelope segment lengths (samples), measured from per-grain gain curves
attS = 1.07 * (480.0 + 224.0 * pow(2.0, att_p / 60.0));
susS = 1042.0 + 290.0 * pow(2.0, sus_p / 60.0);
relS = 1.1 * (544.0 + 224.0 * pow(2.0, rel_p / 60.0));
blk  = 64.0;
grainS = blk * ceil((attS + susS + relS) / blk);
// grain scheduler: one grain starts every ~cycle/Richness samples (random multiple of 64),
// cycle = grain length + idle wait(Density) (measured: ~4760 samples at Density 48, ~15000 at 0)
waitM  = 560.0 + 19500.0 * exp(-dens_p / 7.7);
gapM   = (grainS + waitM) / rich_p;
firstS = 6272.0 + 64.0 * floor(8000.0 * exp(-dens_p / 9.0) / 64.0);
dmin = 90.0;
dmax = 374.0 + 104.0 * pow(3.12, scat_p / 16.0);
smax = 2.8e-4 * fat_p;
maxdel = 65536;

hash(a) = ma.frac(sin(a * 12.9898 + 0.618) * 43758.5453);

env(t) = select2(t < attS, select2(t < attS + susS,
           cos(0.5 * ma.PI * min(1.0, (t - attS - susS) / relS)), 1.0),
           sin(0.5 * ma.PI * t / attS));

gapOf(k) = blk * max(1.0, floor((2.0 * gapM * hash(k * 1.37 + 0.2) - 0.5 * blk) / blk) + 1.0);

// scheduler: (spawn pulse, index of the grain being spawned)
scheduler = core ~ (_, _) : (!, !, _, _)
with {
  core(pnxt, pk) = nxt, k, spawn, pk
  with {
    tgt   = select2(pk < 0.5, pnxt, firstS);
    spawn = float(ba.time) >= tgt;
    k     = pk + spawn;
    nxt   = select2(spawn, pnxt, float(ba.time) + gapOf(pk));
  };
};

// one grain voice: round-robin over the pool; (age, grain index + 1) latched at its start
voice(v, y, spawn, kidx) = g * de.fdelay(maxdel, min(maxdel - 2, d), y)
with {
  st = spawn * (fmod(kidx, nvoice) == v);
  tk = core ~ (_, _)
  with {
    core(pt, pkv) = select2(st, pt + 1.0, 0.0), select2(st, pkv, kidx + 1.0);
  };
  t   = tk : (_, !);
  k   = (tk : (!, _)) - 1.0;
  d0  = dmin + hash(k * 3.1 + 0.11) * (dmax - dmin);
  sl  = (2.0 * hash(k * 3.1 + 0.37) - 1.0) * smax;
  oc  = 0.5 * (hash(k * 3.1 + 0.73) * 100.0 < full_p);
  act = (k >= 0.0) * (t < grainS);
  d   = d0 + (sl + oc) * t;
  g   = act * vgain * env(t);
};

bank(y) = par(v, nvoice, voice(v, y, sp, kk)) :> _
with { sp = scheduler : (_, !); kk = scheduler : (!, _); };
wetsig(x) = (+(x) : bank) ~ (*(fbk) : ma.tanh);
mono(x) = gain(dry_p) * x + gain(wet_p) * wetsig(x) + keep;
process = _, _ : (+ : *(0.5)) : mono <: _, _;
