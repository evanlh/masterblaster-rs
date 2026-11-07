// 11-LoFi v1.0 (Holger Zwar): clean-room Faust port, derived only from oracle measurements.
//
// Signal path, in 16-bit sample units (x = input * 32768):
//   a = trunc(x / step) * step      step = 128 * (Resolution + 1)    (round toward zero)
//   b = sample-and-hold of x:  a new x is captured every Hold+1 samples; the very first
//       capture happens Hold+1 samples after start, so b is 0 before that
//       (for Hold = 0 that gives b = x, except b = 0 on the first sample).
//   out = f_Mode(a, b), with ai = int(a), bi = int(b):
//       0 ADD a + b          1 SUB b - a         2 AND ai & bi      3 OR ai | bi
//       4 XOR ai ^ bi        5 MUL (bi*ai)/32768 (truncated)
//       6 DIV trunc(bi/ai)   (ai == 0 gives bi)
//       7 LO  the operand (a or b) with the smaller |int| (ties give a)
//       8 HI  the operand with the larger |int|            (ties give a)
//   Result is not clipped and not rounded for ADD/SUB/LO/HI (float), integer for the rest.
//
// Silent input: the machine skips (outputs 0 for, and does not advance the hold counter or
// the held value over) every 256-sample block, on a grid restarting each 5250-sample tick,
// whose input is entirely zero. Emulated causally: output is 0 until the current block has
// seen a nonzero sample, and the state is rolled back at the next block start when the block
// stayed silent. Samples before the first nonzero sample of a non-silent block are output as 0
// here (the original processes them), a small loss at block onsets.
//
// Known small differences: MUL differs by +-1 LSB on some samples at very low levels
// (cause unknown).
//
// Parameters are the raw Buzz values: Hold 0..127 (samples), Resolution 0..127, Mode 0..8.
import("stdfaust.lib");

hold = int(hslider("[0]Hold", 0, 0, 127, 1) + 0.5);
resolution = int(hslider("[1]Resolution", 0, 0, 127, 1) + 0.5);
mode = int(hslider("[2]Mode", 0, 0, 8, 1) + 0.5);

tick = 5250;
blk = 256;
unit = 32768.0;

// ---- pure sample functions ----
quantize(step, x) = float(int(x / step)) * step;
mag(v) = abs(int(v));
quot(a, b) = select2(int(a) == 0, float(int(float(int(b)) / float(int(a)))), float(int(b)));
pick(m, a, b) = ba.selectn(9, m,
    a + b,
    b - a,
    float(int(a) & int(b)),
    float(int(a) | int(b)),
    float(int(a) xor int(b)),
    float(int(float(int(b) * int(a)) / unit)),
    quot(a, b),
    select2(mag(a) <= mag(b), b, a),
    select2(mag(a) >= mag(b), b, a));

// ---- hold state: q = processed-sample count, h = held value ----
capture(p, q, h, x) = select2((q % p == 0) & (q > 0), h, x);

// block start on the tick grid (blocks of 256 restarting every tick)
blockStart = ((ba.time % tick) % blk) == 0;

// state: q, h, q0, h0, nz  (q0/h0: committed state at the start of the current block;
// nz: the current block has seen a nonzero sample)
lofiStep(q_, h_, q0_, h0_, nz_, x) = qn, hn, q0n, h0n, nzn, y with {
    roll = blockStart & (nz_ == 0);
    qb = select2(roll, q_, q0_);
    hb = select2(roll, h_, h0_);
    q0n = select2(blockStart, q0_, qb);
    h0n = select2(blockStart, h0_, hb);
    nzn = select2(blockStart, nz_, 0) | (x != 0);
    xs = x * unit;
    hn = capture(hold + 1, qb, hb, xs);
    qn = qb + 1;
    y = select2(nzn, 0.0, pick(mode, quantize(128.0 * (resolution + 1), xs), hn) / unit);
};
lofi1 = lofiStep ~ (_, _, _, _, _) : (!, !, !, !, !, _);

process = par(i, 2, lofi1);
