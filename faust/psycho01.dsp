// asedev Psycho01 — mono-to-stereo linear panner (clean-room port).
//
// Derived only from info.json / describe.json / triage.json and the oracle's
// measurements (no machine source read). Impulse probe (impulse at sample
// 441) at every tested Room value is a single sample per channel, no memory:
//   L = x * (256 - Room) / 256,   R = x * Room / 256
// The noise/ESS probes show the pan alternates every sample: odd sample
// indices use the gains above, even indices swap L and R (Room 0 sends
// even samples hard right, odd hard left; Room 128 is centred). The parity
// never flips across a 3 s render, so it follows the absolute sample index.
// The machine's description "0=max, 80=min" does not match the measured
// gain law; the measured law is used. Being memoryless, silent input
// already gives silent output, so work_returns_false needs no emulation.
import("stdfaust.lib");

// masterblaster delivers raw r as about r - r/65535: round back to integers.
room = rint(hslider("[0]Room", 128, 0, 128, 1));

pan = room / 256.0;
mono(l, r) = (l + r) * 0.5;
odd = ba.time % 2;
near = select2(odd, pan, 1.0 - pan);   // gain to L
spread(x) = x * near, x * (1.0 - near);

process = mono : spread;
