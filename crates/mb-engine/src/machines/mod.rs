//! Built-in machine implementations.

mod amiga_filter;
mod passthrough;
pub mod tracker;

use alloc::boxed::Box;

use crate::machine::Machine;

/// Create a machine by name.
///
/// Returns the matching implementation if available, otherwise a
/// `PassthroughMachine` so the graph shape is preserved.
pub fn create_machine(name: &str) -> Option<Box<dyn Machine>> {
    Some(match name {
        "Amiga Filter" => Box::new(amiga_filter::AmigaFilter::new()),
        _ => Box::new(passthrough::PassthroughMachine),
    })
}
