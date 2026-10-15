//! Everything the binary writes to stdout goes through here.
//!
//! `println!` panics with exit 101 when the reader has closed the pipe, so
//! `terminus sessions | head` printed a Rust panic, and a hook whose reader went
//! away broke its exit-0 rule. These write the same bytes and drop them once the
//! reader is gone: the command still finishes its work and exits with its own
//! code, which is what `doctor` already did for its report.

use std::fmt;
use std::io::{self, Write};

/// `println!`, without the panic on a closed pipe.
macro_rules! outln {
    () => {
        $crate::out::write(format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::out::write(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// `print!`, without the panic on a closed pipe.
macro_rules! out {
    ($($arg:tt)*) => {
        $crate::out::write(format_args!($($arg)*))
    };
}

/// Write to stdout, and drop the bytes if the reader has gone.
///
/// Any other failure panics exactly as `print!` would: a full disk under a
/// redirect is a lost result, not a reader that stopped reading.
pub fn write(args: fmt::Arguments<'_>) {
    match io::stdout().lock().write_fmt(args) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
        Err(e) => panic!("failed printing to stdout: {e}"),
    }
}
