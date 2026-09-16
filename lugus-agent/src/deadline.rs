//! Optional execution deadline; Duration::MAX means execution until cancellation.
use std::time::Duration;
use tokio::time::Instant;
#[derive(Clone, Copy, Debug)]
pub struct Deadline(Option<Instant>);
impl Deadline {
    pub fn after(duration: Duration) -> Self {
        Self(if duration == Duration::MAX {
            None
        } else {
            Instant::now().checked_add(duration)
        })
    }
    pub fn remaining(self) -> Duration {
        self.0
            .map(|at| at.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::MAX)
    }
    pub fn expired(self) -> bool {
        self.0.is_some_and(|at| Instant::now() >= at)
    }
    pub async fn wait(self) {
        match self.0 {
            Some(at) => tokio::time::sleep_until(at).await,
            None => std::future::pending::<()>().await,
        }
    }
}
impl From<Instant> for Deadline {
    fn from(at: Instant) -> Self {
        Self(Some(at))
    }
}
