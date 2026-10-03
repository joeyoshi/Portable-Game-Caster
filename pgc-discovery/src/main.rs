use mdns_sd::{DaemonEvent, ServiceDaemon, ServiceInfo};
use std::env;
use std::net::IpAddr;
use std::thread;

const SERVICE_TYPE: &str = "_pgc._tcp.local.";
const INSTANCE_NAME: &str = "Portable Game Caster";
const PORT: u16 = 8554;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Portable Game Caster Discovery");
    println!("------------------------------");

    // Give each PGC host its own mDNS hostname.
    let computer_name =
        env::var("COMPUTERNAME").unwrap_or_else(|_| "pgc".to_string());

    let safe_name = computer_name
        .to_lowercase()
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "-");

    let host_name = format!("{safe_name}-pgc.local.");

    // mDNS daemon / responder.
    let mdns = ServiceDaemon::new()?;

    // Show useful daemon events while we're prototyping.
    let monitor = mdns.monitor()?;

    thread::spawn(move || {
        while let Ok(event) = monitor.recv() {
            match event {
                DaemonEvent::Announce(_, _) => {
                    println!("[mDNS] Service announced");
                }

                DaemonEvent::IpAdd(ip) => {
                    println!("[mDNS] Address added: {ip}");
                }

                DaemonEvent::IpDel(ip) => {
                    println!("[mDNS] Address removed: {ip}");
                }

                DaemonEvent::NameChange(change) => {
                    println!("[mDNS] Name conflict resolved: {change:?}");
                }

                DaemonEvent::Error(error) => {
                    eprintln!("[mDNS] Error: {error}");
                }

                _ => {}
            }
        }
    });

    let properties = [
        ("protocol", "rtsp"),
        ("path", "/gameplay"),
        ("version", "1"),
    ];

    // Start with no hard-coded addresses.
    // enable_addr_auto() tells mdns-sd to advertise the host's
    // current interface addresses and update them if they change.
    let no_addresses: &[IpAddr] = &[];

    let service = ServiceInfo::new(
        SERVICE_TYPE,
        INSTANCE_NAME,
        &host_name,
        no_addresses,
        PORT,
        &properties[..],
    )?
    .enable_addr_auto();

    mdns.register(service)?;

    println!();
    println!("Advertising:");
    println!("  Name:     {INSTANCE_NAME}");
    println!("  Service:  {SERVICE_TYPE}");
    println!("  Host:     {host_name}");
    println!("  Port:     {PORT}");
    println!("  Protocol: RTSP");
    println!("  Path:     /gameplay");
    println!();
    println!("Press Ctrl+C to stop.");

    // For this first prototype we deliberately keep the helper simple.
    // The eventual Windows service will own its lifecycle.
    loop {
        thread::park();
    }
}