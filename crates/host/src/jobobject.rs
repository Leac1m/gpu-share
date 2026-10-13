//! Windows Job Object for the engine (`P2-T4`, spec §6.2): kill-on-close, so the engine can
//! never outlive the app, and an optional memory cap. Below-normal CPU priority is set when the
//! process is created (see `engine.rs`). Verified by hand on Windows; CI only compiles it.

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

/// Closing the handle (dropping this) kills every process in the job.
pub struct Job(HANDLE);

// The handle is only used to keep the job alive and is closed exactly once.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    pub fn for_engine(child: &tokio::process::Child, memory_cap: Option<u64>) -> Result<Self, String> {
        let pid = child.id().ok_or("the engine exited immediately")?;
        unsafe {
            let job = CreateJobObjectW(None, None).map_err(|e| e.to_string())?;
            let job = Job(job);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Some(cap) = memory_cap {
                info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                info.ProcessMemoryLimit = cap as usize;
            }
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .map_err(|e| e.to_string())?;
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid).map_err(|e| e.to_string())?;
            let assigned = AssignProcessToJobObject(job.0, process);
            let _ = CloseHandle(process);
            assigned.map_err(|e| e.to_string())?;
            Ok(job)
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
