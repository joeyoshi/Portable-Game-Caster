// -----------------------------------------------------------------------------
// Native FFmpeg ownership
// -----------------------------------------------------------------------------
//
// The Host launches FFmpeg itself, holds the process handle, and is the only
// thing that starts or stops it:
//
//     demand present, FFmpeg not running  -> start
//     FFmpeg exits while demand present   -> restart (with a short back-off)
//     demand ended                        -> keep running for a short grace
//                                            period, then stop
//     MediaMTX exited / Host shutdown     -> stop
//
// At most one FFmpeg is owned at a time. When nothing is watching, FFmpeg is
// not running and the capture device and encoder are idle.

use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::job::ChildJob;
use crate::logging;


// Keep FFmpeg running this long after demand ends, so a Client that is
// reconnecting does not pay for a full capture/encoder restart.
const DEMAND_END_GRACE: Duration = Duration::from_secs(5);

// How long FFmpeg gets to exit after being asked to quit before it is killed.
const QUIT_TIMEOUT: Duration = Duration::from_secs(3);

// How long to wait for confirmation after killing FFmpeg.
const KILL_CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);

const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(50);

// Back-off between restarts when FFmpeg keeps exiting quickly.
const RESTART_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
];

// An FFmpeg that ran at least this long is not counted as a failed start.
const HEALTHY_RUN: Duration = Duration::from_secs(10);


// -----------------------------------------------------------------------------
// Capture / encode pipeline
//
// Prototype hardware assumptions (Elgato HD60 S+, Pascal NVENC). Same known-good
// arguments the PowerShell bridge used.
// -----------------------------------------------------------------------------

const CAPTURE_INPUT: &str =
    "video=Game Capture HD60 S+:audio=Digital Audio Interface (Game Capture HD60 S+)";

const PUBLISH_URL: &str =
    "srt://127.0.0.1:8890?streamid=publish:gameplay&pkt_size=1316&latency=20000&tlpktdrop=1";


fn capture_arguments() -> Vec<&'static str> {
    vec![
        "-stats",
        "-f", "dshow",
        "-rtbufsize", "64M",
        "-video_size", "1920x1080",
        "-framerate", "60",
        "-audio_buffer_size", "50",
        "-i", CAPTURE_INPUT,
        "-vf", "format=nv12",
        "-c:v", "h264_nvenc",
        "-preset", "p2",
        "-tune", "ull",
        "-zerolatency", "1",
        "-rc", "cbr",
        "-b:v", "20M",
        "-maxrate", "20M",
        "-bufsize", "1M",
        "-g", "30",
        "-bf", "0",
        "-bsf:v", "dump_extra=freq=keyframe",
        "-r", "60",
        "-fps_mode", "cfr",
        "-af", "aresample=48000:async=1000:first_pts=0",
        "-c:a", "aac",
        "-b:a", "192k",
        "-ac", "2",
        "-flush_packets", "1",
        "-f", "mpegts",
        PUBLISH_URL,
    ]
}


// -----------------------------------------------------------------------------
// Find FFmpeg
// -----------------------------------------------------------------------------

pub fn find_ffmpeg()
    -> Result<PathBuf, Box<dyn std::error::Error>>
{
    // Explicit override for development / packaging.
    if let Ok(path) = env::var("PGC_FFMPEG_PATH") {
        let path = PathBuf::from(path);

        if path.exists() {
            return Ok(path);
        }
    }


    // Current prototype installation.
    let prototype_path =
        PathBuf::from(r"C:\ffmpeg\bin\ffmpeg.exe");

    if prototype_path.exists() {
        return Ok(prototype_path);
    }


    // Future portable layout:
    //
    // pgc-host-windows.exe
    // ffmpeg/
    //   ffmpeg.exe
    //
    let exe = env::current_exe()?;

    if let Some(parent) = exe.parent() {
        let bundled =
            parent
                .join("ffmpeg")
                .join("ffmpeg.exe");

        if bundled.exists() {
            return Ok(bundled);
        }
    }


    Err(
        "Could not locate FFmpeg. Set PGC_FFMPEG_PATH or install FFmpeg at C:\\ffmpeg\\bin\\ffmpeg.exe."
            .into()
    )
}


// -----------------------------------------------------------------------------
// Owned FFmpeg process
// -----------------------------------------------------------------------------

struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
    started: Instant,
    publishing: bool,
}


pub struct Encoder {
    executable: PathBuf,
    running: Option<Running>,
    linger_until: Option<Instant>,
    retry_at: Option<Instant>,
    failed_starts: usize,
}


impl Encoder {
    pub fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            running: None,
            linger_until: None,
            retry_at: None,
            failed_starts: 0,
        }
    }


    // Demand went from present to absent.
    pub fn demand_ended(&mut self) {
        if self.running.is_some() {
            logging::debug("ENCODER", format_args!(
                "FFmpeg will be stopped in {}s unless demand returns.",
                DEMAND_END_GRACE.as_secs()
            ));
        }

        self.linger_until =
            Some(Instant::now() + DEMAND_END_GRACE);
    }


    // MediaMTX accepted the publisher.
    pub fn publisher_ready(&mut self) {
        if let Some(running) = &mut self.running {
            if !running.publishing {
                running.publishing = true;

                logging::info("ENCODER", format_args!(
                    "FFmpeg PID {} is publishing to MediaMTX ({:.1}s after launch).",
                    running.child.id(),
                    running.started.elapsed().as_secs_f64()
                ));
            }
        }
    }


    // Called on every supervisor tick.
    pub fn update(&mut self, demand_active: bool, job: &ChildJob) {
        let now =
            Instant::now();

        if demand_active {
            self.linger_until = None;
        }

        let lingering =
            self.linger_until.is_some_and(|until| now < until);


        self.reap_unexpected_exit(demand_active);


        if demand_active {
            if self.running.is_none()
                && self.retry_at.is_none_or(|retry_at| now >= retry_at)
            {
                self.start(job);
            }

            return;
        }


        // No demand: nothing to retry.
        self.retry_at = None;
        self.failed_starts = 0;

        if !lingering && self.running.is_some() {
            self.linger_until = None;

            self.stop("no demand", true);
        }
    }


    fn reap_unexpected_exit(&mut self, demand_active: bool) {
        let Some(running) = &mut self.running else {
            return;
        };

        let status =
            match running.child.try_wait() {
                Ok(Some(status)) => describe_status(status),
                Ok(None) => return,
                Err(error) => format!("unknown ({error})"),
            };

        let ran_for =
            running.started.elapsed();

        logging::warn("ENCODER", format_args!(
            "FFmpeg PID {} exited unexpectedly after {:.1}s: {status}.",
            running.child.id(),
            ran_for.as_secs_f64()
        ));

        self.running = None;


        if ran_for >= HEALTHY_RUN {
            self.failed_starts = 0;
        } else {
            logging::debug("ENCODER", format_args!(
                "FFmpeg ran for less than {}s; counting it as a failed start ({} so far) for restart back-off.",
                HEALTHY_RUN.as_secs(),
                self.failed_starts + 1
            ));
        }

        let delay =
            RESTART_DELAYS[self.failed_starts.min(RESTART_DELAYS.len() - 1)];

        self.failed_starts += 1;

        self.retry_at =
            Some(Instant::now() + delay);


        if demand_active {
            logging::info("ENCODER", format_args!(
                "Demand is still active; restarting FFmpeg in {}s (attempt {}).",
                delay.as_secs(),
                self.failed_starts
            ));
        } else {
            logging::debug("ENCODER", format_args!(
                "No demand; FFmpeg will not be restarted."
            ));
        }
    }


    fn start(&mut self, job: &ChildJob) {
        logging::debug("ENCODER", format_args!(
            "FFmpeg launch requested: {}",
            self.executable.display()
        ));

        let spawned =
            Command::new(&self.executable)
                .args(capture_arguments())
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn();

        let mut child =
            match spawned {
                Ok(child) => child,

                Err(error) => {
                    let delay =
                        RESTART_DELAYS[RESTART_DELAYS.len() - 1];

                    logging::error("ENCODER", format_args!(
                        "FFmpeg launch failed: {error}. Retrying in {}s while demand remains.",
                        delay.as_secs()
                    ));

                    self.retry_at =
                        Some(Instant::now() + delay);

                    return;
                }
            };

        job.assign(&child, "FFmpeg");

        if let Some(stderr) = child.stderr.take() {
            logging::relay_output(stderr, logging::Source::Ffmpeg, |_| {});
        }

        logging::info("ENCODER", format_args!(
            "FFmpeg launched (PID {}).",
            child.id()
        ));

        self.retry_at = None;

        self.running =
            Some(Running {
                stdin: child.stdin.take(),
                child,
                started: Instant::now(),
                publishing: false,
            });
    }


    // Stops the owned FFmpeg and confirms it is gone. `graceful` first asks
    // FFmpeg to quit so it can release the capture device and encoder cleanly.
    pub fn stop(&mut self, reason: &str, graceful: bool) {
        let Some(mut running) = self.running.take() else {
            return;
        };

        let pid =
            running.child.id();

        logging::info("ENCODER", format_args!(
            "Stopping FFmpeg PID {pid} ({reason})."
        ));


        if graceful {
            logging::debug("ENCODER", format_args!(
                "Asking FFmpeg PID {pid} to quit so it releases the capture device and encoder cleanly (up to {}s).",
                QUIT_TIMEOUT.as_secs()
            ));

            if let Some(mut stdin) = running.stdin.take() {
                let _ = stdin.write_all(b"q");
                let _ = stdin.flush();
            }

            if let Some(status) = wait_for_exit(&mut running.child, QUIT_TIMEOUT) {
                logging::info("ENCODER", format_args!(
                    "FFmpeg PID {pid} confirmed stopped after quit request: {status}."
                ));

                return;
            }

            logging::debug("ENCODER", format_args!(
                "FFmpeg PID {pid} did not quit within {}s; terminating it.",
                QUIT_TIMEOUT.as_secs()
            ));
        }


        logging::debug("ENCODER", format_args!(
            "Terminating FFmpeg PID {pid} and waiting up to {}s for confirmation.",
            KILL_CONFIRM_TIMEOUT.as_secs()
        ));

        let _ = running.child.kill();

        match wait_for_exit(&mut running.child, KILL_CONFIRM_TIMEOUT) {
            Some(status) => {
                logging::info("ENCODER", format_args!(
                    "FFmpeg PID {pid} confirmed stopped after termination: {status}."
                ));
            }

            None => {
                logging::warn("ENCODER", format_args!(
                    "FFmpeg PID {pid} is still running {}s after termination was requested. No new FFmpeg will be started until it exits.",
                    KILL_CONFIRM_TIMEOUT.as_secs()
                ));

                // Still owned: keeps the single-FFmpeg guarantee.
                self.running = Some(running);
            }
        }
    }
}


fn wait_for_exit(child: &mut Child, timeout: Duration) -> Option<String> {
    let deadline =
        Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(describe_status(status)),
            Ok(None) => {}
            Err(error) => return Some(format!("unknown ({error})")),
        }

        if Instant::now() >= deadline {
            return None;
        }

        thread::sleep(EXIT_POLL_INTERVAL);
    }
}


fn describe_status(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => status.to_string(),
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn capture_input_is_one_argument() {
        let arguments = capture_arguments();

        let input =
            arguments
                .iter()
                .position(|argument| *argument == "-i")
                .unwrap();

        assert_eq!(arguments[input + 1], CAPTURE_INPUT);
        assert_eq!(arguments.last(), Some(&PUBLISH_URL));
        assert!(!arguments.contains(&"-use_wallclock_as_timestamps"));
        assert!(!arguments.contains(&"-repeat_headers"));
    }
}
