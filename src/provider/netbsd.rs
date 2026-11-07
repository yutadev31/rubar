use std::{
    ffi::CStr,
    io,
    mem::{MaybeUninit, size_of},
    ptr,
};

pub(super) fn sysctl_exact<T: Copy>(name: &CStr) -> Result<T, String> {
    let mut value = MaybeUninit::<T>::uninit();
    let mut len = size_of::<T>();
    // SAFETY: the output points to `len` writable bytes and the name is terminated.
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            &mut len,
            ptr::null_mut(),
            0,
        )
    };

    if result != 0 {
        return Err(format!(
            "could not read {}: {}",
            name.to_string_lossy(),
            io::Error::last_os_error()
        ));
    }

    if len != size_of::<T>() {
        return Err(format!(
            "unexpected size for {}: {len} bytes, expected {}",
            name.to_string_lossy(),
            size_of::<T>()
        ));
    }

    // SAFETY: sysctl filled the complete value.
    Ok(unsafe { value.assume_init() })
}

pub(super) fn sysctl_bytes(name: &CStr) -> Result<Vec<u8>, String> {
    for _ in 0..3 {
        let mut len = 0;
        // SAFETY: only the required output size is queried.
        let result = unsafe {
            libc::sysctlbyname(name.as_ptr(), ptr::null_mut(), &mut len, ptr::null_mut(), 0)
        };

        if result != 0 {
            return Err(format!(
                "could not size {}: {}",
                name.to_string_lossy(),
                io::Error::last_os_error()
            ));
        }

        if len == 0 {
            return Err(format!("{} returned no data", name.to_string_lossy()));
        }

        let mut bytes = vec![0u8; len];
        // SAFETY: the vector owns `len` writable bytes.
        let result = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                &mut len,
                ptr::null_mut(),
                0,
            )
        };

        if result == 0 {
            bytes.truncate(len);
            return Ok(bytes);
        }

        let error = io::Error::last_os_error();

        if error.raw_os_error() != Some(libc::ENOMEM) {
            return Err(format!(
                "could not read {}: {error}",
                name.to_string_lossy()
            ));
        }
    }

    Err(format!(
        "{} kept growing while reading it",
        name.to_string_lossy()
    ))
}
