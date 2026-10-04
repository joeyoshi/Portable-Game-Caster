use std::fmt;
use std::io::{self, IsTerminal};
use std::sync::atomic::{
    AtomicU8,
    Ordering,
};

use crate::state::AppState;


// -----------------------------------------------------------------------------
// Runtime logging level
// -----------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
)]
#[repr(u8)]
pub enum LogLevel {
    Off = 0,
    Debug = 1,
    Trace = 2,
}


static LOG_LEVEL: AtomicU8 =
    AtomicU8::new(
        LogLevel::Off as u8
    );


// -----------------------------------------------------------------------------
// Startup argument parsing
//
// no flag     → Off
// --debug     → Debug
// --verbose   → Trace
//
// --verbose implies --debug.
// -----------------------------------------------------------------------------

pub fn init_from_args() {
    let mut level =
        LogLevel::Off;


    for argument in
        std::env::args().skip(1)
    {
        match argument.as_str() {
            "--debug" => {
                if level < LogLevel::Debug {
                    level =
                        LogLevel::Debug;
                }
            }


            "--verbose" => {
                level =
                    LogLevel::Trace;
            }


            _ => {}
        }
    }


    LOG_LEVEL.store(
        level as u8,
        Ordering::Relaxed,
    );


    if debug_enabled() {
        debug(
            "APP",
            format_args!(
                "Portable Game Caster diagnostics enabled ({level:?})"
            ),
        );
    }
}


// -----------------------------------------------------------------------------
// Level checks
// -----------------------------------------------------------------------------

pub fn level() -> LogLevel {
    match LOG_LEVEL.load(
        Ordering::Relaxed
    ) {
        2 => LogLevel::Trace,
        1 => LogLevel::Debug,
        _ => LogLevel::Off,
    }
}


pub fn debug_enabled() -> bool {
    level() >= LogLevel::Debug
}


pub fn trace_enabled() -> bool {
    level() >= LogLevel::Trace
}


// -----------------------------------------------------------------------------
// PGC logging
// -----------------------------------------------------------------------------

pub fn debug(
    category: &str,
    args: fmt::Arguments<'_>,
) {
    if !debug_enabled() {
        return;
    }

    print_log(
        category,
        args,
        false,
    );
}


pub fn trace(
    category: &str,
    args: fmt::Arguments<'_>,
) {
    if !trace_enabled() {
        return;
    }

    print_log(
        category,
        args,
        true,
    );
}


// -----------------------------------------------------------------------------
// State logging
//
// This should be called whenever a state is sent to AppKit.
// -----------------------------------------------------------------------------

pub fn state(
    state: &AppState,
) {
    if !debug_enabled() {
        return;
    }


    debug(
        "STATE",
        format_args!(
            "{state:?} | {}",
            state.message()
        ),
    );
}


// -----------------------------------------------------------------------------
// Internal formatter
// -----------------------------------------------------------------------------

fn print_log(
    category: &str,
    args: fmt::Arguments<'_>,
    is_trace: bool,
) {
    let stderr_is_terminal =
        io::stderr().is_terminal();


    if stderr_is_terminal {
        if is_trace {
            eprintln!(
                "\x1b[90m[PGC][{category}]\x1b[0m {args}"
            );
        } else {
            eprintln!(
                "\x1b[36m[PGC][{category}]\x1b[0m {args}"
            );
        }
    } else {
        eprintln!(
            "[PGC][{category}] {args}"
        );
    }
}