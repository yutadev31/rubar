#[cfg(target_os = "freebsd")]
mod freebsd;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "netbsd")]
mod netbsd;

pub trait ProcessProvider {
    fn read(&mut self) -> Result<usize, String>;
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
            _ => Err(format!("unknown process provider `{name}`")),
        }
    }
}

pub fn validate_name(name: &str) -> Result<(), String> {
    ProviderKind::parse(name).map(|_| ())
}

pub fn create(name: &str) -> Result<Box<dyn ProcessProvider>, String> {
    match ProviderKind::parse(name)? {
        ProviderKind::Auto => create_auto(),
        #[cfg(target_os = "freebsd")]
        ProviderKind::FreeBsd => Ok(Box::new(freebsd::KinfoProc)),
        #[cfg(target_os = "linux")]
        ProviderKind::Linux => Ok(Box::new(linux::ProcFs::new("/proc"))),
        #[cfg(target_os = "netbsd")]
        ProviderKind::NetBsd => Ok(Box::new(netbsd::KinfoProc)),
    }
}

#[cfg(target_os = "linux")]
fn create_auto() -> Result<Box<dyn ProcessProvider>, String> {
    create("linux")
}

#[cfg(target_os = "freebsd")]
fn create_auto() -> Result<Box<dyn ProcessProvider>, String> {
    create("freebsd")
}

#[cfg(target_os = "netbsd")]
fn create_auto() -> Result<Box<dyn ProcessProvider>, String> {
    create("netbsd")
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "netbsd")))]
fn create_auto() -> Result<Box<dyn ProcessProvider>, String> {
    Err(format!(
        "no process provider available for {}",
        std::env::consts::OS
    ))
}

pub struct Unavailable {
    pub error: String,
}

impl ProcessProvider for Unavailable {
    fn read(&mut self) -> Result<usize, String> {
        Err(self.error.clone())
    }
}
