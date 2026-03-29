pub mod ffi;
pub mod ui_visitor;
pub mod compiler;
pub mod faust_machine;
mod registry;

pub use registry::create_faust_machine;

#[cfg(test)]
mod tests;
