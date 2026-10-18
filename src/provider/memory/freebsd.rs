use super::MemoryState;

#[cfg(target_os = "freebsd")]
pub struct VmStats;

#[cfg(target_os = "freebsd")]
impl super::MemoryProvider for VmStats {
    fn read(&mut self) -> Result<MemoryState, String> {
        use crate::provider::freebsd::sysctl;

        let page_size: u32 = sysctl(c"vm.stats.vm.v_page_size")?;
        let total_pages: u32 = sysctl(c"vm.stats.vm.v_page_count")?;
        let free_pages: u32 = sysctl(c"vm.stats.vm.v_free_count")?;
        let inactive_pages: u32 = sysctl(c"vm.stats.vm.v_inactive_count")?;
        memory_state(page_size, total_pages, free_pages, inactive_pages)
    }
}

fn memory_state(
    page_size: u32,
    total_pages: u32,
    free_pages: u32,
    inactive_pages: u32,
) -> Result<MemoryState, String> {
    if page_size == 0 {
        return Err("FreeBSD VM page size is zero".to_string());
    }
    // Inactive pages can be reclaimed; laundry pages still need writeback.
    let available_pages = u64::from(free_pages) + u64::from(inactive_pages);
    let total_bytes = u64::from(total_pages) * u64::from(page_size);
    let available_bytes = available_pages
        .checked_mul(u64::from(page_size))
        .ok_or("FreeBSD available memory exceeds the supported range")?;
    MemoryState::new(total_bytes, available_bytes)
}

#[cfg(test)]
mod tests {
    use super::memory_state;

    #[test]
    fn counts_free_and_inactive_pages_as_available() {
        let state = memory_state(4096, 1000, 100, 200).unwrap();
        assert_eq!(state.total_bytes(), 4_096_000);
        assert_eq!(state.available_bytes(), 1_228_800);
        assert_eq!(state.used_bytes(), 2_867_200);
        assert_eq!(state.percent(), 70);
        assert!(memory_state(0, 1000, 100, 200).is_err());
        assert!(memory_state(4096, 1000, 900, 200).is_err());
    }
}
