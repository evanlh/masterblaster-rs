// Jeskola Reverb 2 — Schroeder reverb with early reflections
import("stdfaust.lib");

dry_out = hslider("Dry Out", 127, 0, 127, 1);
rev_out = hslider("Rev Out", 80, 0, 127, 1);
er_out = hslider("ER Out", 64, 0, 127, 1);
rev_time = hslider("Rev Time", 64, 0, 127, 1);
predelay = hslider("Pre-Delay", 10, 0, 127, 1);

// Normalize levels to 0..1
dry_level = dry_out / 127.0;
rev_level = rev_out / 127.0;
er_level = er_out / 127.0;

// Map rev_time 0-127 to roomsize 0-1 and damping inversely
roomsize = rev_time / 127.0;
damping = 1.0 - (rev_time / 127.0) * 0.8;

// Pre-delay: map 0-127 to 0-100ms
predelay_ms = predelay * (100.0 / 127.0);
predelay_samples = ma.SR * predelay_ms / 1000.0;
max_predelay = 4410; // 100ms at 44100
predelayed = de.delay(max_predelay, int(predelay_samples));

// Early reflections: tapped delay line at prime-number sample offsets
er(x) = x * 0.4 : de.delay(2048, 227)
       + x * 0.3 : de.delay(2048, 557)
       + x * 0.2 : de.delay(2048, 953)
       + x * 0.15 : de.delay(2048, 1361);

// Late reverb via freeverb
late = re.mono_freeverb(roomsize, damping, 1.0, 1.0);

// Per-channel: dry + ER + pre-delayed late reverb
channel(x) = x * dry_level + er(x) * er_level + (x : predelayed : late) * rev_level;

process = channel, channel;
