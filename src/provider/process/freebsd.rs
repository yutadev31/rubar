use std::{ffi::c_void, io};

use super::ProcessProvider;

#[link(name = "util")]
unsafe extern "C" {
    fn kinfo_getallproc(count: *mut i32) -> *mut c_void;
}

unsafe extern "C" {
    fn free(pointer: *mut c_void);
}

pub struct KinfoProc;

impl ProcessProvider for KinfoProc {
    fn read(&mut self) -> Result<usize, String> {
        let mut count = 0;
        // SAFETY: `count` is writable and the returned allocation belongs to the C allocator.
        let processes = unsafe { kinfo_getallproc(&mut count) };
        if processes.is_null() {
            return Err(format!(
                "could not read FreeBSD process table: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: kinfo_getallproc returns a malloc allocation that must be freed by the caller.
        unsafe { free(processes) };
        usize::try_from(count).map_err(|_| format!("invalid FreeBSD process count: {count}"))
    }
}
