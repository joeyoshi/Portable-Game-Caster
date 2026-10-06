// -----------------------------------------------------------------------------
// Interactive console hotkeys
// -----------------------------------------------------------------------------
//
// While the Host runs in an interactive console, single key presses are
// delivered to the supervisor loop as events. The supervisor decides what each
// key means, so new keys can be added there without touching this file.
//
// Keys are read as console input records. Only key presses are acted on; the
// console handles the mouse itself (selection, copy, wheel scrolling) for as
// long as QuickEdit is on, so no mouse records arrive here to be swallowed.
//
// Ctrl+Q is the quit command. Ctrl+C is not: console.rs makes it an ordinary
// key while the Host runs, and it is ignored here.

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


// What the key reader sends for Ctrl+Q (the character Ctrl+Q types).
pub const QUIT: char = '\u{11}';

const VK_Q: u16 = 0x51;

const RIGHT_ALT_PRESSED: u32 = 0x0001;
const LEFT_ALT_PRESSED: u32 = 0x0002;
const RIGHT_CTRL_PRESSED: u32 = 0x0004;
const LEFT_CTRL_PRESSED: u32 = 0x0008;


// The key a console key record stands for, if the Host acts on it at all:
// QUIT for Ctrl+Q, the lower-cased character for a plain key, nothing for key
// releases, modifier and navigation keys, and other control combinations
// (Ctrl+C among them).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn key_from_record(
    key_down: bool,
    virtual_key_code: u16,
    unicode_char: u16,
    control_key_state: u32,
) -> Option<char> {
    if !key_down {
        return None;
    }

    let ctrl =
        control_key_state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0;

    // AltGr is reported as Ctrl+Alt and types a character; it is not Ctrl.
    let alt =
        control_key_state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0;

    if ctrl && !alt {
        return (virtual_key_code == VK_Q).then_some(QUIT);
    }

    let key =
        char::from_u32(u32::from(unicode_char))?;

    if key.is_control() {
        return None;
    }

    Some(normalise(key))
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

                if record.event_type != KEY_EVENT {
                    continue;
                }

                let Some(key) = super::key_from_record(
                    record.key.key_down != 0,
                    record.key.virtual_key_code,
                    record.key.unicode_char,
                    record.key.control_key_state,
                ) else {
                    continue;
                };

                if events.send(Event::Key(key)).is_err() {
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


    const VK_C: u16 = 0x43;
    const VK_L: u16 = 0x4C;
    const VK_CONTROL: u16 = 0x11;


    #[test]
    fn ctrl_q_is_quit() {
        assert_eq!(key_from_record(true, VK_Q, 0x11, LEFT_CTRL_PRESSED), Some(QUIT));
        assert_eq!(key_from_record(true, VK_Q, 0x11, RIGHT_CTRL_PRESSED), Some(QUIT));

        // Releasing it is not a second quit.
        assert_eq!(key_from_record(false, VK_Q, 0x11, LEFT_CTRL_PRESSED), None);
    }


    #[test]
    fn plain_q_and_altgr_q_are_not_quit() {
        assert_eq!(key_from_record(true, VK_Q, u16::from(b'q'), 0), Some('q'));
        assert_eq!(key_from_record(true, VK_Q, u16::from(b'Q'), 0), Some('q'));

        assert_eq!(
            key_from_record(true, VK_Q, u16::from(b'@'), LEFT_CTRL_PRESSED | RIGHT_ALT_PRESSED),
            Some('@')
        );
    }


    #[test]
    fn ctrl_c_is_not_a_host_command() {
        // With processed input off, Ctrl+C arrives as this key record.
        assert_eq!(key_from_record(true, VK_C, 0x03, LEFT_CTRL_PRESSED), None);

        // Even if a console reports it without the modifier state.
        assert_eq!(key_from_record(true, VK_C, 0x03, 0), None);
    }


    #[test]
    fn l_opens_logs_and_modifier_keys_are_ignored() {
        assert_eq!(key_from_record(true, VK_L, u16::from(b'L'), 0), Some('l'));
        assert_eq!(key_from_record(true, VK_L, u16::from(b'l'), 0), Some('l'));
        assert_eq!(key_from_record(false, VK_L, u16::from(b'l'), 0), None);

        // Pressing Ctrl on its own.
        assert_eq!(key_from_record(true, VK_CONTROL, 0, LEFT_CTRL_PRESSED), None);
    }
}
