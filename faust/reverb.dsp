import("stdfaust.lib");

// Jeskola Freeverb: Jezar's Freeverb plus hi/lo-cut EQ and reverb pre-delay.
// Param order and mappings follow Jeskola/Freeverb/main.cpp (mi::Tick) and
// tuning.h. All params are Buzz bytes 0..127.

rev_time  = hslider("[0]Rev Time", 40, 0, 127, 1);
hf_damp   = hslider("[1]HF Damp", 50, 0, 127, 1);
pre_delay = hslider("[2]Pre Delay", 0, 0, 127, 1);
low_cut   = hslider("[3]Low Cut", 0, 0, 127, 1);
high_cut  = hslider("[4]High Cut", 127, 0, 127, 1);
rev_out   = hslider("[5]Rev Out", 80, 0, 127, 1);
dry_out   = hslider("[6]Dry Out", 127, 0, 127, 1);

// tuning.h constants
fixedgain = 0.015;
allpass_feedback = 0.5;
stereo_spread = 23;

roomsize = rev_time / 128 * 0.28 + 0.7;
damp = hf_damp / 128 * 0.4;
wet = 3 * ba.db2linear((128 - rev_out) / 128 * -60);
dry = dry_out / 255 * 2;

// Pre-delay is in ms; up to 127ms at 192kHz
max_pre_delay = 32768;
pre_delay_samples = int(pre_delay * ma.SR / 1000);

// RBJ Butterworth (Q = 1/sqrt 2) hi/lo cut, applied to the input (dry + wet).
// Low Cut 0 is a ~0Hz highpass in the original: bypass it exactly.
low_cut_hz = max(1, low_cut * 4);
high_cut_hz = 100 + high_cut * 128;
lo_cut = _ <: _, fi.highpass(2, low_cut_hz) : select2(low_cut > 0);
eq = lo_cut : fi.lowpass(2, high_cut_hz);

pre_delay_line = de.delay(max_pre_delay, pre_delay_samples);
wet_path = par(i, 2, *(fixedgain))
    : re.stereo_freeverb(roomsize, allpass_feedback, damp, stereo_spread)
    : par(i, 2, pre_delay_line : *(wet));
dry_path = par(i, 2, *(dry));

process = par(i, 2, eq) <: dry_path, wet_path :> _, _;
