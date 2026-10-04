mod reconnect;

use std::process::Child;
use std::sync::{
    mpsc::{
        Sender,
        TryRecvError,
    },
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
        let _ = tx.send(state);
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
    // Launch initial player
    // -------------------------------------------------------------------------

    send(
        AppState::WaitingForStream(
            host.clone()
        )
    );


    let mut transport_rx =
        match launch_player(
            &endpoint,
            &player_handle,
        ) {
            Ok(receiver) => {
                receiver
            }

            Err(error) => {
                send(
                    AppState::Error(
                        error
                    )
                );

                return;
            }
        };


    // -------------------------------------------------------------------------
    // Temporary startup readiness test
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
    // Active playback lifecycle
    // -------------------------------------------------------------------------

    loop {
        thread::sleep(
            Duration::from_millis(250)
        );


        // ---------------------------------------------------------------------
        // Transport-level failure
        //
        // ffplay may remain alive even after RTSP dies, so the stderr monitor
        // reports transport loss separately from child-process status.
        // ---------------------------------------------------------------------

        match transport_rx.try_recv() {
            Ok(
                player::TransportEvent::Lost
            ) => {
                // Leave Connected immediately.
                send(
                    AppState::ReconnectingStream {
                        host:
                            endpoint.host.clone(),

                        seconds_remaining:
                            5,
                    }
                );


                // ffplay is now attached to a dead RTSP session.
                terminate_player(
                    &player_handle
                );


                match reconnect::attempt_reconnect(
                    &send,
                    &mut endpoint,
                    &player_handle,
                ) {
                    reconnect::ReconnectResult::Recovered(
                        new_transport_rx
                    ) => {
                        transport_rx =
                            new_transport_rx;


                        continue;
                    }


                    reconnect::ReconnectResult::Failed => {
                        send(
                            AppState::Idle
                        );


                        return;
                    }
                }
            }


            Err(
                TryRecvError::Empty
            ) => {}


            Err(
                TryRecvError::Disconnected
            ) => {
                // The stderr-monitor thread naturally exits when ffplay exits.
                //
                // Child-process monitoring below remains authoritative for
                // determining what happened to the player.
            }
        }


        // ---------------------------------------------------------------------
        // Child-process lifecycle
        // ---------------------------------------------------------------------

        match poll_player(
            &player_handle
        ) {
            // -------------------------------------------------------------
            // Everything is still running.
            // -------------------------------------------------------------

            PlayerPoll::Running => {}


            // -------------------------------------------------------------
            // ffplay was closed normally by the user.
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
            // ffplay exited abnormally.
            //
            // Treat this as stream loss and attempt recovery.
            // -------------------------------------------------------------

            PlayerPoll::ExitedUnexpectedly
            | PlayerPoll::MonitorError => {
                send(
                    AppState::ReconnectingStream {
                        host:
                            endpoint.host.clone(),

                        seconds_remaining:
                            5,
                    }
                );


                clear_player(
                    &player_handle
                );


                match reconnect::attempt_reconnect(
                    &send,
                    &mut endpoint,
                    &player_handle,
                ) {
                    reconnect::ReconnectResult::Recovered(
                        new_transport_rx
                    ) => {
                        transport_rx =
                            new_transport_rx;


                        continue;
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
            // Usually means AppKit intentionally terminated ffplay because
            // the user quit PGC or closed the PGC window.
            // -------------------------------------------------------------

            PlayerPoll::Missing => {
                return;
            }
        }
    }
}


// -----------------------------------------------------------------------------
// Launch ffplay
//
// Returns the transport receiver associated with this specific player instance.
// -----------------------------------------------------------------------------

pub(crate) fn launch_player(
    endpoint: &discovery::StreamEndpoint,
    player_handle: &SharedPlayer,
) -> Result<
    player::TransportReceiver,
    String,
> {
    let (
        child,
        transport_rx,
    ) =
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


    Ok(
        transport_rx
    )
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
// Poll active ffplay process
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
// Terminate an active stale ffplay instance
//
// Used when transport has died but ffplay itself remains alive.
// -----------------------------------------------------------------------------

fn terminate_player(
    player_handle: &SharedPlayer,
) {
    let mut slot =
        player_handle
            .lock()
            .expect(
                "player process lock poisoned"
            );


    if let Some(child) =
        slot.as_mut()
    {
        let _ =
            child.kill();


        let _ =
            child.wait();
    }


    *slot =
        None;
}


// -----------------------------------------------------------------------------
// Remove finished player from shared ownership
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