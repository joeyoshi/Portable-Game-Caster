use std::cell::{Cell, OnceCell, RefCell};
use std::process::Child;
use std::sync::{
    mpsc::{self, Receiver, Sender},
    Arc,
    Mutex,
};
use std::thread;
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{
    define_class,
    msg_send,
    sel,
    DefinedClass,
    MainThreadOnly,
};

use objc2_app_kit::{
    NSApplication,
    NSApplicationActivationPolicy,
    NSApplicationDelegate,
    NSBackingStoreType,
    NSButton,
    NSProgressIndicator,
    NSProgressIndicatorStyle,
    NSTextAlignment,
    NSTextField,
    NSWindow,
    NSWindowDelegate,
    NSWindowStyleMask,
};

use objc2_foundation::{
    ns_string,
    MainThreadMarker,
    NSNotification,
    NSObject,
    NSObjectProtocol,
    NSPoint,
    NSRect,
    NSSize,
    NSString,
    NSTimer,
};

use crate::discovery;
use crate::player;
use crate::state::AppState;


// -----------------------------------------------------------------------------
// Objective-C instance variables
// -----------------------------------------------------------------------------

struct AppDelegateIvars {
    window: OnceCell<Retained<NSWindow>>,
    status_label: OnceCell<Retained<NSTextField>>,
    detail_label: OnceCell<Retained<NSTextField>>,
    spinner: OnceCell<Retained<NSProgressIndicator>>,

    retry_button: OnceCell<Retained<NSButton>>,
    search_button: OnceCell<Retained<NSButton>>,

    receiver: RefCell<Option<Receiver<AppState>>>,
    worker_running: Cell<bool>,

    // Shared ownership of the active ffplay child process.
    //
    // The worker thread launches and monitors it.
    // The AppKit thread can terminate it when the user quits PGC.
    player: Arc<Mutex<Option<Child>>>,
}


// -----------------------------------------------------------------------------
// AppDelegate
// -----------------------------------------------------------------------------

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = AppDelegateIvars]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn application_did_finish_launching(
            &self,
            _notification: &NSNotification,
        ) {
            self.setup_ui();
            self.start_worker();
            self.start_state_timer();
        }
    }

    unsafe impl NSWindowDelegate for AppDelegate {
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(
            &self,
            _notification: &NSNotification,
        ) {
            self.terminate_player();

            let app =
                NSApplication::sharedApplication(self.mtm());

            app.terminate(None);
        }
    }

    impl AppDelegate {
        // ---------------------------------------------------------------------
        // Pull state messages from the worker thread
        // ---------------------------------------------------------------------

        #[unsafe(method(pollState:))]
        fn poll_state(
            &self,
            _timer: &NSTimer,
        ) {
            let states: Vec<AppState> = {
                let receiver =
                    self.ivars().receiver.borrow();

                match receiver.as_ref() {
                    Some(receiver) => {
                        receiver.try_iter().collect()
                    }

                    None => Vec::new(),
                }
            };

            for state in states {
                self.render_state(&state);
            }
        }


        // ---------------------------------------------------------------------
        // Retry after an error
        // ---------------------------------------------------------------------

        #[unsafe(method(retry:))]
        fn retry(
            &self,
            _sender: &AnyObject,
        ) {
            if self.ivars().worker_running.get() {
                return;
            }

            self.start_worker();
        }


        // ---------------------------------------------------------------------
        // Search again after playback has ended normally
        // ---------------------------------------------------------------------

        #[unsafe(method(search:))]
        fn search(
            &self,
            _sender: &AnyObject,
        ) {
            if self.ivars().worker_running.get() {
                return;
            }

            self.start_worker();
        }


        // ---------------------------------------------------------------------
        // Quit PGC and terminate ffplay
        // ---------------------------------------------------------------------

        #[unsafe(method(quit:))]
        fn quit(
            &self,
            _sender: &AnyObject,
        ) {
            self.terminate_player();

            let app =
                NSApplication::sharedApplication(self.mtm());

            app.terminate(None);
        }
    }
);


// -----------------------------------------------------------------------------
// Native UI implementation
// -----------------------------------------------------------------------------

impl AppDelegate {
    fn new(
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this =
            Self::alloc(mtm).set_ivars(AppDelegateIvars {
                window: OnceCell::new(),
                status_label: OnceCell::new(),
                detail_label: OnceCell::new(),
                spinner: OnceCell::new(),

                retry_button: OnceCell::new(),
                search_button: OnceCell::new(),

                receiver: RefCell::new(None),
                worker_running: Cell::new(false),

                player: Arc::new(
                    Mutex::new(None)
                ),
            });

        unsafe {
            msg_send![super(this), init]
        }
    }


    // -------------------------------------------------------------------------
    // Build the native macOS window
    // -------------------------------------------------------------------------

    fn setup_ui(&self) {
        let mtm = self.mtm();

        let app =
            NSApplication::sharedApplication(mtm);


        // ---------------------------------------------------------------------
        // Window
        // ---------------------------------------------------------------------

        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),

                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(460.0, 230.0),
                ),

                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable,

                NSBackingStoreType::Buffered,

                false,
            )
        };

        unsafe {
            window.setReleasedWhenClosed(false);
        }

        window.setTitle(
            ns_string!("Portable Game Caster")
        );

        window.center();

        window.setDelegate(
            Some(
                ProtocolObject::from_ref(self)
            )
        );


        let content = window
            .contentView()
            .expect(
                "PGC window should have a content view"
            );


        // ---------------------------------------------------------------------
        // Title
        // ---------------------------------------------------------------------

        let title_label =
            NSTextField::labelWithString(
                ns_string!("Portable Game Caster"),
                mtm,
            );

        title_label.setFrame(
            NSRect::new(
                NSPoint::new(20.0, 175.0),
                NSSize::new(420.0, 30.0),
            )
        );

        title_label.setAlignment(
            NSTextAlignment::Center
        );

        content.addSubview(&title_label);


        // ---------------------------------------------------------------------
        // Primary status
        // ---------------------------------------------------------------------

        let status_label =
            NSTextField::labelWithString(
                ns_string!("Ready"),
                mtm,
            );

        status_label.setFrame(
            NSRect::new(
                NSPoint::new(20.0, 130.0),
                NSSize::new(420.0, 26.0),
            )
        );

        status_label.setAlignment(
            NSTextAlignment::Center
        );

        content.addSubview(&status_label);


        // ---------------------------------------------------------------------
        // Detail text
        // ---------------------------------------------------------------------

        let detail_label =
            NSTextField::wrappingLabelWithString(
                ns_string!(""),
                mtm,
            );

        detail_label.setFrame(
            NSRect::new(
                NSPoint::new(30.0, 82.0),
                NSSize::new(400.0, 42.0),
            )
        );

        detail_label.setAlignment(
            NSTextAlignment::Center
        );

        content.addSubview(&detail_label);


        // ---------------------------------------------------------------------
        // Spinner
        // ---------------------------------------------------------------------

        let spinner =
            NSProgressIndicator::new(mtm);

        spinner.setFrame(
            NSRect::new(
                NSPoint::new(215.0, 55.0),
                NSSize::new(30.0, 30.0),
            )
        );

        spinner.setIndeterminate(true);

        spinner.setStyle(
            NSProgressIndicatorStyle::Spinning
        );

        spinner.setDisplayedWhenStopped(false);

        content.addSubview(&spinner);


        // ---------------------------------------------------------------------
        // Retry button
        //
        // Only visible after an error.
        // ---------------------------------------------------------------------

        let retry_button = unsafe {
            NSButton::buttonWithTitle_target_action(
                ns_string!("Retry"),
                Some(self),
                Some(sel!(retry:)),
                mtm,
            )
        };

        retry_button.setFrame(
            NSRect::new(
                NSPoint::new(140.0, 15.0),
                NSSize::new(80.0, 32.0),
            )
        );

        retry_button.setHidden(true);

        content.addSubview(&retry_button);


        // ---------------------------------------------------------------------
        // Search button
        //
        // Visible while Idle after playback ends.
        // ---------------------------------------------------------------------

        let search_button = unsafe {
            NSButton::buttonWithTitle_target_action(
                ns_string!("Search for Host"),
                Some(self),
                Some(sel!(search:)),
                mtm,
            )
        };

        search_button.setFrame(
            NSRect::new(
                NSPoint::new(115.0, 15.0),
                NSSize::new(120.0, 32.0),
            )
        );

        search_button.setHidden(true);

        content.addSubview(&search_button);


        // ---------------------------------------------------------------------
        // Quit button
        // ---------------------------------------------------------------------

        let quit_button = unsafe {
            NSButton::buttonWithTitle_target_action(
                ns_string!("Quit"),
                Some(self),
                Some(sel!(quit:)),
                mtm,
            )
        };

        quit_button.setFrame(
            NSRect::new(
                NSPoint::new(245.0, 15.0),
                NSSize::new(80.0, 32.0),
            )
        );

        content.addSubview(&quit_button);


        // ---------------------------------------------------------------------
        // Retain UI objects used after setup
        // ---------------------------------------------------------------------

        self.ivars()
            .window
            .set(window.clone())
            .expect(
                "window should only be initialized once"
            );

        self.ivars()
            .status_label
            .set(status_label)
            .expect(
                "status label should only be initialized once"
            );

        self.ivars()
            .detail_label
            .set(detail_label)
            .expect(
                "detail label should only be initialized once"
            );

        self.ivars()
            .spinner
            .set(spinner)
            .expect(
                "spinner should only be initialized once"
            );

        self.ivars()
            .retry_button
            .set(retry_button)
            .expect(
                "retry button should only be initialized once"
            );

        self.ivars()
            .search_button
            .set(search_button)
            .expect(
                "search button should only be initialized once"
            );


        // ---------------------------------------------------------------------
        // Show application
        // ---------------------------------------------------------------------

        window.makeKeyAndOrderFront(None);

        app.setActivationPolicy(
            NSApplicationActivationPolicy::Regular
        );

        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }


    // -------------------------------------------------------------------------
    // Poll the worker state channel from the AppKit run loop
    // -------------------------------------------------------------------------

    fn start_state_timer(&self) {
        unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                0.1,
                self,
                sel!(pollState:),
                None,
                true,
            );
        }
    }


    // -------------------------------------------------------------------------
    // Start a new discovery / playback worker
    // -------------------------------------------------------------------------

    fn start_worker(&self) {
        if self.ivars().worker_running.get() {
            return;
        }

        self.ivars()
            .worker_running
            .set(true);

        self.render_state(
            &AppState::Discovering
        );


        let (tx, rx) =
            mpsc::channel::<AppState>();

        *self.ivars()
            .receiver
            .borrow_mut() = Some(rx);


        let player_handle =
            Arc::clone(
                &self.ivars().player
            );


        thread::spawn(move || {
            run_stream_flow(
                tx,
                player_handle,
            );
        });
    }


    // -------------------------------------------------------------------------
    // Kill the active ffplay process if one exists
    // -------------------------------------------------------------------------

    fn terminate_player(&self) {
        let mut slot = self
            .ivars()
            .player
            .lock()
            .expect(
                "player process lock poisoned"
            );


        if let Some(child) = slot.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }


        *slot = None;
    }


    // -------------------------------------------------------------------------
    // Render an AppState
    // -------------------------------------------------------------------------

    fn render_state(
        &self,
        state: &AppState,
    ) {
        let status_label = self
            .ivars()
            .status_label
            .get()
            .expect(
                "status label should exist"
            );

        let detail_label = self
            .ivars()
            .detail_label
            .get()
            .expect(
                "detail label should exist"
            );

        let spinner = self
            .ivars()
            .spinner
            .get()
            .expect(
                "spinner should exist"
            );

        let retry_button = self
            .ivars()
            .retry_button
            .get()
            .expect(
                "retry button should exist"
            );

        let search_button = self
            .ivars()
            .search_button
            .get()
            .expect(
                "search button should exist"
            );


        match state {
            // -----------------------------------------------------------------
            // Error
            // -----------------------------------------------------------------

            AppState::Error(error) => {
                status_label.setStringValue(
                    ns_string!("Unable to connect")
                );

                detail_label.setStringValue(
                    &NSString::from_str(error)
                );

                unsafe  {
                    spinner.stopAnimation(None);
                }

                retry_button.setHidden(false);
                search_button.setHidden(true);

                self.ivars()
                    .worker_running
                    .set(false);
            }


            // -----------------------------------------------------------------
            // Idle
            // -----------------------------------------------------------------

            AppState::Idle => {
                status_label.setStringValue(
                    ns_string!("Ready")
                );

                detail_label.setStringValue(
                    ns_string!("No active stream.")
                );

                unsafe {
                    spinner.stopAnimation(None);
                }
        

                retry_button.setHidden(true);
                search_button.setHidden(false);

                self.ivars()
                    .worker_running
                    .set(false);
            }


            // -----------------------------------------------------------------
            // Playing
            // -----------------------------------------------------------------

            AppState::Playing(host) => {
                status_label.setStringValue(
                    &NSString::from_str(
                        &format!(
                            "Connected to {host}"
                        )
                    )
                );

                detail_label.setStringValue(
                    ns_string!(
                        "Portable Game Caster is live."
                    )
                );

                unsafe {
                    spinner.stopAnimation(None);
                }

                retry_button.setHidden(true);
                search_button.setHidden(true);
            }


            // -----------------------------------------------------------------
            // Busy / transitional states
            // -----------------------------------------------------------------

            _ => {
                let message =
                    state.message();

                status_label.setStringValue(
                    &NSString::from_str(
                        &message
                    )
                );

                detail_label.setStringValue(
                    ns_string!("")
                );

                retry_button.setHidden(true);
                search_button.setHidden(true);

                if state.is_busy() {
                    unsafe {
                        spinner.startAnimation(None);
                    }
                } else {
                    unsafe {
                        spinner.stopAnimation(None);
                    }
                }
            }
        }
    }
}


// -----------------------------------------------------------------------------
// Background discovery / playback flow
// -----------------------------------------------------------------------------

fn run_stream_flow(
    tx: Sender<AppState>,
    player_handle: Arc<Mutex<Option<Child>>>,
) {
    let send = |state: AppState| {
        let _ = tx.send(state);
    };


    // -------------------------------------------------------------------------
    // Discover PGC
    // -------------------------------------------------------------------------

    send(AppState::Discovering);

    let endpoint =
        match discovery::discover_stream() {
            Ok(endpoint) => endpoint,

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
    // Host resolved
    // -------------------------------------------------------------------------

    send(
        AppState::Resolving(
            host.clone()
        )
    );


    // -------------------------------------------------------------------------
    // Confirm MediaMTX is reachable
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
    // Launch ffplay
    // -------------------------------------------------------------------------

    send(
        AppState::WaitingForStream(
            host.clone()
        )
    );


    let child =
        match player::launch_ffplay(
            &endpoint.url()
        ) {
            Ok(child) => child,

            Err(error) => {
                send(
                    AppState::Error(
                        error.to_string()
                    )
                );

                return;
            }
        };


    // Store ffplay where the AppKit thread can terminate it.
    {
        let mut slot =
            player_handle
                .lock()
                .expect(
                    "player process lock poisoned"
                );

        *slot = Some(child);
    }


    // -------------------------------------------------------------------------
    // Temporary stream readiness test
    //
    // For now:
    //
    // MediaMTX reachable
    // +
    // ffplay survives one second
    //
    // Later this will be replaced by real host/capture health data.
    // -------------------------------------------------------------------------

    thread::sleep(
        Duration::from_secs(1)
    );


    {
        let mut slot =
            player_handle
                .lock()
                .expect(
                    "player process lock poisoned"
                );


        let Some(child) =
            slot.as_mut()
        else {
            // Player was terminated externally.
            return;
        };


        match child.try_wait() {
            Ok(Some(status)) => {
                *slot = None;

                send(
                    AppState::Error(
                        format!(
                            "Player exited before the stream started ({status})."
                        )
                    )
                );

                return;
            }


            Ok(None) => {}


            Err(error) => {
                *slot = None;

                send(
                    AppState::Error(
                        format!(
                            "Could not monitor player: {error}"
                        )
                    )
                );

                return;
            }
        }
    }


    // -------------------------------------------------------------------------
    // Playback is considered active
    // -------------------------------------------------------------------------

    send(
        AppState::Playing(
            host
        )
    );


    // -------------------------------------------------------------------------
    // Monitor ffplay until it closes
    // -------------------------------------------------------------------------

    loop {
        thread::sleep(
            Duration::from_millis(250)
        );


        let result = {
            let mut slot =
                player_handle
                    .lock()
                    .expect(
                        "player process lock poisoned"
                    );


            let Some(child) =
                slot.as_mut()
            else {
                // Player was terminated by the UI.
                return;
            };


            child.try_wait()
        };


        match result {
            // -----------------------------------------------------------------
            // ffplay exited
            // -----------------------------------------------------------------

            Ok(Some(status)) => {
                {
                    let mut slot =
                        player_handle
                            .lock()
                            .expect(
                                "player process lock poisoned"
                            );

                    *slot = None;
                }


                if status.success() {
                    send(
                        AppState::Idle
                    );
                } else {
                    send(
                        AppState::Error(
                            format!(
                                "Player exited with status {status}."
                            )
                        )
                    );
                }


                return;
            }


            // -----------------------------------------------------------------
            // ffplay still running
            // -----------------------------------------------------------------

            Ok(None) => {}


            // -----------------------------------------------------------------
            // Monitoring error
            // -----------------------------------------------------------------

            Err(error) => {
                {
                    let mut slot =
                        player_handle
                            .lock()
                            .expect(
                                "player process lock poisoned"
                            );

                    *slot = None;
                }


                send(
                    AppState::Error(
                        format!(
                            "Player error: {error}"
                        )
                    )
                );


                return;
            }
        }
    }
}


// -----------------------------------------------------------------------------
// Application entry point
// -----------------------------------------------------------------------------

pub fn run_app() {
    let mtm =
        MainThreadMarker::new()
            .expect(
                "Portable Game Caster must start on the macOS main thread"
            );


    let app =
        NSApplication::sharedApplication(
            mtm
        );


    let delegate =
        AppDelegate::new(
            mtm
        );


    app.setDelegate(
        Some(
            ProtocolObject::from_ref(
                &*delegate
            )
        )
    );


    app.run();
}