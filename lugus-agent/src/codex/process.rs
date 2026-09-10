use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;

use crate::{Error, Result};

const STDERR_TAIL_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy)]
pub(super) struct TransportLimits {
    pub(super) max_frame_bytes: usize,
    pub(super) request_timeout: Duration,
    pub(super) shutdown_grace: Duration,
}

impl Default for TransportLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 8 * 1024 * 1024,
            request_timeout: Duration::from_secs(30),
            shutdown_grace: Duration::from_secs(2),
        }
    }
}

pub(super) struct CodexProcess {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    stderr_drain: Option<JoinHandle<()>>,
    limits: TransportLimits,
    pending_frame: Vec<u8>,
    unusable: bool,
}

impl CodexProcess {
    pub(super) fn spawn(
        command: &Path,
        args: &[String],
        cwd: &Path,
        limits: TransportLimits,
    ) -> Result<Self> {
        if limits.max_frame_bytes == 0 {
            return Err(Error::Configuration(
                "max_frame_bytes must be positive".into(),
            ));
        }
        if limits.request_timeout.is_zero() || limits.shutdown_grace.is_zero() {
            return Err(Error::Configuration(
                "transport durations must be positive".into(),
            ));
        }

        let mut child = Command::new(command)
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(process_error)?;

        let stdin = child.stdin.take().ok_or_else(missing_pipe)?;
        let stdout = child.stdout.take().ok_or_else(missing_pipe)?;
        let stderr = child.stderr.take().ok_or_else(missing_pipe)?;

        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout,
            stderr_drain: Some(tokio::spawn(drain_stderr(stderr))),
            limits,
            pending_frame: Vec::new(),
            unusable: false,
        })
    }

    pub(super) async fn send(&mut self, message: &Value) -> Result<()> {
        self.ensure_usable()?;

        let encoded = serde_json::to_vec(message)
            .map_err(|error| Error::Protocol(format!("cannot encode process message: {error}")))?;
        if encoded.len() > self.limits.max_frame_bytes {
            self.unusable = true;
            self.stdin.take();
            return Err(Error::FrameTooLarge {
                limit: self.limits.max_frame_bytes,
            });
        }

        let stdin = self.stdin.as_mut().ok_or_else(|| {
            self.unusable = true;
            Error::Process("process stdin is unavailable".into())
        })?;
        let deadline = self.limits.request_timeout;
        let write = async {
            stdin.write_all(&encoded).await.map_err(process_error)?;
            stdin.write_all(b"\n").await.map_err(process_error)?;
            stdin.flush().await.map_err(process_error)
        };

        match tokio::time::timeout(deadline, write).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                self.unusable = true;
                self.stdin.take();
                Err(error)
            }
            Err(_) => {
                self.unusable = true;
                self.stdin.take();
                Err(Error::Timeout)
            }
        }
    }

    pub(super) async fn receive(&mut self) -> Result<Value> {
        self.ensure_usable()?;

        loop {
            if let Some(frame) = self.take_complete_frame()? {
                return decode_frame(frame).map_err(|error| self.fail(error));
            }

            let remaining = self.limits.max_frame_bytes + 1 - self.pending_frame.len();
            let mut bytes = [0_u8; 8192];
            let capacity = remaining.min(bytes.len());
            let read = tokio::time::timeout(
                self.limits.request_timeout,
                self.stdout.read(&mut bytes[..capacity]),
            )
            .await;

            let count = match read {
                Ok(Ok(count)) => count,
                Ok(Err(error)) => return Err(self.fail(process_error(error))),
                Err(_) => return Err(Error::Timeout),
            };
            if count == 0 {
                return Err(self.fail(Error::UnexpectedEof));
            }
            self.pending_frame.extend_from_slice(&bytes[..count]);
        }
    }

    pub(super) async fn close(&mut self) -> Result<()> {
        self.stdin.take();

        let mut result = Ok(());
        if let Some(child) = self.child.as_mut() {
            match tokio::time::timeout(self.limits.shutdown_grace, child.wait()).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => result = Err(process_error(error)),
                Err(_) => {
                    if let Err(error) = child.start_kill() {
                        result = Err(process_error(error));
                    }
                    if let Err(error) = child.wait().await {
                        result = Err(process_error(error));
                    }
                }
            }
        }
        self.child.take();

        if let Some(stderr_drain) = self.stderr_drain.take()
            && let Err(error) = stderr_drain.await
            && result.is_ok()
        {
            result = Err(Error::Process(format!("stderr drain task failed: {error}")));
        }

        result
    }

    fn ensure_usable(&self) -> Result<()> {
        if self.unusable {
            return Err(Error::Process("process transport is unusable".into()));
        }
        Ok(())
    }

    fn take_complete_frame(&mut self) -> Result<Option<Vec<u8>>> {
        let Some(newline) = self.pending_frame.iter().position(|byte| *byte == b'\n') else {
            if self.pending_frame.len() > self.limits.max_frame_bytes {
                return Err(self.fail(Error::FrameTooLarge {
                    limit: self.limits.max_frame_bytes,
                }));
            }
            return Ok(None);
        };

        if newline > self.limits.max_frame_bytes {
            return Err(self.fail(Error::FrameTooLarge {
                limit: self.limits.max_frame_bytes,
            }));
        }
        Ok(Some(self.pending_frame.drain(..=newline).collect()))
    }

    fn fail(&mut self, error: Error) -> Error {
        self.unusable = true;
        error
    }
}

fn decode_frame(mut frame: Vec<u8>) -> Result<Value> {
    frame.pop();
    let frame = std::str::from_utf8(&frame).map_err(|_| Error::InvalidUtf8)?;
    serde_json::from_str(frame).map_err(|error| Error::MalformedJson(error.to_string()))
}

async fn drain_stderr<R: AsyncRead + Unpin>(mut stderr: R) {
    let mut bytes = [0_u8; 1024];
    let mut tail = VecDeque::with_capacity(STDERR_TAIL_BYTES);
    loop {
        let Ok(count) = stderr.read(&mut bytes).await else {
            break;
        };
        if count == 0 {
            break;
        }
        for byte in &bytes[..count] {
            if tail.len() == STDERR_TAIL_BYTES {
                tail.pop_front();
            }
            tail.push_back(*byte);
        }
    }
}

fn missing_pipe() -> Error {
    Error::Process("child process pipe was unavailable".into())
}

fn process_error(error: std::io::Error) -> Error {
    Error::Process(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use serde_json::json;

    use super::{CodexProcess, TransportLimits};
    use crate::Error;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex_server.py")
    }

    fn limits() -> TransportLimits {
        TransportLimits {
            max_frame_bytes: 1024,
            request_timeout: Duration::from_secs(1),
            shutdown_grace: Duration::from_millis(100),
        }
    }

    #[tokio::test]
    async fn rejects_an_oversized_frame_and_reaps_the_child() {
        let mut process = CodexProcess::spawn(
            PathBuf::from("python3").as_path(),
            &[fixture().display().to_string(), "oversized".into()],
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).as_path(),
            limits(),
        )
        .unwrap();

        assert!(matches!(
            process.receive().await,
            Err(Error::FrameTooLarge { limit: 1024 })
        ));
        process.close().await.unwrap();
    }

    #[tokio::test]
    async fn preserves_a_partial_frame_when_a_receive_is_cancelled() {
        let mut process = CodexProcess::spawn(
            PathBuf::from("python3").as_path(),
            &[fixture().display().to_string(), "partial".into()],
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).as_path(),
            limits(),
        )
        .unwrap();

        assert!(
            tokio::time::timeout(Duration::from_millis(100), process.receive())
                .await
                .is_err()
        );
        assert_eq!(
            process.receive().await.unwrap(),
            json!({"id": 1, "result": {}})
        );
        process.close().await.unwrap();
    }

    #[tokio::test]
    async fn sends_a_json_frame_and_receives_the_server_response() {
        let mut process = spawn("echo", limits()).unwrap();

        process.send(&json!({"id": "request-1"})).await.unwrap();
        assert_eq!(
            process.receive().await.unwrap(),
            json!({"id": "request-1", "result": {}})
        );
        process.close().await.unwrap();
    }

    #[tokio::test]
    async fn malformed_json_makes_the_transport_unusable() {
        let mut process = spawn("malformed", limits()).unwrap();

        assert!(matches!(
            process.receive().await,
            Err(Error::MalformedJson(_))
        ));
        assert!(
            matches!(process.receive().await, Err(Error::Process(message)) if message.contains("unusable"))
        );
        process.close().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_utf8_makes_the_transport_unusable() {
        let mut process = spawn("invalid_utf8", limits()).unwrap();

        assert!(matches!(process.receive().await, Err(Error::InvalidUtf8)));
        assert!(
            matches!(process.receive().await, Err(Error::Process(message)) if message.contains("unusable"))
        );
        process.close().await.unwrap();
    }

    #[tokio::test]
    async fn receive_observes_the_request_deadline() {
        let mut short_limits = limits();
        short_limits.request_timeout = Duration::from_millis(50);
        let mut process = spawn("silent", short_limits).unwrap();

        assert!(matches!(process.receive().await, Err(Error::Timeout)));
        process.close().await.unwrap();
    }

    #[tokio::test]
    async fn eof_makes_the_transport_unusable() {
        let mut process = spawn("eof", limits()).unwrap();

        assert!(matches!(process.receive().await, Err(Error::UnexpectedEof)));
        assert!(
            matches!(process.receive().await, Err(Error::Process(message)) if message.contains("unusable"))
        );
        process.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_timed_out_write_does_not_leave_the_transport_reusable() {
        let mut short_limits = limits();
        short_limits.max_frame_bytes = 1024 * 1024;
        short_limits.request_timeout = Duration::from_millis(50);
        let mut process = spawn("blocked_stdin", short_limits).unwrap();

        assert!(matches!(
            process
                .send(&json!({"id": 3, "body": "x".repeat(128 * 1024)}))
                .await,
            Err(Error::Timeout)
        ));
        assert!(
            matches!(process.send(&json!({"id": 4})).await, Err(Error::Process(message)) if message.contains("unusable"))
        );
        process.close().await.unwrap();
    }

    #[tokio::test]
    async fn drains_flooded_stderr_while_receiving_stdout() {
        let mut process = spawn("stderr_flood", limits()).unwrap();

        process.send(&json!({"id": 2})).await.unwrap();
        assert_eq!(
            process.receive().await.unwrap(),
            json!({"id": 2, "result": {}})
        );
        process.close().await.unwrap();
    }

    #[tokio::test]
    async fn close_kills_a_child_that_ignores_stdin_eof() {
        let mut short_limits = limits();
        short_limits.shutdown_grace = Duration::from_millis(50);
        let mut process = spawn("ignore_eof", short_limits).unwrap();

        tokio::time::timeout(Duration::from_secs(1), process.close())
            .await
            .expect("close must reap the child")
            .unwrap();
    }

    fn spawn(scenario: &str, limits: TransportLimits) -> crate::Result<CodexProcess> {
        CodexProcess::spawn(
            PathBuf::from("python3").as_path(),
            &[fixture().display().to_string(), scenario.into()],
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).as_path(),
            limits,
        )
    }
}
