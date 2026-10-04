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

#[cfg(windows)]
mod platform {
    const TARGET_COLUMNS: i16 =
        140;

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
}


pub use platform::{enable_ansi, widen};
