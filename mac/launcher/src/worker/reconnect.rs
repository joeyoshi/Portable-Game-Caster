use std::thread;
use std::time::Duration;

use crate::discovery;
use crate::player;
use crate::state::AppState;

use super::{
    clear_player,
    confirm_player_alive,
    launch_player,
    SharedPlayer,
};


// -----------------------------------------------------------------------------
// Reconnect policy
// -----------------------------------------------------------------------------

const RECONNECT_SECONDS: u8 = 5;


// -----------------------------------------------------------------------------
// Result returned to the main worker
// -----------------------------------------------------------------------------

pub enum ReconnectResult {
    Recovered,

    Failed,
}


// -----------------------------------------------------------------------------
// Attempt recovery of the SAME PGC host
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


    for seconds_remaining in
        (1..=RECONNECT_SECONDS).rev()
    {
        // ---------------------------------------------------------------------
        // First test the last-known RTSP endpoint directly.
        //
        // If MediaMTX is already reachable again, we don't need mDNS.
        // ---------------------------------------------------------------------

        if player::check_stream_service(
            &endpoint.address,
            endpoint.port,
        )
        .is_ok()
        {
            send(
                AppState::ReconnectingStream {
                    host:
                        host.clone(),

                    seconds_remaining,
                }
            );


            if try_restart_player(
                endpoint,
                player_handle,
            ) {
                send(
                    AppState::Playing(
                        host.clone()
                    )
                );


                return ReconnectResult::Recovered;
            }
        }


        // ---------------------------------------------------------------------
        // RTSP is not reachable.
        //
        // Now ask mDNS whether the SAME PGC host still exists.
        // ---------------------------------------------------------------------

        let rediscovered =
            discovery::discover_host(
                &host,
                Duration::from_millis(350),
            );


        match rediscovered {
            // -----------------------------------------------------------------
            // Host is still being advertised.
            //
            // This means the PGC host itself exists, but the stream service
            // either died or hasn't recovered yet.
            //
            // This may also give us an updated IP address.
            // -----------------------------------------------------------------

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

                        seconds_remaining,
                    }
                );


                if player::check_stream_service(
                    &endpoint.address,
                    endpoint.port,
                )
                .is_ok()
                    && try_restart_player(
                        endpoint,
                        player_handle,
                    )
                {
                    send(
                        AppState::Playing(
                            host.clone()
                        )
                    );


                    return ReconnectResult::Recovered;
                }
            }


            // -----------------------------------------------------------------
            // The PGC host itself is no longer discoverable.
            // -----------------------------------------------------------------

            Ok(None) => {
                send(
                    AppState::ReconnectingHost {
                        host:
                            host.clone(),

                        seconds_remaining,
                    }
                );
            }


            // -----------------------------------------------------------------
            // Treat transient discovery failure as host unavailable for this
            // retry cycle.
            // -----------------------------------------------------------------

            Err(_) => {
                send(
                    AppState::ReconnectingHost {
                        host:
                            host.clone(),

                        seconds_remaining,
                    }
                );
            }
        }


        // discover_host already consumes part of the second.
        //
        // This sleep keeps the countdown close to one visible update per
        // second without artificially stretching it too far.
        thread::sleep(
            Duration::from_millis(650)
        );
    }


    ReconnectResult::Failed
}


// -----------------------------------------------------------------------------
// Relaunch ffplay and ensure it survives startup
// -----------------------------------------------------------------------------

fn try_restart_player(
    endpoint: &discovery::StreamEndpoint,
    player_handle: &SharedPlayer,
) -> bool {
    clear_player(
        player_handle
    );


    if launch_player(
        endpoint,
        player_handle,
    )
    .is_err()
    {
        return false;
    }


    // Give ffplay enough time to establish its RTSP session before deciding
    // whether recovery succeeded.
    thread::sleep(
        Duration::from_millis(750)
    );


    if confirm_player_alive(
        player_handle
    )
    .is_ok()
    {
        true
    } else {
        clear_player(
            player_handle
        );


        false
    }
}