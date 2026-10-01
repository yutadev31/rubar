use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryStatus {
    Charging,
    Discharging,
    Full,
    Unknown,
}

impl BatteryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Charging => "charging",
            Self::Discharging => "discharging",
            Self::Full => "full",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn is_charging(self) -> bool {
        matches!(self, Self::Charging)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryState {
    pub percent: u8,
    pub status: BatteryStatus,
}

pub trait BatteryProvider {
    fn read(&mut self) -> Result<BatteryState, String>;
}

pub fn create(name: &str) -> Result<Box<dyn BatteryProvider>, String> {
    match name {
        "sysfs" | "linux" => Ok(Box::new(Sysfs::new("/sys/class/power_supply"))),
        _ => Err(format!("unknown battery provider `{name}`")),
    }
}

pub struct Unavailable {
    pub error: String,
}

impl BatteryProvider for Unavailable {
    fn read(&mut self) -> Result<BatteryState, String> {
        Err(self.error.clone())
    }
}

pub struct Sysfs {
    root: PathBuf,
}

impl Sysfs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl BatteryProvider for Sysfs {
    fn read(&mut self) -> Result<BatteryState, String> {
        read_sysfs(&self.root)
    }
}

#[derive(Default)]
struct Aggregate {
    charge: u64,
    full: u64,
    percent_sum: u64,
    batteries: u64,
    energy_batteries: u64,
    charging: bool,
    discharging: bool,
}

fn read_sysfs(root: &Path) -> Result<BatteryState, String> {
    let entries = fs::read_dir(root)
        .map_err(|error| format!("could not read {}: {error}", root.display()))?;
    let mut aggregate = Aggregate::default();
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not read power supply entry: {error}"))?;
        let path = entry.path();
        if read_value(&path, "type").as_deref() != Some("Battery") {
            continue;
        }
        let capacity = read_value(&path, "capacity")
            .and_then(|value| value.parse::<u8>().ok())
            .ok_or_else(|| format!("could not read battery capacity from {}", path.display()))?;
        aggregate.percent_sum += u64::from(capacity);
        aggregate.batteries += 1;
        match read_value(&path, "status").as_deref() {
            Some("Charging") => aggregate.charging = true,
            Some("Discharging") => aggregate.discharging = true,
            _ => {}
        }
        let charge = read_first_number(&path, &["energy_now", "charge_now"]);
        let full = read_first_number(&path, &["energy_full", "charge_full"]);
        if let (Some(charge), Some(full)) = (charge, full) {
            aggregate.charge += charge;
            aggregate.full += full;
            aggregate.energy_batteries += 1;
        }
    }
    if aggregate.batteries == 0 {
        return Err("no battery found in /sys/class/power_supply".to_string());
    }
    // Some wireless peripherals expose themselves as `Battery` devices but
    // only provide `capacity` (for example, Logitech HID++ devices). Prefer
    // the energy/charge based batteries when available so those peripherals
    // do not dilute the laptop battery percentage.
    let percent = if aggregate.full > 0 && aggregate.energy_batteries > 0 {
        ((aggregate.charge as f64 / aggregate.full as f64) * 100.0).round() as u8
    } else {
        (aggregate.percent_sum / aggregate.batteries) as u8
    };
    let status = if aggregate.charging {
        BatteryStatus::Charging
    } else if aggregate.discharging {
        BatteryStatus::Discharging
    } else if percent >= 100 {
        BatteryStatus::Full
    } else {
        BatteryStatus::Unknown
    };
    Ok(BatteryState {
        percent: percent.min(100),
        status,
    })
}

fn read_value(path: &Path, name: &str) -> Option<String> {
    fs::read_to_string(path.join(name))
        .ok()
        .map(|value| value.trim().to_string())
}

fn read_first_number(path: &Path, names: &[&str]) -> Option<u64> {
    names
        .iter()
        .find_map(|name| read_value(path, name)?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reads_and_aggregates_batteries() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("BAT0");
        let second = dir.path().join("BAT1");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        for (path, capacity, status, now, full) in [
            (&first, "50", "Discharging", "500", "1000"),
            (&second, "100", "Not charging", "1000", "1000"),
        ] {
            fs::write(path.join("type"), "Battery").unwrap();
            fs::write(path.join("capacity"), capacity).unwrap();
            fs::write(path.join("status"), status).unwrap();
            fs::write(path.join("energy_now"), now).unwrap();
            fs::write(path.join("energy_full"), full).unwrap();
        }
        let peripheral = dir.path().join("hidpp_battery_0");
        fs::create_dir_all(&peripheral).unwrap();
        fs::write(peripheral.join("type"), "Battery").unwrap();
        fs::write(peripheral.join("capacity"), "10").unwrap();
        assert_eq!(
            Sysfs::new(dir.path()).read().unwrap(),
            BatteryState {
                percent: 75,
                status: BatteryStatus::Discharging,
            }
        );
    }

    #[test]
    fn ignores_non_battery_power_supplies() {
        let dir = tempfile::tempdir().unwrap();
        let ac = dir.path().join("AC");
        fs::create_dir_all(&ac).unwrap();
        fs::write(ac.join("type"), "Mains").unwrap();
        assert!(Sysfs::new(dir.path()).read().is_err());
    }
}
