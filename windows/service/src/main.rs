mod instance;

use mdns_sd::{ServiceDaemon, ServiceInfo};

use std::env;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
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


// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Portable Game Caster - Windows Host");
    println!("-----------------------------------");


    let _instance_guard =
        match instance::acquire()? {
            Some(guard) => {
                guard
            }


            None => {
                println!(
                    "Portable Game Caster Host is already running."
                );

                eprintln!(
                    "[PGC][DEBUG] Rejected second Windows Host startup because another instance owns the machine-wide mutex."
                );


                return Ok(());
            }
        };

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

    println!(
        "MediaMTX: {}",
        mediamtx_path.display()
    );


    // -------------------------------------------------------------------------
    // Start MediaMTX
    // -------------------------------------------------------------------------

    let mut mediamtx =
        start_mediamtx(
            &mediamtx_path
        )?;

    println!("MediaMTX started.");


    // -------------------------------------------------------------------------
    // Start mDNS advertisement
    // -------------------------------------------------------------------------

    let mdns =
        start_discovery()?;

    println!(
        "Advertising Portable Game Caster on the local network."
    );

    println!();
    println!("Host ready.");
    println!("Press Ctrl+C to stop.");
    println!();


    // -------------------------------------------------------------------------
    // Monitor
    // -------------------------------------------------------------------------

    while running.load(Ordering::SeqCst) {
        match mediamtx.try_wait()? {
            Some(status) => {
                eprintln!(
                    "MediaMTX exited unexpectedly: {status}"
                );

                if !running.load(Ordering::SeqCst) {
                    break;
                }

                eprintln!(
                    "Restarting MediaMTX..."
                );

                thread::sleep(
                    Duration::from_secs(1)
                );

                mediamtx =
                    start_mediamtx(
                        &mediamtx_path
                    )?;

                println!(
                    "MediaMTX restarted."
                );
            }

            None => {}
        }


        thread::sleep(
            Duration::from_millis(500)
        );
    }


    // -------------------------------------------------------------------------
    // Shutdown
    // -------------------------------------------------------------------------

    println!();
    println!("Shutting down Portable Game Caster...");


    if mediamtx.try_wait()?.is_none() {
        let _ = mediamtx.kill();
        let _ = mediamtx.wait();
    }


    let _ = mdns.shutdown();


    println!("Shutdown complete.");

    Ok(())
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
) -> Result<Child, Box<dyn std::error::Error>> {
    let working_directory =
        executable
            .parent()
            .ok_or(
                "MediaMTX path has no parent directory"
            )?;


    let child =
        Command::new(executable)
            .current_dir(
                working_directory
            )
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;


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


    println!(
        "Discovery host: {host_name}"
    );


    Ok(mdns)
}
