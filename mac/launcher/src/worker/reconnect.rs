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
// Reconnect result
//
// A recovered connection includes the transport monitor belonging to the new
// ffplay process.
// -----------------------------------------------------------------------------

pub enum ReconnectResult {
    Recovered(
        player::TransportReceiver
    ),

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
        // First try the last-known RTSP service.
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


            if let Some(
                transport_rx
            ) =
                try_restart_player(
                    endpoint,
                    player_handle,
                )
            {
                send(
                    AppState::Playing(
                        host.clone()
                    )
                );


                return ReconnectResult::Recovered(
                    transport_rx
                );
            }
        }


        // ---------------------------------------------------------------------
        // RTSP isn't reachable.
        //
        // Ask mDNS whether the SAME host is still advertised.
        // ---------------------------------------------------------------------

        let rediscovered =
            discovery::discover_host(
                &host,
                Duration::from_millis(350),
            );


        match rediscovered {
            // -----------------------------------------------------------------
            // Host exists, stream is unavailable/recovering.
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
                {
                    if let Some(
                        transport_rx
                    ) =
                        try_restart_player(
                            endpoint,
                            player_handle,
                        )
                    {
                        send(
                            AppState::Playing(
                                host.clone()
                            )
                        );


                        return ReconnectResult::Recovered(
                            transport_rx
                        );
                    }
                }
            }


            // -----------------------------------------------------------------
            // Same host isn't currently discoverable.
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
            // Discovery itself failed during this attempt.
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


        thread::sleep(
            Duration::from_millis(650)
        );
    }


    ReconnectResult::Failed
}


// -----------------------------------------------------------------------------
// Relaunch ffplay
//
// Success returns the new process's transport receiver.
// -----------------------------------------------------------------------------

fn try_restart_player(
    endpoint: &discovery::StreamEndpoint,
    player_handle: &SharedPlayer,
) -> Option<
    player::TransportReceiver
> {
    clear_player(
        player_handle
    );


    let transport_rx =
        match launch_player(
            endpoint,
            player_handle,
        ) {
            Ok(receiver) => {
                receiver
            }

            Err(_) => {
                return None;
            }
        };


    thread::sleep(
        Duration::from_millis(750)
    );


    if confirm_player_alive(
        player_handle
    )
    .is_ok()
    {
        Some(
            transport_rx
        )
    } else {
        clear_player(
            player_handle
        );


        None
    }
}