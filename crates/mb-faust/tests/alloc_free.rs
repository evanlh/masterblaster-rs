//! Allocation-free render-path test for `FaustMachine`.
//!
//! The audio thread runs `Machine::render` inside an `assert_no_alloc` guard,
//! so `FaustMachine::render` must not allocate in the steady state. This test
//! fails until the per-call `Vec` allocations are hoisted into fields
//! (design 026, item A1).
//!
//! Run under `make test-faust` (libfaust is required, single-threaded).

use assert_no_alloc::{assert_no_alloc, AllocDisabler};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

use mb_faust::faust_machine::FaustMachine;
use mb_ir::{AudioBuffer, AudioStream};

/// Identity stereo DSP — enough to exercise the adapter (allocation is in the
/// adapter, not the DSP), while keeping the JIT cheap.
const TRIVIAL_STEREO_DSP: &str = "process = _, _;";

/// Render `frames` twice: once outside the guard to prime internal buffers,
/// then inside `assert_no_alloc` to check the steady-state render path.
fn assert_render_alloc_free(machine: &mut FaustMachine, frames: u16) {
    let mut buf = AudioBuffer::new(2, frames);
    machine.render(&mut buf);

    assert_no_alloc(|| {
        machine.render(&mut buf);
    });
}

#[test]
fn faust_machine_render_is_alloc_free() {
    let mut machine =
        FaustMachine::from_source("alloc-free-test", TRIVIAL_STEREO_DSP).expect("compile DSP");
    assert_render_alloc_free(&mut machine, 256);
}
