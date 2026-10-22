use std::{fs, path::PathBuf};

use super::ProcessProvider;

pub struct ProcFs {
    path: PathBuf,
    mounts_path: PathBuf,
}

impl ProcFs {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mounts_path = path.join("self/mounts");
        Self { path, mounts_path }
    }
}

impl ProcessProvider for ProcFs {
    fn read(&mut self) -> Result<usize, String> {
        let mounts = fs::read_to_string(&self.mounts_path)
            .map_err(|error| format!("could not read {}: {error}", self.mounts_path.display()))?;
        check_proc_mount(&mounts, &self.path)?;
        let entries = fs::read_dir(&self.path)
            .map_err(|error| format!("could not read {}: {error}", self.path.display()))?;
        let mut count = 0;
        for entry in entries {
            let entry = entry
                .map_err(|error| format!("could not enumerate {}: {error}", self.path.display()))?;
            let name = entry.file_name();
            if name
                .to_str()
                .is_some_and(|name| name.parse::<u32>().is_ok())
            {
                count += 1;
            }
        }
        Ok(count)
    }
}

fn check_proc_mount(mounts: &str, path: &std::path::Path) -> Result<(), String> {
    let mount_point = path
        .to_str()
        .ok_or_else(|| format!("invalid procfs path: {}", path.display()))?;
    let mut found = false;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let _source = fields.next();
        if fields.next() != Some(mount_point) {
            continue;
        }
        found = true;
        if fields.next() != Some("proc") {
            return Err(format!("{mount_point} is not mounted as procfs"));
        }
        let options = fields
            .next()
            .ok_or_else(|| format!("mount options are missing for {mount_point}"))?;
        for option in options.split(',') {
            if let Some(hidepid) = option.strip_prefix("hidepid=")
                && !matches!(hidepid, "0" | "off" | "1" | "noaccess")
            {
                return Err(format!(
                    "cannot count all processes: {mount_point} has hidepid={hidepid}"
                ));
            }
        }
    }
    if !found {
        return Err(format!("could not find procfs mount for {mount_point}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_pid_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("self")).unwrap();
        fs::write(
            dir.path().join("self/mounts"),
            format!("proc {} proc rw,hidepid=0 0 0\n", dir.path().display()),
        )
        .unwrap();
        fs::create_dir(dir.path().join("1")).unwrap();
        fs::create_dir(dir.path().join("42")).unwrap();
        assert_eq!(ProcFs::new(dir.path()).read().unwrap(), 2);
    }

    #[test]
    fn rejects_mounts_that_hide_processes() {
        for hidepid in ["2", "invisible", "4", "ptraceable", "unknown"] {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir(dir.path().join("self")).unwrap();
            fs::create_dir(dir.path().join("42")).unwrap();
            fs::write(
                dir.path().join("self/mounts"),
                format!(
                    "proc {} proc rw,hidepid={hidepid} 0 0\n",
                    dir.path().display()
                ),
            )
            .unwrap();
            assert!(
                ProcFs::new(dir.path())
                    .read()
                    .unwrap_err()
                    .contains(&format!("hidepid={hidepid}"))
            );
        }
    }

    #[test]
    fn accepts_mounts_with_visible_process_directories() {
        for options in ["rw", "rw,hidepid=0", "rw,hidepid=1", "rw,hidepid=noaccess"] {
            assert!(
                check_proc_mount(
                    &format!("proc /proc proc {options} 0 0\n"),
                    std::path::Path::new("/proc")
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn rejects_unverified_proc_mounts() {
        let path = std::path::Path::new("/proc");
        assert!(check_proc_mount("tmpfs /other tmpfs rw 0 0\n", path).is_err());
        assert!(check_proc_mount("tmpfs /proc tmpfs rw 0 0\n", path).is_err());
    }

    #[test]
    fn reports_missing_proc_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            ProcFs::new(dir.path().join("missing"))
                .read()
                .unwrap_err()
                .contains("could not read")
        );
    }
}
