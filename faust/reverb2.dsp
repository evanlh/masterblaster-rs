// Jeskola Reverb 2 — Schroeder reverb with early reflections
// No C++ source available. Param order, ranges and defaults come from the
// BMX PARA section (see designs/005); the DSP itself is an approximation.
import("stdfaust.lib");

dry_out = hslider("[0]Dry Out", 4096, 0, 4096, 1);
rev_out = hslider("[1]Rev Out", 768, 0, 4096, 1);
er_out = hslider("[2]ER Out", 0, 0, 4096, 1);
rev_time = hslider("[3]Rev Time", 10, 1, 64, 1);
predelay = hslider("[4]Pre-Delay", 0, 0, 100, 1);
high_cutoff = hslider("[5]High Cutoff", 5000, 100, 16384, 1);

// Levels: 4096 = unity gain
dry_level = dry_out / 4096.0;
rev_level = rev_out / 4096.0;
er_level = er_out / 4096.0;

// Map rev_time 1-64 to roomsize 0-1 and damping inversely
rev_amount = (rev_time - 1) / 63.0;
roomsize = rev_amount;
damping = 1.0 - rev_amount * 0.8;

// Pre-delay in ms; up to 100ms at 192kHz
predelay_samples = ma.SR * predelay / 1000.0;
max_predelay = 32768;
predelayed = de.delay(max_predelay, int(predelay_samples));

// Early reflections: tapped delay line at prime-number sample offsets
er(x) = x * 0.4 : de.delay(2048, 227)
       + x * 0.3 : de.delay(2048, 557)
       + x * 0.2 : de.delay(2048, 953)
       + x * 0.15 : de.delay(2048, 1361);

// Late reverb via freeverb
late = re.mono_freeverb(roomsize, damping, 1.0, 1.0);

// High Cutoff: Butterworth lowpass on the wet (ER + late) signal
wet(x) = er(x) * er_level + (x : predelayed : late) * rev_level
       : fi.lowpass(2, min(high_cutoff, ma.SR * 0.45));

// Per-channel: dry + filtered wet
channel(x) = x * dry_level + wet(x);

process = channel, channel;
