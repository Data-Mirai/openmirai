//! ANSI color helpers for terminal output.

#![allow(dead_code)]

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const RED: &str = "\x1b[31m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const BLUE: &str = "\x1b[34m";
pub const MAGENTA: &str = "\x1b[35m";
pub const CYAN: &str = "\x1b[36m";
pub const WHITE: &str = "\x1b[37m";
pub const BRIGHT_RED: &str = "\x1b[91m";
pub const BG_RED: &str = "\x1b[41m";

/// Wrap text in a color code.
pub fn colored(text: &str, color: &str) -> String {
    format!("{color}{text}{RESET}")
}

/// Wrap text in bold.
pub fn bold(text: &str) -> String {
    format!("{BOLD}{text}{RESET}")
}

/// Wrap text in dim.
pub fn dim(text: &str) -> String {
    format!("{DIM}{text}{RESET}")
}
