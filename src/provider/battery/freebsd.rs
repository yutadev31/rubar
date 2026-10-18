use super::{BatteryState, BatteryStatus};

#[cfg(target_os = "freebsd")]
pub struct Acpi;

#[cfg(target_os = "freebsd")]
impl super::BatteryProvider for Acpi {
    fn read(&mut self) -> Result<BatteryState, String> {
        use crate::provider::freebsd::sysctl_i32;

        let units = sysctl_i32(c"hw.acpi.battery.units")?;
        if units <= 0 {
            return Err("no ACPI battery found".to_string());
        }
        let life = sysctl_i32(c"hw.acpi.battery.life")?;
        let state = sysctl_i32(c"hw.acpi.battery.state")?;
        battery_state(life, state)
    }
}

fn battery_state(life: i32, state: i32) -> Result<BatteryState, String> {
    let percent = u8::try_from(life)
        .ok()
        .filter(|percent| *percent <= 100)
        .ok_or_else(|| format!("invalid ACPI battery capacity: {life}"))?;
    // acpiio.h: DISCHARG=0x01, CHARGING=0x02, NOT_PRESENT=0x07.
    if state == 0x07 {
        return Err("ACPI battery is not present".to_string());
    }
    let status = if state & 0x02 != 0 {
        BatteryStatus::Charging
    } else if state & 0x01 != 0 {
        BatteryStatus::Discharging
    } else if percent == 100 {
        BatteryStatus::Full
    } else {
        BatteryStatus::Unknown
    };
    Ok(BatteryState { percent, status })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_acpi_battery_status() {
        assert_eq!(
            battery_state(42, 0x01).unwrap().status,
            BatteryStatus::Discharging
        );
        assert_eq!(
            battery_state(42, 0x02).unwrap().status,
            BatteryStatus::Charging
        );
        assert_eq!(battery_state(100, 0).unwrap().status, BatteryStatus::Full);
        assert_eq!(battery_state(42, 0).unwrap().status, BatteryStatus::Unknown);
        assert!(battery_state(101, 0).is_err());
        assert!(battery_state(42, 0x07).is_err());
    }
}
