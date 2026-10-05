use mdns_sd::{
    ServiceDaemon,
    ServiceEvent,
};
use std::time::{
    Duration,
    Instant,
};
use std::sync::atomic::{
    AtomicBool,
    Ordering,
};

use crate::logging;


// -----------------------------------------------------------------------------
// PGC discovery protocol
// -----------------------------------------------------------------------------

const SERVICE_TYPE: &str =
    "_pgc._tcp.local.";

const EXPECTED_PROTOCOL: &str =
    "rtsp";

const DEFAULT_STREAM_PATH: &str =
    "/gameplay";

const DISCOVERY_TIMEOUT_SECS: u64 =
    10;


// -----------------------------------------------------------------------------
// Resolved stream endpoint
// -----------------------------------------------------------------------------

#[derive(
    Debug,
    Clone,
)]
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
//
// Finds any available Portable Game Caster host.
// -----------------------------------------------------------------------------

pub fn discover_stream(
    cancelled: &AtomicBool,
    on_countdown: &mut dyn FnMut(u8),
) -> Result<
    StreamEndpoint,
    Box<dyn std::error::Error>,
> {
    logging::debug(
        "DISCOVERY",
        format_args!(
            "Starting Portable Game Caster host discovery"
        ),
    );


    match discover_matching(
        None,
        Duration::from_secs(
            DISCOVERY_TIMEOUT_SECS,
        ),
        cancelled,
        Some(on_countdown),
    )? {
        Some(endpoint) => {
            Ok(endpoint)
        }


        None => {
            logging::debug(
                "DISCOVERY",
                format_args!(
                    "No Portable Game Caster host discovered before timeout"
                ),
            );


            Err(
                "No Portable Game Caster host was found on the local network."
                    .into()
            )
        }
    }
}


// -----------------------------------------------------------------------------
// Targeted rediscovery
//
// Used during reconnect.
//
// Searches only for the exact host we were previously connected to so that
// reconnecting can never silently switch to a different PGC machine.
// -----------------------------------------------------------------------------

pub fn discover_host(
    expected_host: &str,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<
    Option<StreamEndpoint>,
    Box<dyn std::error::Error>,
> {
    logging::verbose(
        "DISCOVERY",
        format_args!(
            "Searching specifically for host {expected_host}"
        ),
    );


    discover_matching(
        Some(expected_host),
        timeout,
        cancelled,
        None,
    )
}


// -----------------------------------------------------------------------------
// Shared discovery implementation
// -----------------------------------------------------------------------------

fn discover_matching(
    expected_host: Option<&str>,
    timeout: Duration,
    cancelled: &AtomicBool,
    mut on_countdown: Option<&mut dyn FnMut(u8)>,
) -> Result<
    Option<StreamEndpoint>,
    Box<dyn std::error::Error>,
> {
    let mdns =
        ServiceDaemon::new()?;


    logging::verbose(
        "DISCOVERY",
        format_args!(
            "Starting mDNS browse for {SERVICE_TYPE}"
        ),
    );


    let receiver =
        mdns.browse(
            SERVICE_TYPE
        )?;


    let started =
        Instant::now();


    let mut service_found =
        false;


    let countdown_start =
        timeout / 2;


    let mut last_countdown:
        Option<u8> =
            None;


    while started.elapsed()
        < timeout
    {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }


        let mut poll_slice =
            Duration::from_millis(250);


        if started.elapsed() >= countdown_start {
            // Wake at the next whole-second boundary so the visible countdown
            // is not delayed by the polling slice below.
            let milliseconds =
                timeout
                    .saturating_sub(started.elapsed())
                    .as_millis();


            poll_slice =
                poll_slice.min(
                    Duration::from_millis(
                        ((milliseconds + 999) % 1000 + 1) as u64
                    )
                );


            let seconds_remaining =
                ((milliseconds + 999) / 1000) as u8;


            if last_countdown != Some(seconds_remaining) {
                last_countdown = Some(seconds_remaining);


                if let Some(callback) = on_countdown.as_mut() {
                    callback(seconds_remaining);
                }
            }
        }


        match receiver.recv_timeout(
            poll_slice
        ) {
            // -----------------------------------------------------------------
            // A PGC service was announced.
            //
            // At this point we don't necessarily have its hostname/IP yet.
            // -----------------------------------------------------------------

            Ok(
                ServiceEvent::ServiceFound(
                    _service_type,
                    fullname,
                )
            ) => {
                service_found =
                    true;


                logging::verbose(
                    "DISCOVERY",
                    format_args!(
                        "mDNS service found: {fullname}"
                    ),
                );
            }


            // -----------------------------------------------------------------
            // Full service resolution
            // -----------------------------------------------------------------

            Ok(
                ServiceEvent::ServiceResolved(
                    service
                )
            ) => {
                service_found =
                    true;


                let resolved_host =
                    service
                        .get_hostname()
                        .trim_end_matches('.')
                        .to_string();


                logging::verbose(
                    "DISCOVERY",
                    format_args!(
                        "Resolved mDNS candidate host: {resolved_host}"
                    ),
                );


                // -------------------------------------------------------------
                // Targeted reconnect must stay on the SAME host.
                // -------------------------------------------------------------

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
                        logging::verbose(
                            "DISCOVERY",
                            format_args!(
                                "Ignoring host {resolved_host}; expected {expected}"
                            ),
                        );


                        continue;
                    }
                }


                // -------------------------------------------------------------
                // Validate protocol
                // -------------------------------------------------------------

                let protocol =
                    service
                        .get_property_val_str(
                            "protocol"
                        )
                        .unwrap_or(
                            EXPECTED_PROTOCOL
                        );


                if !protocol
                    .eq_ignore_ascii_case(
                        EXPECTED_PROTOCOL
                    )
                {
                    logging::verbose(
                        "DISCOVERY",
                        format_args!(
                            "Ignoring {resolved_host}; unsupported protocol {protocol}"
                        ),
                    );


                    continue;
                }


                // -------------------------------------------------------------
                // Stream path
                // -------------------------------------------------------------

                let path =
                    service
                        .get_property_val_str(
                            "path"
                        )
                        .unwrap_or(
                            DEFAULT_STREAM_PATH
                        );


                // -------------------------------------------------------------
                // IPv4 resolution
                //
                // mDNS can emit ServiceResolved before the IPv4 address record
                // has reached us. This is a known transient case, so do not
                // fail immediately.
                // -------------------------------------------------------------

                let addresses =
                    service
                        .get_addresses_v4();


                let Some(address) =
                    addresses
                        .iter()
                        .next()
                else {
                    logging::verbose(
                        "DISCOVERY",
                        format_args!(
                            "Host {resolved_host} resolved without IPv4 address; continuing to listen"
                        ),
                    );


                    continue;
                };


                // -------------------------------------------------------------
                // Build endpoint
                // -------------------------------------------------------------

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


                // -------------------------------------------------------------
                // Human-readable diagnostic output
                // -------------------------------------------------------------

                logging::debug(
                    "DISCOVERY",
                    format_args!(
                        "Resolved Portable Game Caster host"
                    ),
                );


                logging::debug(
                    "DISCOVERY",
                    format_args!(
                        "Hostname: {}",
                        endpoint.host
                    ),
                );


                logging::debug(
                    "DISCOVERY",
                    format_args!(
                        "Address: {}",
                        endpoint.address
                    ),
                );


                logging::debug(
                    "DISCOVERY",
                    format_args!(
                        "Protocol: {}",
                        endpoint.protocol
                    ),
                );


                logging::debug(
                    "DISCOVERY",
                    format_args!(
                        "Port: {}",
                        endpoint.port
                    ),
                );


                logging::debug(
                    "DISCOVERY",
                    format_args!(
                        "Path: {}",
                        endpoint.path
                    ),
                );


                logging::debug(
                    "DISCOVERY",
                    format_args!(
                        "URL: {}",
                        endpoint.url()
                    ),
                );


                // -------------------------------------------------------------
                // Clean up browser before returning.
                // -------------------------------------------------------------

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


            // -----------------------------------------------------------------
            // Other mDNS lifecycle events aren't currently needed.
            // -----------------------------------------------------------------

            Ok(event) => {
                logging::verbose(
                    "DISCOVERY",
                    format_args!(
                        "Ignoring mDNS event: {event:?}"
                    ),
                );
            }


            // -----------------------------------------------------------------
            // recv_timeout only means nothing arrived during this polling slice.
            //
            // Keep listening until our overall deadline expires.
            // -----------------------------------------------------------------

            Err(_) => {}
        }
    }


    // -------------------------------------------------------------------------
    // Discovery timed out
    // -------------------------------------------------------------------------

    let _ =
        mdns.stop_browse(
            SERVICE_TYPE
        );


    let _ =
        mdns.shutdown();


    // -------------------------------------------------------------------------
    // Initial discovery gets a more descriptive resolution error.
    //
    // During targeted reconnect, None is preferable because reconnect.rs owns
    // the retry/deadline behavior.
    // -------------------------------------------------------------------------

    if expected_host.is_none()
        && service_found
    {
        logging::debug(
            "DISCOVERY",
            format_args!(
                "Portable Game Caster service was seen, but its network address did not resolve before timeout"
            ),
        );


        return Err(
            "Portable Game Caster was found, but its network address could not be resolved."
                .into()
        );
    }


    if let Some(expected) =
        expected_host
    {
        logging::verbose(
            "DISCOVERY",
            format_args!(
                "Host {expected} was not rediscovered before timeout"
            ),
        );
    }


    Ok(None)
}
