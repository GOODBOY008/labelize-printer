use anyhow::{Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let root = std::env::var_os("LABELIZE_PRINTER_HOME")
            .map(PathBuf::from)
            .unwrap_or(
                directories::ProjectDirs::from("com", "Labelize", "LabelizePrinter")
                    .context("Cannot locate the user's application directory")?
                    .data_local_dir()
                    .to_path_buf(),
            );
        Self::new(root)
    }
    pub fn new(root: PathBuf) -> Result<Self> {
        anyhow::ensure!(root.is_absolute(), "Application directory must be absolute");
        private_dir(&root)?;
        private_dir(&root.join("jobs"))?;
        private_dir(&root.join("incoming"))?;
        Ok(Self { root })
    }
    pub fn database(&self) -> PathBuf {
        self.root.join("printer.sqlite3")
    }
    pub fn job_dir(&self, id: &str) -> Result<PathBuf> {
        uuid::Uuid::parse_str(id).context("Invalid job ID")?;
        Ok(self.root.join("jobs").join(id))
    }
    pub fn token(&self) -> Result<String> {
        fs::read_to_string(self.root.join("ipc.token")).context("Background service is not running")
    }
    pub fn ensure_token(&self) -> Result<()> {
        let path = self.root.join("ipc.token");
        if !path.exists() {
            atomic_write(
                &path,
                format!(
                    "{}{}",
                    uuid::Uuid::new_v4().simple(),
                    uuid::Uuid::new_v4().simple()
                )
                .as_bytes(),
            )?;
        }
        private_file(&path)
    }
}

pub fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    let _ = path;
    Ok(())
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
