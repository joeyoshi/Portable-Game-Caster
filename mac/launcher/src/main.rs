use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::process::Command;
use std::time::{Duration, Instant};

const SERVICE_TYPE: &str = "_pgc._tcp.local.";
const EXPECTED_PROTOCOL: &str = "rtsp";
const DISCOVERY_TIMEOUT_SECS: u64 = 10;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Portable Game Caster - macOS Launcher");
    println!("-------------------------------------");
    println!("Searching for Portable Game Caster...");

    let mdns = ServiceDaemon::new()?;
    let receiver = mdns.browse(SERVICE_TYPE)?;

    let start = Instant::now();

    while start.elapsed() < Duration::from_secs(DISCOVERY_TIMEOUT_SECS) {
        match receiver.recv_timeout(Duration::from_millis(500)) {
            Ok(ServiceEvent::ServiceResolved(service)) => {
                let protocol = service
                    .txt_properties
                    .iter()
                    .find(|property| property.key() == "protocol")
                    .map(|property| property.val_str())
                    .unwrap_or(EXPECTED_PROTOCOL);

                let path = service
                    .txt_properties
                    .iter()
                    .find(|property| property.key() == "path")
                    .map(|property| property.val_str())
                    .unwrap_or("/gameplay");

                if protocol != EXPECTED_PROTOCOL {
                    eprintln!(
                        "Unsupported PGC protocol advertised: {}",
                        protocol
                    );
                    continue;
                }

                let port = service.port;

                let ipv4_addresses = service.get_addresses_v4();

                let address = ipv4_addresses
                    .iter()
                    .next()
                    .ok_or("Portable Game Caster was found, but no IPv4 address was resolved.")?;

                let url = format!(
                    "{}://{}:{}{}",
                    protocol,
                    address,
                    port,
                    path
                );

                println!("Found Portable Game Caster:");
                println!("  Host: {}", service.host.trim_end_matches('.'));
                println!("  IPv4 addresses: {:?}", service.get_addresses_v4());
                println!("  Port: {}", port);
                println!("  Path: {}", path);
                println!("  URL:  {}", url);
                println!();
                println!("Launching stream...");

                mdns.stop_browse(SERVICE_TYPE)?;
                mdns.shutdown()?;

                launch_ffplay(&url)?;

                return Ok(());
            }

            Ok(_) => {}

            Err(_) => {}
        }
    }

    mdns.stop_browse(SERVICE_TYPE)?;
    mdns.shutdown()?;

    Err("No Portable Game Caster host found on the local network.".into())
}

fn launch_ffplay(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ffplay_path = find_ffplay()?;

    let status = Command::new("caffeinate")
        .arg("-i")
        .arg(ffplay_path)
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
        .status()?;

    if !status.success() {
        return Err(format!(
            "ffplay exited with status: {}",
            status
        )
        .into());
    }

    Ok(())
}

fn find_ffplay() -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("brew")
        .args(["--prefix", "ffmpeg-full"])
        .output()?;

    if !output.status.success() {
        return Err(
            "Could not locate ffmpeg-full through Homebrew.".into()
        );
    }

    let prefix = String::from_utf8(output.stdout)?
        .trim()
        .to_string();

    Ok(format!("{}/bin/ffplay", prefix))
}