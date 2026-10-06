use std::fmt;
use std::fs::{self, File};
use std::io::{self, IsTerminal, LineWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
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

// Level of the session log file. Quiet means no file is open.
static FILE_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Quiet as u8);

static COLOUR: AtomicBool = AtomicBool::new(false);

fn level_from_u8(value: u8) -> LogLevel {
    match value {
        0 => LogLevel::Quiet,
        2 => LogLevel::Debug,
        3 => LogLevel::Verbose,
        _ => LogLevel::Normal,
    }
}

// Terminal level.
fn level() -> LogLevel {
    level_from_u8(LOG_LEVEL.load(Ordering::Relaxed))
}

fn file_level() -> LogLevel {
    level_from_u8(FILE_LEVEL.load(Ordering::Relaxed))
}

fn set_level(level: LogLevel) {
    LOG_LEVEL.store(level as u8, Ordering::Relaxed);
}

fn should_log(current: LogLevel, required: LogLevel) -> bool {
    current >= required
}

// True when any sink (terminal or session file) records Debug lines.
#[allow(dead_code)]
pub fn debug_enabled() -> bool {
    should_log(level(), LogLevel::Debug) || should_log(file_level(), LogLevel::Debug)
}

// True when any sink records Verbose lines.
#[allow(dead_code)]
pub fn verbose_enabled() -> bool {
    should_log(level(), LogLevel::Verbose) || should_log(file_level(), LogLevel::Verbose)
}

// The session file is at least Debug so it is useful for support even when the
// terminal is Normal or Quiet. Raw external output is only written when the
// user asked for it.
fn file_level_for(terminal: LogLevel) -> LogLevel {
    if terminal == LogLevel::Verbose {
        LogLevel::Verbose
    } else {
        LogLevel::Debug
    }
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
        "STATE" => "\x1b[96m",
        "ACTION" => "\x1b[95m",
        "CONNECT" => "\x1b[38;5;27m",
        "DISCOVERY" => "\x1b[32m",
        "PLAYER" => "\x1b[35m",
        "STREAM" => "\x1b[38;5;147m",
        "HEALTH" => "\x1b[33m",
        "RECONNECT" => "\x1b[93m",
        "HOST" => "\x1b[38;5;250m",
        "CAPTURE" => "\x1b[38;5;64m",
        "ENCODER" => "\x1b[38;5;43m",
        "DEMAND" => "\x1b[38;5;208m",
        "UI" => "\x1b[38;5;245m",
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

// -----------------------------------------------------------------------------
// Sinks
//
// Every log event goes through one path and fans out:
//
//     log event
//     |- terminal   (its own level; colour when interactive)
//     `- session file (its own level; always plain text)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Sinks {
    terminal: LogLevel,
    terminal_colour: bool,

    // Quiet when no session file is open.
    file: LogLevel,
}

fn current_sinks() -> Sinks {
    Sinks {
        terminal: level(),
        terminal_colour: COLOUR.load(Ordering::Relaxed),
        file: file_level(),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Rendered {
    terminal: Option<String>,
    file: Option<String>,
}

// `terminal_message` may carry styling (for example a coloured MediaMTX level
// tag); `file_message` is the same text without it.
#[allow(clippy::too_many_arguments)]
fn render(
    sinks: Sinks,
    required: LogLevel,
    timestamp: &str,
    source: Source,
    category: Option<&str>,
    severity: Severity,
    terminal_message: &str,
    file_message: &str,
) -> Rendered {
    let terminal = should_log(sinks.terminal, required).then(|| {
        format_line(
            Style {
                colour: sinks.terminal_colour,
                show_source: sinks.terminal == LogLevel::Verbose,
            },
            timestamp,
            source,
            category,
            severity,
            terminal_message,
        )
    });

    let file = should_log(sinks.file, required).then(|| {
        strip_ansi(&format_line(
            Style {
                colour: false,
                show_source: sinks.file == LogLevel::Verbose,
            },
            timestamp,
            source,
            category,
            severity,
            file_message,
        ))
    });

    Rendered { terminal, file }
}

fn deliver(rendered: Rendered) {
    if let Some(line) = rendered.terminal {
        write_terminal(&line);
    }

    if let Some(line) = rendered.file {
        write_file(&line);
    }
}

// One write per line, so lines from different threads never interleave.
fn write_terminal(line: &str) {
    let mut stderr = io::stderr().lock();

    let _ = stderr.write_all(line.as_bytes());
    let _ = stderr.flush();
}

// Line-buffered: each complete line reaches the operating system as it is
// logged (no periodic flushing, no fsync), so a crash loses nothing that was
// already logged.
static LOG_FILE: Mutex<Option<LineWriter<File>>> = Mutex::new(None);

fn write_file(text: &str) {
    if let Ok(mut file) = LOG_FILE.lock() {
        if let Some(file) = file.as_mut() {
            let _ = file.write_all(text.as_bytes());
        }
    }
}

// The file sink never contains terminal styling, whatever a message carries.
fn strip_ansi(text: &str) -> String {
    if !text.contains('\x1b') {
        return text.to_string();
    }

    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars();

    while let Some(character) = characters.next() {
        if character != '\x1b' {
            output.push(character);

            continue;
        }

        // CSI sequence: ESC [ ... final letter.
        for next in characters.by_ref() {
            if next.is_ascii_alphabetic() {
                break;
            }
        }
    }

    output
}

fn log(required: LogLevel, category: &str, severity: Severity, args: fmt::Arguments<'_>) {
    let sinks = current_sinks();

    if !should_log(sinks.terminal, required) && !should_log(sinks.file, required) {
        return;
    }

    let message = args.to_string();

    deliver(render(
        sinks,
        required,
        &utc_timestamp(),
        Source::Pgc,
        Some(category),
        severity,
        &message,
        &message,
    ));
}

// -----------------------------------------------------------------------------
// Startup header
//
// Deliberately not a timestamped log line: it identifies the application,
// platform, logging mode, and session once at the top of a session. The same
// structure is written to the terminal and to the session file.
// -----------------------------------------------------------------------------

const HEADER_RULE: &str = "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━";

// The header is as wide as its rules.
const HEADER_WIDTH: usize = 50;

// One accent family for the whole header: a dimmer teal for the rules, a
// brighter bold teal for the title, neutral labels, bright values.
const HEADER_RULE_COLOUR: &str = "\x1b[38;5;37m";
const HEADER_TITLE_COLOUR: &str = "\x1b[1;38;5;80m";
const HEADER_VALUE_COLOUR: &str = "\x1b[97m";

// "Version:" padded so every value starts in the same column.
const HEADER_LABEL_WIDTH: usize = 13;

// How a build was produced. Shown next to the semantic version. Only
// Development builds exist today; the others are here so that versioning work
// can select one without reshaping the header.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildChannel {
    Development,
    Nightly,
    Beta,
    Release,
}

impl BuildChannel {
    fn label(self) -> &'static str {
        match self {
            BuildChannel::Development => "Development",
            BuildChannel::Nightly => "Nightly",
            BuildChannel::Beta => "Beta",
            BuildChannel::Release => "Release",
        }
    }
}

// "Development (0.1.0)"
fn version_label(channel: BuildChannel, version: &str) -> String {
    format!("{} ({version})", channel.label())
}

fn platform_name(os: &str, arch: &str) -> String {
    let os = match os {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };

    // Apple calls 64-bit ARM "arm64"; Rust calls it "aarch64".
    let arch = match arch {
        "aarch64" => "arm64",
        other => other,
    };

    format!("{os} {arch}")
}

// Splits after each separator, keeping the separator with the text before it.
fn split_after<'a>(text: &'a str, separators: &[char]) -> Vec<&'a str> {
    let mut pieces = Vec::new();
    let mut start = 0;

    for (index, character) in text.char_indices() {
        if separators.contains(&character) {
            let end = index + character.len_utf8();

            pieces.push(&text[start..end]);

            start = end;
        }
    }

    if start < text.len() {
        pieces.push(&text[start..]);
    }

    pieces
}

// Wraps a header value to `width` columns. Breaks after path separators where
// there are any (macOS or Windows), otherwise at spaces; a single piece that
// is still too long is broken at its spaces, and only then mid-word. Nothing is
// inserted: joining the lines gives back the value (less trailing spaces).
fn wrap_value(value: &str, width: usize) -> Vec<String> {
    let length = |text: &str| text.chars().count();

    let separators: &[char] = if value.contains(['/', '\\']) {
        &['/', '\\']
    } else {
        &[' ']
    };

    let mut pieces = Vec::new();

    for piece in split_after(value, separators) {
        if length(piece.trim_end()) > width {
            pieces.extend(split_after(piece, &[' ']));
        } else {
            pieces.push(piece);
        }
    }

    let mut lines = Vec::new();
    let mut line = String::new();

    for piece in pieces {
        if !line.is_empty() && length(&line) + length(piece.trim_end()) > width {
            lines.push(line.trim_end().to_string());

            line = String::new();
        }

        line.push_str(piece);

        while length(line.trim_end()) > width {
            lines.push(line.chars().take(width).collect());

            line = line.chars().skip(width).collect();
        }
    }

    if !line.trim_end().is_empty() || lines.is_empty() {
        lines.push(line.trim_end().to_string());
    }

    lines
}

// The structure, including where long values wrap, is identical with and
// without colour; colour only wraps the same text.
fn format_header(colour: bool, application: &str, rows: &[(&str, String)]) -> String {
    let paint = |style: &str, text: &str| {
        if colour {
            format!("{style}{text}{RESET}")
        } else {
            text.to_string()
        }
    };

    let mut header = String::new();

    header.push('\n');
    header.push_str(&paint(HEADER_RULE_COLOUR, HEADER_RULE));
    header.push('\n');
    header.push_str(&paint(HEADER_TITLE_COLOUR, &application.to_uppercase()));
    header.push('\n');

    for (label, value) in rows {
        let lines = wrap_value(value, HEADER_WIDTH - HEADER_LABEL_WIDTH);

        for (index, line) in lines.iter().enumerate() {
            // Continuation lines start in the value column.
            let label = if index == 0 { label } else { "" };

            header.push_str(&paint(DIM, &format!("{label:<HEADER_LABEL_WIDTH$}")));
            header.push_str(&paint(HEADER_VALUE_COLOUR, line));
            header.push('\n');
        }
    }

    header.push_str(&paint(HEADER_RULE_COLOUR, HEADER_RULE));
    header.push_str("\n\n");

    header
}

// `version` is the channel and semantic version, e.g. "Development (0.1.0)".
// `logging` is the level of the sink the header is written to.
fn header_rows(
    version: &str,
    platform: &str,
    logging: String,
    protocol: &str,
    session_id: &str,
    log_directory: Option<&Path>,
) -> Vec<(&'static str, String)> {
    vec![
        ("Version:", version.to_string()),
        ("Platform:", platform.to_string()),
        ("Logging:", logging),
        ("Protocol:", protocol.to_string()),
        ("Session:", session_id.to_string()),
        (
            "Logs:",
            match log_directory {
                Some(directory) => directory.display().to_string(),
                None => "unavailable".to_string(),
            },
        ),
    ]
}

// -----------------------------------------------------------------------------
// Session log files
//
// One plain-text file per launch. Two naming models:
//
//   Timestamped   pgc-host-YYYY-MM-DDTHH-MM-SSZ.log
//                 The file is created with its final name.
//
//   Latest        pgc-client-latest.log
//                 The running session always writes here. At the next launch
//                 the previous file is renamed to its archive name,
//                 pgc-client-YYYY-MM-DDTHH-MM-SS.sssZ.log, taken from the
//                 session ID in its own header.
//
// In both models the UTC start time is the session ID, and only the newest few
// timestamped files of the application are kept. Rotation only ever touches
// files whose names match the exact pattern, so a log the user renames or
// copies is never rotated, renamed, or deleted.
// -----------------------------------------------------------------------------

const RETAINED_SESSION_LOGS: usize = 5;

const LOG_FOLDER_NAME: &str = "Portable Game Caster";

// Development / support override for where session logs are written.
const LOG_DIRECTORY_ENV: &str = "PGC_LOG_DIR";

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionNaming {
    Timestamped,
    Latest,
}

// 'd' is any digit; every other character must match exactly.
const SECONDS_ID_SHAPE: &str = "dddd-dd-ddTdd:dd:ddZ";
const MILLIS_ID_SHAPE: &str = "dddd-dd-ddTdd:dd:dd.dddZ";

impl SessionNaming {
    fn id_shape(self) -> &'static str {
        match self {
            SessionNaming::Timestamped => SECONDS_ID_SHAPE,
            SessionNaming::Latest => MILLIS_ID_SHAPE,
        }
    }
}

fn matches_shape(text: &str, shape: &str) -> bool {
    text.len() == shape.len()
        && text
            .bytes()
            .zip(shape.bytes())
            .all(|(byte, shape)| match shape {
                b'd' => byte.is_ascii_digit(),
                literal => byte == literal,
            })
}

// Set once the session file is open.
static LOG_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

// The directory holding this session's log file, if one could be created.
pub fn log_directory() -> Option<PathBuf> {
    LOG_DIRECTORY.get().cloned()
}

fn default_log_directory() -> Option<PathBuf> {
    if let Some(directory) = std::env::var_os(LOG_DIRECTORY_ENV) {
        if !directory.is_empty() {
            return Some(PathBuf::from(directory));
        }
    }

    platform_log_directory()
}

// %LOCALAPPDATA%\Portable Game Caster\Logs
#[cfg(windows)]
fn platform_log_directory() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;

    Some(PathBuf::from(base).join(LOG_FOLDER_NAME).join("Logs"))
}

// ~/Library/Logs/Portable Game Caster
#[cfg(target_os = "macos")]
fn platform_log_directory() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;

    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Logs")
            .join(LOG_FOLDER_NAME),
    )
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_log_directory() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;

    Some(
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join(LOG_FOLDER_NAME)
            .join("logs"),
    )
}

// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    (year, month, day)
}

// Timestamped: "2026-10-05T01:56:37Z"
// Latest:      "2026-10-05T01:56:37.123Z" (milliseconds, so that two sessions
//              can never share an archive name)
fn format_session_id(unix_millis: u128, naming: SessionNaming) -> String {
    let unix_seconds = (unix_millis / 1000) as u64;

    let (year, month, day) = civil_from_days((unix_seconds / 86_400) as i64);
    let second_of_day = unix_seconds % 86_400;

    let date_time = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        second_of_day / 3600,
        second_of_day / 60 % 60,
        second_of_day % 60,
    );

    match naming {
        SessionNaming::Timestamped => format!("{date_time}Z"),
        SessionNaming::Latest => format!("{date_time}.{:03}Z", unix_millis % 1000),
    }
}

// "pgc-client-2026-10-05T01-56-37.123Z.log" (no colons: not valid in Windows
// file names).
fn session_file_name(application: &str, session_id: &str) -> String {
    format!("pgc-{application}-{}.log", session_id.replace(':', "-"))
}

fn latest_file_name(application: &str) -> String {
    format!("pgc-{application}-latest.log")
}

// Exactly the shape session_file_name produces for this application and naming
// model, so that rotation can never touch anything else in the directory.
fn is_session_log(application: &str, naming: SessionNaming, name: &str) -> bool {
    name.strip_prefix("pgc-")
        .and_then(|rest| rest.strip_prefix(application))
        .and_then(|rest| rest.strip_prefix('-'))
        .and_then(|rest| rest.strip_suffix(".log"))
        .is_some_and(|stamp| matches_shape(stamp, &naming.id_shape().replace(':', "-")))
}

// Which of `names` to delete so that only the newest `keep` timestamped session
// logs of this application remain. Timestamped names sort chronologically.
fn logs_to_delete(
    application: &str,
    naming: SessionNaming,
    names: &[String],
    keep: usize,
) -> Vec<String> {
    let mut session_logs: Vec<&String> = names
        .iter()
        .filter(|name| is_session_log(application, naming, name))
        .collect();

    session_logs.sort();
    session_logs.reverse();

    session_logs.into_iter().skip(keep).cloned().collect()
}

// Returns how many old session logs were deleted.
fn rotate_session_logs(
    directory: &Path,
    application: &str,
    naming: SessionNaming,
    keep: usize,
) -> usize {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };

    let names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();

    logs_to_delete(application, naming, &names, keep)
        .iter()
        .filter(|name| fs::remove_file(directory.join(name)).is_ok())
        .count()
}

// The session ID a log file recorded in its own startup header.
fn session_id_in_header(text: &str) -> Option<String> {
    text.lines()
        .take(16)
        .filter_map(|line| line.strip_prefix("Session:"))
        .map(str::trim)
        .find(|id| matches_shape(id, MILLIS_ID_SHAPE))
        .map(str::to_string)
}

fn unix_millis_of(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

// Renames a previous session's `latest` file to its archive name. Returns the
// archive file name, or None when there was nothing to archive.
//
// The archive name comes from the session ID in the file's own header. If the
// header cannot be read, the file's modification time is used instead, so the
// previous session is still kept rather than overwritten.
fn archive_latest(directory: &Path, application: &str) -> io::Result<Option<String>> {
    let latest = directory.join(latest_file_name(application));

    let metadata = match fs::metadata(&latest) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    let mut start = [0_u8; 2048];

    let read = File::open(&latest)
        .and_then(|mut file| file.read(&mut start))
        .unwrap_or(0);

    let modified = metadata
        .modified()
        .map(unix_millis_of)
        .unwrap_or_else(|_| unix_millis_of(SystemTime::now()));

    let recorded = session_id_in_header(&String::from_utf8_lossy(&start[..read]));

    // Never replace an existing archive: step forward a millisecond at a time
    // from the modification time until the name is free.
    let candidates = recorded
        .into_iter()
        .chain((0..1000).map(|offset| format_session_id(modified + offset, SessionNaming::Latest)));

    for session_id in candidates {
        let name = session_file_name(application, &session_id);
        let archive = directory.join(&name);

        if !archive.exists() {
            fs::rename(&latest, &archive)?;

            return Ok(Some(name));
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free archive name for the previous session log",
    ))
}

struct OpenedSession {
    file: File,

    // Latest model: the previous session's archive name, if one was archived.
    archived: Option<String>,

    // Latest model: why the previous session could not be archived.
    archive_error: Option<io::Error>,
}

fn open_session_file(
    directory: &Path,
    application: &str,
    naming: SessionNaming,
    session_id: &str,
) -> io::Result<OpenedSession> {
    fs::create_dir_all(directory)?;

    let (name, archived, archive_error) = match naming {
        SessionNaming::Timestamped => (session_file_name(application, session_id), None, None),

        SessionNaming::Latest => match archive_latest(directory, application) {
            Ok(archived) => (latest_file_name(application), archived, None),
            Err(error) => (latest_file_name(application), None, Some(error)),
        },
    };

    // Append, never truncate: if the previous `latest` could not be archived,
    // this session is added after it instead of destroying it.
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join(name))?;

    Ok(OpenedSession {
        file,
        archived,
        archive_error,
    })
}

// Opens this launch's session log, rotates old ones, and writes the startup
// header to the terminal and to the file. `application` is "client" or "host".
// Call once, after the terminal level is set.
pub fn start_session(
    application: &str,
    title: &str,
    channel: BuildChannel,
    version: &str,
    protocol: &str,
    naming: SessionNaming,
) {
    let session_id = format_session_id(unix_millis_of(SystemTime::now()), naming);

    let directory = default_log_directory();

    let opened = match &directory {
        Some(directory) => open_session_file(directory, application, naming, &session_id),

        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no log directory is available for this user",
        )),
    };

    let mut failure = None;
    let mut archived = None;
    let mut archive_error = None;
    let mut removed = 0;

    match (opened, &directory) {
        (Ok(session), Some(directory)) => {
            if let Ok(mut slot) = LOG_FILE.lock() {
                *slot = Some(LineWriter::new(session.file));
            }

            FILE_LEVEL.store(file_level_for(level()) as u8, Ordering::Relaxed);

            let _ = LOG_DIRECTORY.set(directory.clone());

            archived = session.archived;
            archive_error = session.archive_error;

            removed = rotate_session_logs(directory, application, naming, RETAINED_SESSION_LOGS);
        }

        (Err(error), _) => failure = Some(error),

        (Ok(_), None) => {}
    }

    let version = version_label(channel, version);
    let platform = platform_name(std::env::consts::OS, std::env::consts::ARCH);
    let log_directory = log_directory();

    if should_log(level(), LogLevel::Normal) {
        write_terminal(&format_header(
            COLOUR.load(Ordering::Relaxed),
            title,
            &header_rows(
                &version,
                &platform,
                format!("{:?}", level()),
                protocol,
                &session_id,
                log_directory.as_deref(),
            ),
        ));
    }

    if file_level() != LogLevel::Quiet {
        write_file(&format_header(
            false,
            title,
            &header_rows(
                &version,
                &platform,
                file_logging_label(file_level()),
                protocol,
                &session_id,
                log_directory.as_deref(),
            ),
        ));
    }

    if let Some(error) = failure {
        warn(
            "APP",
            format_args!("Session log file could not be created: {error}"),
        );
    }

    if let Some(error) = archive_error {
        warn(
            "APP",
            format_args!(
                "Previous session log could not be archived ({error}); this session is appended to it."
            ),
        );
    }

    if let Some(name) = archived {
        debug(
            "APP",
            format_args!("Archived previous session log as {name}."),
        );
    }

    if removed > 0 {
        debug(
            "APP",
            format_args!(
                "Removed {removed} old session log(s); keeping the newest {RETAINED_SESSION_LOGS}."
            ),
        );
    }
}

// What the file's own header says about its level.
fn file_logging_label(file: LogLevel) -> String {
    format!("{file:?} (file)")
}

// Flushes and closes the session file. Lines logged afterwards go to the
// terminal only.
pub fn shutdown() {
    FILE_LEVEL.store(LogLevel::Quiet as u8, Ordering::Relaxed);

    if let Ok(mut slot) = LOG_FILE.lock() {
        if let Some(mut file) = slot.take() {
            let _ = file.flush();
        }
    }
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
    let sinks = current_sinks();

    if !should_log(sinks.terminal, LogLevel::Verbose) && !should_log(sinks.file, LogLevel::Verbose)
    {
        return;
    }

    deliver(render(
        sinks,
        LogLevel::Verbose,
        &utc_timestamp(),
        source,
        None,
        Severity::Info,
        &decorate_raw(source, line, sinks.terminal_colour),
        line,
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

    start_session(
        "client",
        "Portable Game Caster Client",
        crate::BUILD_CHANNEL,
        env!("CARGO_PKG_VERSION"),
        crate::PROTOCOL_VERSION,
        SessionNaming::Latest,
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

// Product diagnostics, not Rust serialisation: the state name, then what it
// means in words. A countdown is only mentioned while the UI is showing one.
fn state_text(state: &AppState) -> String {
    let remaining = |seconds: &u8| format!("{seconds}s remaining.");

    match state {
        AppState::Idle => "Idle | Ready to search.".to_string(),

        AppState::Discovering {
            seconds_remaining: None,
        } => "Discovering | Searching for a Portable Game Caster host…".to_string(),

        AppState::Discovering {
            seconds_remaining: Some(seconds),
        } => format!(
            "Discovering | Searching for a Portable Game Caster host… {}",
            remaining(seconds)
        ),

        AppState::Resolving(host) => format!("Resolving | Host found: {host}."),

        AppState::Connecting(host) => {
            format!("Connecting | Connecting to streaming service on {host}…")
        }

        AppState::WaitingForStream {
            host,
            fallback_seconds_remaining: None,
        } => format!("WaitingForStream | Waiting for stream from {host}."),

        AppState::WaitingForStream {
            fallback_seconds_remaining: Some(seconds),
            ..
        } => format!(
            "WaitingForStream | Stream is taking longer than expected. {}",
            remaining(seconds)
        ),

        AppState::Playing(host) => format!("Playing | Connected to {host}."),

        AppState::ReconnectingStream {
            host,
            seconds_remaining,
        } => format!(
            "ReconnectingStream | Connection to stream lost. Reconnecting to {host}… {}",
            remaining(seconds_remaining)
        ),

        AppState::ReconnectingHost {
            host,
            seconds_remaining,
        } => format!(
            "ReconnectingHost | Connection to host lost. Looking for {host}… {}",
            remaining(seconds_remaining)
        ),

        AppState::Error(message) => format!("Error | {message}"),
    }
}

pub fn state(state: &AppState) {
    log(
        state_level(state),
        "STATE",
        Severity::Info,
        format_args!("{}", state_text(state)),
    );

    // The exact internal value, for excavation.
    verbose("STATE", format_args!("Internal state: {state:?}"));
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
        assert_eq!(category_colour("HEALTH"), "\x1b[33m");
        assert_eq!(category_colour("STREAM"), "\x1b[38;5;147m");
        assert_eq!(category_colour("DEMAND"), "\x1b[38;5;208m");
    }

    const PLAIN_HEADER: &str = "\n\
        ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
        PORTABLE GAME CASTER HOST\n\
        Version:     Development (0.1.0)\n\
        Platform:    Windows x86_64\n\
        Logging:     Normal\n\
        Protocol:    1\n\
        Session:     2026-10-05T01:56:37Z\n\
        Logs:        C:\\Logs\n\
        ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
        \n";

    fn test_header(colour: bool) -> String {
        format_header(
            colour,
            "Portable Game Caster Host",
            &header_rows(
                &version_label(BuildChannel::Development, "0.1.0"),
                "Windows x86_64",
                "Normal".to_string(),
                "1",
                "2026-10-05T01:56:37Z",
                Some(Path::new("C:\\Logs")),
            ),
        )
    }

    #[test]
    fn plain_header_is_enclosed_and_has_no_escape_codes() {
        let header = test_header(false);

        assert_eq!(header, PLAIN_HEADER);

        // Not a timestamped log line, and no styling.
        assert!(!header.contains('['));
        assert!(!header.contains('\x1b'));

        // One blank line before the top rule and one after the bottom rule.
        let lines: Vec<&str> = header.split('\n').collect();

        assert_eq!(lines.first(), Some(&""));
        assert_eq!(lines[1], HEADER_RULE);
        assert_eq!(lines[9], HEADER_RULE);
        assert_eq!(&lines[10..], ["", ""]);
        assert_eq!(HEADER_RULE.chars().count(), HEADER_WIDTH);
    }

    #[test]
    fn styled_header_has_the_same_structure_as_plain() {
        let header = test_header(true);

        assert_eq!(strip_ansi(&header), PLAIN_HEADER);

        // Bold bright title, dimmer rules in the same family, dim labels,
        // bright values, and nothing left switched on.
        assert!(header.contains("\x1b[1;38;5;80mPORTABLE GAME CASTER HOST\x1b[0m"));
        assert!(header.contains(&format!("\x1b[38;5;37m{HEADER_RULE}\x1b[0m")));
        assert!(header.contains("\x1b[90mVersion:     \x1b[0m\x1b[97mDevelopment (0.1.0)\x1b[0m"));
        assert!(header.trim_end().ends_with("\x1b[0m"));

        // No background fill.
        assert!(!header.contains("\x1b[4"));
    }

    #[test]
    fn header_values_share_one_column() {
        let header = test_header(false);

        for label in [
            "Version:",
            "Platform:",
            "Logging:",
            "Protocol:",
            "Session:",
            "Logs:",
        ] {
            let line = header.lines().find(|line| line.starts_with(label)).unwrap();

            let column = line.len() - line[label.len()..].trim_start().len();

            assert_eq!(column, 13, "{label}");
        }
    }

    #[test]
    fn version_shows_channel_and_semantic_version() {
        assert_eq!(
            version_label(BuildChannel::Development, "0.1.0"),
            "Development (0.1.0)"
        );

        assert_eq!(
            version_label(BuildChannel::Nightly, "0.7.0"),
            "Nightly (0.7.0)"
        );
        assert_eq!(version_label(BuildChannel::Beta, "0.7.0"), "Beta (0.7.0)");
        assert_eq!(
            version_label(BuildChannel::Release, "1.0.0"),
            "Release (1.0.0)"
        );
    }

    #[test]
    fn header_identifies_session_and_missing_log_directory() {
        let rows = header_rows(
            "Development (0.1.0)",
            "macOS arm64",
            file_logging_label(LogLevel::Debug),
            "1",
            "2026-10-05T01:56:37.123Z",
            None,
        );

        let header = format_header(false, "Portable Game Caster Client", &rows);

        assert!(header.contains("Session:     2026-10-05T01:56:37.123Z\n"));
        assert!(header.contains("Logging:     Debug (file)\n"));
        assert!(header.contains("Logs:        unavailable\n"));
    }

    // -------------------------------------------------------------------------
    // Header wrapping
    // -------------------------------------------------------------------------

    #[test]
    fn long_paths_wrap_at_separators_within_the_header_width() {
        assert_eq!(
            wrap_value("/Users/justinkhan/Library/Logs/Portable Game Caster", 27),
            ["/Users/justinkhan/Library/", "Logs/Portable Game Caster"]
        );

        assert_eq!(
            wrap_value(
                "C:\\Users\\Justin\\AppData\\Local\\Portable Game Caster\\Logs",
                27
            ),
            [
                "C:\\Users\\Justin\\AppData\\",
                "Local\\Portable Game Caster\\",
                "Logs"
            ]
        );

        // Short values are untouched.
        assert_eq!(
            wrap_value("Development (0.1.0)", 27),
            ["Development (0.1.0)"]
        );
        assert_eq!(wrap_value("", 27), [""]);
    }

    #[test]
    fn wrapping_never_adds_or_drops_path_characters() {
        for path in [
            "/Users/justinkhan/Library/Logs/Portable Game Caster",
            "C:\\Users\\Justin\\AppData\\Local\\Portable Game Caster\\Logs",
            "/Users/a-very-long-user-name-that-is-wider-than-the-column/Library/Logs",
            "/Volumes/External Drive With Spaces In Its Name/Logs/Portable Game Caster",
        ] {
            let lines = wrap_value(path, 27);

            assert!(
                lines.iter().all(|line| line.chars().count() <= 27),
                "{lines:?}"
            );

            // Only spaces at a break may be dropped; nothing is inserted.
            let rejoined: String = lines.concat();

            assert_eq!(
                rejoined.replace(' ', ""),
                path.replace(' ', ""),
                "{lines:?}"
            );
        }
    }

    #[test]
    fn wrapped_header_lines_align_and_match_when_styled() {
        let rows = header_rows(
            "Development (0.1.0)",
            "macOS arm64",
            "Debug".to_string(),
            "1",
            "2026-10-05T01:56:37.123Z",
            Some(Path::new(
                "/Users/justinkhan/Library/Logs/Portable Game Caster",
            )),
        );

        let plain = format_header(false, "Portable Game Caster Client", &rows);

        assert!(plain.contains(
            "Logs:        /Users/justinkhan/Library/Logs/\n             Portable Game Caster\n"
        ));

        assert_eq!(HEADER_WIDTH, 50);

        // Nothing is wider than the rules.
        assert!(
            plain
                .lines()
                .all(|line| line.chars().count() <= HEADER_WIDTH)
        );

        assert_eq!(
            strip_ansi(&format_header(true, "Portable Game Caster Client", &rows)),
            plain
        );
    }

    // -------------------------------------------------------------------------
    // Session files
    // -------------------------------------------------------------------------

    #[test]
    fn session_id_is_utc_date_and_time() {
        let seconds =
            |unix_seconds: u128| format_session_id(unix_seconds * 1000, SessionNaming::Timestamped);

        assert_eq!(seconds(0), "1970-01-01T00:00:00Z");
        assert_eq!(seconds(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(seconds(1_791_165_397), "2026-10-05T01:56:37Z");
        assert_eq!(seconds(1_798_761_599), "2026-12-31T23:59:59Z");
        assert_eq!(seconds(1_798_761_600), "2027-01-01T00:00:00Z");

        assert_eq!(
            format_session_id(1_791_165_397_045, SessionNaming::Latest),
            "2026-10-05T01:56:37.045Z"
        );
    }

    #[test]
    fn session_file_names_carry_application_and_timestamp() {
        assert_eq!(
            session_file_name("host", "2026-10-05T01:56:37Z"),
            "pgc-host-2026-10-05T01-56-37Z.log"
        );

        // Client archives carry milliseconds.
        assert_eq!(
            session_file_name("client", "2026-10-05T01:56:37.045Z"),
            "pgc-client-2026-10-05T01-56-37.045Z.log"
        );

        assert_eq!(latest_file_name("client"), "pgc-client-latest.log");
    }

    #[test]
    fn recognises_only_exact_archive_names() {
        let latest = SessionNaming::Latest;

        assert!(is_session_log(
            "client",
            latest,
            "pgc-client-2026-10-05T01-56-37.045Z.log"
        ));

        for other in [
            // The live file is never an archive.
            "pgc-client-latest.log",
            // Renamed or copied by the user.
            "pgc-client-2026-10-05T01-56-37.045Z copy.log",
            "pgc-client-2026-10-05T01-56-37.045Z.log.bak",
            "pgc-client-2026-10-05T01-56-37.045Z.txt",
            "keep-pgc-client-2026-10-05T01-56-37.045Z.log",
            "pgc-client-bug-report.log",
            // Second-precision names from the previous naming model.
            "pgc-client-2026-10-05T01-56-37Z.log",
            // Another application.
            "pgc-host-2026-10-05T01-56-37.045Z.log",
            "pgc-clientx-2026-10-05T01-56-37.045Z.log",
            "notes.txt",
        ] {
            assert!(!is_session_log("client", latest, other), "{other}");
        }

        // The Host keeps second-precision names.
        assert!(is_session_log(
            "host",
            SessionNaming::Timestamped,
            "pgc-host-2026-10-05T01-56-37Z.log"
        ));

        assert!(!is_session_log(
            "host",
            SessionNaming::Timestamped,
            "pgc-host-2026-10-05T01-56-37.045Z.log"
        ));
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn rotation_keeps_newest_five_of_this_application() {
        let directory = names(&[
            "pgc-host-2026-10-01T00-00-00Z.log",
            "pgc-host-2026-10-07T00-00-00Z.log",
            "pgc-host-2026-10-03T00-00-00Z.log",
            "pgc-host-2026-10-02T00-00-00Z.log",
            "pgc-host-2026-10-06T00-00-00Z.log",
            "pgc-host-2026-10-05T00-00-00Z.log",
            "pgc-host-2026-10-04T00-00-00Z.log",
            "pgc-client-2026-09-01T00-00-00.000Z.log",
            "notes.txt",
            "pgc-host-notes.log",
        ]);

        let mut deleted = logs_to_delete(
            "host",
            SessionNaming::Timestamped,
            &directory,
            RETAINED_SESSION_LOGS,
        );

        deleted.sort();

        assert_eq!(
            deleted,
            names(&[
                "pgc-host-2026-10-01T00-00-00Z.log",
                "pgc-host-2026-10-02T00-00-00Z.log",
            ])
        );

        // Five or fewer: nothing is deleted.
        assert!(
            logs_to_delete(
                "client",
                SessionNaming::Latest,
                &directory,
                RETAINED_SESSION_LOGS
            )
            .is_empty()
        );
    }

    fn test_directory(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("pgc-logging-test-{name}-{}", std::process::id()));

        let _ = fs::remove_dir_all(&directory);

        fs::create_dir_all(&directory).unwrap();

        directory
    }

    fn listing(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(directory)
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();

        names.sort();

        names
    }

    #[test]
    fn timestamped_rotation_on_disk_leaves_unrelated_files_untouched() {
        let directory = test_directory("timestamped");
        let naming = SessionNaming::Timestamped;

        // Seven existing sessions, then this launch's file.
        for day in 1..=7 {
            let id = format!("2026-10-{day:02}T00:00:00Z");

            open_session_file(&directory, "host", naming, &id).unwrap();
        }

        for unrelated in ["pgc-client-latest.log", "notes.txt", "pgc-host-notes.log"] {
            fs::write(directory.join(unrelated), "keep me").unwrap();
        }

        fs::create_dir(directory.join("pgc-host-2026-01-01T00-00-00Z.log")).unwrap();

        open_session_file(&directory, "host", naming, "2026-10-08T00:00:00Z").unwrap();

        assert_eq!(
            rotate_session_logs(&directory, "host", naming, RETAINED_SESSION_LOGS),
            3
        );

        assert_eq!(
            listing(&directory),
            names(&[
                "notes.txt",
                "pgc-client-latest.log",
                // A directory with a matching name is not a session log.
                "pgc-host-2026-01-01T00-00-00Z.log",
                "pgc-host-2026-10-04T00-00-00Z.log",
                "pgc-host-2026-10-05T00-00-00Z.log",
                "pgc-host-2026-10-06T00-00-00Z.log",
                "pgc-host-2026-10-07T00-00-00Z.log",
                "pgc-host-2026-10-08T00-00-00Z.log",
                "pgc-host-notes.log",
            ])
        );

        fs::remove_dir_all(&directory).unwrap();
    }

    // Writes what a session would have written to its `latest` file.
    fn write_latest(directory: &Path, session_id: &str, body: &str) {
        let header = format_header(
            false,
            "Portable Game Caster Client",
            &header_rows(
                "Development (0.1.0)",
                "macOS arm64",
                file_logging_label(LogLevel::Debug),
                "1",
                session_id,
                Some(directory),
            ),
        );

        fs::write(
            directory.join(latest_file_name("client")),
            format!("{header}{body}"),
        )
        .unwrap();
    }

    #[test]
    fn latest_is_archived_under_its_own_session_id_at_next_launch() {
        let directory = test_directory("latest");
        let naming = SessionNaming::Latest;

        // First launch: nothing to archive.
        let first =
            open_session_file(&directory, "client", naming, "2026-10-05T01:00:00.111Z").unwrap();

        assert_eq!(first.archived, None);

        drop(first);

        write_latest(&directory, "2026-10-05T01:00:00.111Z", "first session\n");

        // Second launch: the first session is archived under its own ID, and
        // the new session starts an empty `latest`.
        let second =
            open_session_file(&directory, "client", naming, "2026-10-05T02:00:00.222Z").unwrap();

        assert_eq!(
            second.archived.as_deref(),
            Some("pgc-client-2026-10-05T01-00-00.111Z.log")
        );

        assert!(second.archive_error.is_none());

        assert_eq!(
            listing(&directory),
            names(&[
                "pgc-client-2026-10-05T01-00-00.111Z.log",
                "pgc-client-latest.log",
            ])
        );

        assert!(
            fs::read_to_string(directory.join("pgc-client-2026-10-05T01-00-00.111Z.log"))
                .unwrap()
                .ends_with("first session\n")
        );

        assert_eq!(
            fs::read_to_string(directory.join("pgc-client-latest.log")).unwrap(),
            ""
        );

        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn latest_without_a_readable_header_is_archived_not_overwritten() {
        let directory = test_directory("latest-unreadable");

        fs::write(directory.join("pgc-client-latest.log"), "no header here\n").unwrap();

        let archived = archive_latest(&directory, "client").unwrap().unwrap();

        assert!(is_session_log("client", SessionNaming::Latest, &archived));

        assert_eq!(
            fs::read_to_string(directory.join(&archived)).unwrap(),
            "no header here\n"
        );

        assert!(!directory.join("pgc-client-latest.log").exists());

        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn archiving_never_replaces_an_existing_archive() {
        let directory = test_directory("latest-collision");

        fs::write(
            directory.join("pgc-client-2026-10-05T01-00-00.111Z.log"),
            "existing archive\n",
        )
        .unwrap();

        write_latest(&directory, "2026-10-05T01:00:00.111Z", "same id again\n");

        let archived = archive_latest(&directory, "client").unwrap().unwrap();

        assert_ne!(archived, "pgc-client-2026-10-05T01-00-00.111Z.log");
        assert!(is_session_log("client", SessionNaming::Latest, &archived));

        assert_eq!(
            fs::read_to_string(directory.join("pgc-client-2026-10-05T01-00-00.111Z.log")).unwrap(),
            "existing archive\n"
        );

        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn latest_rotation_keeps_five_archives_and_protects_renamed_logs() {
        let directory = test_directory("latest-rotation");
        let naming = SessionNaming::Latest;

        // Eight sessions in a row.
        for hour in 1..=8 {
            let id = format!("2026-10-05T{hour:02}:00:00.000Z");

            open_session_file(&directory, "client", naming, &id).unwrap();

            write_latest(&directory, &id, "session\n");

            // The user preserves the third session by renaming a copy of it,
            // and keeps some files of their own.
            if hour == 3 {
                fs::copy(
                    directory.join("pgc-client-latest.log"),
                    directory.join("pgc-client-2026-10-05T03-00-00.000Z bug report.log"),
                )
                .unwrap();

                fs::write(directory.join("notes.txt"), "keep me").unwrap();

                fs::write(
                    directory.join("pgc-client-2026-10-05T01-56-37Z.log"),
                    "old naming model",
                )
                .unwrap();
            }

            rotate_session_logs(&directory, "client", naming, RETAINED_SESSION_LOGS);
        }

        // A ninth launch archives session 8 and rotates.
        open_session_file(&directory, "client", naming, "2026-10-05T09:00:00.000Z").unwrap();

        rotate_session_logs(&directory, "client", naming, RETAINED_SESSION_LOGS);

        assert_eq!(
            listing(&directory),
            names(&[
                "notes.txt",
                // Not an exact archive name: never rotated.
                "pgc-client-2026-10-05T01-56-37Z.log",
                "pgc-client-2026-10-05T03-00-00.000Z bug report.log",
                // The newest five archives.
                "pgc-client-2026-10-05T04-00-00.000Z.log",
                "pgc-client-2026-10-05T05-00-00.000Z.log",
                "pgc-client-2026-10-05T06-00-00.000Z.log",
                "pgc-client-2026-10-05T07-00-00.000Z.log",
                "pgc-client-2026-10-05T08-00-00.000Z.log",
                // The running session.
                "pgc-client-latest.log",
            ])
        );

        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn reads_session_id_from_a_log_header() {
        assert_eq!(
            session_id_in_header(
                "\n━━━━\nPORTABLE GAME CASTER CLIENT\nVersion:     Development (0.1.0)\nSession:     2026-10-05T01:56:37.045Z\nLogs:        /tmp\n"
            )
            .as_deref(),
            Some("2026-10-05T01:56:37.045Z")
        );

        // Not a session ID, or too far into the file to be the header.
        assert_eq!(session_id_in_header("Session:     yesterday\n"), None);

        assert_eq!(
            session_id_in_header(&format!(
                "{}Session:     2026-10-05T01:56:37.045Z\n",
                "line\n".repeat(40)
            )),
            None
        );
    }

    #[test]
    fn file_level_policy() {
        assert_eq!(file_level_for(LogLevel::Quiet), LogLevel::Debug);
        assert_eq!(file_level_for(LogLevel::Normal), LogLevel::Debug);
        assert_eq!(file_level_for(LogLevel::Debug), LogLevel::Debug);
        assert_eq!(file_level_for(LogLevel::Verbose), LogLevel::Verbose);
    }

    // -------------------------------------------------------------------------
    // Sinks
    // -------------------------------------------------------------------------

    fn sinks(terminal: LogLevel, terminal_colour: bool) -> Sinks {
        Sinks {
            terminal,
            terminal_colour,
            file: file_level_for(terminal),
        }
    }

    fn pgc_line(sinks: Sinks, required: LogLevel) -> Rendered {
        render(
            sinks,
            required,
            "07:50:41.098Z",
            Source::Pgc,
            Some("HEALTH"),
            Severity::Info,
            "message",
            "message",
        )
    }

    #[test]
    fn quiet_terminal_still_writes_a_debug_file() {
        let quiet = sinks(LogLevel::Quiet, false);

        for required in [LogLevel::Normal, LogLevel::Debug] {
            let rendered = pgc_line(quiet, required);

            assert_eq!(rendered.terminal, None);

            assert_eq!(
                rendered.file.as_deref(),
                Some("[07:50:41.098Z] [HEALTH]     message\n")
            );
        }

        // Verbose detail is not written unless the user asked for Verbose.
        assert_eq!(
            pgc_line(quiet, LogLevel::Verbose),
            Rendered {
                terminal: None,
                file: None
            }
        );
    }

    #[test]
    fn normal_terminal_gets_normal_lines_and_file_gets_debug_lines() {
        let normal = sinks(LogLevel::Normal, false);

        assert!(pgc_line(normal, LogLevel::Normal).terminal.is_some());
        assert!(pgc_line(normal, LogLevel::Normal).file.is_some());

        assert!(pgc_line(normal, LogLevel::Debug).terminal.is_none());
        assert!(pgc_line(normal, LogLevel::Debug).file.is_some());
    }

    #[test]
    fn verbose_file_includes_labelled_raw_source_lines() {
        let raw_line = |sinks| {
            render(
                sinks,
                LogLevel::Verbose,
                "07:50:54.337Z",
                Source::Ffmpeg,
                None,
                Severity::Info,
                "frame=  181 fps= 33",
                "frame=  181 fps= 33",
            )
        };

        let rendered = raw_line(sinks(LogLevel::Verbose, false));

        assert_eq!(
            rendered.file.as_deref(),
            Some("[07:50:54.337Z] [FFMPEG]              frame=  181 fps= 33\n")
        );

        assert_eq!(rendered.file, rendered.terminal);

        // PGC lines in a Verbose file carry their source too.
        assert_eq!(
            pgc_line(sinks(LogLevel::Verbose, false), LogLevel::Debug)
                .file
                .as_deref(),
            Some("[07:50:41.098Z] [PGC]    [HEALTH]     message\n")
        );

        // Not Verbose: no raw output in the file or on the terminal.
        assert_eq!(
            raw_line(sinks(LogLevel::Debug, false)),
            Rendered {
                terminal: None,
                file: None
            }
        );
    }

    #[test]
    fn file_sink_never_contains_escape_codes() {
        let rendered = render(
            sinks(LogLevel::Verbose, true),
            LogLevel::Verbose,
            "07:50:54.336Z",
            Source::Mtx,
            None,
            Severity::Warning,
            "2026/10/04 05:02:13 \x1b[33mWAR\x1b[0m something",
            // Even if styling leaks into the file message, it is removed.
            "2026/10/04 05:02:13 \x1b[33mWAR\x1b[0m something",
        );

        assert!(rendered.terminal.unwrap().contains('\x1b'));

        assert_eq!(
            rendered.file.as_deref(),
            Some(
                "[07:50:54.336Z] [MTX]                 WARNING: 2026/10/04 05:02:13 WAR something\n"
            )
        );
    }

    #[test]
    fn strips_ansi_sequences() {
        assert_eq!(strip_ansi("plain"), "plain");
        assert_eq!(strip_ansi("\x1b[1;38;5;80mTITLE\x1b[0m"), "TITLE");
        assert_eq!(strip_ansi("a\x1b[90mb\x1b[0mc"), "abc");
    }

    #[test]
    fn platform_names_are_explicit() {
        assert_eq!(platform_name("macos", "aarch64"), "macOS arm64");
        assert_eq!(platform_name("windows", "x86_64"), "Windows x86_64");
        assert_eq!(platform_name("linux", "x86_64"), "Linux x86_64");
    }

    #[test]
    fn state_discovery_and_connect_are_different_hues() {
        // Bright cyan, green, deep blue, and a periwinkle STREAM that is none
        // of those.
        assert_eq!(category_colour("STATE"), "\x1b[96m");
        assert_eq!(category_colour("DISCOVERY"), "\x1b[32m");
        assert_eq!(category_colour("CONNECT"), "\x1b[38;5;27m");
        assert_eq!(category_colour("STREAM"), "\x1b[38;5;147m");
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
    fn state_lines_read_as_prose() {
        let host = || "carock-pgc.local".to_string();

        let cases = [
            (AppState::Idle, "Idle | Ready to search."),
            (
                AppState::Discovering {
                    seconds_remaining: None,
                },
                "Discovering | Searching for a Portable Game Caster host…",
            ),
            (
                AppState::Discovering {
                    seconds_remaining: Some(5),
                },
                "Discovering | Searching for a Portable Game Caster host… 5s remaining.",
            ),
            (
                AppState::Resolving(host()),
                "Resolving | Host found: carock-pgc.local.",
            ),
            (
                AppState::Connecting(host()),
                "Connecting | Connecting to streaming service on carock-pgc.local…",
            ),
            (
                AppState::WaitingForStream {
                    host: host(),
                    fallback_seconds_remaining: None,
                },
                "WaitingForStream | Waiting for stream from carock-pgc.local.",
            ),
            (
                AppState::WaitingForStream {
                    host: host(),
                    fallback_seconds_remaining: Some(15),
                },
                "WaitingForStream | Stream is taking longer than expected. 15s remaining.",
            ),
            (
                AppState::Playing(host()),
                "Playing | Connected to carock-pgc.local.",
            ),
            (
                AppState::ReconnectingStream {
                    host: host(),
                    seconds_remaining: 15,
                },
                "ReconnectingStream | Connection to stream lost. Reconnecting to carock-pgc.local… 15s remaining.",
            ),
            (
                AppState::ReconnectingHost {
                    host: host(),
                    seconds_remaining: 9,
                },
                "ReconnectingHost | Connection to host lost. Looking for carock-pgc.local… 9s remaining.",
            ),
            (
                AppState::Error("Stream did not start.".into()),
                "Error | Stream did not start.",
            ),
        ];

        for (state, expected) in cases {
            let text = state_text(&state);

            assert_eq!(text, expected);

            // No Rust debug formatting leaks through.
            for leak in ["None", "Some(", "{", "}", "(\"", "seconds_remaining"] {
                assert!(!text.contains(leak), "{text}");
            }
        }
    }

    #[test]
    fn no_countdown_is_mentioned_while_the_ui_hides_it() {
        for state in [
            AppState::Discovering {
                seconds_remaining: None,
            },
            AppState::WaitingForStream {
                host: "host".into(),
                fallback_seconds_remaining: None,
            },
        ] {
            assert!(!state_text(&state).contains("remaining"));
        }
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
