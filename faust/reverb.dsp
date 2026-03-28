import("stdfaust.lib");

roomsize = hslider("Room Size", 0.5, 0.0, 1.0, 0.01);
damping = hslider("Damping", 0.5, 0.0, 1.0, 0.01);
wet = hslider("Wet", 0.33, 0.0, 1.0, 0.01);

reverb = re.mono_freeverb(roomsize, damping, 1.0, 1.0);
channel = _ <: *(1-wet), reverb * wet :> _;
process = channel, channel;
