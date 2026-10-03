use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::time::{Duration, Instant};

const SERVICE_TYPE: &str = "_pgc._tcp.local.";
const EXPECTED_PROTOCOL: &str = "rtsp";
const DISCOVERY_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone)]
pub struct StreamEndpoint {
    pub host: String,
    pub address: String,
    pub port: u16,
    pub protocol: String,
    pub path: String,
}

impl StreamEndpoint {
    pub fn url(&self) -> String {
        format!(
            "{}://{}:{}{}",
            self.protocol,
            self.address,
            self.port,
            self.path
        )
    }
}

pub fn discover_stream() -> Result<StreamEndpoint, Box<dyn std::error::Error>> {
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
                    continue;
                }

                let ipv4_addresses = service.get_addresses_v4();

                let address = ipv4_addresses
                    .iter()
                    .next()
                    .ok_or(
                        "Portable Game Caster was found, but no IPv4 address was resolved."
                    )?
                    .to_string();

                let endpoint = StreamEndpoint {
                    host: service.host.trim_end_matches('.').to_string(),
                    address,
                    port: service.port,
                    protocol: protocol.to_string(),
                    path: path.to_string(),
                };

                let _ = mdns.stop_browse(SERVICE_TYPE);
                let _ = mdns.shutdown();

                return Ok(endpoint);
            }

            Ok(_) => {}

            Err(_) => {}
        }
    }

    let _ = mdns.stop_browse(SERVICE_TYPE);
    let _ = mdns.shutdown();

    Err("No Portable Game Caster host found on the local network.".into())
}