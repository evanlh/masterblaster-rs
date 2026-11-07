// Ryg's Analog Distort (Fabian Giesen, "ryg's analog distort.dll") - clean-room port.
//
// Derived only from info.json / describe.json and the buzz-machines-oracles measurements
// (no machine source was read). Memoryless; fits every development job of all four probes
// (noise-pink, ess, stepped-sine, two-tone) to float precision (relative rms error ~1e-7):
//
//   X     = input on the Buzz sample scale (x * 32768)
//   G     = 10^(64 * Drive / 32768 / 20)          (describe: "0.0dB" .. "64.0dB", linear in dB)
//   wet   = Clamp * atan(G) * atan(G * X / Clamp) (Clamp = 0 -> silence)
//   y     = (1 - Amount/127) * X + (Amount/127) * (Gain/127) * wet
//
// Evidence: least-squares fits of A*atan(B*x) per job give B = G*32768/Clamp and
// A = (Clamp/32768)*atan(G) exactly (drive 0 -> small-signal gain pi/4 = atan(1); full drive
// -> asymptote Clamp*(pi/2)^2). Amount = 0 returns the input bit-exactly whatever Gain is, and
// Gain = 0 at Amount = 127 gives silence, so Gain scales only the wet path. Amount and Gain are
// linear 0..100% (describe: "-32.5dB (2%)" at 3, i.e. 20*log10(3/127)).
//
// Deliberate difference: at Clamp = 0 the original computes 0 * atan(0/0) = NaN on every
// exact-zero input sample inside a non-silent block (seen in the oracle on ess/stepped-sine/
// two-tone). This port outputs 0 (wet) there instead, since a NaN would poison the
// song's mix; those development jobs score ~63-71 for that reason alone.
//
// work_returns_false: the shaper maps silence to silence and the machine has no memory, so the
// silent-block output cut needs no emulation here. The machine is mono; this port is 2-in/2-out
// with the same shaper on each channel.
import("stdfaust.lib");

// masterblaster hands sliders min + floor(raw*65535/R)/65535 * R; recover the raw integer.
// k = the 0..65535 scaled value; raw = ceil(k*R/65535), computed in exact integer arithmetic.
scaledIndex(R, s) = int(s * 65535.0 / R + 0.5);
floorDiv(a, b) = int(float(a - a % b) / b + 0.5);
ceilDiv(a, b) = floorDiv(a, b) + (a % b > 0);
// For R > 32768, k*R would overflow int32: use ceil(k*R/65535) = k - floor(k*(65535-R)/65535).
rawValue(R, s) = ba.if(R <= 32768, ceilDiv(k * R, 65535), k - floorDiv(k * (65535 - R), 65535))
with { k = scaledIndex(R, s); };

drive = hslider("[0]Drive", 128, 0, 32768, 1) : rawValue(32768);
clamp = hslider("[1]Clamp", 32767, 0, 65534, 1) : rawValue(65534);
amount = hslider("[2]Amount", 127, 0, 127, 1) : rawValue(127);
gain = hslider("[3]Gain", 127, 0, 127, 1) : rawValue(127);
// Integer weights so Amount = 127 cancels the dry path exactly (no 1 - a*(1/127) residue).
dryWeight = (127 - amount) / 127.0;
wetWeight = (amount * gain) / 16129.0;

driveGain = pow(10.0, 64.0 * drive / 32768.0 / 20.0);
safeClamp = max(clamp, 1);
atanShaper(X) = (clamp > 0) * clamp * atan(driveGain) * atan(driveGain * X / safeClamp);
dryWet(X) = dryWeight * X + wetWeight * atanShaper(X);

distort(x) = dryWet(x * 32768.0) / 32768.0;

process = distort, distort;
