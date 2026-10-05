// -----------------------------------------------------------------------------
// Interactive console hotkeys
// -----------------------------------------------------------------------------
//
// While the Host runs in an interactive console, single key presses are
// delivered to the supervisor loop as events. The supervisor decides what each
// key means, so new keys can be added there without touching this file.
//
// Keys are read as console input records without changing the console mode, so
// Ctrl+C is still handled by the system exactly as before.

use std::io::{self, IsTerminal};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

use crate::Event;


// Starts the key reader if console input is available. Returns whether hotkeys
// are active.
pub fn listen(events: Sender<Event>) -> bool {
    // Redirected or absent input: no hotkeys.
    if !io::stdin().is_terminal() {
        return false;
    }

    platform::listen(events)
}


// Keys are matched case-insensitively.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn normalise(key: char) -> char {
    key.to_ascii_lowercase()
}


// Opens `directory` in the platform file manager without waiting for it.
pub fn open_directory(directory: &Path) -> io::Result<()> {
    let program =
        if cfg!(windows) {
            "explorer.exe"
        } else {
            "open"
        };

    Command::new(program)
        .arg(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}


#[cfg(windows)]
mod platform {
    use std::sync::mpsc::Sender;
    use std::thread;

    use crate::Event;

    const STD_INPUT_HANDLE: u32 =
        -10_i32 as u32;

    const KEY_EVENT: u16 =
        0x0001;


    type Handle =
        *mut std::ffi::c_void;


    // KEY_EVENT_RECORD
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct KeyEventRecord {
        key_down: i32,
        repeat_count: u16,
        virtual_key_code: u16,
        virtual_scan_code: u16,
        unicode_char: u16,
        control_key_state: u32,
    }


    // INPUT_RECORD. The event union is 16 bytes; KEY_EVENT_RECORD is its
    // largest member, and the only one read here.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct InputRecord {
        event_type: u16,
        padding: u16,
        key: KeyEventRecord,
    }


    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(
            std_handle: u32,
        ) -> Handle;

        fn ReadConsoleInputW(
            handle: Handle,
            buffer: *mut InputRecord,
            length: u32,
            events_read: *mut u32,
        ) -> i32;
    }


    pub fn listen(events: Sender<Event>) -> bool {
        thread::spawn(move || {
            let handle =
                unsafe { GetStdHandle(STD_INPUT_HANDLE) };

            loop {
                let mut record =
                    InputRecord::default();

                let mut events_read =
                    0_u32;

                let ok = unsafe {
                    ReadConsoleInputW(handle, &mut record, 1, &mut events_read)
                };

                // Input is gone (console closed or detached): stop quietly.
                if ok == 0 || events_read == 0 {
                    return;
                }

                if record.event_type != KEY_EVENT || record.key.key_down == 0 {
                    continue;
                }

                let Some(key) = char::from_u32(u32::from(record.key.unicode_char)) else {
                    continue;
                };

                // Modifier and navigation keys carry no character.
                if key == '\0' {
                    continue;
                }

                if events.send(Event::Key(super::normalise(key))).is_err() {
                    return;
                }
            }
        });

        true
    }
}


#[cfg(not(windows))]
mod platform {
    use std::sync::mpsc::Sender;

    use crate::Event;


    // The Host is a Windows application; single-key console input is only
    // implemented there.
    pub fn listen(_events: Sender<Event>) -> bool {
        false
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn keys_are_case_insensitive() {
        assert_eq!(normalise('L'), 'l');
        assert_eq!(normalise('l'), 'l');
    }
}
