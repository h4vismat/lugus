use std::path::Path;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::{Error, Result};

const SUPPORTED_VERSIONS: &[&str] = &["0.153.4", "0.154.0"];
const OUTPUT_LIMIT: usize = 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) async fn probe_supported_version(executable: &Path, workspace: &Path) -> Result<()> {
    let mut child = Command::new(executable)
        .arg("--version")
        .current_dir(workspace)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(process_error)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Process("version probe stdout is unavailable".into()))?;
    let operation = async {
        let mut output = Vec::new();
        let mut bounded_stdout = stdout.take((OUTPUT_LIMIT + 1) as u64);
        let read = bounded_stdout.read_to_end(&mut output);
        let (read, status) = tokio::join!(read, child.wait());
        read.map_err(process_error)?;
        let status = status.map_err(process_error)?;
        if !status.success() {
            return Err(Error::Process("Codex version probe failed".into()));
        }
        validate_output(&output)
    };

    match tokio::time::timeout(PROBE_TIMEOUT, operation).await {
        Ok(result) => result,
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(Error::Timeout)
        }
    }
}

fn validate_output(output: &[u8]) -> Result<()> {
    if output.len() > OUTPUT_LIMIT {
        return Err(Error::Process("Codex version output exceeds limit".into()));
    }
    let output = std::str::from_utf8(output)
        .map_err(|_| Error::Process("Codex version output is not UTF-8".into()))?;
    let version = output
        .split_whitespace()
        .last()
        .ok_or_else(|| Error::Process("Codex version output is empty".into()))?;
    if !SUPPORTED_VERSIONS.contains(&version) {
        return Err(Error::Configuration(format!(
            "unsupported Codex version {version}; expected {}",
            SUPPORTED_VERSIONS.join(" or ")
        )));
    }
    Ok(())
}

fn process_error(error: std::io::Error) -> Error {
    Error::Process(error.to_string())
}
