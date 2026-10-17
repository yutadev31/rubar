use std::{fs, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryState {
    pub total_kib: u64,
    pub available_kib: u64,
}

impl MemoryState {
    pub fn used_kib(self) -> u64 {
        self.total_kib - self.available_kib
    }

    pub fn percent(self) -> u64 {
        ((u128::from(self.used_kib()) * 100 + u128::from(self.total_kib) / 2)
            / u128::from(self.total_kib)) as u64
    }
}

pub struct ProcMeminfo {
    path: PathBuf,
}

impl ProcMeminfo {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn read(&self) -> Result<MemoryState, String> {
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
    if total_kib == 0 {
        return Err("MemTotal must be greater than zero".to_string());
    }
    if available_kib > total_kib {
        return Err("MemAvailable exceeds MemTotal".to_string());
    }
    Ok(MemoryState {
        total_kib,
        available_kib,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_used_memory_from_available_memory() {
        let state = parse_meminfo(
            "MemTotal:       8388608 kB\nMemFree:         1048576 kB\nMemAvailable:   6291456 kB\n",
        )
        .unwrap();
        assert_eq!(state.used_kib(), 2097152);
        assert_eq!(state.percent(), 25);
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
