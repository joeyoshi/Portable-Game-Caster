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


// -----------------------------------------------------------------------------
// Transport events reported by the ffplay stderr monitor
// -----------------------------------------------------------------------------

#[derive(Debug)]
pub enum TransportEvent {
    Lost,
}


pub type TransportReceiver =
    Receiver<TransportEvent>;


// -----------------------------------------------------------------------------
// Check whether the advertised RTSP service is reachable
// -----------------------------------------------------------------------------

pub fn check_stream_service(
    address: &str,
    port: u16,
) -> Result<(), Box<dyn Error>> {
    let socket_address =
        format!(
            "{address}:{port}"
        )
        .parse::<SocketAddr>()?;


    TcpStream::connect_timeout(
        &socket_address,
        Duration::from_secs(2),
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
//
// stderr is piped through PGC so we can:
// - preserve ffplay's raw terminal diagnostics
// - preserve carriage-return live stats
// - detect RTSP transport loss
// -----------------------------------------------------------------------------

pub fn launch_ffplay(
    url: &str,
) -> Result<
    (Child, TransportReceiver),
    Box<dyn Error>,
> {
    let ffplay =
        find_ffplay()?;


    let mut child =
        Command::new(ffplay)

            // Force FFmpeg/ffplay colour output even though stderr is piped.
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
        const FAILURE_TEXT: &[u8] =
            b"Failed reading RTSP data";


        let mut output =
            std::io::stderr();


        let mut buffer =
            [0_u8; 4096];


        // Keeps just enough bytes between reads to detect a failure string
        // split across two OS read operations.
        let mut scan_buffer:
            Vec<u8> = Vec::new();


        let mut loss_reported =
            false;


        loop {
            let bytes_read =
                match ffplay_stderr.read(
                    &mut buffer
                ) {
                    Ok(0) => {
                        break;
                    }


                    Ok(bytes_read) => {
                        bytes_read
                    }


                    Err(error)
                        if error.kind()
                            == ErrorKind::Interrupted =>
                    {
                        continue;
                    }


                    Err(_) => {
                        break;
                    }
                };


            let chunk =
                &buffer[..bytes_read];


            // -------------------------------------------------------------
            // Preserve ffplay's output exactly as emitted.
            //
            // This retains:
            // - ANSI colour codes
            // - carriage returns
            // - live stats updates
            // - native spacing/formatting
            // -------------------------------------------------------------

            let _ =
                output.write_all(
                    chunk
                );


            let _ =
                output.flush();


            // -------------------------------------------------------------
            // Transport monitoring
            //
            // A single explicit RTSP read failure is enough to know that the
            // active RTSP session has been lost.
            // -------------------------------------------------------------

            if loss_reported {
                continue;
            }


            scan_buffer.extend_from_slice(
                chunk
            );


            let failure_found =
                scan_buffer
                    .windows(
                        FAILURE_TEXT.len()
                    )
                    .any(
                        |window| {
                            window
                                == FAILURE_TEXT
                        }
                    );


            if failure_found {
                let _ =
                    transport_tx.send(
                        TransportEvent::Lost
                    );


                loss_reported =
                    true;


                scan_buffer.clear();

                continue;
            }


            // Keep only enough trailing data to detect FAILURE_TEXT if it
            // happens to be divided between two reads.
            let keep =
                FAILURE_TEXT
                    .len()
                    .saturating_sub(1);


            if scan_buffer.len()
                > keep
            {
                let remove =
                    scan_buffer.len()
                        - keep;


                scan_buffer.drain(
                    ..remove
                );
            }
        }
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