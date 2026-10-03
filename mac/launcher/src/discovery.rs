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


// -----------------------------------------------------------------------------
// Standard discovery
// -----------------------------------------------------------------------------

pub fn discover_stream(
) -> Result<StreamEndpoint, Box<dyn std::error::Error>> {
    match discover_matching(
        None,
        Duration::from_secs(
            DISCOVERY_TIMEOUT_SECS
        ),
    )? {
        Some(endpoint) => {
            Ok(endpoint)
        }

        None => {
            Err(
                "No Portable Game Caster host was found on the local network."
                    .into()
            )
        }
    }
}


// -----------------------------------------------------------------------------
// Reconnect discovery
//
// Searches only for the host we were previously connected to.
// -----------------------------------------------------------------------------

pub fn discover_host(
    expected_host: &str,
    timeout: Duration,
) -> Result<Option<StreamEndpoint>, Box<dyn std::error::Error>> {
    discover_matching(
        Some(expected_host),
        timeout,
    )
}


// -----------------------------------------------------------------------------
// Shared discovery implementation
// -----------------------------------------------------------------------------

fn discover_matching(
    expected_host: Option<&str>,
    timeout: Duration,
) -> Result<Option<StreamEndpoint>, Box<dyn std::error::Error>> {
    let mdns =
        ServiceDaemon::new()?;


    let receiver =
        mdns.browse(
            SERVICE_TYPE
        )?;


    let start =
        Instant::now();


    let mut service_found =
        false;


    while start.elapsed() < timeout {
        match receiver.recv_timeout(
            Duration::from_millis(250)
        ) {
            // -----------------------------------------------------------------
            // We know a service exists, but don't have its full data yet.
            // -----------------------------------------------------------------

            Ok(
                ServiceEvent::ServiceFound(
                    _,
                    _,
                )
            ) => {
                service_found = true;
            }


            // -----------------------------------------------------------------
            // Fully resolved PGC service
            // -----------------------------------------------------------------

            Ok(
                ServiceEvent::ServiceResolved(
                    service
                )
            ) => {
                service_found = true;


                let resolved_host =
                    service
                        .get_hostname()
                        .trim_end_matches('.')
                        .to_string();


                // During reconnect we only accept the host we were already
                // connected to.
                if let Some(expected) =
                    expected_host
                {
                    let expected =
                        expected
                            .trim_end_matches('.');


                    if !resolved_host
                        .eq_ignore_ascii_case(
                            expected
                        )
                    {
                        continue;
                    }
                }


                let protocol =
                    service
                        .get_property_val_str(
                            "protocol"
                        )
                        .unwrap_or(
                            EXPECTED_PROTOCOL
                        );


                if protocol != EXPECTED_PROTOCOL {
                    continue;
                }


                let path =
                    service
                        .get_property_val_str(
                            "path"
                        )
                        .unwrap_or(
                            "/gameplay"
                        );


                let addresses =
                    service
                        .get_addresses_v4();


                let Some(address) =
                    addresses
                        .iter()
                        .next()
                else {
                    // mDNS can resolve the service before its IPv4 record
                    // arrives. Keep listening rather than immediately failing.
                    continue;
                };


                let endpoint =
                    StreamEndpoint {
                        host:
                            resolved_host,

                        address:
                            address.to_string(),

                        port:
                            service.get_port(),

                        protocol:
                            protocol.to_string(),

                        path:
                            path.to_string(),
                    };


                let _ =
                    mdns.stop_browse(
                        SERVICE_TYPE
                    );


                let _ =
                    mdns.shutdown();


                return Ok(
                    Some(endpoint)
                );
            }


            // Ignore other mDNS events.
            Ok(_) => {}


            // recv_timeout simply means nothing arrived during this slice.
            Err(_) => {}
        }
    }


    let _ =
        mdns.stop_browse(
            SERVICE_TYPE
        );


    let _ =
        mdns.shutdown();


    // Initial discovery preserves the useful distinction we already had:
    //
    // "nothing exists"
    //
    // versus
    //
    // "a service appeared but its address never resolved."
    //
    // Reconnect discovery simply reports None so the retry loop can continue.
    if expected_host.is_none()
        && service_found
    {
        return Err(
            "Portable Game Caster was found, but its network address could not be resolved."
                .into()
        );
    }


    Ok(None)
}