// -----------------------------------------------------------------------------
// Host diagnostics
// -----------------------------------------------------------------------------
//
// Timestamped in UTC with millisecond precision so Host, MediaMTX, script, and
// Client logs can be lined up on one timeline.

use std::fmt;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};


static COLOUR: AtomicBool = AtomicBool::new(false);

const RESET: &str = "\x1b[0m";
const GRAY: &str = "\x1b[90m";
const CYAN: &str = "\x1b[36m";

// Child output arrives through a pipe, so MediaMTX does not colour its own
// level tags (it only does that when writing to a terminal, and has no option
// to force it). Restore that here.
const LEVEL_COLOURS: [(&str, &str); 4] = [
    (" INF ", "\x1b[32m"),
    (" WAR ", "\x1b[33m"),
    (" ERR ", "\x1b[31m"),
    (" DEB ", "\x1b[90m"),
];


pub fn set_colour(enabled: bool) {
    COLOUR.store(enabled, Ordering::Relaxed);
}


pub fn host(args: fmt::Arguments<'_>) {
    if COLOUR.load(Ordering::Relaxed) {
        eprintln!(
            "{CYAN}[{}][PGC][HOST]{RESET} {args}",
            utc_timestamp()
        );
    } else {
        eprintln!(
            "[{}][PGC][HOST] {args}",
            utc_timestamp()
        );
    }
}


// Relays a child process's output to the Host console, prefixing each complete
// line with a UTC timestamp and `tag`. `observe` sees every complete line.
pub fn relay_output<R>(
    source: R,
    to_stderr: bool,
    tag: &'static str,
    observe: impl Fn(&str) + Send + 'static,
)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let colour =
            COLOUR.load(Ordering::Relaxed);

        relay(source, tag, colour, observe, |chunk| {
            if to_stderr {
                let mut stream = io::stderr().lock();
                let _ = stream.write_all(chunk);
                let _ = stream.flush();
            } else {
                let mut stream = io::stdout().lock();
                let _ = stream.write_all(chunk);
                let _ = stream.flush();
            }
        });
    });
}


fn relay<R: Read>(
    mut source: R,
    tag: &str,
    colour: bool,
    observe: impl Fn(&str),
    mut write: impl FnMut(&[u8]),
) {
    let mut line = |bytes: &[u8]| {
        let text =
            String::from_utf8_lossy(bytes);

        observe(&text);

        stamped_line(&text, tag, colour, &utc_timestamp())
    };


    let mut read_buffer = [0_u8; 4096];
    let mut pending: Vec<u8> = Vec::new();

    loop {
        let count =
            match source.read(&mut read_buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };

        pending.extend_from_slice(&read_buffer[..count]);

        drain_segments(&mut pending, &mut line, &mut write);
    }

    if !pending.is_empty() {
        write(&line(&pending));
    }
}


// Complete lines get a timestamp. Segments ending in a bare carriage return
// (FFmpeg's in-place progress line) are passed through unchanged so they keep
// overwriting themselves instead of scrolling.
fn drain_segments(
    pending: &mut Vec<u8>,
    line: &mut impl FnMut(&[u8]) -> Vec<u8>,
    write: &mut impl FnMut(&[u8]),
) {
    loop {
        let Some(position) =
            pending
                .iter()
                .position(|byte| *byte == b'\n' || *byte == b'\r')
        else {
            return;
        };

        if pending[position] == b'\n' {
            write(&line(&pending[..position]));

            pending.drain(..=position);

            continue;
        }

        // Carriage return: wait for the next byte to tell CRLF from progress.
        let Some(next) = pending.get(position + 1) else {
            return;
        };

        if *next == b'\n' {
            write(&line(&pending[..position]));

            pending.drain(..=position + 1);
        } else {
            write(&pending[..=position]);

            pending.drain(..=position);
        }
    }
}


fn stamped_line(line: &str, tag: &str, colour: bool, timestamp: &str) -> Vec<u8> {
    if line.is_empty() {
        return b"\n".to_vec();
    }

    if !colour {
        return format!("[{timestamp}][{tag}] {line}\n").into_bytes();
    }

    format!(
        "{GRAY}[{timestamp}][{tag}]{RESET} {}\n",
        colour_level(line)
    )
    .into_bytes()
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


fn utc_timestamp() -> String {
    let millis =
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0);

    format_time_of_day(millis)
}


fn format_time_of_day(unix_millis: u128) -> String {
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
        assert_eq!(format_time_of_day(0), "00:00:00.000Z");

        assert_eq!(
            format_time_of_day(86_400_000 + 3_723_045),
            "01:02:03.045Z"
        );
    }


    // Replaces the timestamp so output can be compared exactly.
    fn relay_to_string(input: &[u8]) -> String {
        let mut output = Vec::new();

        relay(input, "MTX", false, |_| {}, |chunk| output.extend_from_slice(chunk));

        let mut text = String::from_utf8(output).unwrap();

        // "[HH:MM:SS.mmmZ" is 14 characters.
        let mut from = 0;

        while let Some(found) = text[from..].find("][MTX] ") {
            let end = from + found;

            text.replace_range(end - 14..end, "[T");

            from = end - 14 + 2 + 1;
        }

        text
    }


    #[test]
    fn stamps_lf_and_crlf_lines() {
        assert_eq!(
            relay_to_string(b"INF one\nINF two\r\n\nlast"),
            "[T][MTX] INF one\n[T][MTX] INF two\n\n[T][MTX] last\n"
        );
    }


    #[test]
    fn passes_progress_lines_through() {
        assert_eq!(
            relay_to_string(b"frame=1\rframe=2\rdone\n"),
            "frame=1\rframe=2\r[T][MTX] done\n"
        );
    }


    #[test]
    fn observer_sees_complete_lines_only() {
        let seen = std::cell::RefCell::new(Vec::new());

        relay(
            &b"frame=1\rINF is publishing\r\npartial"[..],
            "MTX",
            false,
            |line| seen.borrow_mut().push(line.to_string()),
            |_| {},
        );

        assert_eq!(
            *seen.borrow(),
            vec!["INF is publishing".to_string(), "partial".to_string()]
        );
    }


    #[test]
    fn colours_level_tag_when_enabled() {
        let line =
            stamped_line(
                "2026/10/04 01:23:13 WAR [path gameplay] something",
                "MTX",
                true,
                "07:23:13.412Z",
            );

        assert_eq!(
            String::from_utf8(line).unwrap(),
            "\x1b[90m[07:23:13.412Z][MTX]\x1b[0m 2026/10/04 01:23:13 \x1b[33mWAR\x1b[0m [path gameplay] something\n"
        );

        // Level-like text far into a message is left alone.
        assert_eq!(
            colour_level("frame= 100 fps= 60 q=20.0 size= 1024KiB INF x"),
            "frame= 100 fps= 60 q=20.0 size= 1024KiB INF x"
        );
    }
}
