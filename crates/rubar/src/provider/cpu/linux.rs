use std::{fs, path::PathBuf};

use super::{CpuProvider, CpuState};

pub struct ProcStat {
    path: PathBuf,
    previous: Option<Counters>,
}

impl ProcStat {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            previous: None,
        }
    }
}

impl CpuProvider for ProcStat {
    fn read(&mut self) -> Result<Option<CpuState>, String> {
        let contents = fs::read_to_string(&self.path)
            .map_err(|error| format!("could not read {}: {error}", self.path.display()))?;
        let current = parse_stat(&contents)
            .map_err(|error| format!("invalid {}: {error}", self.path.display()))?;
        let previous = self.previous.replace(current);
        Ok(previous.and_then(|previous| usage(previous, current)))
    }
}

#[derive(Clone, Copy, Debug)]
struct Counters {
    fields: [u64; 8],
}

fn parse_stat(contents: &str) -> Result<Counters, String> {
    let line = contents.lines().next().ok_or("CPU line is missing")?;
    let mut parts = line.split_whitespace();
    if parts.next() != Some("cpu") {
        return Err("aggregate CPU line is missing".to_string());
    }
    let mut fields = [0; 8];
    for (index, field) in fields.iter_mut().enumerate() {
        match parts.next() {
            Some(value) => {
                *field = value
                    .parse()
                    .map_err(|error| format!("CPU field {} is invalid: {error}", index + 1))?;
            }
            None if index >= 4 => break,
            None => return Err(format!("CPU field {} is missing", index + 1)),
        }
    }
    Ok(Counters { fields })
}

fn usage(previous: Counters, current: Counters) -> Option<CpuState> {
    let mut total = 0u128;
    for (before, after) in previous.fields.into_iter().zip(current.fields) {
        total += u128::from(after.checked_sub(before)?);
    }
    if total == 0 {
        return None;
    }
    let idle = u128::from(current.fields[3] - previous.fields[3])
        + u128::from(current.fields[4] - previous.fields[4]);
    let busy = total - idle;
    Some(CpuState {
        percent: ((busy * 100 + total / 2) / total) as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aggregate_counters() {
        let counters = parse_stat("cpu  10 20 30 40 5 6 7 8 999 999\ncpu0 1 2 3 4\n").unwrap();
        assert_eq!(counters.fields, [10, 20, 30, 40, 5, 6, 7, 8]);
        assert!(parse_stat("cpu0 1 2 3 4\n").is_err());
        assert!(parse_stat("cpu 1 2 3\n").is_err());
        assert!(parse_stat("cpu 1 2 bad 4\n").is_err());
    }

    #[test]
    fn computes_usage_from_counter_deltas() {
        let first = parse_stat("cpu 100 0 50 850 0 0 0 0\n").unwrap();
        let second = parse_stat("cpu 120 0 60 900 10 0 0 0\n").unwrap();
        assert_eq!(usage(first, second).unwrap().percent(), 33);
        assert!(usage(first, first).is_none());
        assert!(usage(second, first).is_none());
    }
}
