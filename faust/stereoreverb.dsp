// Jeskola Stereo Reverb — clean-room port (mono in, stereo out).
//
// Derived only from the machine's parameter metadata (info/describe.json)
// and the buzz-machines-oracles measurements (impulse, burst, pink noise);
// no machine source was read.
//
// Measured signal path (impulse probe, sparse deconvolution; the wet IR is
// reproduced to about -130 dB):
//
//   x = mono input (mean of the two Faust inputs)
//   wet_c = RevOut/4096 * AP(1097,.7)(AP(351,.7)(comb_c(x)))
//         + EROut/4096  * sum of three early taps per channel
//   out_c = DryOut/4096 * x + LP(wet_c)
//
//   comb_L: delay 2137, feedback +0.87324 * 10^(-K/t)
//   comb_R: delay 1774, feedback       -10^(-K/t)    (K = 1.2114536)
//     (combs output their delay line, so the first echo is at D samples;
//      feedback magnitudes measured at t = 1,10,17,33,48,64 fit
//      log10|g| = -K/t (+ log10 0.87324 on L) exactly)
//   AP(D,g): y = -g x + x[n-D] + g y[n-D]
//   ER taps L: 877 x0.868, 1716 x0.54, 3083 x0.628
//           R: 1561 x0.696, 1826 x0.612, 3510 x0.204
//   LP: RBJ Butterworth lowpass (Q = 1/sqrt 2) at High Cutoff Hz, applied
//       to the wet sum only (dry is unfiltered).
//
// Pre-Delay has no measurable effect in any probe (dead, as triage says).
//
// Tail cut (work_returns_false): once the input has been silent for
// t * 8820 samples (counted in host chunks of 256 samples on a grid that
// restarts every tick), the reverb core stops feeding the output for one
// chunk while the output lowpass rings; from the next chunk on the output
// is exactly zero (Work returns false; the burst probe at High Cutoff 100
// shows both stages). The tick length is not visible to a Faust
// DSP, so the oracle host's 5250 samples (126 BPM, 4 TPB) is assumed.
import("stdfaust.lib");

dryOut  = hslider("[0]Dry Out", 4096, 0, 4096, 1);
revOut  = hslider("[1]Rev Out", 768, 0, 4096, 1);
erOut   = hslider("[2]ER Out", 0, 0, 4096, 1);
revTime = hslider("[3]Rev Time", 10, 1, 64, 1) : rint;
preDly  = hslider("[4]Pre-Delay", 0, 0, 100, 1);  // dead in the original
hiCut   = hslider("[5]High Cutoff", 5000, 100, 16384, 1);

level(raw) = raw / 4096.0;

// ---- reverb core ---------------------------------------------------------
K = 1.2114536;
LEFT_SCALE = 0.87324;
fbMag = pow(10.0, -K / revTime);

comb(d, g) = (+ ~ (@(d - 1) : *(g))) : @(d);
allpass(d, g) = _ <: (@(d) - *(g)) : (+ ~ (@(d - 1) : *(g)));
diffuse = allpass(351, 0.7) : allpass(1097, 0.7);

coreL = comb(2137, LEFT_SCALE * fbMag) : diffuse;
coreR = comb(1774, -fbMag) : diffuse;

// ---- early reflections ---------------------------------------------------
erL = _ <: @(877) * 0.868 + @(1716) * 0.54 + @(3083) * 0.628;
erR = _ <: @(1561) * 0.696 + @(1826) * 0.612 + @(3510) * 0.204;

// ---- RBJ Butterworth lowpass ----------------------------------------------
lowpass(fc) = fi.tf2(b0, b1, b0, a1, a2)
with {
  w = 2.0 * ma.PI * fc / ma.SR;
  alpha = sin(w) / (2.0 * sqrt(0.5));
  a0 = 1.0 + alpha;
  b0 = (1.0 - cos(w)) / 2.0 / a0;
  b1 = (1.0 - cos(w)) / a0;
  a1 = -2.0 * cos(w) / a0;
  a2 = (1.0 - alpha) / a0;
};

// ---- tail cut --------------------------------------------------------------
TICK = 5250;
CHUNK = 256;
cutLimit = revTime * 8820;

// Number of consecutive chunks started after >= cutLimit silent samples,
// forced to 0 as soon as input arrives in the current chunk.
cutChunks(x) = chunks * (1 - pend)
with {
  n = ba.time;
  bnd = ((n % TICK) % CHUNK) == 0;
  nz = abs(x) > 0;
  pend = (\(p).(nz | (p & (1 - bnd)))) ~ _;
  silStart = (\(s).(select2(bnd & pend', s, n))) ~ _;
  expired = (n - silStart) >= cutLimit;
  chunks = (\(c).(select2(bnd, c, select2(expired, 0, c + 1)))) ~ _;
};

// First cut chunk: the core is muted but the lowpass still rings.
// Later cut chunks: Work returns false, so the whole output is silent.
muteCore(x) = cutChunks(x) >= 1;
muteAll(x) = cutChunks(x) >= 2;

// ---- machine ---------------------------------------------------------------
wet(core, er, x) = (x : core) * level(revOut) + (x : er) * level(erOut);

channel(core, er, x) = (level(dryOut) * x
  + (wet(core, er, x) * (1 - muteCore(x)) : lowpass(hiCut))) * (1 - muteAll(x));

process = + : *(0.5) <: channel(coreL, erL), channel(coreR, erR)
  : attach(_, preDly), _;
