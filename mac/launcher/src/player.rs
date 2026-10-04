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
// Stream information
// -----------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
    Default,
)]
pub struct StreamInfo {
    pub video_codec:
        Option<String>,

    pub resolution:
        Option<String>,

    pub frame_rate:
        Option<String>,

    pub audio_codec:
        Option<String>,

    pub sample_rate:
        Option<String>,

    pub channels:
        Option<String>,

    pub bitrate:
        Option<String>,
}


// -----------------------------------------------------------------------------
// Events derived from ffplay
// -----------------------------------------------------------------------------

#[derive(Debug)]
pub enum TransportEvent {
    MediaStarted(
        StreamInfo
    ),

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
            "Probing RTSP service at {socket_address} ({timeout:?})"
        ),
    );


    TcpStream::connect_timeout(
        &socket_address,
        timeout,
    )
    .map_err(
        |_| {
            format!(
                "Portable Game Caster was found at {address}, but the streaming service is unavailable."
            )
        }
    )?;


    Ok(())
}


// -----------------------------------------------------------------------------
// Launch ffplay
// -----------------------------------------------------------------------------

pub fn launch_ffplay(
    url: &str,
) -> Result<
    (
        Child,
        TransportReceiver,
    ),
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


    let mut command =
        Command::new(
            &ffplay
        );


    // Only force ANSI colour when we're actually going to display ffplay's
    // raw output.
    if logging::trace_enabled() {
        command.env(
            "AV_LOG_FORCE_COLOR",
            "1",
        );
    }


    let mut child =
        command
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


    thread::spawn(
        move || {
            const FAILURE_TEXT:
                &str =
                    "Failed reading RTSP data";


            let mut terminal =
                std::io::stderr();


            let mut read_buffer =
                [0_u8; 4096];


            let mut parse_buffer:
                Vec<u8> =
                    Vec::new();


            let mut stream_info =
                StreamInfo::default();


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


                // ---------------------------------------------------------
                // Raw ffplay firehose only in verbose mode
                // ---------------------------------------------------------

                if logging::trace_enabled() {
                    let _ =
                        terminal.write_all(
                            chunk
                        );


                    let _ =
                        terminal.flush();
                }


                // ---------------------------------------------------------
                // Parse ffplay's newline + carriage-return output
                // ---------------------------------------------------------

                parse_buffer.extend_from_slice(
                    chunk
                );


                while let Some(separator) =
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
                                ..separator
                            )
                            .collect::<Vec<_>>();


                    if !parse_buffer.is_empty() {
                        parse_buffer.remove(0);
                    }


                    if segment.is_empty() {
                        continue;
                    }


                    let raw_text =
                        String::from_utf8_lossy(
                            &segment
                        );


                    let text =
                        strip_ansi(
                            &raw_text
                        );


                    inspect_stream_metadata(
                        &text,
                        &mut stream_info,
                    );


                    // -----------------------------------------------------
                    // RTSP transport failure
                    // -----------------------------------------------------

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


                    // -----------------------------------------------------
                    // Actual decoded media flow
                    // -----------------------------------------------------

                    if !media_reported
                        && line_confirms_media_flow(
                            &text
                        )
                    {
                        logging::debug(
                            "HEALTH",
                            format_args!(
                                "Confirmed decoded media flow"
                            ),
                        );


                        log_stream_summary(
                            &stream_info
                        );


                        let _ =
                            transport_tx.send(
                                TransportEvent::MediaStarted(
                                    stream_info.clone()
                                )
                            );


                        media_reported =
                            true;
                    }
                }


                // Safety against malformed/no-separator output.
                if parse_buffer.len()
                    > 16 * 1024
                {
                    let keep =
                        4096;


                    let remove =
                        parse_buffer
                            .len()
                            .saturating_sub(
                                keep
                            );


                    parse_buffer.drain(
                        ..remove
                    );
                }
            }


            logging::trace(
                "PLAYER",
                format_args!(
                    "ffplay stderr monitor stopped"
                ),
            );
        }
    );


    Ok(
        (
            child,
            transport_rx,
        )
    )
}


// -----------------------------------------------------------------------------
// Media confirmation
// -----------------------------------------------------------------------------

fn line_confirms_media_flow(
    text: &str,
) -> bool {
    let lower =
        text.to_ascii_lowercase();


    if lower.contains("nan") {
        return false;
    }


    let has_sync =
        text.contains("M-V:")
            || text.contains("A-V:");


    let has_queue =
        text.contains("aq=")
            || text.contains("vq=");


    has_sync
        && has_queue
}


// -----------------------------------------------------------------------------
// Stream metadata parsing
// -----------------------------------------------------------------------------

fn inspect_stream_metadata(
    text: &str,
    info: &mut StreamInfo,
) {
    if text.contains("Video:") {
        if let Some(after) =
            text.split("Video:")
                .nth(1)
        {
            let codec =
                after
                    .split(',')
                    .next()
                    .map(
                        |value| {
                            value
                                .trim()
                                .to_string()
                        }
                    );


            if codec.is_some() {
                info.video_codec =
                    codec;
            }


            if let Some(resolution) =
                find_resolution(
                    after
                )
            {
                info.resolution =
                    Some(resolution);
            }


            if let Some(frame_rate) =
                find_frame_rate(
                    after
                )
            {
                info.frame_rate =
                    Some(frame_rate);
            }
        }
    }


    if text.contains("Audio:") {
        if let Some(after) =
            text.split("Audio:")
                .nth(1)
        {
            info.audio_codec =
                after
                    .split(',')
                    .next()
                    .map(
                        |value| {
                            value
                                .trim()
                                .to_string()
                        }
                    );


            if let Some(sample_rate) =
                find_sample_rate(
                    after
                )
            {
                info.sample_rate =
                    Some(sample_rate);
            }


            if after
                .to_ascii_lowercase()
                .contains(
                    "stereo"
                )
            {
                info.channels =
                    Some(
                        "stereo".into()
                    );
            } else if after
                .to_ascii_lowercase()
                .contains(
                    "mono"
                )
            {
                info.channels =
                    Some(
                        "mono".into()
                    );
            }
        }
    }


    if info.bitrate.is_none() {
        if let Some(bitrate) =
            find_bitrate(
                text
            )
        {
            info.bitrate =
                Some(bitrate);
        }
    }
}


fn find_resolution(
    text: &str,
) -> Option<String> {
    for token in
        text.split_whitespace()
    {
        let cleaned =
            token.trim_matches(
                |character: char| {
                    !character.is_ascii_alphanumeric()
                        && character != 'x'
                }
            );


        let Some(
            (
                width,
                height,
            )
        ) =
            cleaned.split_once('x')
        else {
            continue;
        };


        if !width.is_empty()
            && !height.is_empty()
            && width
                .chars()
                .all(
                    |character| {
                        character.is_ascii_digit()
                    }
                )
            && height
                .chars()
                .all(
                    |character| {
                        character.is_ascii_digit()
                    }
                )
        {
            return Some(
                cleaned.to_string()
            );
        }
    }


    None
}


fn find_frame_rate(
    text: &str,
) -> Option<String> {
    let tokens:
        Vec<&str> =
            text
                .split_whitespace()
                .collect();


    for index in 1..tokens.len() {
        if tokens[index]
            .trim_matches(
                |character: char| {
                    !character.is_ascii_alphabetic()
                }
            )
            .eq_ignore_ascii_case(
                "fps"
            )
        {
            let value =
                tokens[index - 1]
                    .trim_matches(
                        |character: char| {
                            !character.is_ascii_digit()
                                && character != '.'
                        }
                    );


            if !value.is_empty() {
                return Some(
                    format!(
                        "{value} fps"
                    )
                );
            }
        }
    }


    None
}


fn find_sample_rate(
    text: &str,
) -> Option<String> {
    let tokens:
        Vec<&str> =
            text
                .split_whitespace()
                .collect();


    for index in 1..tokens.len() {
        if tokens[index]
            .trim_matches(
                |character: char| {
                    !character.is_ascii_alphabetic()
                }
            )
            .eq_ignore_ascii_case(
                "Hz"
            )
        {
            let value =
                tokens[index - 1]
                    .trim_matches(
                        |character: char| {
                            !character.is_ascii_digit()
                        }
                    );


            if !value.is_empty() {
                return Some(
                    format!(
                        "{value} Hz"
                    )
                );
            }
        }
    }


    None
}


fn find_bitrate(
    text: &str,
) -> Option<String> {
    let marker =
        "bitrate:";


    let lower =
        text.to_ascii_lowercase();


    let position =
        lower.find(
            marker
        )?;


    let remainder =
        text[
            position
                + marker.len()
            ..
        ]
            .trim();


    if remainder
        .to_ascii_lowercase()
        .starts_with(
            "n/a"
        )
    {
        return None;
    }


    let words:
        Vec<&str> =
            remainder
                .split_whitespace()
                .take(2)
                .collect();


    if words.is_empty() {
        return None;
    }


    let value =
        words.join(" ");


    if value
        .to_ascii_lowercase()
        .contains(
            "kb/s"
        )
        || value
            .to_ascii_lowercase()
            .contains(
                "mb/s"
            )
    {
        Some(value)
    } else {
        None
    }
}


// -----------------------------------------------------------------------------
// Debug-mode stream summary
// -----------------------------------------------------------------------------

fn log_stream_summary(
    info: &StreamInfo,
) {
    if !logging::debug_enabled() {
        return;
    }


    let video_codec =
        info.video_codec
            .as_deref()
            .unwrap_or(
                "unknown"
            );


    let resolution =
        info.resolution
            .as_deref()
            .unwrap_or(
                "unknown resolution"
            );


    let frame_rate =
        info.frame_rate
            .as_deref()
            .unwrap_or(
                "unknown fps"
            );


    logging::debug(
        "STREAM",
        format_args!(
            "Video: {video_codec}, {resolution} @ {frame_rate}"
        ),
    );


    let audio_codec =
        info.audio_codec
            .as_deref()
            .unwrap_or(
                "unknown"
            );


    let sample_rate =
        info.sample_rate
            .as_deref()
            .unwrap_or(
                "unknown sample rate"
            );


    let channels =
        info.channels
            .as_deref()
            .unwrap_or(
                "unknown channels"
            );


    logging::debug(
        "STREAM",
        format_args!(
            "Audio: {audio_codec}, {sample_rate}, {channels}"
        ),
    );


    match &info.bitrate {
        Some(bitrate) => {
            logging::debug(
                "STREAM",
                format_args!(
                    "Bitrate: {bitrate}"
                ),
            );
        }


        None => {
            logging::debug(
                "STREAM",
                format_args!(
                    "Bitrate: unavailable from ffplay"
                ),
            );
        }
    }
}


// -----------------------------------------------------------------------------
// Remove ANSI escape sequences before parsing
// -----------------------------------------------------------------------------

fn strip_ansi(
    input: &str,
) -> String {
    let mut output =
        String::with_capacity(
            input.len()
        );


    let mut characters =
        input.chars()
            .peekable();


    while let Some(character) =
        characters.next()
    {
        if character
            != '\x1b'
        {
            output.push(
                character
            );

            continue;
        }


        if characters
            .peek()
            == Some(&'[')
        {
            characters.next();


            while let Some(next) =
                characters.next()
            {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }


    output
}


// -----------------------------------------------------------------------------
// Locate ffplay
// -----------------------------------------------------------------------------

fn find_ffplay(
) -> Result<
    PathBuf,
    Box<dyn Error>,
> {
    if let Ok(path) =
        env::var(
            "PGC_FFPLAY_PATH"
        )
    {
        let path =
            PathBuf::from(
                path
            );


        if path.exists() {
            logging::trace(
                "PLAYER",
                format_args!(
                    "Using PGC_FFPLAY_PATH override"
                ),
            );


            return Ok(
                path
            );
        }
    }


    let home =
        env::var(
            "HOME"
        )
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


    for brew in
        brew_candidates
    {
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
            Command::new(
                &brew
            )
                .arg(
                    "--prefix"
                )
                .arg(
                    "ffmpeg-full"
                )
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
            PathBuf::from(
                prefix
            )
                .join(
                    "bin"
                )
                .join(
                    "ffplay"
                );


        if ffplay.exists() {
            return Ok(
                ffplay
            );
        }
    }


    Err(
        "Could not find ffplay from the Homebrew ffmpeg-full package."
            .into()
    )
}