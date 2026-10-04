mod reconnect;

use std::process::Child;
use std::sync::{
    mpsc::{
        Receiver,
        Sender,
        TryRecvError,
    },
    Arc,
    Mutex,
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


pub type SharedPlayer =
    Arc<Mutex<Option<Child>>>;


const INITIAL_MEDIA_TIMEOUT:
    Duration =
        Duration::from_secs(10);


// -----------------------------------------------------------------------------
// Player status
// -----------------------------------------------------------------------------

pub(crate) enum PlayerPoll {
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
    let send =
        |state: AppState| {
            logging::state(
                &state
            );

            let _ =
                tx.send(state);
        };


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


    send(
        AppState::Resolving(
            host.clone()
        )
    );


    send(
        AppState::Connecting(
            host.clone()
        )
    );


    logging::debug(
        "CONNECT",
        format_args!(
            "Checking RTSP service at {}:{}",
            endpoint.address,
            endpoint.port
        ),
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


    logging::debug(
        "CONNECT",
        format_args!(
            "RTSP service reachable"
        ),
    );


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


    logging::debug(
        "HEALTH",
        format_args!(
            "Waiting for confirmed media flow"
        ),
    );


    match wait_for_media_start(
        &transport_rx,
        &player_handle,
        Instant::now()
            + INITIAL_MEDIA_TIMEOUT,
    ) {
        MediaStartResult::Started => {
            send(
                AppState::Playing(
                    host.clone()
                )
            );
        }


        MediaStartResult::ClosedNormally => {
            clear_player(
                &player_handle
            );

            send(
                AppState::Idle
            );

            return;
        }


        MediaStartResult::Lost
        | MediaStartResult::Exited
        | MediaStartResult::MonitorError => {
            terminate_player(
                &player_handle
            );


            send(
                AppState::ReconnectingStream {
                    host:
                        host.clone(),

                    seconds_remaining:
                        5,
                }
            );


            match reconnect::attempt_reconnect(
                &send,
                &mut endpoint,
                &player_handle,
            ) {
                reconnect::ReconnectResult::Recovered(
                    receiver
                ) => {
                    transport_rx =
                        receiver;
                }


                reconnect::ReconnectResult::Failed => {
                    send(
                        AppState::Idle
                    );

                    return;
                }
            }
        }


        MediaStartResult::TimedOut => {
            logging::debug(
                "HEALTH",
                format_args!(
                    "ffplay started but no media flow was confirmed within 10 seconds"
                ),
            );


            terminate_player(
                &player_handle
            );


            send(
                AppState::Error(
                    format!(
                        "Connected to {host}, but the stream did not begin producing media."
                    )
                )
            );


            return;
        }


        MediaStartResult::Missing => {
            return;
        }
    }


    // -------------------------------------------------------------------------
    // Active playback lifecycle
    // -------------------------------------------------------------------------

    loop {
        thread::sleep(
            Duration::from_millis(100)
        );


        // ---------------------------------------------------------------------
        // ffplay-derived transport/media events
        // ---------------------------------------------------------------------

        match transport_rx.try_recv() {
            Ok(
                player::TransportEvent::Lost
            ) => {
                send(
                    AppState::ReconnectingStream {
                        host:
                            endpoint.host.clone(),

                        seconds_remaining:
                            5,
                    }
                );


                terminate_player(
                    &player_handle
                );


                match reconnect::attempt_reconnect(
                    &send,
                    &mut endpoint,
                    &player_handle,
                ) {
                    reconnect::ReconnectResult::Recovered(
                        receiver
                    ) => {
                        transport_rx =
                            receiver;

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


            Ok(
                player::TransportEvent::MediaStarted
            ) => {
                // This event belongs to a stream already marked Playing.
                // No state change is needed.
            }


            Err(
                TryRecvError::Empty
            ) => {}


            Err(
                TryRecvError::Disconnected
            ) => {}
        }


        // ---------------------------------------------------------------------
        // Child-process lifecycle
        // ---------------------------------------------------------------------

        match poll_player(
            &player_handle
        ) {
            PlayerPoll::Running => {}


            PlayerPoll::ExitedSuccessfully => {
                logging::debug(
                    "PLAYER",
                    format_args!(
                        "ffplay closed normally"
                    ),
                );


                clear_player(
                    &player_handle
                );


                send(
                    AppState::Idle
                );


                return;
            }


            PlayerPoll::ExitedUnexpectedly
            | PlayerPoll::MonitorError => {
                logging::debug(
                    "PLAYER",
                    format_args!(
                        "ffplay ended unexpectedly"
                    ),
                );


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
                        receiver
                    ) => {
                        transport_rx =
                            receiver;

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


            PlayerPoll::Missing => {
                return;
            }
        }
    }
}


// -----------------------------------------------------------------------------
// Media-start result
// -----------------------------------------------------------------------------

pub(crate) enum MediaStartResult {
    Started,

    Lost,

    ClosedNormally,

    Exited,

    MonitorError,

    Missing,

    TimedOut,
}


// -----------------------------------------------------------------------------
// Wait until actual decoded media begins
// -----------------------------------------------------------------------------

pub(crate) fn wait_for_media_start(
    transport_rx: &Receiver<
        player::TransportEvent
    >,
    player_handle: &SharedPlayer,
    deadline: Instant,
) -> MediaStartResult {
    loop {
        if Instant::now()
            >= deadline
        {
            return MediaStartResult::TimedOut;
        }


        match transport_rx.try_recv() {
            Ok(
                player::TransportEvent::MediaStarted
            ) => {
                return MediaStartResult::Started;
            }


            Ok(
                player::TransportEvent::Lost
            ) => {
                return MediaStartResult::Lost;
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


            PlayerPoll::ExitedSuccessfully => {
                return MediaStartResult::ClosedNormally;
            }


            PlayerPoll::ExitedUnexpectedly => {
                return MediaStartResult::Exited;
            }


            PlayerPoll::MonitorError => {
                return MediaStartResult::MonitorError;
            }


            PlayerPoll::Missing => {
                return MediaStartResult::Missing;
            }
        }


        thread::sleep(
            Duration::from_millis(100)
        );
    }
}


// -----------------------------------------------------------------------------
// Launch player
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
// Poll player
// -----------------------------------------------------------------------------

pub(crate) fn poll_player(
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
// Kill stale player
// -----------------------------------------------------------------------------

pub(crate) fn terminate_player(
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
        logging::debug(
            "PLAYER",
            format_args!(
                "Terminating ffplay PID {}",
                child.id()
            ),
        );


        let _ =
            child.kill();


        let _ =
            child.wait();
    }


    *slot =
        None;
}


// -----------------------------------------------------------------------------
// Clear finished player
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