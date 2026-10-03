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
    let mut service_found = false;

    while start.elapsed() < Duration::from_secs(DISCOVERY_TIMEOUT_SECS) {
        match receiver.recv_timeout(Duration::from_millis(500)) {
            Ok(ServiceEvent::ServiceFound(_, _)) => {
                service_found = true;
            }

            Ok(ServiceEvent::ServiceResolved(service)) => {
                service_found = true;

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

                let Some(address) = ipv4_addresses.iter().next() else {
                    // Service exists, but its A record hasn't arrived yet.
                    // Keep listening instead of treating this as a failure.
                    continue;
                };

                let endpoint = StreamEndpoint {
                    host: service.host.trim_end_matches('.').to_string(),
                    address: address.to_string(),
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

    if service_found {
        Err(
            "Portable Game Caster was found, but its network address could not be resolved."
                .into(),
        )
    } else {
        Err(
            "No Portable Game Caster host was found on the local network."
                .into(),
        )
    }
}