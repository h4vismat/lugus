//! Exclusive local-file execution ownership. Hard-link aliases and network filesystems are unsupported.
use crate::{AppError, ErrorKind, Result};
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Debug)]
pub(crate) struct LeaseGuard {
    pub key: PathBuf,
    pub _file: File,
    pub activated: AtomicBool,
}
impl Drop for LeaseGuard {
    fn drop(&mut self) {
        // Explicit unlock releases this owner's lease even while a concurrently spawned child
        // transiently holds an inherited descriptor before exec closes it.
        let _ = self._file.unlock();
    }
}
#[derive(Debug)]
pub struct LocalExecutionLease {
    pub(crate) guard: Arc<LeaseGuard>,
}
impl LocalExecutionLease {
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self> {
        let key = std::fs::canonicalize(path).map_err(|_| failure())?;
        let mut lock = key.as_os_str().to_os_string();
        lock.push(".conversation.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(PathBuf::from(lock))
            .map_err(|_| failure())?;
        file.try_lock().map_err(|e| match e {
            TryLockError::WouldBlock => AppError::new(
                ErrorKind::Conflict,
                "conversation execution store is already owned",
                false,
            ),
            _ => failure(),
        })?;
        Ok(Self {
            guard: Arc::new(LeaseGuard {
                key,
                _file: file,
                activated: AtomicBool::new(false),
            }),
        })
    }
    pub fn store_key(&self) -> &Path {
        &self.guard.key
    }
    pub(crate) fn claim(&self) -> Result<()> {
        self.guard
            .activated
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| ())
            .map_err(|_| {
                AppError::new(
                    ErrorKind::Conflict,
                    "execution lease is already activated",
                    false,
                )
            })
    }
    pub(crate) fn unclaim(&self) {
        self.guard.activated.store(false, Ordering::SeqCst);
    }
}
fn failure() -> AppError {
    AppError::new(
        ErrorKind::Storage,
        "conversation execution lease operation failed",
        false,
    )
}
#[derive(Debug, Clone)]
pub struct ExecutionEpoch {
    pub(crate) guard: Arc<LeaseGuard>,
    pub(crate) generation: u64,
}
impl ExecutionEpoch {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn store_key(&self) -> &Path {
        &self.guard.key
    }
}
#[derive(Debug, Clone)]
pub struct RunAttempt {
    pub(crate) epoch: ExecutionEpoch,
    pub(crate) conversation_id: String,
    pub(crate) run_id: String,
}
impl RunAttempt {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
    pub fn epoch(&self) -> &ExecutionEpoch {
        &self.epoch
    }
}
