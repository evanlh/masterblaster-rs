// Geonik's Compressor — clean-room port (no machine source read).
//
// Everything here was inferred from describe.json display text and from the
// buzz-machines-oracles measurements (stepped-sine, tone-bursts, noise-pink):
//
//   Parameter mapping (describe.json; linear factors confirmed by the oracle):
//     Threshold  thr  = 2^((v-128)/12)      ("-18.0 dB" at 92: 1/2 dB per step,
//                                             realised as 1/12 octave)
//     Ratio      R    = 1 + v/10            ("3.0 : 1" at 20)
//     Attack/Release  v ms                  ("4003 ms")
//     Gain       G    = 2^(v/12)            (6 -> sqrt 2, measured exactly)
//
//   Level detector: per-sample one-pole on x^2, time constant 698.5 samples
//   (fitted on burst onsets; residual < 0.1 %). Static curve, measured on
//   steady sine steps, compresses in the *linear* domain:
//     e = sqrt(ms);  e > thr: out = thr + (e - thr)/R,  target gain T = out/e
//
//   Gain smoothing runs on Buzz's block grid (256-sample blocks, restarting
//   every tick = 5250 samples at the oracle's 126 BPM / 4 TPB; masterblaster
//   gives Faust no tempo, so the tick length is a constant). Per block the
//   machine picks T from the detector value at the *end* of the block, then
//   ramps the gain linearly from its current value g0 with slope
//   (T - g0) / (t * SR / 5000)  (t = attack or release ms), clamped at T.
//   Attack is used whenever the level is above threshold (even at 1:1),
//   release otherwise. Gain starts at 0, so output fades in over the release
//   time. Blocks whose input is all zero are skipped: detector and gain keep
//   their values (the DLL's Work returns false there).
//
//   Faust can't look ahead, so within a block T comes from a prediction of the
//   end-of-block detector value. On steady input it is the block-start
//   detector advanced by one block of the previous block's mean square
//   (constant over the block, so the gain ramp stays as smooth as the
//   original's). When the block's energy so far departs from that, the
//   prediction blends towards the running detector extrapolated to the block
//   end with the mean of itself and the block's mean square so far (onsets,
//   decays, noise). The block's end state
//   uses the exact end-of-block detector, so it matches the original.
//
//   The "Level Sensing Buffer" and "Hard Limiter" attributes are not
//   parameters and stay at their defaults (20, off).
import("stdfaust.lib");

thr_raw = hslider("[0]Threshold", 92, 0, 128, 1) : rint;
ratio_raw = hslider("[1]Ratio", 20, 0, 240, 1) : rint;
attack_ms = hslider("[2]Attack", 10, 5, 8000, 1) : rint;
release_ms = hslider("[3]Release", 2000, 30, 8000, 1) : rint;
gain_raw = hslider("[4]Gain", 6, 0, 128, 1) : rint;

thr = pow(2.0, (thr_raw - 128.0) / 12.0);
ratio = 1.0 + ratio_raw / 10.0;
ramp_samples(ms) = max(1.0, ms * ma.SR / 5000.0);
gain = pow(2.0, gain_raw / 12.0);

TICK = 5250;
BLOCK = 256;
K = 1.0 / 698.5;

// Position within the tick, 0 at the first sample.
tick_pos = ((+(1) : %(TICK)) ~ _) + (TICK - 1) : %(TICK);
block_pos = tick_pos % BLOCK;
block_len = min(BLOCK, TICK - (tick_pos - block_pos));

target(e) = select2(e > thr, 1.0, (thr + (e - thr) / ratio) / max(e, 1e-30));
ramp_len(e) = select2(e > thr, ramp_samples(release_ms), ramp_samples(attack_ms));
toward(t, s, v) = select2(s > 0, max(v, t), min(v, t));

// One sample of the block-grid compressor. State: detector m, detector at
// block start mS, block sum of squares cs, block-silent flag sil, block start
// gain g0, block end gain gE, previous block's mean square pw. Outputs the
// new state and the applied gain.
step(x, m, mS, cs, sil, g0, gE, pw) = m1, mS1, cs1, sil1, g01, gE1, pw1, g
with {
  start = block_pos == 0;
  x2 = x * x;
  m0 = select2(start, m, select2(sil, m, mS));
  mS1 = select2(start, mS, m0);
  m1 = m0 * (1.0 - K) + K * x2;
  cs1 = select2(start, cs + x2, x2);
  sil1 = select2(start, sil, 1) & (x == 0);
  g01 = select2(start, g0, select2(sil, gE, g0));
  prev_len = select2(tick_pos == 0, BLOCK, TICK % BLOCK);
  pw1 = select2(start, pw, select2(sil, cs / prev_len, pw));
  steady = mS1 * pow(1.0 - K, block_len) + (1.0 - pow(1.0 - K, block_len)) * pw1;
  w = min(1.0, abs(cs1 - pw1 * (block_pos + 1)) / (0.08 * pw1 * block_len + 1e-20)) ^ 4;
  rest = block_len - 1 - block_pos;
  d = pow(1.0 - K, rest);
  running = m1 * d + (1.0 - d) * 0.5 * (m1 + cs1 / (block_pos + 1));
  e = sqrt((1.0 - w) * steady + w * running);
  t = target(e);
  s = (t - g01) / ramp_len(e);
  g = toward(t, s, g01 + s * block_pos);
  e_end = sqrt(m1);
  t_end = target(e_end);
  s_end = (t_end - g01) / ramp_len(e_end);
  gE1 = toward(t_end, s_end, g01 + s_end * block_len);
};

compressor_gain(x) = (step(x) ~ si.bus(7)) : (si.block(7), _);

// Buzz runs the machine in mono; masterblaster hands Faust a stereo buffer,
// so the gain is computed from the mid signal (= the mono input when L = R)
// and applied to both channels.
process(l, r) = l * g, r * g
with {
  g = compressor_gain((l + r) * 0.5) * gain;
};
