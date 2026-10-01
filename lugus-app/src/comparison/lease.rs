use crate::{AppError, ErrorKind, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};
#[derive(Debug)]
pub struct ComparisonLease {
    file: File,
    store_key: PathBuf,
    workspace_id: String,
}
impl ComparisonLease {
    pub(crate) fn acquire(store: &Path, workspace_id: &str) -> Result<Self> {
        crate::conversations::validate_id(workspace_id)?;
        let key = std::fs::canonicalize(store).map_err(failure)?;
        let mut name = key.as_os_str().to_owned();
        name.push(format!(
            ".comparison-{:x}.lock",
            Sha256::digest(workspace_id.as_bytes())
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(PathBuf::from(name))
            .map_err(failure)?;
        file.try_lock().map_err(|e| match e {
            TryLockError::WouldBlock => AppError::new(
                ErrorKind::Conflict,
                "company comparison is already running",
                false,
            ),
            _ => failure(e),
        })?;
        Ok(Self {
            file,
            store_key: key,
            workspace_id: workspace_id.into(),
        })
    }
    pub(crate) fn matches(&self, path: &Path, id: &str) -> bool {
        self.store_key == path && self.workspace_id == id
    }
}
impl Drop for ComparisonLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
fn failure(_: impl std::fmt::Display) -> AppError {
    AppError::new(
        ErrorKind::Storage,
        "comparison execution lease unavailable",
        false,
    )
}
