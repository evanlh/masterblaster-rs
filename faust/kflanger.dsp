// Argüelles KFlanger — integer-delay flanger with output feedback.
//
// No source was available (clean-room port). Everything below was recovered
// from the buzz-machines-oracles measurements (development set only), by
// regenerating the probe inputs and matching the oracle sample by sample:
//
// * The machine works in Buzz units (±32768). Per sample:
//     v   = x + c * y[n-1]          (feedback from the previous *output*)
//     buf = trunc(v)                (stored truncated toward zero: mode 1 at
//                                    zero delay outputs exactly frac(x))
//     wet = buf[n - d]              (integer delay, no interpolation)
//     y   = x + wet   (mode 0)  /  x - wet   (mode 1)
//   with c = (Feedback - 32768) / 32767 (impulse decay ratios: 49151 → 0.49998,
//   0 → -1.0000305, 65534 → 0.99997).
// * The delay d is a triangle LFO updated once per Work call (256-sample
//   blocks, restarted at each tick): d changes only at block starts, and a
//   130-sample block at each tick boundary counts as a full step. Here a block
//   is a Faust compute() call (masterblaster renders <= 256 samples per call
//   without crossing ticks, like Buzz), detected through ma.BS.
// * LFO state: pos starts at 0 going up, whatever Min is; each block
//   pos += Speed/256 (delay = floor(pos)); going up, pos > Max clamps to Max
//   and turns down in the same block (so the peak is not held); going down,
//   pos < Min clamps to Min and turns up in the next block (the trough is held
//   one extra block). This reproduces every development sweep exactly.
// * Silent chunks: the original returns false from Work on all-zero input
//   and its LFO does not advance on those chunks (impulse probe: the LFO
//   steps once fewer when the first block is silent). The port carries a
//   per-block "heard input" flag and only commits LFO steps of blocks that had
//   input.
// * Min > Max: once the LFO reaches Max it clamps to Min (> Max) and the
//   original's wet path freezes to a constant (stale memory: 0.034, 0.041,
//   -9.9 … depending on the configuration). That garbage DC can't be
//   reproduced; the port mutes the wet path while pos > Max instead, which
//   gives the same transfer function (dry only).
// * Known differences: the original's output stops with its input (the
//   Faust path rings on, so tails differ: large at Feedback near 0 or 65534); with |c| ≈ 1 the
//   original's (double precision) feedback drifts from this float32 version
//   after many seconds of resonance.
import("stdfaust.lib");

minFlanger  = hslider("[0]Min Flanger", 0, 0, 254, 1) : rint;
maxFlanger  = hslider("[1]Max Flanger", 127, 0, 254, 1) : rint;
flangerMode = hslider("[2]Flanger Mode", 0, 0, 1, 1) : rint;
sweepSpeed  = hslider("[3]Sweep Speed", 1, 0, 254, 1) : rint;
feedback    = hslider("[4]Feedback", 32767, 0, 65534, 1) : rint;

buzzScale = 32768.0;
maxDelay = 256;

feedbackCoef = (feedback - 32768.0) / 32767.0;
lfoStep = sweepSpeed / 256.0;
wetSign = 1.0 - 2.0 * flangerMode;

// Sample index inside the current compute() call (0 on its first sample).
blockIndex = (\(prev).(ba.if(prev + 1 >= (ma.BS : mem), 0, prev + 1))) ~ _;
blockStart = blockIndex == 0;

// One LFO update: (pos, down) -> (pos', down').
lfoUp(pos, down) = ba.if(hit, maxFlanger, moved), down | hit
with {
    moved = ba.if(down, pos, pos + lfoStep);
    hit = (1 - down) & (moved > maxFlanger);
};
lfoDown(pos, down) = ba.if(low, minFlanger, moved), down & (1 - low)
with {
    moved = ba.if(down, pos - lfoStep, pos);
    low = down & (moved < minFlanger);
};
lfoUpdate = lfoUp : lfoDown;

// LFO state (pos, down, committed pos, committed down, block heard input).
// The original skips Work on silent chunks (returns false), so its LFO only
// advances on chunks with input. Whether a block had input is known once it
// ends, so each block starts from the state committed after the last block
// that had input, and advances it once.
lfoState(heard) = lfoNext ~ si.bus(5)
with {
    lfoNext(pos, down, cpos, cdown, nz) = ba.if(blockStart, newPos, pos),
        ba.if(blockStart, newDown, down),
        ba.if(blockStart, basePos, cpos),
        ba.if(blockStart, baseDown, cdown),
        ba.if(blockStart, heard, nz | heard)
    with {
        basePos = ba.if(nz, pos, cpos);
        baseDown = ba.if(nz, down, cdown);
        newPos = lfoUpdate(basePos, baseDown) : (_, !);
        newDown = lfoUpdate(basePos, baseDown) : (!, _);
    };
};
lfoPos(x) = lfoState(x != 0) : (_, !, !, !, !);

truncToward0 = int : float;

flanger(x) = (\(prevOut).(x + wetSign * wet(prevOut))) ~ _
with {
    pos = lfoPos(x);
    delaySamples = int(pos);
    wetMuted = pos > maxFlanger;
    wet(prevOut) = (x + feedbackCoef * prevOut) : truncToward0
        : de.delay(maxDelay, delaySamples) : *(1 - wetMuted);
};

kflanger = *(buzzScale) : flanger : /(buzzScale);

process = _, ! : kflanger <: _, _;
