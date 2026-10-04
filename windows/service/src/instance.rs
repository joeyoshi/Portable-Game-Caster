// -----------------------------------------------------------------------------
// Windows Host single-instance guard
// -----------------------------------------------------------------------------

#[cfg(windows)]
mod platform {
    use std::io;

    const MUTEX_NAME: &str =
        "Global\\PortableGameCasterHost.v1";

    const ERROR_ACCESS_DENIED: i32 =
        5;

    const ERROR_ALREADY_EXISTS: i32 =
        183;


    type Handle =
        *mut std::ffi::c_void;


    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateMutexW(
            attributes: *const std::ffi::c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> Handle;

        fn CloseHandle(
            handle: Handle,
        ) -> i32;

        fn GetLastError() -> u32;
    }


    pub struct InstanceGuard {
        handle: Handle,
    }


    impl Drop for InstanceGuard {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.handle) };
        }
    }


    pub fn acquire() -> io::Result<Option<InstanceGuard>> {
        let mut name: Vec<u16> =
            MUTEX_NAME.encode_utf16().collect();

        name.push(0);


        let handle = unsafe {
            CreateMutexW(
                std::ptr::null(),
                0,
                name.as_ptr(),
            )
        };


        let last_error =
            unsafe { GetLastError() as i32 };


        if handle.is_null() {
            // Another Host owns the mutex under a different privilege/account
            // context, so this process is not allowed to open it.
            if last_error == ERROR_ACCESS_DENIED {
                return Ok(None);
            }


            return Err(io::Error::from_raw_os_error(last_error));
        }


        let already_exists =
            last_error == ERROR_ALREADY_EXISTS;


        if already_exists {
            let _ = unsafe { CloseHandle(handle) };


            return Ok(None);
        }


        Ok(Some(InstanceGuard { handle }))
    }
}


#[cfg(not(windows))]
mod platform {
    use std::io;


    pub struct InstanceGuard;


    pub fn acquire() -> io::Result<Option<InstanceGuard>> {
        Ok(Some(InstanceGuard))
    }
}


pub use platform::acquire;
