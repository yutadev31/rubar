use super::CpuState;

#[cfg(target_os = "netbsd")]
#[derive(Default)]
pub struct CpTime {
    previous: Option<[u64; 5]>,
}

#[cfg(target_os = "netbsd")]
impl super::CpuProvider for CpTime {
    fn read(&mut self) -> Result<Option<CpuState>, String> {
        // NetBSD's CP_USER, CP_NICE, CP_SYS, CP_INTR, CP_IDLE counters.
        let current = crate::provider::netbsd::sysctl_exact(c"kern.cp_time")?;
        let previous = self.previous.replace(current);
        Ok(previous.and_then(|previous| usage(previous, current)))
    }
}

fn usage(previous: [u64; 5], current: [u64; 5]) -> Option<CpuState> {
    let deltas =
        std::array::from_fn::<_, 5, _>(|index| current[index].wrapping_sub(previous[index]));
    let total: u128 = deltas.into_iter().map(u128::from).sum();
    (total != 0).then(|| CpuState {
        percent: u64::try_from(((total - u128::from(deltas[4])) * 100 + total / 2) / total)
            .expect("percentage is at most 100"),
    })
}

#[cfg(test)]
mod tests {
    use super::usage;

    #[test]
    fn calculates_cpu_usage_and_handles_wrap() {
        assert_eq!(
            usage([10, 0, 0, 0, 90], [20, 0, 10, 0, 100])
                .unwrap()
                .percent(),
            67
        );
        assert!(usage([0; 5], [0; 5]).is_none());
        assert_eq!(
            usage([u64::MAX, 0, 0, 0, 0], [0, 0, 0, 0, 1])
                .unwrap()
                .percent(),
            50
        );
    }
}
