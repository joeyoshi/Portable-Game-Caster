mod reconnect;

use std::process::Child;
use std::sync::{
    mpsc::Sender,
    Arc,
    Mutex,
};
use std::thread;
use std::time::Duration;

use crate::discovery;
use crate::player;
use crate::state::AppState;


pub type SharedPlayer =
    Arc<Mutex<Option<Child>>>;


// -----------------------------------------------------------------------------
// Player monitoring result
// -----------------------------------------------------------------------------

enum PlayerPoll {
    Running,

    ExitedSuccessfully,

    ExitedUnexpectedly,

    MonitorError,

    Missing,
}


// -----------------------------------------------------------------------------
// Main worker lifecycle
// -----------------------------------------------------------------------------

pub fn run_stream_flow(
    tx: Sender<AppState>,
    player_handle: SharedPlayer,
) {
    let send = |state: AppState| {
        let _ =
            tx.send(state);
    };


    // -------------------------------------------------------------------------
    // Discover host
    // -------------------------------------------------------------------------

    send(
        AppState::Discovering
    );


    let mut endpoint =
        match discovery::discover_stream() {
            Ok(endpoint) => {
                endpoint
            }

            Err(error) => {
                send(
                    AppState::Error(
                        error.to_string()
                    )
                );

                return;
            }
        };


    let host =
        endpoint.host.clone();


    // -------------------------------------------------------------------------
    // Resolve
    // -------------------------------------------------------------------------

    send(
        AppState::Resolving(
            host.clone()
        )
    );


    // -------------------------------------------------------------------------
    // Connect to MediaMTX
    // -------------------------------------------------------------------------

    send(
        AppState::Connecting(
            host.clone()
        )
    );


    if let Err(error) =
        player::check_stream_service(
            &endpoint.address,
            endpoint.port,
        )
    {
        send(
            AppState::Error(
                error.to_string()
            )
        );

        return;
    }


    // -------------------------------------------------------------------------
    // Launch player
    // -------------------------------------------------------------------------

    send(
        AppState::WaitingForStream(
            host.clone()
        )
    );


    if let Err(error) =
        launch_player(
            &endpoint,
            &player_handle,
        )
    {
        send(
            AppState::Error(
                error
            )
        );

        return;
    }


    // -------------------------------------------------------------------------
    // Temporary readiness test
    //
    // This is still our current approximation:
    //
    // MediaMTX reachable
    // +
    // ffplay survives startup
    //
    // Eventually host status will replace this.
    // -------------------------------------------------------------------------

    thread::sleep(
        Duration::from_secs(1)
    );


    if let Err(error) =
        confirm_player_alive(
            &player_handle
        )
    {
        send(
            AppState::Error(
                error
            )
        );

        return;
    }


    send(
        AppState::Playing(
            host.clone()
        )
    );


    // -------------------------------------------------------------------------
    // Active playback monitor
    // -------------------------------------------------------------------------

    loop {
        thread::sleep(
            Duration::from_millis(250)
        );


        match poll_player(
            &player_handle
        ) {
            // -------------------------------------------------------------
            // Everything is healthy.
            // -------------------------------------------------------------

            PlayerPoll::Running => {}


            // -------------------------------------------------------------
            // ffplay was closed normally by the user.
            //
            // Return to Idle and expose Search for Host.
            // -------------------------------------------------------------

            PlayerPoll::ExitedSuccessfully => {
                clear_player(
                    &player_handle
                );


                send(
                    AppState::Idle
                );


                return;
            }


            // -------------------------------------------------------------
            // Playback died unexpectedly.
            //
            // Enter reconnect logic.
            // -------------------------------------------------------------

            PlayerPoll::ExitedUnexpectedly
            | PlayerPoll::MonitorError => {
                clear_player(
                    &player_handle
                );


                match reconnect::attempt_reconnect(
                    &send,
                    &mut endpoint,
                    &player_handle,
                ) {
                    reconnect::ReconnectResult::Recovered => {
                        // Successfully relaunched playback.
                        //
                        // Return to the normal player-monitor loop.
                    }


                    reconnect::ReconnectResult::Failed => {
                        send(
                            AppState::Idle
                        );


                        return;
                    }
                }
            }


            // -------------------------------------------------------------
            // No child exists.
            //
            // Usually means AppKit deliberately killed ffplay during Quit.
            // -------------------------------------------------------------

            PlayerPoll::Missing => {
                return;
            }
        }
    }
}


// -----------------------------------------------------------------------------
// Launch ffplay and store its Child handle
// -----------------------------------------------------------------------------

pub(crate) fn launch_player(
    endpoint: &discovery::StreamEndpoint,
    player_handle: &SharedPlayer,
) -> Result<(), String> {
    let child =
        player::launch_ffplay(
            &endpoint.url()
        )
        .map_err(
            |error| {
                error.to_string()
            }
        )?;


    let mut slot =
        player_handle
            .lock()
            .expect(
                "player process lock poisoned"
            );


    *slot =
        Some(child);


    Ok(())
}


// -----------------------------------------------------------------------------
// Confirm ffplay survived startup
// -----------------------------------------------------------------------------

pub(crate) fn confirm_player_alive(
    player_handle: &SharedPlayer,
) -> Result<(), String> {
    let mut slot =
        player_handle
            .lock()
            .expect(
                "player process lock poisoned"
            );


    let Some(child) =
        slot.as_mut()
    else {
        return Err(
            "Player process disappeared during startup."
                .into()
        );
    };


    match child.try_wait() {
        Ok(Some(status)) => {
            *slot =
                None;


            Err(
                format!(
                    "Player exited before the stream started ({status})."
                )
            )
        }


        Ok(None) => {
            Ok(())
        }


        Err(error) => {
            *slot =
                None;


            Err(
                format!(
                    "Could not monitor player: {error}"
                )
            )
        }
    }
}


// -----------------------------------------------------------------------------
// Check active player
// -----------------------------------------------------------------------------

fn poll_player(
    player_handle: &SharedPlayer,
) -> PlayerPoll {
    let mut slot =
        player_handle
            .lock()
            .expect(
                "player process lock poisoned"
            );


    let Some(child) =
        slot.as_mut()
    else {
        return PlayerPoll::Missing;
    };


    match child.try_wait() {
        Ok(None) => {
            PlayerPoll::Running
        }


        Ok(Some(status))
            if status.success() =>
        {
            PlayerPoll::ExitedSuccessfully
        }


        Ok(Some(_)) => {
            PlayerPoll::ExitedUnexpectedly
        }


        Err(_) => {
            PlayerPoll::MonitorError
        }
    }
}


// -----------------------------------------------------------------------------
// Remove finished player from the shared slot
// -----------------------------------------------------------------------------

pub(crate) fn clear_player(
    player_handle: &SharedPlayer,
) {
    let mut slot =
        player_handle
            .lock()
            .expect(
                "player process lock poisoned"
            );


    *slot =
        None;
}