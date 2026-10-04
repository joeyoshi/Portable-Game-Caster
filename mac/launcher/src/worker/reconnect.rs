use std::thread;
use std::sync::atomic::{
    AtomicBool,
    Ordering,
};
use std::time::{
    Duration,
    Instant,
};

use crate::discovery;
use crate::logging;
use crate::player;
use crate::state::AppState;


// -----------------------------------------------------------------------------
// Reconnect policy
// -----------------------------------------------------------------------------

const RECONNECT_TIMEOUT:
    Duration =
        Duration::from_secs(15);


const SERVICE_PROBE_TIMEOUT:
    Duration =
        Duration::from_millis(300);


// -----------------------------------------------------------------------------
// Result
// -----------------------------------------------------------------------------

pub enum ReconnectResult {
    HandshakeRestored,

    Failed,

    Cancelled,
}


// -----------------------------------------------------------------------------
// Restore the same host / RTSP service
//
// IMPORTANT:
//
// Reconnecting means:
//
//     "I have lost the connection and am trying to restore the host/service."
//
// Once RTSP is reachable again, this function returns immediately.
//
// Media warm-up is handled separately by worker/mod.rs while the UI displays:
//
//     Waiting for Stream
// -----------------------------------------------------------------------------

pub fn restore_handshake<F>(
    send: &F,
    endpoint: &mut discovery::StreamEndpoint,
    cancelled: &AtomicBool,
) -> ReconnectResult
where
    F: Fn(AppState),
{
    let host =
        endpoint.host.clone();


    let deadline =
        Instant::now()
            + RECONNECT_TIMEOUT;


    logging::debug(
        "RECONNECT",
        format_args!(
            "Starting 15-second handshake recovery window for {host}"
        ),
    );


    let mut last_state:
        Option<(
            bool,
            u8,
        )> =
            None;


    while Instant::now()
        < deadline
    {
        if cancelled.load(Ordering::Relaxed) {
            return ReconnectResult::Cancelled;
        }

        let seconds =
            seconds_remaining(
                deadline
            );


        // ---------------------------------------------------------------------
        // First try the last-known address.
        // ---------------------------------------------------------------------

        if player::check_stream_service_timeout(
            &endpoint.address,
            endpoint.port,
            SERVICE_PROBE_TIMEOUT,
        )
        .is_ok()
        {
            logging::debug(
                "RECONNECT",
                format_args!(
                    "RTSP handshake restored at {}:{}",
                    endpoint.address,
                    endpoint.port
                ),
            );


            return ReconnectResult::HandshakeRestored;
        }


        // ---------------------------------------------------------------------
        // RTSP is unavailable.
        //
        // Determine whether the same host can still be found through mDNS.
        // ---------------------------------------------------------------------

        match discovery::discover_host(
            &host,
            Duration::from_millis(
                300
            ),
            cancelled,
        ) {
            Ok(
                Some(
                    updated_endpoint
                )
            ) => {
                *endpoint =
                    updated_endpoint;


                emit_reconnect_state(
                    send,
                    &host,
                    false,
                    seconds,
                    &mut last_state,
                );


                logging::trace(
                    "RECONNECT",
                    format_args!(
                        "Host present at {}; RTSP not ready yet",
                        endpoint.address
                    ),
                );
            }


            Ok(None) => {
                emit_reconnect_state(
                    send,
                    &host,
                    true,
                    seconds,
                    &mut last_state,
                );
            }


            Err(error) => {
                emit_reconnect_state(
                    send,
                    &host,
                    true,
                    seconds,
                    &mut last_state,
                );


                logging::trace(
                    "RECONNECT",
                    format_args!(
                        "Targeted rediscovery failed: {error}"
                    ),
                );
            }
        }


        thread::sleep(
            Duration::from_millis(
                100
            )
        );
    }


    logging::debug(
        "RECONNECT",
        format_args!(
            "Handshake recovery deadline expired"
        ),
    );


    ReconnectResult::Failed
}


// -----------------------------------------------------------------------------
// Avoid spamming identical state updates/log entries 10 times per second.
// -----------------------------------------------------------------------------

fn emit_reconnect_state<F>(
    send: &F,
    host: &str,
    host_missing: bool,
    seconds: u8,
    last_state: &mut Option<(
        bool,
        u8,
    )>,
)
where
    F: Fn(AppState),
{
    let state_key =
        (
            host_missing,
            seconds,
        );


    if *last_state
        == Some(
            state_key
        )
    {
        return;
    }


    *last_state =
        Some(
            state_key
        );


    if host_missing {
        send(
            AppState::ReconnectingHost {
                host:
                    host.to_string(),

                seconds_remaining:
                    seconds,
            }
        );
    } else {
        send(
            AppState::ReconnectingStream {
                host:
                    host.to_string(),

                seconds_remaining:
                    seconds,
            }
        );
    }
}


// -----------------------------------------------------------------------------
// Human-readable ceiling countdown
// -----------------------------------------------------------------------------

fn seconds_remaining(
    deadline: Instant,
) -> u8 {
    let remaining =
        deadline
            .saturating_duration_since(
                Instant::now()
            );


    let millis =
        remaining
            .as_millis();


    if millis == 0 {
        return 0;
    }


    let seconds =
        (
            millis
                + 999
        )
            / 1000;


    seconds
        .min(15)
        as u8
}
