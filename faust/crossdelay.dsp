// Jeskola Cross Delay: mono in, stereo out ping-pong ("cross") delay.
//
// Clean-room port: derived only from info.json/describe.json and the
// buzz-machines-oracles measurements (impulse / burst / noise responses).
// No machine source was read.
//
// Signal path (impulse responses match to float precision):
//   a  = x + fb * rd          left delay line input
//   ld = a  @ DL              left tap  (exact single-sample tap, no interp.)
//   rd = (fb * ld) @ DR       right tap (fed from the left tap)
//   outL = dry * x + wet * ld,  outR = dry * x + wet * rd
// fb = (raw - 64) / 64, wet = raw / 128.
// Lengths: unit 0 = ticks (spt samples), 1 = ms (floor), 2 = samples,
// 3 = 1/256 tick (floor), each capped at 1 s (the "Max Delay" attribute,
// default 1000 ms).
//
// Tail cut (Buzz work_returns_false): the host works in 256-sample blocks on
// a grid that restarts every tick. Once the input has been silent for
// T = maxD * (1 + 15 / log2(1/|fb|)) samples (counted at block starts;
// decay term 0 when |fb| is 0 or 1), the machine outputs silence until input
// returns. Measured: |fb| = 1/2 -> 16 maxD, 1/16 -> 4.75 maxD, 0 or 1 -> maxD.
//
// Tempo: Faust gets no tempo from masterblaster, so a tick is fixed at
// 126 BPM / 4 TPB (5250 samples at 44.1 kHz), the oracle's master settings.
//
// Known limitation: "Dry thru" is a Buzz switch (min == max == -1), which
// masterblaster always delivers as 0; it is pinned to its default (1, on).
import("stdfaust.lib");

dryThru = attach(1, hslider("[0]Dry thru", 1, 0, 1, 1)); // pinned, see header
lLen    = rint(hslider("[1]L. Length", 3, 1, 32768, 1));
rLen    = rint(hslider("[2]R. Length", 3, 1, 32768, 1));
unit    = rint(hslider("[3]Length unit", 0, 0, 3, 1));
fbRaw   = rint(hslider("[4]Feedback", 96, 0, 128, 1));
wetRaw  = rint(hslider("[5]Wet out", 48, 0, 128, 1));

MAXBUF = 65536;
spt = int(ma.SR * 60 / (126 * 4));

toSamples(v) = ba.selectn(4, unit, v * spt, int(v * ma.SR / 1000), v, int(v * spt / 256))
             : min(int(ma.SR)) : int;
dL = toSamples(lLen);
dR = toSamples(rLen);
fb = (fbRaw - 64) / 64;
wet = wetRaw / 128;

// Feedback loop: the fed-back signal is f*ld delayed DR samples (DR-1 here,
// plus the one-sample delay of ~).
crossLoop(rd, x) = v, ld, rd
with {
  ld = (x + fb * rd) : de.delay(MAXBUF, dL);
  v = (fb * ld) : de.delay(MAXBUF, dR - 1);
};
cross(x) = (crossLoop ~ _)(x) : !, _, _;

// Tail gate.
absFb = abs(fb);
decayPeriods = select2((absFb > 0) & (absFb < 1), 0, 15 / (0 - log(max(absFb, 1e-9)) / log(2)));
tailLen = max(dL, dR) * (1 + decayPeriods);

n = ba.time;
blockStart = ((n % spt) % 256) == 0;
hadInput(x) = (x != 0) | (_ & (1 - blockStart)) ~ _;
gateOn(x) = on
with {
  h = hadInput(x);
  mark = (select2(blockStart & h', _, n)) ~ _;
  on = (select2(blockStart, _, (n - mark) >= tailLen)) ~ _;
};
gate(x, y) = select2(gateOn(x) & (1 - hadInput(x)), y, 0);

process(x, y) = dryThru * x + wet * ld, dryThru * x + wet * rd
               : gate(x), gate(x)
with {
  ld = cross(x) : _, !;
  rd = cross(x) : !, _;
};
