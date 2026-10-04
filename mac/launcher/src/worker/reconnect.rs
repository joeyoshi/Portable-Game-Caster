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

use super::countdown;


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
    F: Fn(AppState) + Sync,
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


    let host_missing =
        AtomicBool::new(false);


    let countdown_stopped =
        AtomicBool::new(false);


    // The visible countdown runs on its own thread so its pacing follows the
    // deadline rather than the duration of each probe/rediscovery iteration.
    thread::scope(
        |scope| {
            let ticker =
                scope.spawn(
                    || {
                        // recover_stream has already shown the full countdown.
                        let mut last_state =
                            Some(
                                (
                                    false,
                                    RECONNECT_TIMEOUT.as_secs() as u8,
                                )
                            );


                        countdown::run(
                            Instant::now(),
                            deadline,
                            RECONNECT_TIMEOUT.as_secs() as u8,
                            &countdown_stopped,
                            |seconds| {
                                emit_reconnect_state(
                                    send,
                                    &host,
                                    host_missing.load(Ordering::Relaxed),
                                    seconds,
                                    &mut last_state,
                                );
                            },
                        );
                    }
                );


            // Reflect a host-present/host-missing change immediately instead
            // of waiting for the next whole-second tick.
            let set_host_missing =
                |missing: bool| {
                    if host_missing.swap(
                        missing,
                        Ordering::Relaxed,
                    ) != missing
                    {
                        ticker.thread().unpark();
                    }
                };


            let result =
                probe_until_restored(
                    endpoint,
                    &host,
                    deadline,
                    cancelled,
                    &set_host_missing,
                );


            countdown_stopped.store(
                true,
                Ordering::Relaxed,
            );

            ticker.thread().unpark();


            result
        }
    )
}


fn probe_until_restored(
    endpoint: &mut discovery::StreamEndpoint,
    host: &str,
    deadline: Instant,
    cancelled: &AtomicBool,
    set_host_missing: &dyn Fn(bool),
) -> ReconnectResult {
    while Instant::now()
        < deadline
    {
        if cancelled.load(Ordering::Relaxed) {
            return ReconnectResult::Cancelled;
        }


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
            host,
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


                set_host_missing(false);


                logging::trace(
                    "RECONNECT",
                    format_args!(
                        "Host present at {}; RTSP not ready yet",
                        endpoint.address
                    ),
                );
            }


            Ok(None) => {
                set_host_missing(true);
            }


            Err(error) => {
                set_host_missing(true);


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
