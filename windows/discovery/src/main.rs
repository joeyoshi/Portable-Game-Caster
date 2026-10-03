use mdns_sd::{DaemonEvent, ServiceDaemon, ServiceInfo};
use std::env;
use std::net::IpAddr;
use std::thread;

const SERVICE_TYPE: &str = "_pgc._tcp.local.";
const SERVICE_INSTANCE: &str = "Portable Game Caster";
const RTSP_PORT: u16 = 8554;
const STREAM_PATH: &str = "/gameplay";
const PROTOCOL_VERSION: &str = "1";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Portable Game Caster - Windows Discovery Advertiser");
    println!("---------------------------------------------------");

    let computer_name =
        env::var("COMPUTERNAME").unwrap_or_else(|_| "pgc".to_string());

    let safe_name = computer_name
        .to_lowercase()
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "-");

    let host_name = format!("{safe_name}-pgc.local.");

    let mdns = ServiceDaemon::new()?;
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
        ("path", STREAM_PATH),
        ("version", PROTOCOL_VERSION),
    ];

    let no_addresses: &[IpAddr] = &[];

    let service = ServiceInfo::new(
        SERVICE_TYPE,
        SERVICE_INSTANCE,
        &host_name,
        no_addresses,
        RTSP_PORT,
        &properties[..],
    )?
    .enable_addr_auto();

    mdns.register(service)?;

    println!();
    println!("Advertising:");
    println!("  Name:     {SERVICE_INSTANCE}");
    println!("  Service:  {SERVICE_TYPE}");
    println!("  Host:     {host_name}");
    println!("  Port:     {RTSP_PORT}");
    println!("  Protocol: RTSP");
    println!("  Path:     {STREAM_PATH}");
    println!("  Version:  {PROTOCOL_VERSION}");
    println!();
    println!("Press Ctrl+C to stop.");

    loop {
        thread::park();
    }
}