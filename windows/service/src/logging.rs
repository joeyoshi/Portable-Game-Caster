// -----------------------------------------------------------------------------
// Unified PGC logging (Host)
//
// The level semantics, line format, timestamp, and category colours in this
// file are intentionally identical to mac/launcher/src/logging.rs.
// -----------------------------------------------------------------------------

use std::fmt;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
//     no flags    -> Normal
//     --quiet     -> Quiet
//     --debug     -> Debug
//     --verbose   -> Verbose
//
// `ansi` is whether the console accepts ANSI escape sequences. Colour is
// presentation only: never when redirected, never fatal.
// -----------------------------------------------------------------------------

pub fn init_from_args(ansi: bool) -> LogLevel {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    let level = level_from_arguments(&arguments, LogLevel::Normal);

    set_level(level);

    COLOUR.store(
        ansi && std::env::var_os("NO_COLOR").is_none(),
        Ordering::Relaxed,
    );

    level
}

// -----------------------------------------------------------------------------
// External process output
// -----------------------------------------------------------------------------

// MediaMTX only colours its level tag when writing to a terminal and has no
// option to force it, so restore that for relayed lines.
const LEVEL_COLOURS: [(&str, &str); 4] = [
    (" INF ", "\x1b[32m"),
    (" WAR ", "\x1b[33m"),
    (" ERR ", "\x1b[31m"),
    (" DEB ", "\x1b[90m"),
];

fn decorate_raw(source: Source, line: &str, colour: bool) -> String {
    if colour && source == Source::Mtx {
        colour_level(line)
    } else {
        line.to_string()
    }
}

// Colours the first MediaMTX-style level tag near the start of the line.
fn colour_level(line: &str) -> String {
    for (level, colour) in LEVEL_COLOURS {
        if let Some(position) = line.find(level) {
            if position <= 24 {
                let tag = level.trim();

                return format!(
                    "{} {colour}{tag}{RESET} {}",
                    &line[..position],
                    &line[position + level.len()..]
                );
            }
        }
    }

    line.to_string()
}

// Reads a child process's output to the end. The pipe is always drained and
// `observe` sees every complete line at every log level; the lines themselves
// are only printed in Verbose, labelled with `source`.
pub fn relay_output<R>(source: R, label: Source, observe: impl Fn(&str) + Send + 'static)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut throttle = ProgressThrottle::new();

        read_segments(source, |segment| match segment {
            Segment::Line(line) => {
                observe(line);

                if !line.is_empty() {
                    raw(label, line);
                }
            }

            Segment::Progress(line) => {
                if !line.is_empty() && throttle.allow(Instant::now()) {
                    raw(label, line);
                }
            }
        });
    });
}

#[derive(Debug, PartialEq, Eq)]
enum Segment<'a> {
    // Terminated by a line feed (or CRLF).
    Line(&'a str),

    // Terminated by a bare carriage return: an in-place progress update.
    Progress(&'a str),
}

fn read_segments<R: Read>(mut source: R, mut handle: impl FnMut(Segment<'_>)) {
    let mut read_buffer = [0_u8; 4096];
    let mut pending: Vec<u8> = Vec::new();

    loop {
        let count = match source.read(&mut read_buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };

        pending.extend_from_slice(&read_buffer[..count]);

        drain_segments(&mut pending, &mut handle);
    }

    if !pending.is_empty() {
        let text = String::from_utf8_lossy(&pending);

        handle(Segment::Line(text.trim_end()));
    }
}

fn drain_segments(pending: &mut Vec<u8>, handle: &mut impl FnMut(Segment<'_>)) {
    loop {
        let Some(position) = pending
            .iter()
            .position(|byte| *byte == b'\n' || *byte == b'\r')
        else {
            return;
        };

        let line_feed = pending[position] == b'\n';

        // Carriage return: wait for the next byte to tell CRLF from progress.
        let crlf = !line_feed
            && match pending.get(position + 1) {
                Some(next) => *next == b'\n',
                None => return,
            };

        {
            let text = String::from_utf8_lossy(&pending[..position]);
            let text = text.trim_end();

            if line_feed || crlf {
                handle(Segment::Line(text));
            } else {
                handle(Segment::Progress(text));
            }
        }

        pending.drain(..=position + usize::from(crlf));
    }
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

    fn segments(input: &[u8]) -> Vec<String> {
        let mut output = Vec::new();

        read_segments(input, |segment| {
            output.push(match segment {
                Segment::Line(line) => format!("L:{line}"),
                Segment::Progress(line) => format!("P:{line}"),
            });
        });

        output
    }

    #[test]
    fn splits_lf_and_crlf_lines() {
        assert_eq!(
            segments(b"INF one\nINF two\r\n\nlast"),
            vec!["L:INF one", "L:INF two", "L:", "L:last"]
        );
    }

    #[test]
    fn carriage_return_progress_becomes_separate_segments() {
        assert_eq!(
            segments(b"frame=   1 fps=60   \rframe=   2 fps=60   \rdone\n"),
            vec!["P:frame=   1 fps=60", "P:frame=   2 fps=60", "L:done"]
        );
    }

    #[test]
    fn progress_never_shares_a_line_with_other_output() {
        // What reaches the console for FFmpeg progress followed by a message.
        let mut throttle = ProgressThrottle::new();
        let now = Instant::now();
        let mut console = String::new();

        read_segments(
            &b"frame=1\rframe=2\rframe=3\r[srt] connection lost\n"[..],
            |segment| {
                let text = match segment {
                    Segment::Line(line) => Some(line),
                    Segment::Progress(line) => throttle.allow(now).then_some(line),
                };

                if let Some(text) = text {
                    console.push_str(&format_line(
                        PLAIN_VERBOSE,
                        "07:50:54.337Z",
                        Source::Ffmpeg,
                        None,
                        Severity::Info,
                        text,
                    ));
                }
            },
        );

        assert_eq!(
            console,
            "[07:50:54.337Z] [FFMPEG]              frame=1\n\
             [07:50:54.337Z] [FFMPEG]              [srt] connection lost\n"
        );

        assert!(!console.contains('\r'));
    }

    #[test]
    fn colours_mediamtx_level_tag_only_when_enabled() {
        let line = "2026/10/04 01:23:13 WAR [path gameplay] something";

        assert_eq!(decorate_raw(Source::Mtx, line, false), line);

        assert_eq!(
            decorate_raw(Source::Mtx, line, true),
            "2026/10/04 01:23:13 \x1b[33mWAR\x1b[0m [path gameplay] something"
        );

        // Not MediaMTX, or level-like text far into a message: left alone.
        assert_eq!(decorate_raw(Source::Ffmpeg, line, true), line);

        assert_eq!(
            colour_level("frame= 100 fps= 60 q=20.0 size= 1024KiB INF x"),
            "frame= 100 fps= 60 q=20.0 size= 1024KiB INF x"
        );
    }
}
