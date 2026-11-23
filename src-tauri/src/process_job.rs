use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::GetCurrentProcess,
    },
};

pub struct ProcessJob(isize);
impl ProcessJob {
    pub fn attach() -> Result<Self, String> {
        unsafe {
            let handle = CreateJobObjectW(None, None).map_err(|e| e.to_string())?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let result = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
            .and_then(|_| AssignProcessToJobObject(handle, GetCurrentProcess()));
            if let Err(error) = result {
                let _ = CloseHandle(handle);
                return Err(error.to_string());
            }
            Ok(Self(handle.0 as isize))
        }
    }
}
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(HANDLE(self.0 as *mut _));
        }
    }
}
