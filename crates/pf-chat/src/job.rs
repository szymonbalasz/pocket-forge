//! The Windows Job Object for chat children (§10, #60, R75). Each child gets its own job
//! with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: ending the job (Stop, timeout) ends every
//! process in it, grandchildren included, and when the app dies its handle closes and
//! Windows ends them too.
//!
//! The only module in `pf-chat` that allows `unsafe` (R75); every block has a SAFETY note.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};

/// An owned job handle.
#[derive(Debug)]
pub struct JobObject(HANDLE);

// SAFETY: a job handle is a kernel object handle; the Win32 job functions may be called on it
// from any thread, and we only close it once (in `Drop`).
unsafe impl Send for JobObject {}
// SAFETY: as above; `terminate` and `assign` take `&self` and the kernel serialises them.
unsafe impl Sync for JobObject {}

impl JobObject {
    /// A new anonymous job that kills its processes when its last handle closes.
    pub fn new() -> io::Result<Self> {
        // SAFETY: null security attributes and a null name are documented as valid; the
        // returned handle is checked before use.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(handle);
        // SAFETY: JOBOBJECT_EXTENDED_LIMIT_INFORMATION is a plain C struct of integers, for
        // which all-zero bytes are a valid value ("no limits").
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .map_err(io::Error::other)?;
        // SAFETY: `job.0` is a valid job handle; the pointer and size describe `info`, which
        // outlives the call, and match the JobObjectExtendedLimitInformation class.
        let ok = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast::<c_void>(),
                size,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    /// Puts `child` (and every process it starts later) in the job.
    pub fn assign(&self, child: &Child) -> io::Result<()> {
        let process: HANDLE = child.as_raw_handle().cast();
        // SAFETY: both handles are valid for the call: the job is owned by `self` and the
        // process handle is owned by `child`, which the caller keeps alive.
        let ok = unsafe { AssignProcessToJobObject(self.0, process) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Ends every process in the job (exit code 1). Errors are ignored: the processes may
    /// have exited already.
    pub fn terminate(&self) {
        // SAFETY: `self.0` is a valid job handle until `Drop`.
        unsafe {
            TerminateJobObject(self.0, 1);
        }
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by CreateJobObjectW and is closed exactly once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
