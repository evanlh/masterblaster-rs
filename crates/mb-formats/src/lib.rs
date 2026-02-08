//! Format parsers for masterblaster tracker.
//!
//! Parses MOD, XM, IT, S3M, and BMX files into the IR.

mod mod_format;

pub use mod_format::load_mod;

/// Error type for format parsing.
#[derive(Debug)]
pub enum FormatError {
    /// Invalid file header or magic bytes
    InvalidHeader,
    /// Unexpected end of file
    UnexpectedEof,
    /// Unsupported format version
    UnsupportedVersion,
    /// I/O error
    Io(alloc::string::String),
}

extern crate alloc;
