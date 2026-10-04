mod reconnect;

use std::process::Child;
use std::sync::{
    atomic::{
        AtomicBool,
        Ordering,
    },
    mpsc::{
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


pub type CancellationToken =
    Arc<AtomicBool>;


// -----------------------------------------------------------------------------
// Connection policy
// -----------------------------------------------------------------------------

const MEDIA_WARMUP_GRACE_PERIOD:
    Duration =
        Duration::from_secs(10);


const MEDIA_WARMUP_FALLBACK_TIMEOUT:
    Duration =
        Duration::from_secs(15);


const MEDIA_WARMUP_TIMEOUT:
    Duration =
        Duration::from_secs(
            MEDIA_WARMUP_GRACE_PERIOD.as_secs()
                + MEDIA_WARMUP_FALLBACK_TIMEOUT.as_secs()
        );


const COLD_SERVICE_PROBE_TIMEOUT:
    Duration =
        Duration::from_millis(500);


// -----------------------------------------------------------------------------
// Player monitoring
// -----------------------------------------------------------------------------

enum PlayerPoll {
    Running,

    ExitedSuccessfully,

    ExitedUnexpectedly,

    MonitorError,

    Missing,
}


// -----------------------------------------------------------------------------
// Media warm-up result
// -----------------------------------------------------------------------------

enum MediaWarmupResult {
    Started {
        receiver:
            player::TransportReceiver,

        info:
            player::StreamInfo,
    },

    ServiceLost,

    TimedOut,

    Cancelled,

    UserClosed,

    Missing,
}


// -----------------------------------------------------------------------------
// Main worker lifecycle
// -----------------------------------------------------------------------------

pub fn run_stream_flow(
    tx: Sender<AppState>,
    player_handle: SharedPlayer,
    cancellation: CancellationToken,
) {
    let send =
        |state: AppState| {
            if cancellation.load(Ordering::Relaxed) {
                return;
            }


            logging::state(&state);


            let _ =
                tx.send(
                    state
                );
        };


    // -------------------------------------------------------------------------
    // FRESH / COLD CONNECTION
    //
    // This path never emits Reconnecting states.
    // -------------------------------------------------------------------------

    send(AppState::Discovering {
        seconds_remaining: None,
    });


    let mut emit_search_countdown =
        |seconds_remaining| {
            send(AppState::Discovering {
                seconds_remaining: Some(seconds_remaining),
            });
        };


    let mut endpoint =
        match discovery::discover_stream(
            &cancellation,
            &mut emit_search_countdown,
        ) {
            Ok(endpoint) => {
                endpoint
            }


            Err(error) => {
                if cancellation.load(Ordering::Relaxed) {
                    return;
                }

                send(
                    AppState::Error(
                        error.to_string()
                    )
                );


                return;
            }
        };


    if cancellation.load(Ordering::Relaxed) {
        return;
    }


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
        if cancellation.load(Ordering::Relaxed) {
            return;
        }

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
            "RTSP handshake established"
        ),
    );


    send(
        AppState::WaitingForStream {
            host: host.clone(),
            fallback_seconds_remaining: None,
        }
    );


    let (
        mut transport_rx,
        _initial_info,
    ) =
        match wait_for_media_with_retries(
            &send,
            &endpoint,
            &player_handle,
            MEDIA_WARMUP_TIMEOUT,
            &cancellation,
        ) {
            MediaWarmupResult::Started {
                receiver,
                info,
            } => {
                (
                    receiver,
                    info,
                )
            }


            MediaWarmupResult::TimedOut => {
                send(
                    AppState::Error(
                        format!(
                            "The streaming service at {host} was reachable, but the stream did not begin producing media within 25 seconds."
                        )
                    )
                );


                return;
            }


            MediaWarmupResult::Cancelled => {
                return;
            }


            MediaWarmupResult::ServiceLost => {
                // This was still a cold connection.
                //
                // Do NOT claim we are reconnecting because we never reached
                // Playing in the first place.
                send(
                    AppState::Error(
                        format!(
                            "The streaming service at {host} became unavailable before media started."
                        )
                    )
                );


                return;
            }


            MediaWarmupResult::UserClosed => {
                send(
                    AppState::Idle
                );


                return;
            }


            MediaWarmupResult::Missing => {
                return;
            }
        };


    // Only now is "Connected" truthful.
    send(
        AppState::Playing(
            host.clone()
        )
    );


    // -------------------------------------------------------------------------
    // ACTIVE PLAYBACK
    // -------------------------------------------------------------------------

    loop {
        if cancellation.load(Ordering::Relaxed) {
            return;
        }

        thread::sleep(
            Duration::from_millis(
                100
            )
        );


        // ---------------------------------------------------------------------
        // Transport event from ffplay
        // ---------------------------------------------------------------------

        match transport_rx.try_recv() {
            Ok(
                player::TransportEvent::Lost
            ) => {
                logging::debug(
                    "HEALTH",
                    format_args!(
                        "Active stream transport lost"
                    ),
                );


                terminate_player(
                    &player_handle
                );


                match recover_stream(
                    &send,
                    &mut endpoint,
                    &player_handle,
                    &cancellation,
                ) {
                    Some(
                        new_receiver
                    ) => {
                        transport_rx =
                            new_receiver;


                        continue;
                    }


                    None => {
                        return;
                    }
                }
            }


            Ok(
                player::TransportEvent::MediaStarted(
                    _
                )
            ) => {
                // Already Playing.
            }


            Err(
                TryRecvError::Empty
            ) => {}


            Err(
                TryRecvError::Disconnected
            ) => {}
        }


        // ---------------------------------------------------------------------
        // ffplay lifecycle
        // ---------------------------------------------------------------------

        match poll_player(
            &player_handle
        ) {
            PlayerPoll::Running => {}


            // Manual close of the player window is intentional.
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
                        "ffplay exited unexpectedly during active playback"
                    ),
                );


                clear_player(
                    &player_handle
                );


                match recover_stream(
                    &send,
                    &mut endpoint,
                    &player_handle,
                    &cancellation,
                ) {
                    Some(
                        new_receiver
                    ) => {
                        transport_rx =
                            new_receiver;


                        continue;
                    }


                    None => {
                        return;
                    }
                }
            }


            PlayerPoll::Missing => {
                // Usually AppKit deliberately terminated it during Quit.
                return;
            }
        }
    }
}


// -----------------------------------------------------------------------------
// Recover a stream that was PREVIOUSLY Playing.
//
// This is the only path that is allowed to emit Reconnecting states.
// -----------------------------------------------------------------------------

fn recover_stream<F>(
    send: &F,
    endpoint: &mut discovery::StreamEndpoint,
    player_handle: &SharedPlayer,
    cancellation: &AtomicBool,
) -> Option<
    player::TransportReceiver
>
where
    F: Fn(AppState),
{
    let host =
        endpoint.host.clone();


    loop {
        if cancellation.load(Ordering::Relaxed) {
            return None;
        }

        // ---------------------------------------------------------------------
        // Stage 1:
        //
        // Restore host / RTSP handshake.
        // ---------------------------------------------------------------------

        send(
            AppState::ReconnectingStream {
                host:
                    host.clone(),

                seconds_remaining:
                    15,
            }
        );


        match reconnect::restore_handshake(
            send,
            endpoint,
            cancellation,
        ) {
            reconnect::ReconnectResult::HandshakeRestored => {}


            reconnect::ReconnectResult::Failed => {
                logging::debug(
                    "RECONNECT",
                    format_args!(
                        "Unable to restore connection to {host}"
                    ),
                );


                send(
                    AppState::Idle
                );


                return None;
            }


            reconnect::ReconnectResult::Cancelled => {
                return None;
            }
        }


        // ---------------------------------------------------------------------
        // Stage 2:
        //
        // Handshake is back.
        //
        // We are no longer reconnecting. Now we are simply waiting for actual
        // media to warm up.
        // ---------------------------------------------------------------------

        logging::debug(
            "RECONNECT",
            format_args!(
                "Handshake restored; waiting for media"
            ),
        );


        send(
        AppState::WaitingForStream {
            host: host.clone(),
            fallback_seconds_remaining: None,
        }
        );


        match wait_for_media_with_retries(
            send,
            endpoint,
            player_handle,
            MEDIA_WARMUP_TIMEOUT,
            cancellation,
        ) {
            MediaWarmupResult::Started {
                receiver,
                info: _,
            } => {
                logging::debug(
                    "RECONNECT",
                    format_args!(
                        "Stream recovery complete"
                    ),
                );


                send(
                    AppState::Playing(
                        host.clone()
                    )
                );


                return Some(
                    receiver
                );
            }


            // RTSP disappeared again while warming up.
            //
            // Go back to Stage 1.
            MediaWarmupResult::ServiceLost => {
                logging::debug(
                    "RECONNECT",
                    format_args!(
                        "RTSP service was lost again during media warm-up"
                    ),
                );


                continue;
            }


            MediaWarmupResult::TimedOut => {
                logging::debug(
                    "HEALTH",
                    format_args!(
                        "Handshake was restored, but media never resumed"
                    ),
                );


                send(
                    AppState::Error(
                        format!(
                            "Connection to {host} was restored, but the stream did not begin producing media within 25 seconds."
                        )
                    )
                );


                return None;
            }


            MediaWarmupResult::Cancelled => {
                return None;
            }


            MediaWarmupResult::UserClosed => {
                send(
                    AppState::Idle
                );


                return None;
            }


            MediaWarmupResult::Missing => {
                return None;
            }
        }
    }
}


// -----------------------------------------------------------------------------
// Wait for actual media.
//
// During this phase the UI should already be WaitingForStream.
//
// If ffplay dies while RTSP remains available, we can quietly relaunch it
// without pretending the connection itself was lost.
//
// This is especially useful while runOnDemand / FFmpeg are warming up.
// -----------------------------------------------------------------------------

fn wait_for_media_with_retries<F>(
    send: &F,
    endpoint: &discovery::StreamEndpoint,
    player_handle: &SharedPlayer,
    timeout: Duration,
    cancellation: &AtomicBool,
) -> MediaWarmupResult
where
    F: Fn(AppState),
{
    let warmup_started =
        Instant::now();


    let grace_deadline =
        warmup_started
            + MEDIA_WARMUP_GRACE_PERIOD;


    let deadline =
        warmup_started
            + timeout;


    logging::debug(
        "HEALTH",
        format_args!(
            "Media warm-up beginning for {}; allowing up to {} seconds for confirmed media flow",
            endpoint.host,
            timeout.as_secs()
        ),
    );


    logging::debug(
        "HEALTH",
        format_args!(
            "Starting initial {}-second media warm-up grace period",
            MEDIA_WARMUP_GRACE_PERIOD.as_secs()
        ),
    );


    let mut fallback_started =
        false;


    let mut last_fallback_seconds:
        Option<u8> =
            None;


    while Instant::now()
        < deadline
    {
        if cancellation.load(Ordering::Relaxed) {
            terminate_player(player_handle);
            return MediaWarmupResult::Cancelled;
        }

        update_media_warmup_state(
            send,
            &endpoint.host,
            grace_deadline,
            deadline,
            &mut fallback_started,
            &mut last_fallback_seconds,
        );


        // ---------------------------------------------------------------------
        // Is RTSP still reachable?
        // ---------------------------------------------------------------------

        if player::check_stream_service_timeout(
            &endpoint.address,
            endpoint.port,
            COLD_SERVICE_PROBE_TIMEOUT,
        )
        .is_err()
        {
            terminate_player(
                player_handle
            );


            return MediaWarmupResult::ServiceLost;
        }


        // ---------------------------------------------------------------------
        // Launch a player attempt
        // ---------------------------------------------------------------------

        let receiver =
            match launch_player(
                endpoint,
                player_handle,
                cancellation,
            ) {
                Ok(receiver) => {
                    receiver
                }


                Err(error) => {
                    logging::debug(
                        "PLAYER",
                        format_args!(
                            "ffplay launch attempt failed: {error}"
                        ),
                    );


                    thread::sleep(
                        Duration::from_millis(
                            250
                        )
                    );


                    continue;
                }
            };


        // ---------------------------------------------------------------------
        // Monitor this attempt until:
        //
        // - media starts
        // - transport dies
        // - ffplay exits
        // - overall warm-up deadline expires
        // ---------------------------------------------------------------------

        loop {
            if cancellation.load(Ordering::Relaxed) {
                terminate_player(player_handle);
                return MediaWarmupResult::Cancelled;
            }

            update_media_warmup_state(
                send,
                &endpoint.host,
                grace_deadline,
                deadline,
                &mut fallback_started,
                &mut last_fallback_seconds,
            );


            if Instant::now()
                >= deadline
            {
                terminate_player(
                    player_handle
                );


                log_media_warmup_timeout(
                    warmup_started
                );


                return MediaWarmupResult::TimedOut;
            }


            match receiver.try_recv() {
                Ok(
                    player::TransportEvent::MediaStarted(
                        info
                    )
                ) => {
                    logging::debug(
                        "HEALTH",
                        format_args!(
                            "Media warm-up succeeded after {:.1} seconds",
                            warmup_started.elapsed().as_secs_f64()
                        ),
                    );


                    return MediaWarmupResult::Started {
                        receiver,
                        info,
                    };
                }


                Ok(
                    player::TransportEvent::Lost
                ) => {
                    terminate_player(
                        player_handle
                    );


                    // If MediaMTX itself vanished, this is a real handshake
                    // loss. Otherwise the server is still there and the
                    // source/player is simply not ready yet.
                    if player::check_stream_service_timeout(
                        &endpoint.address,
                        endpoint.port,
                        COLD_SERVICE_PROBE_TIMEOUT,
                    )
                    .is_err()
                    {
                        return MediaWarmupResult::ServiceLost;
                    }


                    logging::debug(
                        "HEALTH",
                        format_args!(
                            "Player lost RTSP during warm-up; service remains reachable, retrying"
                        ),
                    );


                    break;
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
                    clear_player(
                        player_handle
                    );


                    return MediaWarmupResult::UserClosed;
                }


                PlayerPoll::ExitedUnexpectedly
                | PlayerPoll::MonitorError => {
                    clear_player(
                        player_handle
                    );


                    if player::check_stream_service_timeout(
                        &endpoint.address,
                        endpoint.port,
                        COLD_SERVICE_PROBE_TIMEOUT,
                    )
                    .is_err()
                    {
                        return MediaWarmupResult::ServiceLost;
                    }


                    logging::debug(
                        "HEALTH",
                        format_args!(
                            "Player exited before media began; RTSP remains reachable, retrying"
                        ),
                    );


                    break;
                }


                PlayerPoll::Missing => {
                    return MediaWarmupResult::Missing;
                }
            }


            thread::sleep(
                Duration::from_millis(
                    100
                )
            );
        }


        thread::sleep(
            Duration::from_millis(
                250
            )
        );
    }


    log_media_warmup_timeout(
        warmup_started
    );


    MediaWarmupResult::TimedOut
}


// -----------------------------------------------------------------------------
// Media warm-up UI and diagnostics
// -----------------------------------------------------------------------------

fn update_media_warmup_state<F>(
    send: &F,
    host: &str,
    grace_deadline: Instant,
    deadline: Instant,
    fallback_started: &mut bool,
    last_fallback_seconds: &mut Option<u8>,
)
where
    F: Fn(AppState),
{
    let now =
        Instant::now();


    if now < grace_deadline {
        return;
    }


    if !*fallback_started {
        *fallback_started = true;


        logging::debug(
            "HEALTH",
            format_args!(
                "Initial {}-second media warm-up grace period expired",
                MEDIA_WARMUP_GRACE_PERIOD.as_secs()
            ),
        );


        logging::debug(
            "HEALTH",
            format_args!(
                "Starting visible {}-second media warm-up fallback countdown",
                MEDIA_WARMUP_FALLBACK_TIMEOUT.as_secs()
            ),
        );
    }


    let seconds_remaining =
        countdown_seconds_remaining(
            deadline,
            MEDIA_WARMUP_FALLBACK_TIMEOUT.as_secs() as u8,
        );


    if *last_fallback_seconds
        == Some(
            seconds_remaining
        )
    {
        return;
    }


    *last_fallback_seconds =
        Some(
            seconds_remaining
        );


    send(
        AppState::WaitingForStream {
            host: host.to_string(),
            fallback_seconds_remaining: Some(seconds_remaining),
        }
    );
}


fn countdown_seconds_remaining(
    deadline: Instant,
    maximum: u8,
) -> u8 {
    let milliseconds =
        deadline
            .saturating_duration_since(
                Instant::now()
            )
            .as_millis();


    if milliseconds == 0 {
        return 0;
    }


    ((milliseconds + 999) / 1000)
        .min(maximum as u128)
        as u8
}


fn log_media_warmup_timeout(
    warmup_started: Instant,
) {
    logging::debug(
        "HEALTH",
        format_args!(
            "Media warm-up timed out after {:.1} seconds without confirmed media flow",
            warmup_started.elapsed().as_secs_f64()
        ),
    );
}


// -----------------------------------------------------------------------------
// Launch ffplay
// -----------------------------------------------------------------------------

fn launch_player(
    endpoint: &discovery::StreamEndpoint,
    player_handle: &SharedPlayer,
    cancellation: &AtomicBool,
) -> Result<
    player::TransportReceiver,
    String,
> {
    let (
        mut child,
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


    if cancellation.load(Ordering::Relaxed) {
        let _ = child.kill();
        let _ = child.wait();


        return Err(
            "Session was cancelled before ffplay could start."
                .to_string()
        );
    }


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
// Poll ffplay
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
// Kill stale ffplay
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
// Clear completed ffplay
// -----------------------------------------------------------------------------

fn clear_player(
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
