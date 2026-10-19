use std::{
    ffi::{CStr, c_char, c_int, c_long, c_void},
    io,
    mem::{MaybeUninit, size_of},
    ptr,
};

unsafe extern "C" {
    fn sysctlbyname(
        name: *const c_char,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *const c_void,
        newlen: usize,
    ) -> c_int;
}

pub(super) fn sysctl_i32(name: &CStr) -> Result<i32, String> {
    read_sysctl(name)
}

pub(super) fn sysctl_u32(name: &CStr) -> Result<u32, String> {
    read_sysctl(name)
}

pub(super) fn sysctl_cpu_ticks(name: &CStr) -> Result<[c_long; 5], String> {
    read_sysctl(name)
}

fn read_sysctl<T: Copy>(name: &CStr) -> Result<T, String> {
    let mut value = MaybeUninit::<T>::uninit();
    let mut size = size_of::<T>();
    // SAFETY: `name` is NUL-terminated, `value` has `size` writable bytes,
    // and no new value is supplied to the read-only sysctl call.
    let result = unsafe {
        sysctlbyname(
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            &mut size,
            ptr::null(),
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
    if size != size_of::<T>() {
        return Err(format!(
            "unexpected size for {}: {size} bytes, expected {}",
            name.to_string_lossy(),
            size_of::<T>()
        ));
    }
    // SAFETY: sysctlbyname succeeded and filled exactly the size of T.
    Ok(unsafe { value.assume_init() })
}
