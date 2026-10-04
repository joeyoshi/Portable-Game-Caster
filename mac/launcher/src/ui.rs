use std::cell::{Cell, OnceCell, RefCell};
use std::process::Child;
use std::sync::{
    atomic::{
        AtomicBool,
        Ordering,
    },
    mpsc::{self, Receiver},
    Arc,
    Mutex,
};
use std::thread;

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

use crate::logging;
use crate::worker;
use crate::state::AppState;


// -----------------------------------------------------------------------------
// Objective-C instance variables
// -----------------------------------------------------------------------------

struct AppDelegateIvars {
    window: OnceCell<Retained<NSWindow>>,
    host_label: OnceCell<Retained<NSTextField>>,
    status_label: OnceCell<Retained<NSTextField>>,
    detail_label: OnceCell<Retained<NSTextField>>,
    spinner: OnceCell<Retained<NSProgressIndicator>>,

    retry_button: OnceCell<Retained<NSButton>>,
    search_button: OnceCell<Retained<NSButton>>,

    receiver: RefCell<Option<Receiver<AppState>>>,
    worker_running: Cell<bool>,
    cancellation: RefCell<Option<worker::CancellationToken>>,
    primary_action: Cell<PrimaryAction>,

    // Shared ownership of the active ffplay child process.
    //
    // The worker thread launches and monitors it.
    // The AppKit thread can terminate it when the user quits PGC.
    player: Arc<Mutex<Option<Child>>>,
}


#[derive(Clone, Copy)]
enum PrimaryAction {
    Retry,
    Cancel,
    Stop,
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
            logging::action("Quit (window closed)");

            self.quit_application();
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

            logging::action("Retry");

            self.start_worker();
        }


        #[unsafe(method(primaryAction:))]
        fn primary_action(
            &self,
            _sender: &AnyObject,
        ) {
            match self.ivars().primary_action.get() {
                PrimaryAction::Retry => {
                    logging::action("Retry");

                    self.start_worker();
                }

                PrimaryAction::Cancel => {
                    logging::action("Cancel");

                    self.cancel_active_session();
                }

                PrimaryAction::Stop => {
                    logging::action("Stop Stream");

                    self.cancel_active_session();
                }
            }
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

            logging::action("Search");

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
            logging::action("Quit");

            self.quit_application();
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
                host_label: OnceCell::new(),
                status_label: OnceCell::new(),
                detail_label: OnceCell::new(),
                spinner: OnceCell::new(),

                retry_button: OnceCell::new(),
                search_button: OnceCell::new(),

                receiver: RefCell::new(None),
                worker_running: Cell::new(false),
                cancellation: RefCell::new(None),
                primary_action: Cell::new(PrimaryAction::Retry),

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
        // Resolved host identity
        // ---------------------------------------------------------------------

        let host_label =
            NSTextField::labelWithString(
                ns_string!(""),
                mtm,
            );

        host_label.setFrame(
            NSRect::new(
                NSPoint::new(20.0, 151.0),
                NSSize::new(420.0, 20.0),
            )
        );

        host_label.setAlignment(
            NSTextAlignment::Center
        );

        host_label.setHidden(true);

        content.addSubview(&host_label);


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
                NSPoint::new(20.0, 122.0),
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
                NSPoint::new(30.0, 86.0),
                NSSize::new(400.0, 30.0),
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
                Some(sel!(primaryAction:)),
                mtm,
            )
        };

        retry_button.setFrame(
            NSRect::new(
                NSPoint::new(115.0, 15.0),
                NSSize::new(120.0, 32.0),
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
            .host_label
            .set(host_label)
            .expect(
                "host label should only be initialized once"
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
            &AppState::Discovering {
                seconds_remaining: None,
            }
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


        let cancellation =
            Arc::new(
                AtomicBool::new(false)
            );


        *self.ivars()
            .cancellation
            .borrow_mut() =
            Some(
                Arc::clone(&cancellation)
            );


        thread::spawn(move || {
            worker::run_stream_flow(
                tx,
                player_handle,
                cancellation,
            );
        });
    }


    // -------------------------------------------------------------------------
    // Cancel discovery, connection, recovery, or playback by user request.
    // -------------------------------------------------------------------------

    fn cancel_active_session(&self) {
        if let Some(cancellation) = self
            .ivars()
            .cancellation
            .borrow()
            .as_ref()
        {
            cancellation.store(
                true,
                Ordering::Relaxed,
            );
        }


        self.terminate_player();


        // Dropping the receiver invalidates every pending state from the
        // cancelled worker before a later search starts a fresh session.
        self.ivars()
            .receiver
            .borrow_mut()
            .take();


        self.ivars()
            .worker_running
            .set(false);


        // The cancelled worker can no longer report states, so the UI's
        // return to Idle is logged here. The log must end where the UI does.
        logging::state(&AppState::Idle);

        self.render_state(&AppState::Idle);
    }


    // -------------------------------------------------------------------------
    // Quit: stop the session, terminate ffplay, exit
    // -------------------------------------------------------------------------

    fn quit_application(&self) {
        // Stop the worker from reporting or starting anything further.
        if let Some(cancellation) = self
            .ivars()
            .cancellation
            .borrow()
            .as_ref()
        {
            cancellation.store(
                true,
                Ordering::Relaxed,
            );
        }


        self.terminate_player();


        logging::info(
            "APP",
            format_args!(
                "Portable Game Caster Client exiting."
            ),
        );


        let app =
            NSApplication::sharedApplication(self.mtm());

        app.terminate(None);
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
            logging::debug(
                "PLAYER",
                format_args!(
                    "Terminating ffplay PID {}",
                    child.id()
                ),
            );

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
        let host_label = self
            .ivars()
            .host_label
            .get()
            .expect(
                "host label should exist"
            );

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


        if let Some(host) = state.host() {
            host_label.setStringValue(
                &NSString::from_str(
                    &friendly_host_name(host)
                )
            );

            host_label.setHidden(false);
        } else if matches!(
            state,
            AppState::Idle | AppState::Discovering { .. }
        ) {
            host_label.setStringValue(
                ns_string!("")
            );

            host_label.setHidden(true);
        }


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

                retry_button.setTitle(
                    ns_string!("Retry")
                );

                self.ivars()
                    .primary_action
                    .set(PrimaryAction::Retry);

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

            AppState::Playing(_) => {
                status_label.setStringValue(
                    &NSString::from_str(&state.message())
                );

                detail_label.setStringValue(
                    ns_string!(
                        "Portable Game Caster is live."
                    )
                );

                unsafe {
                    spinner.stopAnimation(None);
                }

                retry_button.setTitle(
                    ns_string!("Stop Stream")
                );

                retry_button.setHidden(false);
                search_button.setHidden(true);

                self.ivars()
                    .primary_action
                    .set(PrimaryAction::Stop);
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
                    &NSString::from_str(
                        &state.detail_message().unwrap_or_default()
                    )
                );

                retry_button.setTitle(
                    ns_string!("Cancel")
                );

                retry_button.setHidden(false);
                search_button.setHidden(true);

                self.ivars()
                    .primary_action
                    .set(PrimaryAction::Cancel);

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
// Presentation-only mDNS hostname cleanup
// -----------------------------------------------------------------------------

fn friendly_host_name(
    host: &str,
) -> String {
    let trimmed =
        host.trim_end_matches('.');


    let without_local =
        trimmed
            .strip_suffix(".local")
            .unwrap_or(trimmed);


    let without_pgc =
        without_local
            .strip_suffix("-pgc")
            .unwrap_or(without_local);


    if without_pgc.is_empty() {
        trimmed.to_string()
    } else {
        without_pgc.to_ascii_uppercase()
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
