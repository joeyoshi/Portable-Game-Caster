// -----------------------------------------------------------------------------
// Console presentation
// -----------------------------------------------------------------------------
//
// Timestamped Host, MediaMTX, and FFmpeg lines wrap badly at the default 120
// columns. When attached to a normal console, widen it. Best effort: every
// failure is ignored, and hosts that manage their own window size (for example
// Windows Terminal) may ignore the request.
//
// Colour is only used when both output streams are a console that accepts ANSI
// escape sequences.
//
// The console keeps its own mouse behaviour: QuickEdit selection, copy, and
// wheel scrolling are left exactly as the user has them. A selection makes the
// console block writes to it; the Host stays unaffected because terminal
// output goes through a queue that never waits (see logging.rs).
//
// The one input flag the Host changes is processed input. With it on, the
// system turns Ctrl+C into a signal sent to the Host and to the MediaMTX and
// FFmpeg processes sharing its console. With it off, Ctrl+C is an ordinary key
// (and still copies a selection, which the console window handles itself), and
// the Host quits on Ctrl+Q instead (see hotkeys.rs).

const ENABLE_PROCESSED_INPUT: u32 =
    0x0001;


// The console input mode while the Host takes key commands: processed input
// off, every other flag (QuickEdit, mouse, window, insert) exactly as found.
#[cfg_attr(not(windows), allow(dead_code))]
fn input_mode_for_key_commands(mode: u32) -> u32 {
    mode & !ENABLE_PROCESSED_INPUT
}


#[cfg(windows)]
mod platform {
    const TARGET_COLUMNS: i16 =
        140;

    const STD_INPUT_HANDLE: u32 =
        -10_i32 as u32;

    const STD_OUTPUT_HANDLE: u32 =
        -11_i32 as u32;

    const STD_ERROR_HANDLE: u32 =
        -12_i32 as u32;

    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 =
        0x0004;


    type Handle =
        *mut std::ffi::c_void;


    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Coord {
        x: i16,
        y: i16,
    }


    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }


    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct ScreenBufferInfo {
        size: Coord,
        cursor_position: Coord,
        attributes: u16,
        window: SmallRect,
        maximum_window_size: Coord,
    }


    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(
            std_handle: u32,
        ) -> Handle;

        fn GetConsoleMode(
            handle: Handle,
            mode: *mut u32,
        ) -> i32;

        fn SetConsoleMode(
            handle: Handle,
            mode: u32,
        ) -> i32;

        fn GetConsoleScreenBufferInfo(
            handle: Handle,
            info: *mut ScreenBufferInfo,
        ) -> i32;

        fn SetConsoleScreenBufferSize(
            handle: Handle,
            size: Coord,
        ) -> i32;

        fn SetConsoleWindowInfo(
            handle: Handle,
            absolute: i32,
            window: *const SmallRect,
        ) -> i32;
    }


    fn buffer_info(handle: Handle) -> Option<ScreenBufferInfo> {
        let mut info =
            ScreenBufferInfo::default();

        let ok =
            unsafe { GetConsoleScreenBufferInfo(handle, &mut info) };

        (ok != 0).then_some(info)
    }


    fn enable_ansi_on(std_handle: u32) -> bool {
        let handle =
            unsafe { GetStdHandle(std_handle) };

        let mut mode =
            0_u32;

        // Fails when the stream is redirected to a file or pipe.
        if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
            return false;
        }

        if mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 {
            return true;
        }

        unsafe {
            SetConsoleMode(
                handle,
                mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            ) != 0
        }
    }


    pub fn enable_ansi() -> bool {
        // Evaluate both: each stream has its own console mode.
        let stdout = enable_ansi_on(STD_OUTPUT_HANDLE);
        let stderr = enable_ansi_on(STD_ERROR_HANDLE);

        stdout && stderr
    }


    // Puts the console input mode back when the Host exits, so a shell that
    // launched the Host gets Ctrl+C back.
    pub struct InputModeGuard {
        handle: Handle,
        original: u32,
    }


    impl Drop for InputModeGuard {
        fn drop(&mut self) {
            let _ = unsafe { SetConsoleMode(self.handle, self.original) };
        }
    }


    // Makes Ctrl+C an ordinary key for as long as the guard lives. Returns
    // None, leaving Ctrl+C as the system's interrupt, when input is not a
    // console (redirected or absent) or the console refused the change.
    pub fn take_key_commands() -> Option<InputModeGuard> {
        let handle =
            unsafe { GetStdHandle(STD_INPUT_HANDLE) };

        let mut original =
            0_u32;

        // Fails when input is redirected or there is no console.
        if unsafe { GetConsoleMode(handle, &mut original) } == 0 {
            return None;
        }

        let wanted =
            super::input_mode_for_key_commands(original);

        if unsafe { SetConsoleMode(handle, wanted) } == 0 {
            return None;
        }

        Some(InputModeGuard { handle, original })
    }


    pub fn widen() {
        let handle =
            unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };


        // Fails when stdout is redirected to a file or pipe.
        let mut mode =
            0_u32;

        if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
            return;
        }


        let Some(info) = buffer_info(handle) else {
            return;
        };


        // The buffer must be at least as wide as the window, so grow it first.
        if info.size.x < TARGET_COLUMNS {
            let size =
                Coord {
                    x: TARGET_COLUMNS,
                    y: info.size.y,
                };

            if unsafe { SetConsoleScreenBufferSize(handle, size) } == 0 {
                return;
            }
        }


        // Re-read: the maximum window size depends on the new buffer size.
        let Some(info) = buffer_info(handle) else {
            return;
        };


        let columns =
            TARGET_COLUMNS.min(info.maximum_window_size.x);

        let current_columns =
            info.window.right - info.window.left + 1;

        if columns <= current_columns {
            return;
        }


        let window =
            SmallRect {
                right: info.window.left + columns - 1,
                ..info.window
            };

        let _ = unsafe { SetConsoleWindowInfo(handle, 1, &window) };
    }
}


#[cfg(not(windows))]
mod platform {
    use std::io::IsTerminal;


    pub fn widen() {}


    pub fn enable_ansi() -> bool {
        std::io::stdout().is_terminal()
            && std::io::stderr().is_terminal()
    }


    pub struct InputModeGuard;


    // Key commands are only read from a Windows console.
    pub fn take_key_commands() -> Option<InputModeGuard> {
        None
    }
}


pub use platform::{enable_ansi, take_key_commands, widen};


#[cfg(test)]
mod tests {
    use super::*;

    const ENABLE_LINE_INPUT: u32 = 0x0002;
    const ENABLE_ECHO_INPUT: u32 = 0x0004;
    const ENABLE_WINDOW_INPUT: u32 = 0x0008;
    const ENABLE_MOUSE_INPUT: u32 = 0x0010;
    const ENABLE_INSERT_MODE: u32 = 0x0020;
    const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
    const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;


    #[test]
    fn processed_input_is_the_only_flag_removed() {
        // A classic console's default input mode.
        let default_mode =
            ENABLE_PROCESSED_INPUT
                | ENABLE_LINE_INPUT
                | ENABLE_ECHO_INPUT
                | ENABLE_MOUSE_INPUT
                | ENABLE_INSERT_MODE
                | ENABLE_QUICK_EDIT_MODE
                | ENABLE_EXTENDED_FLAGS;

        assert_eq!(
            input_mode_for_key_commands(default_mode),
            default_mode & !ENABLE_PROCESSED_INPUT
        );
    }


    #[test]
    fn quick_edit_and_other_input_flags_are_left_as_found() {
        for flag in [
            ENABLE_LINE_INPUT,
            ENABLE_ECHO_INPUT,
            ENABLE_WINDOW_INPUT,
            ENABLE_MOUSE_INPUT,
            ENABLE_INSERT_MODE,
            ENABLE_QUICK_EDIT_MODE,
            ENABLE_EXTENDED_FLAGS,
        ] {
            // On stays on, off stays off.
            assert_eq!(
                input_mode_for_key_commands(flag | ENABLE_PROCESSED_INPUT),
                flag,
                "{flag:#06x}"
            );
        }

        assert_eq!(input_mode_for_key_commands(ENABLE_PROCESSED_INPUT), 0);
        assert_eq!(input_mode_for_key_commands(0), 0);
    }
}
