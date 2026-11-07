use std::{io, mem::size_of, ptr};

use super::ProcessProvider;

pub struct KinfoProc;

impl ProcessProvider for KinfoProc {
    fn read(&mut self) -> Result<usize, String> {
        let entry_size = size_of::<libc::kinfo_proc2>();
        let mut mib = [
            libc::CTL_KERN,
            libc::KERN_PROC2,
            libc::KERN_PROC_ALL,
            0,
            i32::try_from(entry_size).map_err(|_| "NetBSD process entry is too large")?,
            i32::MAX,
        ];

        for _ in 0..3 {
            let mut len = 0;

            // SAFETY: this call only queries the required table size.
            if unsafe {
                libc::sysctl(
                    mib.as_mut_ptr(),
                    6,
                    ptr::null_mut(),
                    &mut len,
                    ptr::null_mut(),
                    0,
                )
            } != 0
            {
                return Err(format!(
                    "could not size NetBSD process table: {}",
                    io::Error::last_os_error()
                ));
            }

            let mut bytes = vec![0u8; len.max(entry_size)];
            let mut written = bytes.len();

            // SAFETY: the buffer owns `written` bytes for the kernel to fill.
            if unsafe {
                libc::sysctl(
                    mib.as_mut_ptr(),
                    6,
                    bytes.as_mut_ptr().cast(),
                    &mut written,
                    ptr::null_mut(),
                    0,
                )
            } == 0
            {
                if written % entry_size != 0 {
                    return Err(format!(
                        "NetBSD process table returned {written} non-aligned bytes"
                    ));
                }

                return Ok(written / entry_size);
            }

            let error = io::Error::last_os_error();

            if error.raw_os_error() != Some(libc::ENOMEM) {
                return Err(format!("could not read NetBSD process table: {error}"));
            }
        }

        Err("NetBSD process table kept growing while reading it".to_string())
    }
}
