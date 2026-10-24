use std::{fs, path::PathBuf};

use super::{MemoryProvider, MemoryState};

pub struct ProcMeminfo {
    path: PathBuf,
}

impl ProcMeminfo {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl MemoryProvider for ProcMeminfo {
    fn read(&mut self) -> Result<MemoryState, String> {
        let contents = fs::read_to_string(&self.path)
            .map_err(|error| format!("could not read {}: {error}", self.path.display()))?;
        parse_meminfo(&contents)
            .map_err(|error| format!("invalid {}: {error}", self.path.display()))
    }
}

fn parse_meminfo(contents: &str) -> Result<MemoryState, String> {
    let mut total = None;
    let mut available = None;
    for line in contents.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let target = match key {
            "MemTotal" => &mut total,
            "MemAvailable" => &mut available,
            _ => continue,
        };
        let mut parts = value.split_whitespace();
        let number = parts
            .next()
            .ok_or_else(|| format!("{key} has no value"))?
            .parse::<u64>()
            .map_err(|error| format!("{key} has an invalid value: {error}"))?;
        if parts.next() != Some("kB") || parts.next().is_some() {
            return Err(format!("{key} must be measured in kB"));
        }
        *target = Some(number);
    }
    let total_kib = total.ok_or("MemTotal is missing")?;
    let available_kib = available.ok_or("MemAvailable is missing")?;
    let total_bytes = total_kib
        .checked_mul(1024)
        .ok_or("MemTotal exceeds the supported range")?;
    let available_bytes = available_kib
        .checked_mul(1024)
        .ok_or("MemAvailable exceeds the supported range")?;
    MemoryState::new(total_bytes, available_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_memory_values() {
        let state = parse_meminfo(
            "MemTotal:       8388608 kB\nMemFree:         1048576 kB\nMemAvailable:   6291456 kB\n",
        )
        .unwrap();
        assert_eq!(state.total_bytes(), 8 * 1_073_741_824);
        assert_eq!(state.available_bytes(), 6 * 1_073_741_824);
    }

    #[test]
    fn rejects_missing_or_inconsistent_values() {
        assert!(
            parse_meminfo("MemTotal: 100 kB\n")
                .unwrap_err()
                .contains("MemAvailable")
        );
        assert!(parse_meminfo("MemTotal: 0 kB\nMemAvailable: 0 kB\n").is_err());
        assert!(parse_meminfo("MemTotal: 100 kB\nMemAvailable: 101 kB\n").is_err());
        assert!(parse_meminfo("MemTotal: nope kB\nMemAvailable: 1 kB\n").is_err());
    }
}
