mod console;
mod demand;
mod ffmpeg;
mod instance;
mod job;
mod logging;

use mdns_sd::{ServiceDaemon, ServiceInfo};

use std::collections::HashSet;
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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


// MediaMTX logs this when it accepts a publisher on the gameplay path.
const PUBLISHER_READY_TEXT: &str = "is publishing to path 'gameplay'";

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
}


// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // MediaMTX runs this executable as its runOnDemand command. In that mode it
    // is only a demand signal, not a Host.
    if env::args().any(|argument| argument == demand::SIGNAL_FLAG) {
        return demand::run_signal();
    }


    console::widen();

    logging::set_colour(
        console::enable_ansi()
    );

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
    // Find FFmpeg
    // -------------------------------------------------------------------------

    let ffmpeg_path =
        ffmpeg::find_ffmpeg()?;

    println!(
        "FFmpeg: {}",
        ffmpeg_path.display()
    );


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

    logging::host(format_args!(
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

    println!("MediaMTX started.");

    logging::host(format_args!(
        "MediaMTX started (PID {}).",
        mediamtx.id()
    ));


    // -------------------------------------------------------------------------
    // Start mDNS advertisement
    // -------------------------------------------------------------------------

    let mdns =
        start_discovery()?;

    println!(
        "Advertising Portable Game Caster on the local network."
    );

    println!();
    println!("Host ready. FFmpeg starts when a viewer connects.");
    println!("Press Ctrl+C to stop.");
    println!();


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

                    if !was_active {
                        logging::host(format_args!(
                            "Demand became active (signal #{id})."
                        ));
                    }
                }

                Event::DemandClosed(id) => {
                    if demand_signals.remove(&id)
                        && demand_signals.is_empty()
                    {
                        logging::host(format_args!(
                            "Demand became inactive (signal #{id} ended)."
                        ));

                        encoder.demand_ended();
                    }
                }

                Event::PublisherReady => {
                    encoder.publisher_ready();
                }
            }

            next_event =
                events_rx.try_recv().ok();
        }


        // ---------------------------------------------------------------------
        // MediaMTX
        // ---------------------------------------------------------------------

        if let Some(status) = mediamtx.try_wait()? {
            eprintln!(
                "MediaMTX exited unexpectedly: {status}"
            );

            logging::host(format_args!(
                "MediaMTX PID {} exited: {status}",
                mediamtx.id()
            ));

            if !running.load(Ordering::SeqCst) {
                break;
            }

            // FFmpeg was publishing to the MediaMTX that just died, and any
            // demand it signalled died with it. Stop FFmpeg now; a fresh one
            // starts when the new MediaMTX reports demand.
            if !demand_signals.is_empty() {
                logging::host(format_args!(
                    "Demand cleared (MediaMTX exited)."
                ));
            }

            demand_listener.reset();
            demand_signals.clear();

            encoder.stop("MediaMTX exited", false);

            eprintln!(
                "Restarting MediaMTX..."
            );

            thread::sleep(
                Duration::from_secs(1)
            );

            logging::host(format_args!(
                "Starting MediaMTX."
            ));

            mediamtx =
                match start_mediamtx(
                    &mediamtx_path,
                    demand_address,
                    &events_tx,
                    &job,
                ) {
                    Ok(child) => child,

                    Err(error) => {
                        logging::host(format_args!(
                            "MediaMTX restart failed: {error}"
                        ));

                        return Err(error);
                    }
                };

            println!(
                "MediaMTX restarted."
            );

            logging::host(format_args!(
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

    println!();
    println!("Shutting down Portable Game Caster...");


    encoder.stop("Host shutdown", true);


    if mediamtx.try_wait()?.is_none() {
        let _ = mediamtx.kill();
        let _ = mediamtx.wait();
    }

    logging::host(format_args!(
        "MediaMTX PID {} stopped.",
        mediamtx.id()
    ));


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


    // Output is relayed rather than inherited so every MediaMTX line carries
    // the same UTC timestamp format as the Host and Client logs.
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

        logging::relay_output(stdout, false, "MTX", move |line| {
            if line.contains(PUBLISHER_READY_TEXT) {
                let _ = events.send(Event::PublisherReady);
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        logging::relay_output(stderr, true, "MTX", |_| {});
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


    println!(
        "Discovery host: {host_name}"
    );


    Ok(mdns)
}
