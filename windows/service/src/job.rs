// -----------------------------------------------------------------------------
// Child process containment
// -----------------------------------------------------------------------------
//
// The Host stops FFmpeg and MediaMTX itself during a normal shutdown. A Windows
// Job Object with kill-on-close covers the case where the Host is terminated
// without the chance to do that (Task Manager, crash): the OS closes the job
// handle and terminates every process assigned to it, so no FFmpeg is left
// holding the capture device and no MediaMTX is left holding the ports.
//
// Best effort: if the job cannot be created the Host still works, without that
// guarantee.

use std::process::Child;

#[cfg(windows)]
mod platform {
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 =
        9;

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 =
        0x2000;


    type Handle =
        *mut std::ffi::c_void;


    #[repr(C)]
    #[derive(Default)]
    struct BasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }


    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }


    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimitInformation {
        basic_limit_information: BasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }


    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(
            attributes: *const std::ffi::c_void,
            name: *const u16,
        ) -> Handle;

        fn SetInformationJobObject(
            job: Handle,
            information_class: i32,
            information: *const std::ffi::c_void,
            information_length: u32,
        ) -> i32;

        fn AssignProcessToJobObject(
            job: Handle,
            process: Handle,
        ) -> i32;

        fn CloseHandle(
            handle: Handle,
        ) -> i32;
    }


    pub struct Job {
        handle: Handle,
    }


    impl Drop for Job {
        fn drop(&mut self) {
            // Closing the last handle terminates anything still in the job.
            let _ = unsafe { CloseHandle(self.handle) };
        }
    }


    impl Job {
        pub fn create() -> io::Result<Self> {
            let handle =
                unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };

            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }

            let job =
                Self { handle };

            let mut information =
                ExtendedLimitInformation::default();

            information.basic_limit_information.limit_flags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            let ok = unsafe {
                SetInformationJobObject(
                    job.handle,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                    (&raw const information).cast(),
                    size_of::<ExtendedLimitInformation>() as u32,
                )
            };

            if ok == 0 {
                return Err(io::Error::last_os_error());
            }

            Ok(job)
        }


        pub fn assign(&self, child: &Child) -> io::Result<()> {
            let ok = unsafe {
                AssignProcessToJobObject(
                    self.handle,
                    child.as_raw_handle().cast(),
                )
            };

            if ok == 0 {
                return Err(io::Error::last_os_error());
            }

            Ok(())
        }
    }
}


#[cfg(not(windows))]
mod platform {
    use std::io;
    use std::process::Child;


    pub struct Job;


    impl Job {
        pub fn create() -> io::Result<Self> {
            Ok(Self)
        }


        pub fn assign(&self, _child: &Child) -> io::Result<()> {
            Ok(())
        }
    }
}


pub struct ChildJob {
    job: Option<platform::Job>,
}


impl ChildJob {
    pub fn create() -> Self {
        match platform::Job::create() {
            Ok(job) => {
                Self { job: Some(job) }
            }

            Err(error) => {
                crate::logging::host(format_args!(
                    "WARNING: could not create a job object ({error}). Child processes will not be terminated automatically if the Host is killed."
                ));

                Self { job: None }
            }
        }
    }


    pub fn assign(&self, child: &Child, name: &str) {
        let Some(job) = &self.job else {
            return;
        };

        if let Err(error) = job.assign(child) {
            crate::logging::host(format_args!(
                "WARNING: could not add {name} PID {} to the Host job object ({error}).",
                child.id()
            ));
        }
    }
}
