use std::fmt;
use std::io::{
    self,
    IsTerminal,
};
use std::path::Path;
use std::sync::atomic::{
    AtomicU8,
    Ordering,
};
use std::time::{
    SystemTime,
    UNIX_EPOCH,
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
// Startup configuration
//
// Packaged .app:
//     no flags    → Off
//
// Raw Cargo-built executable:
//     no flags    → Debug
//
// Explicit overrides:
//     --quiet     → Off
//     --debug     → Debug
//     --verbose   → Trace
//
// --verbose has highest priority.
// -----------------------------------------------------------------------------

pub fn init_from_args() {
    let arguments:
        Vec<String> =
            std::env::args()
                .skip(1)
                .collect();


    let explicit_verbose =
        arguments
            .iter()
            .any(
                |argument| {
                    argument == "--verbose"
                }
            );


    let explicit_debug =
        arguments
            .iter()
            .any(
                |argument| {
                    argument == "--debug"
                }
            );


    let explicit_quiet =
        arguments
            .iter()
            .any(
                |argument| {
                    argument == "--quiet"
                }
            );


    let default_level =
        if running_from_app_bundle() {
            LogLevel::Off
        } else {
            LogLevel::Debug
        };


    let level =
        if explicit_verbose {
            LogLevel::Trace
        } else if explicit_debug {
            LogLevel::Debug
        } else if explicit_quiet {
            LogLevel::Off
        } else {
            default_level
        };


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


        if running_from_app_bundle() {
            debug(
                "APP",
                format_args!(
                    "Runtime: packaged macOS app"
                ),
            );
        } else {
            debug(
                "APP",
                format_args!(
                    "Runtime: development executable"
                ),
            );
        }
    }
}


// -----------------------------------------------------------------------------
// Determine whether this executable lives inside a .app bundle
// -----------------------------------------------------------------------------

fn running_from_app_bundle() -> bool {
    let Ok(executable) =
        std::env::current_exe()
    else {
        return false;
    };


    path_is_inside_app_bundle(
        &executable
    )
}


fn path_is_inside_app_bundle(
    path: &Path,
) -> bool {
    let path_text =
        path
            .to_string_lossy();


    path_text.contains(
        ".app/Contents/MacOS/"
    )
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
    level()
        >= LogLevel::Debug
}


pub fn trace_enabled() -> bool {
    level()
        >= LogLevel::Trace
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
    let terminal =
        io::stderr()
            .is_terminal();


    let timestamp =
        utc_timestamp();


    if terminal {
        if is_trace {
            eprintln!(
                "\x1b[90m[{timestamp}][PGC][{category}]\x1b[0m {args}"
            );
        } else {
            eprintln!(
                "\x1b[36m[{timestamp}][PGC][{category}]\x1b[0m {args}"
            );
        }
    } else {
        eprintln!(
            "[{timestamp}][PGC][{category}] {args}"
        );
    }
}


// -----------------------------------------------------------------------------
// UTC time of day with milliseconds
//
// Same format as the Windows Host diagnostics so Client and Host logs can be
// lined up.
// -----------------------------------------------------------------------------

fn utc_timestamp() -> String {
    let millis =
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(
                |elapsed| {
                    elapsed.as_millis()
                }
            )
            .unwrap_or(0);


    format_time_of_day(
        millis
    )
}


fn format_time_of_day(
    unix_millis: u128,
) -> String {
    let millis_of_day =
        unix_millis % 86_400_000;


    format!(
        "{:02}:{:02}:{:02}.{:03}Z",
        millis_of_day / 3_600_000,
        millis_of_day / 60_000 % 60,
        millis_of_day / 1000 % 60,
        millis_of_day % 1000,
    )
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn formats_time_of_day() {
        assert_eq!(
            format_time_of_day(0),
            "00:00:00.000Z"
        );

        assert_eq!(
            format_time_of_day(86_400_000 + 3_723_045),
            "01:02:03.045Z"
        );
    }
}
