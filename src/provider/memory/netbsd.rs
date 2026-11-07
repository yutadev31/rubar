use super::MemoryState;

#[cfg(target_os = "netbsd")]
pub struct UvmExp;

#[cfg(target_os = "netbsd")]
impl super::MemoryProvider for UvmExp {
    fn read(&mut self) -> Result<MemoryState, String> {
        let bytes = crate::provider::netbsd::sysctl_bytes(c"vm.uvmexp2")?;

        // uvmexp_sysctl starts with 64-bit pagesize, pagemask, pageshift,
        // npages, free, active, inactive. NetBSD only appends ABI fields.
        if bytes.len() < 7 * 8 {
            return Err(format!("vm.uvmexp2 returned only {} bytes", bytes.len()));
        }

        let field = |index: usize| {
            i64::from_ne_bytes(bytes[index * 8..(index + 1) * 8].try_into().unwrap())
        };
        memory_state(field(0), field(3), field(4), field(6))
    }
}

fn memory_state(
    page_size: i64,
    pages: i64,
    free: i64,
    inactive: i64,
) -> Result<MemoryState, String> {
    let page_size = u64::try_from(page_size)
        .ok()
        .filter(|value| *value > 0)
        .ok_or("invalid NetBSD page size")?;
    let pages = u64::try_from(pages).map_err(|_| "invalid NetBSD page count")?;
    let free = u64::try_from(free).map_err(|_| "invalid NetBSD free page count")?;
    let inactive = u64::try_from(inactive).map_err(|_| "invalid NetBSD inactive page count")?;
    let available = free
        .checked_add(inactive)
        .ok_or("NetBSD available page count overflowed")?;
    let total_bytes = pages
        .checked_mul(page_size)
        .ok_or("NetBSD total memory overflowed")?;
    let available_bytes = available
        .checked_mul(page_size)
        .ok_or("NetBSD available memory overflowed")?;
    MemoryState::new(total_bytes, available_bytes)
}

#[cfg(test)]
mod tests {
    use super::memory_state;

    #[test]
    fn counts_free_and_inactive_pages() {
        let state = memory_state(4096, 1000, 100, 200).unwrap();
        assert_eq!(state.total_bytes(), 4_096_000);
        assert_eq!(state.available_bytes(), 1_228_800);
        assert!(memory_state(0, 1000, 100, 200).is_err());
        assert!(memory_state(4096, 1000, 900, 200).is_err());
    }
}
