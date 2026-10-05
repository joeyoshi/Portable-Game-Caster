// -----------------------------------------------------------------------------
// FFmpeg diagnostics for Normal / Debug logging
// -----------------------------------------------------------------------------
//
// FFmpeg's stderr is only printed in Verbose. So that a failure can still be
// explained at Normal and Debug, a very small window of its most recent output
// is kept in memory and summarised when FFmpeg exits unexpectedly or is slow to
// start publishing.
//
// Observability only: nothing here starts, stops, or restarts FFmpeg.

use std::collections::VecDeque;
use std::time::Duration;


// Retained window: at most this many lines, each cut to this many characters.
const MAX_LINES: usize = 12;
const MAX_LINE_CHARS: usize = 300;

// How many error lines Debug shows after an unexpected exit.
pub const MAX_DETAIL_LINES: usize = 4;

// When to report that FFmpeg is running but has still not become a publisher.
// A few escalating notes, then silence.
const PUBLISHER_WAIT_NOTES: [Duration; 3] = [
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(30),
];

// Start-up banner lines that never explain a failure.
const BANNER_PREFIXES: [&str; 4] = [
    "ffmpeg version",
    "built with",
    "configuration:",
    "lib",
];

// Words that mark a line as describing a problem.
const PROBLEM_WORDS: [&str; 16] = [
    "error",
    "failed",
    "failure",
    "could not",
    "cannot",
    "can't",
    "unable",
    "invalid",
    "no such",
    "not found",
    "denied",
    "in use",
    "busy",
    "refused",
    "timed out",
    "broken",
];


// -----------------------------------------------------------------------------
// Recent FFmpeg output
// -----------------------------------------------------------------------------

#[derive(Default)]
pub struct RecentOutput {
    lines: VecDeque<String>,
}


impl RecentOutput {
    pub fn push(&mut self, line: &str) {
        let line = line.trim();

        if line.is_empty() || is_banner(line) {
            return;
        }

        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
        }

        self.lines.push_back(
            line.chars().take(MAX_LINE_CHARS).collect()
        );
    }


    // Lines that describe a problem, oldest first.
    pub fn problems(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter(|line| is_problem(line))
            .map(|line| tidy(line))
            .collect()
    }


    // The single most useful line for explaining an exit: the earliest problem
    // FFmpeg reported in the retained window, which is normally the root cause
    // (later lines tend to be generic consequences such as "Conversion
    // failed!").
    pub fn cause(&self) -> Option<String> {
        self.problems().into_iter().next()
    }


    // The most recent thing FFmpeg said, problem or not.
    pub fn last_line(&self) -> Option<String> {
        self.lines.back().map(|line| tidy(line))
    }
}


fn is_banner(line: &str) -> bool {
    BANNER_PREFIXES
        .iter()
        .any(|prefix| line.starts_with(prefix))
}


fn is_problem(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();

    PROBLEM_WORDS
        .iter()
        .any(|word| lower.contains(word))
}


// "[dshow @ 000001f2a3b4c5d6] Could not run graph" -> "[dshow] Could not run graph"
fn tidy(line: &str) -> String {
    if let Some(rest) = line.strip_prefix('[') {
        if let Some((component, message)) = rest.split_once(']') {
            if let Some((name, _address)) = component.split_once(" @ ") {
                return format!("[{name}]{message}");
            }
        }
    }

    line.to_string()
}


// -----------------------------------------------------------------------------
// Waiting for the publisher
// -----------------------------------------------------------------------------

#[derive(Default)]
pub struct PublisherWait {
    notes_given: usize,
}


impl PublisherWait {
    // Returns the threshold that has just been crossed, at most once each.
    pub fn due(&mut self, waited: Duration) -> Option<Duration> {
        let threshold =
            *PUBLISHER_WAIT_NOTES.get(self.notes_given)?;

        if waited < threshold {
            return None;
        }

        self.notes_given += 1;

        Some(threshold)
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    fn output(lines: &[&str]) -> RecentOutput {
        let mut recent = RecentOutput::default();

        for line in lines {
            recent.push(line);
        }

        recent
    }


    #[test]
    fn retention_is_bounded() {
        let mut recent = RecentOutput::default();

        let long_line = "x".repeat(MAX_LINE_CHARS * 4);

        for index in 0..1000 {
            recent.push(&format!("line {index} {long_line}"));
        }

        assert_eq!(recent.lines.len(), MAX_LINES);

        assert!(
            recent
                .lines
                .iter()
                .all(|line| line.chars().count() <= MAX_LINE_CHARS)
        );

        // Oldest lines are the ones dropped.
        assert!(recent.lines.front().unwrap().starts_with("line 988 "));
        assert!(recent.lines.back().unwrap().starts_with("line 999 "));
    }


    #[test]
    fn banner_and_blank_lines_are_not_retained() {
        let recent = output(&[
            "ffmpeg version 8.0.1-full_build-www.gyan.dev Copyright (c) 2000-2025 the FFmpeg developers",
            "  built with gcc 15.2.0 (Rev8, Built by MSYS2 project)",
            "  configuration: --enable-gpl --enable-version3 --enable-static",
            "  libavutil      60.  8.100 / 60.  8.100",
            "  libavcodec     62. 11.100 / 62. 11.100",
            "",
            "   ",
        ]);

        assert!(recent.lines.is_empty());
        assert_eq!(recent.cause(), None);
        assert_eq!(recent.last_line(), None);
    }


    #[test]
    fn cause_when_capture_device_cannot_open() {
        let recent = output(&[
            "ffmpeg version 8.0.1-full_build-www.gyan.dev",
            "[dshow @ 000001f2a3b4c5d6] Could not run graph (sometimes caused by a device already in use by other application)",
            "[in#0 @ 000001f2a3b4d000] Error opening input: I/O error",
            "Error opening input file video=Game Capture HD60 S+:audio=Digital Audio Interface (Game Capture HD60 S+).",
            "Error opening input files: I/O error",
        ]);

        assert_eq!(
            recent.cause().as_deref(),
            Some("[dshow] Could not run graph (sometimes caused by a device already in use by other application)")
        );

        assert_eq!(recent.problems().len(), 4);
    }


    #[test]
    fn cause_after_a_healthy_stream() {
        let recent = output(&[
            "Input #0, dshow, from 'video=Game Capture HD60 S+':",
            "  Stream #0:0: Video: rawvideo (YUY2), yuyv422, 1920x1080, 60 fps",
            "Output #0, mpegts, to 'srt://127.0.0.1:8890':",
            "[srt @ 000001f2a3b4c5d6] Connection to srt://127.0.0.1:8890 was broken",
            "[out#0/mpegts @ 000001f2a3b4e000] Error muxing a packet",
            "Conversion failed!",
        ]);

        assert_eq!(
            recent.cause().as_deref(),
            Some("[srt] Connection to srt://127.0.0.1:8890 was broken")
        );

        assert_eq!(recent.last_line().as_deref(), Some("Conversion failed!"));
    }


    #[test]
    fn no_cause_when_ffmpeg_reported_no_problem() {
        let recent = output(&[
            "Input #0, dshow, from 'video=Game Capture HD60 S+':",
            "Stream mapping:",
            "Press [q] to stop, [?] for help",
        ]);

        assert_eq!(recent.cause(), None);

        assert_eq!(
            recent.last_line().as_deref(),
            Some("Press [q] to stop, [?] for help")
        );
    }


    #[test]
    fn publisher_wait_notes_are_few_and_never_repeat() {
        let mut wait = PublisherWait::default();

        let mut notes = Vec::new();

        // A supervisor ticking every 200ms for two minutes.
        for tick in 0..600_u64 {
            if let Some(threshold) = wait.due(Duration::from_millis(tick * 200)) {
                notes.push((tick * 200, threshold.as_secs()));
            }
        }

        assert_eq!(notes, vec![(5000, 5), (15000, 15), (30000, 30)]);
    }


    #[test]
    fn publisher_wait_is_silent_when_publishing_is_prompt() {
        let mut wait = PublisherWait::default();

        assert_eq!(wait.due(Duration::from_millis(1500)), None);
        assert_eq!(wait.due(Duration::from_millis(4999)), None);
    }
}
