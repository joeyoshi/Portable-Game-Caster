mod console;
mod demand;
mod encoder_output;
mod ffmpeg;
mod hotkeys;
mod instance;
mod job;
mod logging;

use mdns_sd::{ServiceDaemon, ServiceInfo};

use std::collections::HashSet;
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, RecvTimeoutError, Sender},
    Arc,
};
use std::thread;
use std::time::Duration;


// -----------------------------------------------------------------------------
// PGC service identity
// -----------------------------------------------------------------------------

const SERVICE_TYPE: &str = "_pgc._tcp.local.";
const SERVICE_INSTANCE: &str = "Portable Game Caster";

const RTSP_PORT: u16 = 8554;
const STREAM_PATH: &str = "/gameplay";
const PROTOCOL_VERSION: &str = "1";

// How this build was produced. Versioning work will select this at build time;
// until then every build is a development build.
const BUILD_CHANNEL: logging::BuildChannel = logging::BuildChannel::Development;


// MediaMTX logs this when it accepts a publisher on the gameplay path.
const PUBLISHER_READY_TEXT: &str = "is publishing to path 'gameplay'";

// MediaMTX logs this, followed by a short track description, when the gameplay
// stream becomes readable.
const STREAM_AVAILABLE_TEXT: &str = "[path gameplay] stream is available and online, ";

const SUPERVISOR_TICK: Duration = Duration::from_millis(200);


// -----------------------------------------------------------------------------
// Events delivered to the supervisor loop
// -----------------------------------------------------------------------------

pub enum Event {
    // A MediaMTX demand signal connected / disconnected (see demand.rs).
    DemandOpened(u64),
    DemandClosed(u64),

    // MediaMTX accepted FFmpeg as the gameplay publisher.
    PublisherReady,

    // MediaMTX's own description of the stream it is receiving.
    StreamReported(String),

    // A key pressed in the interactive Host console (lower-cased).
    Key(char),
}


// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

fn main() -> ExitCode {
    // MediaMTX runs this executable as its runOnDemand command. In that mode it
    // is only a demand signal, not a Host.
    if env::args().any(|argument| argument == demand::SIGNAL_FLAG) {
        return match demand::run_signal() {
            Ok(()) => ExitCode::SUCCESS,

            Err(error) => {
                eprintln!("{error}");

                ExitCode::FAILURE
            }
        };
    }


    console::widen();

    logging::init_from_args(
        console::enable_ansi()
    );

    // Before the session log: a launch that is rejected here must not create
    // a session file or rotate the running Host's logs. These lines reach the
    // console only.
    let _instance_guard =
        match instance::acquire() {
            Ok(Some(guard)) => {
                guard
            }

            Ok(None) => {
                logging::info("HOST", format_args!(
                    "Portable Game Caster Host is already running. Exiting."
                ));

                logging::debug("HOST", format_args!(
                    "Rejected this startup because another instance owns the machine-wide mutex."
                ));

                return ExitCode::SUCCESS;
            }

            Err(error) => {
                logging::error("HOST", format_args!(
                    "{error}"
                ));

                return ExitCode::FAILURE;
            }
        };

    logging::start_session(
        "host",
        "Portable Game Caster Host",
        BUILD_CHANNEL,
        env!("CARGO_PKG_VERSION"),
        PROTOCOL_VERSION,
        logging::SessionNaming::Timestamped,
    );

    // Held until exit, which restores the console's own setting.
    let input_mode_guard =
        console::disable_quick_edit();

    if input_mode_guard.is_some() {
        logging::debug("HOST", format_args!(
            "Console QuickEdit is off while the Host runs, so clicking the window cannot pause it."
        ));
    }


    match run() {
        Ok(()) => ExitCode::SUCCESS,

        Err(error) => {
            logging::error("HOST", format_args!(
                "{error}"
            ));

            logging::shutdown();

            ExitCode::FAILURE
        }
    }
}


fn run() -> Result<(), Box<dyn std::error::Error>> {
    let running =
        Arc::new(AtomicBool::new(true));

    {
        let running =
            Arc::clone(&running);

        ctrlc::set_handler(move || {
            running.store(
                false,
                Ordering::SeqCst,
            );
        })?;
    }


    // -------------------------------------------------------------------------
    // Find MediaMTX
    // -------------------------------------------------------------------------

    let mediamtx_path =
        find_mediamtx()?;

    logging::debug("HOST", format_args!(
        "MediaMTX: {}",
        mediamtx_path.display()
    ));


    // -------------------------------------------------------------------------
    // Find FFmpeg
    // -------------------------------------------------------------------------

    let ffmpeg_path =
        ffmpeg::find_ffmpeg()?;

    logging::debug("ENCODER", format_args!(
        "FFmpeg: {}",
        ffmpeg_path.display()
    ));


    // -------------------------------------------------------------------------
    // Demand signal + child containment
    // -------------------------------------------------------------------------

    let (events_tx, events_rx) =
        mpsc::channel::<Event>();

    let demand_listener =
        demand::listen(
            events_tx.clone()
        )?;

    let demand_address =
        demand_listener.address();

    logging::debug("DEMAND", format_args!(
        "Demand signal listening on {demand_address}."
    ));

    let job =
        job::ChildJob::create();

    let mut encoder =
        ffmpeg::Encoder::new(
            ffmpeg_path
        );

    let mut demand_signals:
        HashSet<u64> =
            HashSet::new();


    // -------------------------------------------------------------------------
    // Start MediaMTX
    // -------------------------------------------------------------------------

    let mut mediamtx =
        start_mediamtx(
            &mediamtx_path,
            demand_address,
            &events_tx,
            &job,
        )?;

    logging::info("HOST", format_args!(
        "MediaMTX started (PID {}).",
        mediamtx.id()
    ));


    // -------------------------------------------------------------------------
    // Start mDNS advertisement
    // -------------------------------------------------------------------------

    let mdns =
        start_discovery()?;

    logging::info("HOST", format_args!(
        "Advertising Portable Game Caster on the local network."
    ));

    logging::info("HOST", format_args!(
        "Host ready. FFmpeg starts when a viewer connects."
    ));

    // Single-key console commands, only when console input is available.
    if hotkeys::listen(events_tx.clone()) {
        logging::info("HOST", format_args!(
            "Press L to open logs folder."
        ));
    } else {
        logging::debug("HOST", format_args!(
            "Console input is not available; hotkeys are disabled."
        ));
    }

    logging::info("HOST", format_args!(
        "Press Ctrl+C to stop."
    ));


    // -------------------------------------------------------------------------
    // Supervise
    // -------------------------------------------------------------------------

    while running.load(Ordering::SeqCst) {
        // ---------------------------------------------------------------------
        // Demand / publisher events
        // ---------------------------------------------------------------------

        let mut next_event =
            match events_rx.recv_timeout(SUPERVISOR_TICK) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => None,
            };

        while let Some(event) = next_event {
            match event {
                Event::DemandOpened(id) => {
                    let was_active =
                        !demand_signals.is_empty();

                    demand_signals.insert(id);

                    if was_active {
                        logging::debug("DEMAND", format_args!(
                            "Additional demand signal #{id} connected ({} open).",
                            demand_signals.len()
                        ));
                    } else {
                        logging::info("DEMAND", format_args!(
                            "Viewer demand became active."
                        ));

                        logging::debug("DEMAND", format_args!(
                            "MediaMTX opened demand signal #{id}: a reader asked for the stream while nothing was publishing."
                        ));
                    }
                }

                Event::DemandClosed(id) => {
                    if demand_signals.remove(&id) {
                        if demand_signals.is_empty() {
                            // The signal closes both when MediaMTX ends demand
                            // and when MediaMTX itself dies. Only say what is
                            // known.
                            let mediamtx_exited =
                                matches!(mediamtx.try_wait(), Ok(Some(_)));

                            logging::info("DEMAND", format_args!(
                                "{}",
                                demand_closed_message(mediamtx_exited)
                            ));

                            logging::debug("DEMAND", format_args!(
                                "{}",
                                demand_closed_detail(id, mediamtx_exited)
                            ));

                            // When MediaMTX has exited, the MediaMTX handling
                            // below stops FFmpeg immediately; there is no
                            // grace period to start.
                            if !mediamtx_exited {
                                encoder.demand_ended();
                            }
                        } else {
                            logging::debug("DEMAND", format_args!(
                                "Demand signal #{id} closed ({} still open).",
                                demand_signals.len()
                            ));
                        }
                    }
                }

                Event::PublisherReady => {
                    encoder.publisher_ready();
                }

                Event::StreamReported(tracks) => {
                    encoder.stream_reported(tracks);
                }

                Event::Key('l') => {
                    open_logs_folder();
                }

                // Other keys are reserved for future console commands.
                Event::Key(_) => {}
            }

            next_event =
                events_rx.try_recv().ok();
        }


        // ---------------------------------------------------------------------
        // MediaMTX
        // ---------------------------------------------------------------------

        if let Some(status) = mediamtx.try_wait()? {
            logging::warn("HOST", format_args!(
                "MediaMTX PID {} exited unexpectedly: {status}",
                mediamtx.id()
            ));

            if !running.load(Ordering::SeqCst) {
                break;
            }

            // FFmpeg was publishing to the MediaMTX that just died, and any
            // demand it signalled died with it. Stop FFmpeg now; a fresh one
            // starts when the new MediaMTX reports demand.
            if !demand_signals.is_empty() {
                logging::info("DEMAND", format_args!(
                    "{}",
                    demand_closed_message(true)
                ));

                logging::debug("DEMAND", format_args!(
                    "Dropping {} demand signal(s) that belonged to the MediaMTX that exited.",
                    demand_signals.len()
                ));
            }

            demand_listener.reset();
            demand_signals.clear();

            encoder.stop("MediaMTX exited", false);

            logging::info("HOST", format_args!(
                "Restarting MediaMTX."
            ));

            thread::sleep(
                Duration::from_secs(1)
            );

            mediamtx =
                match start_mediamtx(
                    &mediamtx_path,
                    demand_address,
                    &events_tx,
                    &job,
                ) {
                    Ok(child) => child,

                    Err(error) => {
                        return Err(
                            format!("MediaMTX restart failed: {error}").into()
                        );
                    }
                };

            logging::info("HOST", format_args!(
                "MediaMTX restarted (PID {}).",
                mediamtx.id()
            ));

            continue;
        }


        // ---------------------------------------------------------------------
        // FFmpeg
        // ---------------------------------------------------------------------

        encoder.update(
            !demand_signals.is_empty(),
            &job,
        );
    }


    // -------------------------------------------------------------------------
    // Shutdown
    // -------------------------------------------------------------------------

    logging::info("HOST", format_args!(
        "Shutting down Portable Game Caster Host."
    ));

    logging::debug("HOST", format_args!(
        "Shutdown order: FFmpeg first (so it releases the capture device), then MediaMTX, then discovery."
    ));


    encoder.stop("Host shutdown", true);


    if mediamtx.try_wait()?.is_none() {
        let _ = mediamtx.kill();
        let _ = mediamtx.wait();
    }

    logging::info("HOST", format_args!(
        "MediaMTX PID {} stopped.",
        mediamtx.id()
    ));


    let _ = mdns.shutdown();


    logging::info("HOST", format_args!(
        "Shutdown complete."
    ));

    logging::shutdown();

    Ok(())
}


// -----------------------------------------------------------------------------
// Open the session log directory (L hotkey)
// -----------------------------------------------------------------------------

fn open_logs_folder() {
    logging::info("HOST", format_args!(
        "Open logs folder requested."
    ));

    let Some(directory) = logging::log_directory() else {
        logging::warn("HOST", format_args!(
            "No logs folder to open: this session has no log file."
        ));

        return;
    };

    match hotkeys::open_directory(&directory) {
        Ok(()) => {
            logging::debug("HOST", format_args!(
                "Opened {}",
                directory.display()
            ));
        }

        Err(error) => {
            logging::warn("HOST", format_args!(
                "Could not open logs folder {}: {error}",
                directory.display()
            ));
        }
    }
}


// -----------------------------------------------------------------------------
// Demand-loss wording
//
// A demand signal closing does not say why it closed. MediaMTX may have ended
// demand (last reader left, or the publisher never appeared), or MediaMTX may
// have died. The Host can only tell those apart by whether MediaMTX is still
// running, so that is all these messages claim.
// -----------------------------------------------------------------------------

fn demand_closed_message(mediamtx_exited: bool) -> &'static str {
    if mediamtx_exited {
        "MediaMTX exited; viewer demand state reset."
    } else {
        "Viewer demand became inactive (demand signal closed)."
    }
}


fn demand_closed_detail(id: u64, mediamtx_exited: bool) -> String {
    if mediamtx_exited {
        format!("Demand signal #{id} closed because MediaMTX is no longer running.")
    } else {
        format!("MediaMTX ended demand signal #{id}. MediaMTX does not report why; its own log (Verbose) states the reason.")
    }
}


// "... [path gameplay] stream is available and online, 2 tracks (H264, MPEG-4 Audio)"
//     -> "2 tracks (H264, MPEG-4 Audio)"
fn reported_tracks(mediamtx_line: &str) -> Option<String> {
    let (_, tracks) =
        mediamtx_line.split_once(STREAM_AVAILABLE_TEXT)?;

    let tracks = tracks.trim();

    (!tracks.is_empty()).then(|| tracks.to_string())
}


// -----------------------------------------------------------------------------
// Find MediaMTX
// -----------------------------------------------------------------------------

fn find_mediamtx()
    -> Result<PathBuf, Box<dyn std::error::Error>>
{
    // Explicit override for development / packaging.
    if let Ok(path) =
        env::var("PGC_MEDIAMTX_PATH")
    {
        let path =
            PathBuf::from(path);

        if path.exists() {
            return Ok(path);
        }
    }


    // Current prototype installation.
    let prototype_path =
        PathBuf::from(
            r"C:\MediaMTX\mediamtx.exe"
        );

    if prototype_path.exists() {
        return Ok(prototype_path);
    }


    // Future portable layout:
    //
    // pgc-host-windows.exe
    // mediamtx/
    //   mediamtx.exe
    //
    let exe =
        env::current_exe()?;

    if let Some(parent) =
        exe.parent()
    {
        let bundled =
            parent
                .join("mediamtx")
                .join("mediamtx.exe");

        if bundled.exists() {
            return Ok(bundled);
        }
    }


    Err(
        "Could not locate MediaMTX. Set PGC_MEDIAMTX_PATH or install MediaMTX at C:\\MediaMTX\\mediamtx.exe."
            .into()
    )
}


// -----------------------------------------------------------------------------
// Start MediaMTX
// -----------------------------------------------------------------------------

fn start_mediamtx(
    executable: &Path,
    demand_address: SocketAddr,
    events: &Sender<Event>,
    job: &job::ChildJob,
) -> Result<Child, Box<dyn std::error::Error>> {
    let working_directory =
        executable
            .parent()
            .ok_or(
                "MediaMTX path has no parent directory"
            )?;


    // Output is piped rather than inherited: the Host watches it for publisher
    // readiness, and in Verbose relays it labelled as MTX with the same UTC
    // timestamp format as every other line.
    //
    // The environment tells MediaMTX's runOnDemand command (this executable in
    // demand-signal mode) where to find the Host.
    let mut child =
        Command::new(executable)
            .current_dir(
                working_directory
            )
            .env(
                demand::ADDRESS_ENV,
                demand_address.to_string(),
            )
            .env(
                demand::HOST_EXECUTABLE_ENV,
                env::current_exe()?,
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;


    job.assign(&child, "MediaMTX");


    if let Some(stdout) = child.stdout.take() {
        let events =
            events.clone();

        logging::relay_output(stdout, logging::Source::Mtx, move |line| {
            if line.contains(PUBLISHER_READY_TEXT) {
                let _ = events.send(Event::PublisherReady);
            } else if let Some(tracks) = reported_tracks(line) {
                let _ = events.send(Event::StreamReported(tracks));
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        logging::relay_output(stderr, logging::Source::Mtx, |_| {});
    }


    Ok(child)
}


// -----------------------------------------------------------------------------
// Start DNS-SD / mDNS advertiser
// -----------------------------------------------------------------------------

fn start_discovery()
    -> Result<ServiceDaemon, Box<dyn std::error::Error>>
{
    let computer_name =
        env::var("COMPUTERNAME")
            .unwrap_or_else(
                |_| "pgc".to_string()
            );


    let safe_name =
        computer_name
            .to_lowercase()
            .replace(
                |c: char| {
                    !c.is_ascii_alphanumeric()
                        && c != '-'
                },
                "-",
            );


    let host_name =
        format!(
            "{safe_name}-pgc.local."
        );


    let mdns =
        ServiceDaemon::new()?;


    let properties = [
        ("protocol", "rtsp"),
        ("path", STREAM_PATH),
        ("version", PROTOCOL_VERSION),
    ];


    let no_addresses: &[IpAddr] =
        &[];


    let service =
        ServiceInfo::new(
            SERVICE_TYPE,
            SERVICE_INSTANCE,
            &host_name,
            no_addresses,
            RTSP_PORT,
            &properties[..],
        )?
        .enable_addr_auto();


    mdns.register(service)?;


    logging::debug("HOST", format_args!(
        "Discovery host: {host_name}"
    ));


    Ok(mdns)
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn demand_loss_wording_claims_only_what_is_known() {
        let closed = demand_closed_message(false);
        let closed_detail = demand_closed_detail(3, false);

        // No guess about readers or publisher timing.
        for text in [closed, closed_detail.as_str()] {
            let lower = text.to_ascii_lowercase();

            assert!(!lower.contains("no readers"), "{text}");
            assert!(!lower.contains("did not appear"), "{text}");
        }

        assert_eq!(closed, "Viewer demand became inactive (demand signal closed).");

        assert_eq!(
            demand_closed_message(true),
            "MediaMTX exited; viewer demand state reset."
        );

        assert_eq!(
            demand_closed_detail(3, true),
            "Demand signal #3 closed because MediaMTX is no longer running."
        );
    }


    #[test]
    fn extracts_tracks_from_mediamtx_line() {
        assert_eq!(
            reported_tracks(
                "2026/10/04 05:02:13 INF [path gameplay] stream is available and online, 2 tracks (H264, MPEG-4 Audio)"
            )
            .as_deref(),
            Some("2 tracks (H264, MPEG-4 Audio)")
        );

        assert_eq!(
            reported_tracks("2026/10/04 05:02:13 INF [SRT] [conn [::1]:56642] is publishing to path 'gameplay'"),
            None
        );

        // Another path must not be mistaken for gameplay.
        assert_eq!(
            reported_tracks("INF [path other] stream is available and online, 1 track (H264)"),
            None
        );
    }
}
