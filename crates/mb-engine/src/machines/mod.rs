//! Built-in machine implementations.

mod amiga_filter;

use alloc::boxed::Box;

use crate::machine::Machine;

/// Create a machine by name, or `None` if unknown.
pub fn create_machine(name: &str) -> Option<Box<dyn Machine>> {
    match name {
        "Amiga Filter" => Some(Box::new(amiga_filter::AmigaFilter::new())),
        _ => None,
    }
}
