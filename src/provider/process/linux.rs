use std::{fs, path::PathBuf};

use super::ProcessProvider;

pub struct ProcFs {
    path: PathBuf,
}

impl ProcFs {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl ProcessProvider for ProcFs {
    fn read(&mut self) -> Result<usize, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_pid_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("1")).unwrap();
        fs::create_dir(dir.path().join("42")).unwrap();
        fs::create_dir(dir.path().join("self")).unwrap();
        assert_eq!(ProcFs::new(dir.path()).read().unwrap(), 2);
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
