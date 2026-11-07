#[cfg(any(target_os = "freebsd", test))]
mod freebsd;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "netbsd", test))]
mod netbsd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuState {
    percent: u64,
}

impl CpuState {
    pub fn percent(self) -> u64 {
        self.percent
    }
}

pub trait CpuProvider {
    /// The first sample establishes a baseline and produces no usage value.
    fn read(&mut self) -> Result<Option<CpuState>, String>;
}

enum ProviderKind {
    Auto,
    #[cfg(target_os = "freebsd")]
    FreeBsd,
    #[cfg(target_os = "linux")]
    Linux,
    #[cfg(target_os = "netbsd")]
    NetBsd,
}

impl ProviderKind {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "auto" => Ok(Self::Auto),
            #[cfg(target_os = "freebsd")]
            "freebsd" => Ok(Self::FreeBsd),
            #[cfg(target_os = "linux")]
            "linux" => Ok(Self::Linux),
            #[cfg(target_os = "netbsd")]
            "netbsd" => Ok(Self::NetBsd),
            _ => Err(format!("unknown CPU provider `{name}`")),
        }
    }
}

pub fn validate_name(name: &str) -> Result<(), String> {
    ProviderKind::parse(name).map(|_| ())
}

pub fn create(name: &str) -> Result<Box<dyn CpuProvider>, String> {
    match ProviderKind::parse(name)? {
        ProviderKind::Auto => create_auto(),
        #[cfg(target_os = "freebsd")]
        ProviderKind::FreeBsd => Ok(Box::new(freebsd::CpTime::default())),
        #[cfg(target_os = "linux")]
        ProviderKind::Linux => Ok(Box::new(linux::ProcStat::new("/proc/stat"))),
        #[cfg(target_os = "netbsd")]
        ProviderKind::NetBsd => Ok(Box::new(netbsd::CpTime::default())),
    }
}

#[cfg(target_os = "linux")]
fn create_auto() -> Result<Box<dyn CpuProvider>, String> {
    create("linux")
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "netbsd")))]
fn create_auto() -> Result<Box<dyn CpuProvider>, String> {
    Err(format!(
        "no CPU provider available for {}",
        std::env::consts::OS
    ))
}

#[cfg(target_os = "freebsd")]
fn create_auto() -> Result<Box<dyn CpuProvider>, String> {
    create("freebsd")
}

#[cfg(target_os = "netbsd")]
fn create_auto() -> Result<Box<dyn CpuProvider>, String> {
    create("netbsd")
}

pub struct Unavailable {
    pub error: String,
}

impl CpuProvider for Unavailable {
    fn read(&mut self) -> Result<Option<CpuState>, String> {
        Err(self.error.clone())
    }
}
