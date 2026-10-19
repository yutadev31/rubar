#[cfg(any(target_os = "freebsd", test))]
mod freebsd;
#[cfg(target_os = "linux")]
mod linux;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryState {
    total_bytes: u64,
    available_bytes: u64,
}

impl MemoryState {
    pub fn new(total_bytes: u64, available_bytes: u64) -> Result<Self, String> {
        if total_bytes == 0 {
            return Err("total memory must be greater than zero".to_string());
        }
        if available_bytes > total_bytes {
            return Err("available memory exceeds total memory".to_string());
        }
        Ok(Self {
            total_bytes,
            available_bytes,
        })
    }

    pub fn total_bytes(self) -> u64 {
        self.total_bytes
    }

    pub fn available_bytes(self) -> u64 {
        self.available_bytes
    }

    pub fn used_bytes(self) -> u64 {
        self.total_bytes - self.available_bytes
    }

    pub fn percent(self) -> u64 {
        ((u128::from(self.used_bytes()) * 100 + u128::from(self.total_bytes) / 2)
            / u128::from(self.total_bytes)) as u64
    }
}

pub trait MemoryProvider {
    fn read(&mut self) -> Result<MemoryState, String>;
}

enum ProviderKind {
    Auto,
    #[cfg(target_os = "freebsd")]
    FreeBsd,
    #[cfg(target_os = "linux")]
    Linux,
}

impl ProviderKind {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "auto" => Ok(Self::Auto),
            #[cfg(target_os = "freebsd")]
            "freebsd" => Ok(Self::FreeBsd),
            #[cfg(target_os = "linux")]
            "linux" => Ok(Self::Linux),
            _ => Err(format!("unknown memory provider `{name}`")),
        }
    }
}

pub fn validate_name(name: &str) -> Result<(), String> {
    ProviderKind::parse(name).map(|_| ())
}

pub fn create(name: &str) -> Result<Box<dyn MemoryProvider>, String> {
    match ProviderKind::parse(name)? {
        ProviderKind::Auto => create_auto(),
        #[cfg(target_os = "freebsd")]
        ProviderKind::FreeBsd => Ok(Box::new(freebsd::VmStats)),
        #[cfg(target_os = "linux")]
        ProviderKind::Linux => Ok(Box::new(linux::ProcMeminfo::new("/proc/meminfo"))),
    }
}

#[cfg(target_os = "linux")]
fn create_auto() -> Result<Box<dyn MemoryProvider>, String> {
    create("linux")
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn create_auto() -> Result<Box<dyn MemoryProvider>, String> {
    Err(format!(
        "no memory provider available for {}",
        std::env::consts::OS
    ))
}

#[cfg(target_os = "freebsd")]
fn create_auto() -> Result<Box<dyn MemoryProvider>, String> {
    create("freebsd")
}

pub struct Unavailable {
    pub error: String,
}

impl MemoryProvider for Unavailable {
    fn read(&mut self) -> Result<MemoryState, String> {
        Err(self.error.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_used_memory_and_percentage() {
        let state = MemoryState::new(8 * 1_073_741_824, 6 * 1_073_741_824).unwrap();
        assert_eq!(state.used_bytes(), 2 * 1_073_741_824);
        assert_eq!(state.percent(), 25);
        assert!(MemoryState::new(0, 0).is_err());
        assert!(MemoryState::new(100, 101).is_err());
    }
}
