// WhiteNoise's Pixelate (David Wallin) -- clean-room port.
//
// Derived only from the buzz-machines-oracles measurements (no machine source read):
// - Impulse probe (-6 dBFS = 16423 in 16-bit units): output is a single sample
//   trunc(16423 / Amount) * Amount, e.g. 9216 -> 9216, 6144 -> 12288, 12988 -> 12988,
//   Amount >= 18432 -> 0. So the machine quantizes the amplitude, in Buzz's 16-bit
//   sample units, to multiples of Amount, rounding toward zero.
// - ESS and white-noise probes match that model bit-exactly except the last sample of
//   every work() block (256-sample blocks on a grid restarting each tick), which passes
//   through untouched. The port reproduces this with a counter over Faust's block size.
// - Amount 0, 3072 (grid) and 3511 (ramp automation) are exact identity, 6144 already
//   quantizes: the machine bypasses small amounts. The threshold lies in (3511, 6144];
//   4096 (= 1/8 of full scale) is assumed.
// - Silent input gives silent output through the quantizer, so the work()==false cut
//   needs no extra modelling.
import("stdfaust.lib");

amountSlider = hslider("[0]Amount", 0, 0, 24576, 1);

// masterblaster rescales raw -> 0..65535 -> slider by truncation; s lies in (raw-0.375, raw].
amount = floor(amountSlider + 0.5);

bypassBelow = 4096;
fullScale = 32768.0;

// Samples elapsed in the current host block (1..BS); a block ends when it reaches
// the previous sample's block size.
blockCount = (\(q).(select2(q >= ma.BS', q, 0) + 1)) ~ _;
isBlockEnd = blockCount >= ma.BS;

quantize(x) = float(int(x * fullScale / amount)) * amount / fullScale;
pixelate(x) = select2((amount >= bypassBelow) & (isBlockEnd == 0), x, quantize(x));

process = pixelate, pixelate;
