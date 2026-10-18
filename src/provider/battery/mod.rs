#[cfg(target_os = "linux")]
mod linux;

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
        "auto" => create_auto(),
        #[cfg(target_os = "linux")]
        "sysfs" | "linux" => Ok(Box::new(linux::Sysfs::new("/sys/class/power_supply"))),
        _ => Err(format!("unknown battery provider `{name}`")),
    }
}

#[cfg(target_os = "linux")]
fn create_auto() -> Result<Box<dyn BatteryProvider>, String> {
    let mut provider = linux::Sysfs::new("/sys/class/power_supply");
    provider.read()?;
    Ok(Box::new(provider))
}

#[cfg(not(target_os = "linux"))]
fn create_auto() -> Result<Box<dyn BatteryProvider>, String> {
    Err(format!(
        "no battery provider available for {}",
        std::env::consts::OS
    ))
}

pub struct Unavailable {
    pub error: String,
}

impl BatteryProvider for Unavailable {
    fn read(&mut self) -> Result<BatteryState, String> {
        Err(self.error.clone())
    }
}
