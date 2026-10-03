use std::env;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Child, Command};
use std::time::Duration;

pub fn check_stream_service(
    address: &str,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket: SocketAddr = format!("{}:{}", address, port).parse()?;

    TcpStream::connect_timeout(
        &socket,
        Duration::from_secs(2),
    )
    .map(|_| ())
    .map_err(|_| {
        format!(
            "Portable Game Caster was found at {}, but the streaming service is unavailable.",
            socket
        )
        .into()
    })
}

pub fn launch_ffplay(
    url: &str,
) -> Result<Child, Box<dyn std::error::Error>> {
    let ffplay_path = find_ffplay()?;

    let child = Command::new(&ffplay_path)
        .args([
            "-rtsp_transport",
            "tcp",
            "-fflags",
            "nobuffer",
            "-flags",
            "low_delay",
            "-noinfbuf",
            "-framedrop",
            "-sync",
            "ext",
            "-probesize",
            "2M",
            "-analyzeduration",
            "500000",
            "-max_delay",
            "0",
            "-stats",
        ])
        .arg(url)
        .spawn()?;

    Ok(child)
}

fn find_ffplay() -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(path) = env::var("PGC_FFPLAY_PATH") {
        if Path::new(&path).exists() {
            return Ok(path);
        }
    }

    let home = env::var("HOME").unwrap_or_default();

    let brew_candidates = [
        format!("{}/homebrew/bin/brew", home),
        "/opt/homebrew/bin/brew".to_string(),
        "/usr/local/bin/brew".to_string(),
    ];

    for brew in brew_candidates {
        if !Path::new(&brew).exists() {
            continue;
        }

        let output = Command::new(&brew)
            .args(["--prefix", "ffmpeg-full"])
            .output()?;

        if output.status.success() {
            let prefix = String::from_utf8(output.stdout)?
                .trim()
                .to_string();

            let ffplay = format!("{}/bin/ffplay", prefix);

            if Path::new(&ffplay).exists() {
                return Ok(ffplay);
            }
        }
    }

    Err(
        "Could not locate ffplay. Install ffmpeg-full with Homebrew or set PGC_FFPLAY_PATH."
            .into(),
    )
}