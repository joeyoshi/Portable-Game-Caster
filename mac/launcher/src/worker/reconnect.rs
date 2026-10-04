use std::sync::mpsc::{
    TryRecvError,
};
use std::thread;
use std::time::{
    Duration,
    Instant,
};

use crate::discovery;
use crate::logging;
use crate::player;
use crate::state::AppState;

use super::{
    clear_player,
    launch_player,
    poll_player,
    terminate_player,
    PlayerPoll,
    SharedPlayer,
};


// -----------------------------------------------------------------------------
// Reconnect policy
// -----------------------------------------------------------------------------

const RECONNECT_DURATION:
    Duration =
        Duration::from_secs(5);


const FAST_SERVICE_TIMEOUT:
    Duration =
        Duration::from_millis(300);


// -----------------------------------------------------------------------------
// Reconnect result
// -----------------------------------------------------------------------------

pub enum ReconnectResult {
    Recovered(
        player::TransportReceiver
    ),

    Failed,
}


// -----------------------------------------------------------------------------
// Attempt recovery of the SAME host
// -----------------------------------------------------------------------------

pub fn attempt_reconnect<F>(
    send: &F,
    endpoint: &mut discovery::StreamEndpoint,
    player_handle: &SharedPlayer,
) -> ReconnectResult
where
    F: Fn(AppState),
{
    let host =
        endpoint.host.clone();


    let deadline =
        Instant::now()
            + RECONNECT_DURATION;


    logging::debug(
        "RECONNECT",
        format_args!(
            "Starting 5-second recovery window for {host}"
        ),
    );


    while Instant::now()
        < deadline
    {
        let remaining =
            seconds_remaining(
                deadline
            );


        // ---------------------------------------------------------------------
        // Is RTSP already back?
        // ---------------------------------------------------------------------

        if player::check_stream_service_timeout(
            &endpoint.address,
            endpoint.port,
            FAST_SERVICE_TIMEOUT,
        )
        .is_ok()
        {
            send(
                AppState::ReconnectingStream {
                    host:
                        host.clone(),

                    seconds_remaining:
                        remaining,
                }
            );


            logging::debug(
                "RECONNECT",
                format_args!(
                    "RTSP service reachable; launching replacement player"
                ),
            );


            if let Some(receiver) =
                try_restart_player(
                    send,
                    endpoint,
                    player_handle,
                    deadline,
                )
            {
                logging::debug(
                    "RECONNECT",
                    format_args!(
                        "Media recovery successful"
                    ),
                );


                send(
                    AppState::Playing(
                        host.clone()
                    )
                );


                return ReconnectResult::Recovered(
                    receiver
                );
            }
        } else {
            // -----------------------------------------------------------------
            // RTSP unavailable. Check whether this same PGC host is still
            // advertised.
            // -----------------------------------------------------------------

            logging::trace(
                "RECONNECT",
                format_args!(
                    "RTSP unavailable; rediscovering {host}"
                ),
            );


            match discovery::discover_host(
                &host,
                Duration::from_millis(300),
            ) {
                Ok(
                    Some(
                        updated_endpoint
                    )
                ) => {
                    *endpoint =
                        updated_endpoint;


                    send(
                        AppState::ReconnectingStream {
                            host:
                                host.clone(),

                            seconds_remaining:
                                remaining,
                        }
                    );


                    logging::debug(
                        "RECONNECT",
                        format_args!(
                            "Host rediscovered at {} but stream is not ready",
                            endpoint.address
                        ),
                    );
                }


                Ok(None) => {
                    send(
                        AppState::ReconnectingHost {
                            host:
                                host.clone(),

                            seconds_remaining:
                                remaining,
                        }
                    );


                    logging::debug(
                        "RECONNECT",
                        format_args!(
                            "Host is not currently discoverable"
                        ),
                    );
                }


                Err(error) => {
                    send(
                        AppState::ReconnectingHost {
                            host:
                                host.clone(),

                            seconds_remaining:
                                remaining,
                        }
                    );


                    logging::trace(
                        "RECONNECT",
                        format_args!(
                            "Rediscovery error: {error}"
                        ),
                    );
                }
            }
        }


        thread::sleep(
            Duration::from_millis(100)
        );
    }


    logging::debug(
        "RECONNECT",
        format_args!(
            "Recovery deadline expired"
        ),
    );


    ReconnectResult::Failed
}


// -----------------------------------------------------------------------------
// Start a replacement ffplay and REQUIRE actual media before recovery succeeds.
// -----------------------------------------------------------------------------

fn try_restart_player<F>(
    send: &F,
    endpoint: &discovery::StreamEndpoint,
    player_handle: &SharedPlayer,
    deadline: Instant,
) -> Option<
    player::TransportReceiver
>
where
    F: Fn(AppState),
{
    clear_player(
        player_handle
    );


    let receiver =
        match launch_player(
            endpoint,
            player_handle,
        ) {
            Ok(receiver) => {
                receiver
            }


            Err(error) => {
                logging::debug(
                    "RECONNECT",
                    format_args!(
                        "Replacement ffplay failed to launch: {error}"
                    ),
                );


                return None;
            }
        };


    logging::debug(
        "RECONNECT",
        format_args!(
            "Replacement player launched; waiting for media"
        ),
    );


    loop {
        if Instant::now()
            >= deadline
        {
            terminate_player(
                player_handle
            );


            return None;
        }


        let remaining =
            seconds_remaining(
                deadline
            );


        send(
            AppState::ReconnectingStream {
                host:
                    endpoint.host.clone(),

                seconds_remaining:
                    remaining,
            }
        );


        match receiver.try_recv() {
            Ok(
                player::TransportEvent::MediaStarted
            ) => {
                return Some(
                    receiver
                );
            }


            Ok(
                player::TransportEvent::Lost
            ) => {
                logging::debug(
                    "RECONNECT",
                    format_args!(
                        "Replacement RTSP session failed before media began"
                    ),
                );


                terminate_player(
                    player_handle
                );


                return None;
            }


            Err(
                TryRecvError::Empty
            ) => {}


            Err(
                TryRecvError::Disconnected
            ) => {}
        }


        match poll_player(
            player_handle
        ) {
            PlayerPoll::Running => {}


            PlayerPoll::ExitedSuccessfully
            | PlayerPoll::ExitedUnexpectedly
            | PlayerPoll::MonitorError
            | PlayerPoll::Missing => {
                clear_player(
                    player_handle
                );


                return None;
            }
        }


        thread::sleep(
            Duration::from_millis(100)
        );
    }
}


// -----------------------------------------------------------------------------
// Human-readable countdown
//
// Ceiling division means:
// 4.1 sec → 5
// 3.8 sec → 4
// ...
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
        .min(5)
        as u8
}