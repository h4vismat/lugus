use crate::{AppError, ErrorKind, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};
#[derive(Debug)]
pub struct HistoryLease {
    file: File,
    store_key: PathBuf,
    portfolio_id: String,
}
impl HistoryLease {
    pub(crate) fn acquire(store: &Path, portfolio_id: &str) -> Result<Self> {
        super::id(portfolio_id)?;
        let key = std::fs::canonicalize(store).map_err(failure)?;
        let mut name = key.as_os_str().to_owned();
        name.push(format!(
            ".portfolio-history-{:x}.lock",
            Sha256::digest(portfolio_id.as_bytes())
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
                "portfolio history is already running",
                false,
            ),
            _ => failure(e),
        })?;
        Ok(Self {
            file,
            store_key: key,
            portfolio_id: portfolio_id.into(),
        })
    }
    pub(crate) fn matches(&self, path: &Path, id: &str) -> bool {
        self.store_key == path && self.portfolio_id == id
    }
}
impl Drop for HistoryLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
fn failure(_: impl std::fmt::Display) -> AppError {
    AppError::new(
        ErrorKind::Storage,
        "historical execution lease unavailable",
        false,
    )
}
