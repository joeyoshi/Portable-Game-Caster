use std::env;
use std::error::Error;
use std::io::{
    ErrorKind,
    Read,
    Write,
};
use std::net::{
    SocketAddr,
    TcpStream,
};
use std::path::PathBuf;
use std::process::{
    Child,
    Command,
    Stdio,
};
use std::sync::mpsc::{
    self,
    Receiver,
};
use std::thread;
use std::time::Duration;

use crate::logging;


// -----------------------------------------------------------------------------
// Events derived from ffplay
// -----------------------------------------------------------------------------

#[derive(Debug)]
pub enum TransportEvent {
    MediaStarted,
    Lost,
}


pub type TransportReceiver =
    Receiver<TransportEvent>;


// -----------------------------------------------------------------------------
// Stream service checks
// -----------------------------------------------------------------------------

pub fn check_stream_service(
    address: &str,
    port: u16,
) -> Result<(), Box<dyn Error>> {
    check_stream_service_timeout(
        address,
        port,
        Duration::from_secs(2),
    )
}


pub fn check_stream_service_timeout(
    address: &str,
    port: u16,
    timeout: Duration,
) -> Result<(), Box<dyn Error>> {
    let socket_address =
        format!(
            "{address}:{port}"
        )
        .parse::<SocketAddr>()?;


    logging::trace(
        "PLAYER",
        format_args!(
            "Probing RTSP service at {socket_address} with timeout {timeout:?}"
        ),
    );


    TcpStream::connect_timeout(
        &socket_address,
        timeout,
    )
    .map_err(|_| {
        format!(
            "Portable Game Caster was found at {address}, but the streaming service is unavailable."
        )
    })?;


    Ok(())
}


// -----------------------------------------------------------------------------
// Launch ffplay
// -----------------------------------------------------------------------------

pub fn launch_ffplay(
    url: &str,
) -> Result<
    (Child, TransportReceiver),
    Box<dyn Error>,
> {
    let ffplay =
        find_ffplay()?;


    logging::debug(
        "PLAYER",
        format_args!(
            "ffplay: {}",
            ffplay.display()
        ),
    );


    logging::debug(
        "PLAYER",
        format_args!(
            "Opening stream: {url}"
        ),
    );


    let mut child =
        Command::new(&ffplay)

            // ffplay believes stderr is a pipe, so explicitly retain its
            // normal colorized diagnostics for --verbose passthrough.
            .env(
                "AV_LOG_FORCE_COLOR",
                "1",
            )

            .arg("-rtsp_transport")
            .arg("tcp")

            .arg("-fflags")
            .arg("nobuffer")

            .arg("-flags")
            .arg("low_delay")

            .arg("-noinfbuf")

            .arg("-framedrop")

            .arg("-sync")
            .arg("ext")

            .arg("-probesize")
            .arg("2M")

            .arg("-analyzeduration")
            .arg("500000")

            .arg("-max_delay")
            .arg("0")

            .arg("-stats")

            .arg(url)

            .stdin(
                Stdio::null()
            )

            .stdout(
                Stdio::inherit()
            )

            .stderr(
                Stdio::piped()
            )

            .spawn()?;


    logging::debug(
        "PLAYER",
        format_args!(
            "ffplay launched with PID {}",
            child.id()
        ),
    );


    let mut ffplay_stderr =
        child
            .stderr
            .take()
            .ok_or(
                "Could not capture ffplay stderr."
            )?;


    let (
        transport_tx,
        transport_rx,
    ) =
        mpsc::channel::<TransportEvent>();


    thread::spawn(move || {
        const FAILURE_TEXT: &str =
            "Failed reading RTSP data";


        let mut stderr =
            std::io::stderr();


        let mut read_buffer =
            [0_u8; 4096];


        // Accumulates just enough textual data to split ffplay's combination
        // of newline and carriage-return output into logical updates.
        let mut parse_buffer =
            Vec::<u8>::new();


        let mut media_reported =
            false;


        let mut loss_reported =
            false;


        loop {
            let bytes_read =
                match ffplay_stderr.read(
                    &mut read_buffer
                ) {
                    Ok(0) => {
                        break;
                    }


                    Ok(count) => {
                        count
                    }


                    Err(error)
                        if error.kind()
                            == ErrorKind::Interrupted =>
                    {
                        continue;
                    }


                    Err(error) => {
                        logging::trace(
                            "PLAYER",
                            format_args!(
                                "ffplay stderr monitor ended: {error}"
                            ),
                        );

                        break;
                    }
                };


            let chunk =
                &read_buffer[..bytes_read];


            // -------------------------------------------------------------
            // --verbose gets ffplay's output byte-for-byte.
            //
            // Normal and --debug modes still inspect it internally but don't
            // pay the terminal-rendering cost.
            // -------------------------------------------------------------

            if logging::trace_enabled() {
                let _ =
                    stderr.write_all(
                        chunk
                    );

                let _ =
                    stderr.flush();
            }


            // -------------------------------------------------------------
            // Parse logical ffplay updates.
            //
            // ffplay's live statistics use '\r', while regular diagnostics
            // use '\n'.
            // -------------------------------------------------------------

            parse_buffer.extend_from_slice(
                chunk
            );


            while let Some(separator_index) =
                parse_buffer
                    .iter()
                    .position(
                        |byte| {
                            *byte == b'\n'
                                || *byte == b'\r'
                        }
                    )
            {
                let segment =
                    parse_buffer
                        .drain(
                            ..separator_index
                        )
                        .collect::<Vec<_>>();


                // Remove the separator itself.
                if !parse_buffer.is_empty() {
                    parse_buffer.remove(0);
                }


                if segment.is_empty() {
                    continue;
                }


                let text =
                    String::from_utf8_lossy(
                        &segment
                    );


                // ---------------------------------------------------------
                // RTSP transport failure
                // ---------------------------------------------------------

                if !loss_reported
                    && text.contains(
                        FAILURE_TEXT
                    )
                {
                    logging::debug(
                        "HEALTH",
                        format_args!(
                            "RTSP transport failure reported by ffplay"
                        ),
                    );


                    let _ =
                        transport_tx.send(
                            TransportEvent::Lost
                        );


                    loss_reported =
                        true;
                }


                // ---------------------------------------------------------
                // Actual media-flow confirmation
                //
                // ffplay emits live A/V synchronization statistics only once
                // decoded playback is running. During stalled startup these
                // commonly remain NaN.
                //
                // This is intentionally an interim signal until PGC has its
                // proper media-health protocol.
                // ---------------------------------------------------------

                if !media_reported {
                    let has_av_stats =
                        text.contains(
                            "M-V:"
                        )
                            || text.contains(
                                "A-V:"
                            );


                    let has_queue_stats =
                        text.contains(
                            "vq="
                        )
                            || text.contains(
                                "aq="
                            );


                    let is_nan =
                        text
                            .to_ascii_lowercase()
                            .contains(
                                "nan"
                            );


                    if has_av_stats
                        && has_queue_stats
                        && !is_nan
                    {
                        logging::debug(
                            "HEALTH",
                            format_args!(
                                "Confirmed decoded media flow from ffplay"
                            ),
                        );


                        let _ =
                            transport_tx.send(
                                TransportEvent::MediaStarted
                            );


                        media_reported =
                            true;
                    }
                }
            }


            // Prevent a malformed/no-separator stream from causing unlimited
            // memory growth.
            if parse_buffer.len()
                > 16 * 1024
            {
                let keep =
                    4096;


                let drain =
                    parse_buffer.len()
                        .saturating_sub(
                            keep
                        );


                parse_buffer.drain(
                    ..drain
                );
            }
        }


        logging::trace(
            "PLAYER",
            format_args!(
                "ffplay stderr monitor stopped"
            ),
        );
    });


    Ok(
        (
            child,
            transport_rx,
        )
    )
}


// -----------------------------------------------------------------------------
// Locate ffplay
// -----------------------------------------------------------------------------

fn find_ffplay(
) -> Result<PathBuf, Box<dyn Error>> {
    if let Ok(path) =
        env::var(
            "PGC_FFPLAY_PATH"
        )
    {
        let path =
            PathBuf::from(path);


        if path.exists() {
            logging::trace(
                "PLAYER",
                format_args!(
                    "Using PGC_FFPLAY_PATH override"
                ),
            );

            return Ok(path);
        }
    }


    let home =
        env::var("HOME")
            .unwrap_or_default();


    let brew_candidates = [
        PathBuf::from(
            format!(
                "{home}/homebrew/bin/brew"
            )
        ),

        PathBuf::from(
            "/opt/homebrew/bin/brew"
        ),

        PathBuf::from(
            "/usr/local/bin/brew"
        ),
    ];


    for brew in brew_candidates {
        if !brew.exists() {
            continue;
        }


        logging::trace(
            "PLAYER",
            format_args!(
                "Checking Homebrew: {}",
                brew.display()
            ),
        );


        let output =
            Command::new(&brew)
                .arg("--prefix")
                .arg("ffmpeg-full")
                .output();


        let Ok(output) =
            output
        else {
            continue;
        };


        if !output.status.success() {
            continue;
        }


        let prefix =
            String::from_utf8_lossy(
                &output.stdout
            )
            .trim()
            .to_string();


        if prefix.is_empty() {
            continue;
        }


        let ffplay =
            PathBuf::from(prefix)
                .join("bin")
                .join("ffplay");


        if ffplay.exists() {
            return Ok(ffplay);
        }
    }


    Err(
        "Could not find ffplay from the Homebrew ffmpeg-full package."
            .into()
    )
}