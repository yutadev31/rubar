use super::CpuState;
use std::ffi::c_ulong;

#[cfg(target_os = "freebsd")]
#[derive(Default)]
pub struct CpTime {
    previous: Option<[c_ulong; 5]>,
}

#[cfg(target_os = "freebsd")]
impl super::CpuProvider for CpTime {
    fn read(&mut self) -> Result<Option<CpuState>, String> {
        let ticks = crate::provider::freebsd::sysctl_cpu_ticks(c"kern.cp_time")?;
        let current = ticks.map(|tick| tick as c_ulong);
        let previous = self.previous.replace(current);
        Ok(previous.and_then(|previous| usage(previous, current)))
    }
}

fn usage(previous: [c_ulong; 5], current: [c_ulong; 5]) -> Option<CpuState> {
    // FreeBSD's sys/resource.h: user, nice, system, interrupt, idle.
    let mut deltas = [0; 5];
    for ((delta, after), before) in deltas.iter_mut().zip(current).zip(previous) {
        *delta = after.wrapping_sub(before);
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
    use std::ffi::{c_long, c_ulong};

    #[test]
    fn calculates_usage_from_freebsd_counter_deltas() {
        let first = [100, 10, 50, 10, 830];
        let second = [120, 10, 60, 20, 890];
        assert_eq!(usage(first, second).unwrap().percent(), 40);
        assert!(usage(first, first).is_none());
    }

    #[test]
    fn handles_signed_boundary_and_counter_wrap() {
        let before = [c_long::MAX as c_ulong, 0, 0, 0, 100];
        let after = [c_long::MIN as c_ulong, 0, 0, 0, 109];
        assert_eq!(usage(before, after).unwrap().percent(), 10);

        let before_wrap = [c_ulong::MAX - 1, 0, 0, 0, 100];
        let after_wrap = [1, 0, 0, 0, 107];
        assert_eq!(usage(before_wrap, after_wrap).unwrap().percent(), 30);
    }
}
