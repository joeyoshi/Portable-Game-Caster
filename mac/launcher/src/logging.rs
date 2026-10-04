use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::state::AppState;

// -----------------------------------------------------------------------------
// Unified PGC logging (Client)
//
// The level semantics, line format, timestamp, and category colours in this
// file are intentionally identical to windows/service/src/logging.rs.
// -----------------------------------------------------------------------------

// -----------------------------------------------------------------------------
// Levels
//
// Shared semantics across Host and Client:
//
//     Quiet    nothing (explicit --quiet only)
//     Normal   important lifecycle / user-facing events, warnings, errors
//     Debug    Normal + structured PGC diagnostics (event-driven)
//     Verbose  Debug + raw external-process output, every line labelled with
//              its source
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Quiet = 0,
    Normal = 1,
    Debug = 2,
    Verbose = 3,
}

static LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Normal as u8);

static COLOUR: AtomicBool = AtomicBool::new(false);

pub fn level() -> LogLevel {
    match LOG_LEVEL.load(Ordering::Relaxed) {
        0 => LogLevel::Quiet,
        2 => LogLevel::Debug,
        3 => LogLevel::Verbose,
        _ => LogLevel::Normal,
    }
}

fn set_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

fn should_log(current: LogLevel, required: LogLevel) -> bool {
    current >= required
}

#[allow(dead_code)]
pub fn debug_enabled() -> bool {
    should_log(level(), LogLevel::Debug)
}

pub fn verbose_enabled() -> bool {
    should_log(level(), LogLevel::Verbose)
}

// --verbose wins over --debug, which wins over --quiet.
fn level_from_arguments(arguments: &[String], default: LogLevel) -> LogLevel {
    let has = |flag: &str| arguments.iter().any(|argument| argument == flag);

    if has("--verbose") {
        LogLevel::Verbose
    } else if has("--debug") {
        LogLevel::Debug
    } else if has("--quiet") {
        LogLevel::Quiet
    } else {
        default
    }
}

// -----------------------------------------------------------------------------
// Sources
//
// Who produced a line. Only shown in Verbose, where external process output is
// mixed in and every line must identify its origin.
// -----------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Pgc,
    Ffplay,
    Ffmpeg,
    Mtx,
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Source::Pgc => "PGC",
            Source::Ffplay => "FFPLAY",
            Source::Ffmpeg => "FFMPEG",
            Source::Mtx => "MTX",
        }
    }

    fn colour(self) -> &'static str {
        match self {
            Source::Pgc => "\x1b[37m",
            Source::Ffplay | Source::Ffmpeg => "\x1b[38;5;141m",
            Source::Mtx => "\x1b[38;5;109m",
        }
    }
}

// -----------------------------------------------------------------------------
// Styling
//
// The only place ANSI codes live. Category names and colours are the same in
// the Host and the Client.
// -----------------------------------------------------------------------------

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[90m";
const WARNING_COLOUR: &str = "\x1b[33m";
const ERROR_COLOUR: &str = "\x1b[31m";

// "[FFMPEG]" / "[FFPLAY]"
const SOURCE_WIDTH: usize = 8;

// "[DISCOVERY]" / "[RECONNECT]", plus one column of breathing room
const CATEGORY_WIDTH: usize = 12;

fn category_colour(category: &str) -> &'static str {
    match category {
        "APP" => "\x1b[97m",
        "STATE" => "\x1b[36m",
        "ACTION" => "\x1b[95m",
        "CONNECT" => "\x1b[34m",
        "DISCOVERY" => "\x1b[96m",
        "PLAYER" => "\x1b[35m",
        "STREAM" => "\x1b[32m",
        "HEALTH" => "\x1b[33m",
        "RECONNECT" => "\x1b[93m",
        "HOST" => "\x1b[94m",
        "CAPTURE" => "\x1b[92m",
        "ENCODER" => "\x1b[38;5;177m",
        "DEMAND" => "\x1b[38;5;208m",
        _ => "\x1b[37m",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy)]
struct Style {
    colour: bool,

    // Verbose: every line carries its source.
    show_source: bool,
}

fn current_style() -> Style {
    Style {
        colour: COLOUR.load(Ordering::Relaxed),
        show_source: verbose_enabled(),
    }
}

// "[LABEL]" padded with spaces to `width`, coloured without disturbing the
// padding.
fn push_field(line: &mut String, label: &str, width: usize, colour: Option<&str>) {
    match colour {
        Some(colour) => {
            line.push_str(colour);
            line.push('[');
            line.push_str(label);
            line.push(']');
            line.push_str(RESET);
        }

        None => {
            line.push('[');
            line.push_str(label);
            line.push(']');
        }
    }

    for _ in (label.len() + 2)..width {
        line.push(' ');
    }

    line.push(' ');
}

fn push_blank_field(line: &mut String, width: usize) {
    for _ in 0..=width {
        line.push(' ');
    }
}

// Debug:    [07:50:41.098Z] [STATE]      message
// Verbose:  [07:50:41.098Z] [PGC]    [STATE]      message
//           [07:50:41.098Z] [FFMPEG]              message
fn format_line(
    style: Style,
    timestamp: &str,
    source: Source,
    category: Option<&str>,
    severity: Severity,
    message: &str,
) -> String {
    let mut line = String::with_capacity(48 + message.len());

    if style.colour {
        line.push_str(DIM);
    }

    line.push('[');
    line.push_str(timestamp);
    line.push(']');

    if style.colour {
        line.push_str(RESET);
    }

    line.push(' ');

    if style.show_source {
        push_field(
            &mut line,
            source.label(),
            SOURCE_WIDTH,
            style.colour.then(|| source.colour()),
        );
    }

    match category {
        Some(category) => push_field(
            &mut line,
            category,
            CATEGORY_WIDTH,
            style.colour.then(|| category_colour(category)),
        ),

        None => push_blank_field(&mut line, CATEGORY_WIDTH),
    }

    let (prefix, prefix_colour) = match severity {
        Severity::Info => ("", ""),
        Severity::Warning => ("WARNING: ", WARNING_COLOUR),
        Severity::Error => ("ERROR: ", ERROR_COLOUR),
    };

    if !prefix.is_empty() {
        if style.colour {
            line.push_str(prefix_colour);
            line.push_str(prefix);
            line.push_str(RESET);
        } else {
            line.push_str(prefix);
        }
    }

    line.push_str(message);
    line.push('\n');

    line
}

// One write per line, so lines from different threads never interleave.
fn emit(line: &str) {
    let mut stderr = io::stderr().lock();

    let _ = stderr.write_all(line.as_bytes());
    let _ = stderr.flush();
}

fn log(required: LogLevel, category: &str, severity: Severity, args: fmt::Arguments<'_>) {
    if !should_log(level(), required) {
        return;
    }

    emit(&format_line(
        current_style(),
        &utc_timestamp(),
        Source::Pgc,
        Some(category),
        severity,
        &args.to_string(),
    ));
}

// -----------------------------------------------------------------------------
// PGC logging
// -----------------------------------------------------------------------------

// Normal: important lifecycle / user-facing events.
#[allow(dead_code)]
pub fn info(category: &str, args: fmt::Arguments<'_>) {
    log(LogLevel::Normal, category, Severity::Info, args);
}

#[allow(dead_code)]
pub fn warn(category: &str, args: fmt::Arguments<'_>) {
    log(LogLevel::Normal, category, Severity::Warning, args);
}

#[allow(dead_code)]
pub fn error(category: &str, args: fmt::Arguments<'_>) {
    log(LogLevel::Normal, category, Severity::Error, args);
}

// Debug: structured, event-driven PGC diagnostics.
pub fn debug(category: &str, args: fmt::Arguments<'_>) {
    log(LogLevel::Debug, category, Severity::Info, args);
}

// A warning that only matters when diagnosing.
#[allow(dead_code)]
pub fn debug_warn(category: &str, args: fmt::Arguments<'_>) {
    log(LogLevel::Debug, category, Severity::Warning, args);
}

// Verbose: fine-grained PGC detail.
#[allow(dead_code)]
pub fn verbose(category: &str, args: fmt::Arguments<'_>) {
    log(LogLevel::Verbose, category, Severity::Info, args);
}

// Verbose: one line of raw external-process output.
pub fn raw(source: Source, line: &str) {
    if !verbose_enabled() {
        return;
    }

    let style = current_style();

    emit(&format_line(
        style,
        &utc_timestamp(),
        source,
        None,
        Severity::Info,
        &decorate_raw(source, line, style.colour),
    ));
}

// -----------------------------------------------------------------------------
// Carriage-return progress output
//
// FFmpeg and ffplay rewrite a status line in place using a bare carriage
// return. Passed through as-is, that line collides with timestamped log lines.
// Instead each progress update becomes an ordinary complete line, and only one
// is let through per interval.
// -----------------------------------------------------------------------------

pub const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

pub struct ProgressThrottle {
    last: Option<Instant>,
}

impl ProgressThrottle {
    pub fn new() -> Self {
        Self { last: None }
    }

    pub fn allow(&mut self, now: Instant) -> bool {
        let due = self
            .last
            .is_none_or(|last| now.saturating_duration_since(last) >= PROGRESS_INTERVAL);

        if due {
            self.last = Some(now);
        }

        due
    }
}

// -----------------------------------------------------------------------------
// UTC time of day with milliseconds
//
// Identical on Host and Client so logs from both machines line up.
// -----------------------------------------------------------------------------

fn utc_timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);

    format_time_of_day(millis)
}

fn format_time_of_day(unix_millis: u128) -> String {
    let millis_of_day = unix_millis % 86_400_000;

    format!(
        "{:02}:{:02}:{:02}.{:03}Z",
        millis_of_day / 3_600_000,
        millis_of_day / 60_000 % 60,
        millis_of_day / 1000 % 60,
        millis_of_day % 1000,
    )
}

// -----------------------------------------------------------------------------
// Startup configuration
//
// Packaged .app:
//     no flags    -> Normal
//
// Raw Cargo-built executable:
//     no flags    -> Debug
//
// Explicit overrides:
//     --quiet     -> Quiet
//     --debug     -> Debug
//     --verbose   -> Verbose
// -----------------------------------------------------------------------------

pub fn init_from_args() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    let packaged = running_from_app_bundle();

    let default_level = if packaged {
        LogLevel::Normal
    } else {
        LogLevel::Debug
    };

    let level = level_from_arguments(&arguments, default_level);

    set_level(level);

    // Colour is presentation only: never when redirected, never fatal.
    COLOUR.store(
        io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        Ordering::Relaxed,
    );

    info(
        "APP",
        format_args!("Portable Game Caster Client starting (logging: {level:?})."),
    );

    debug(
        "APP",
        format_args!(
            "Runtime: {}",
            if packaged {
                "packaged macOS app"
            } else {
                "development executable"
            }
        ),
    );
}

fn running_from_app_bundle() -> bool {
    let Ok(executable) = std::env::current_exe() else {
        return false;
    };

    path_is_inside_app_bundle(&executable)
}

fn path_is_inside_app_bundle(path: &Path) -> bool {
    path.to_string_lossy().contains(".app/Contents/MacOS/")
}

// -----------------------------------------------------------------------------
// State logging
//
// Every state the UI shows is logged, so the last STATE line is always what the
// user is looking at. The states a user would describe a session by are Normal;
// the transient ones in between are Debug.
// -----------------------------------------------------------------------------

fn state_level(state: &AppState) -> LogLevel {
    match state {
        AppState::Idle | AppState::Playing(_) | AppState::Error(_) => LogLevel::Normal,

        AppState::Discovering { .. }
        | AppState::Resolving(_)
        | AppState::Connecting(_)
        | AppState::WaitingForStream { .. }
        | AppState::ReconnectingStream { .. }
        | AppState::ReconnectingHost { .. } => LogLevel::Debug,
    }
}

fn state_text(state: &AppState) -> String {
    format!("{state:?} | {}", state.message())
}

pub fn state(state: &AppState) {
    log(
        state_level(state),
        "STATE",
        Severity::Info,
        format_args!("{}", state_text(state)),
    );
}

// -----------------------------------------------------------------------------
// Explicit user actions
// -----------------------------------------------------------------------------

fn action_text(name: &str) -> String {
    format!("{name} requested.")
}

pub fn action(name: &str) {
    debug("ACTION", format_args!("{}", action_text(name)));
}

// ffplay output is passed through unchanged.
fn decorate_raw(_source: Source, line: &str, _colour: bool) -> String {
    line.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN_DEBUG: Style = Style {
        colour: false,
        show_source: false,
    };

    const PLAIN_VERBOSE: Style = Style {
        colour: false,
        show_source: true,
    };

    #[test]
    fn formats_time_of_day() {
        assert_eq!(format_time_of_day(0), "00:00:00.000Z");

        assert_eq!(format_time_of_day(86_400_000 + 3_723_045), "01:02:03.045Z");
    }

    #[test]
    fn levels_filter_in_order() {
        assert!(LogLevel::Quiet < LogLevel::Normal);
        assert!(LogLevel::Normal < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Verbose);

        assert!(!should_log(LogLevel::Quiet, LogLevel::Normal));
        assert!(should_log(LogLevel::Normal, LogLevel::Normal));
        assert!(!should_log(LogLevel::Normal, LogLevel::Debug));
        assert!(should_log(LogLevel::Debug, LogLevel::Normal));
        assert!(!should_log(LogLevel::Debug, LogLevel::Verbose));
        assert!(should_log(LogLevel::Verbose, LogLevel::Debug));
    }

    #[test]
    fn flags_select_level() {
        let arguments =
            |flags: &[&str]| -> Vec<String> { flags.iter().map(|flag| flag.to_string()).collect() };

        assert_eq!(
            level_from_arguments(&arguments(&[]), LogLevel::Normal),
            LogLevel::Normal
        );

        assert_eq!(
            level_from_arguments(&arguments(&["--debug"]), LogLevel::Normal),
            LogLevel::Debug
        );

        assert_eq!(
            level_from_arguments(&arguments(&["--quiet"]), LogLevel::Debug),
            LogLevel::Quiet
        );

        assert_eq!(
            level_from_arguments(
                &arguments(&["--quiet", "--debug", "--verbose"]),
                LogLevel::Normal
            ),
            LogLevel::Verbose
        );
    }

    #[test]
    fn debug_line_is_aligned_and_omits_source() {
        assert_eq!(
            format_line(
                PLAIN_DEBUG,
                "07:50:41.098Z",
                Source::Pgc,
                Some("STATE"),
                Severity::Info,
                "Playing(\"carock-pgc.local\") | Connected.",
            ),
            "[07:50:41.098Z] [STATE]      Playing(\"carock-pgc.local\") | Connected.\n"
        );
    }

    #[test]
    fn verbose_lines_identify_every_source_and_share_a_message_column() {
        let pgc = format_line(
            PLAIN_VERBOSE,
            "07:50:41.098Z",
            Source::Pgc,
            Some("STATE"),
            Severity::Info,
            "message",
        );

        assert_eq!(pgc, "[07:50:41.098Z] [PGC]    [STATE]      message\n");

        for (source, label) in [
            (Source::Ffplay, "[FFPLAY]"),
            (Source::Ffmpeg, "[FFMPEG]"),
            (Source::Mtx, "[MTX]"),
        ] {
            let line = format_line(
                PLAIN_VERBOSE,
                "07:50:41.098Z",
                source,
                None,
                Severity::Info,
                "message",
            );

            assert!(line.starts_with(&format!("[07:50:41.098Z] {label} ")));
            assert_eq!(line.find("message"), pgc.find("message"));
        }
    }

    #[test]
    fn every_category_aligns_to_the_same_column() {
        let column = |category| {
            format_line(
                PLAIN_DEBUG,
                "07:50:41.098Z",
                Source::Pgc,
                Some(category),
                Severity::Info,
                "message",
            )
            .find("message")
        };

        for category in [
            "APP",
            "ACTION",
            "CONNECT",
            "DISCOVERY",
            "PLAYER",
            "STREAM",
            "HEALTH",
            "RECONNECT",
            "HOST",
            "CAPTURE",
            "ENCODER",
            "DEMAND",
        ] {
            assert_eq!(column(category), column("STATE"), "{category}");
        }
    }

    #[test]
    fn plain_output_has_no_escape_codes() {
        for severity in [Severity::Info, Severity::Warning, Severity::Error] {
            for style in [PLAIN_DEBUG, PLAIN_VERBOSE] {
                let line = format_line(
                    style,
                    "07:50:41.098Z",
                    Source::Mtx,
                    Some("HEALTH"),
                    severity,
                    "message",
                );

                assert!(!line.contains('\x1b'), "{line:?}");
            }
        }
    }

    #[test]
    fn colour_does_not_change_the_visible_text() {
        let strip = |text: &str| {
            let mut output = String::new();
            let mut characters = text.chars();

            while let Some(character) = characters.next() {
                if character == '\x1b' {
                    for next in characters.by_ref() {
                        if next == 'm' {
                            break;
                        }
                    }
                } else {
                    output.push(character);
                }
            }

            output
        };

        for show_source in [false, true] {
            let coloured = format_line(
                Style {
                    colour: true,
                    show_source,
                },
                "07:50:41.098Z",
                Source::Pgc,
                Some("RECONNECT"),
                Severity::Warning,
                "message",
            );

            let plain = format_line(
                Style {
                    colour: false,
                    show_source,
                },
                "07:50:41.098Z",
                Source::Pgc,
                Some("RECONNECT"),
                Severity::Warning,
                "message",
            );

            assert!(coloured.contains("\x1b[90m[07:50:41.098Z]\x1b[0m"));
            assert_eq!(strip(&coloured), plain);
        }
    }

    #[test]
    fn severity_prefixes_the_message() {
        let line = |severity| {
            format_line(
                PLAIN_DEBUG,
                "07:50:41.098Z",
                Source::Pgc,
                Some("HOST"),
                severity,
                "message",
            )
        };

        assert!(line(Severity::Warning).ends_with("WARNING: message\n"));
        assert!(line(Severity::Error).ends_with("ERROR: message\n"));
    }

    #[test]
    fn shared_categories_have_stable_distinct_colours() {
        let categories = [
            "APP",
            "STATE",
            "ACTION",
            "CONNECT",
            "DISCOVERY",
            "PLAYER",
            "STREAM",
            "HEALTH",
            "RECONNECT",
            "HOST",
            "CAPTURE",
            "ENCODER",
            "DEMAND",
        ];

        for (index, category) in categories.iter().enumerate() {
            for other in &categories[index + 1..] {
                assert_ne!(
                    category_colour(category),
                    category_colour(other),
                    "{category} / {other}"
                );
            }
        }

        // Pinned: these must match the other PGC application.
        assert_eq!(category_colour("STATE"), "\x1b[36m");
        assert_eq!(category_colour("HEALTH"), "\x1b[33m");
        assert_eq!(category_colour("DEMAND"), "\x1b[38;5;208m");
    }

    #[test]
    fn progress_is_throttled() {
        let start = Instant::now();
        let mut throttle = ProgressThrottle::new();

        assert!(throttle.allow(start));
        assert!(!throttle.allow(start + Duration::from_millis(500)));
        assert!(!throttle.allow(start + Duration::from_millis(4999)));
        assert!(throttle.allow(start + PROGRESS_INTERVAL));
        assert!(!throttle.allow(start + PROGRESS_INTERVAL + Duration::from_secs(1)));
    }

    #[test]
    fn action_breadcrumb_format() {
        assert_eq!(
            format_line(
                PLAIN_DEBUG,
                "07:50:41.098Z",
                Source::Pgc,
                Some("ACTION"),
                Severity::Info,
                &action_text("Stop Stream"),
            ),
            "[07:50:41.098Z] [ACTION]     Stop Stream requested.\n"
        );
    }

    #[test]
    fn state_line_shows_state_and_user_facing_message() {
        assert_eq!(state_text(&AppState::Idle), "Idle | Ready");

        assert_eq!(
            state_text(&AppState::Playing("carock-pgc.local".into())),
            "Playing(\"carock-pgc.local\") | Connected."
        );
    }

    #[test]
    fn session_outcome_states_are_normal_level() {
        assert_eq!(state_level(&AppState::Idle), LogLevel::Normal);

        assert_eq!(
            state_level(&AppState::Playing("host".into())),
            LogLevel::Normal
        );

        assert_eq!(
            state_level(&AppState::Error("Stream did not start.".into())),
            LogLevel::Normal
        );

        assert_eq!(
            state_level(&AppState::Connecting("host".into())),
            LogLevel::Debug
        );

        assert_eq!(
            state_level(&AppState::ReconnectingStream {
                host: "host".into(),
                seconds_remaining: 15,
            }),
            LogLevel::Debug
        );
    }

    #[test]
    fn detects_app_bundle_path() {
        assert!(path_is_inside_app_bundle(Path::new(
            "/Applications/Portable Game Caster.app/Contents/MacOS/pgc"
        )));

        assert!(!path_is_inside_app_bundle(Path::new(
            "/Users/me/pgc/target/release/pgc-launcher-macos"
        )));
    }
}
