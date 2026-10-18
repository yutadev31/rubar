use super::CpuState;

#[cfg(target_os = "freebsd")]
#[derive(Default)]
pub struct CpTime {
    previous: Option<[u64; 5]>,
}

#[cfg(target_os = "freebsd")]
impl super::CpuProvider for CpTime {
    fn read(&mut self) -> Result<Option<CpuState>, String> {
        use std::ffi::c_long;

        let ticks: [c_long; 5] = crate::provider::freebsd::sysctl(c"kern.cp_time")?;
        let mut current = [0; 5];
        for (value, tick) in current.iter_mut().zip(ticks) {
            *value = u64::try_from(tick)
                .map_err(|_| "kern.cp_time contains a negative counter".to_string())?;
        }
        let previous = self.previous.replace(current);
        Ok(previous.and_then(|previous| usage(previous, current)))
    }
}

fn usage(previous: [u64; 5], current: [u64; 5]) -> Option<CpuState> {
    // FreeBSD's sys/resource.h: user, nice, system, interrupt, idle.
    let mut deltas = [0; 5];
    for ((delta, after), before) in deltas.iter_mut().zip(current).zip(previous) {
        *delta = after.checked_sub(before)?;
    }
    let total: u128 = deltas.into_iter().map(u128::from).sum();
    if total == 0 {
        return None;
    }
    let busy = total - u128::from(deltas[4]);
    Some(CpuState {
        percent: ((busy * 100 + total / 2) / total) as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::usage;

    #[test]
    fn calculates_usage_from_freebsd_counter_deltas() {
        let first = [100, 10, 50, 10, 830];
        let second = [120, 10, 60, 20, 890];
        assert_eq!(usage(first, second).unwrap().percent(), 40);
        assert!(usage(first, first).is_none());
        assert!(usage(second, first).is_none());
    }
}
