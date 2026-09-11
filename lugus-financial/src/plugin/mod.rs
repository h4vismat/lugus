//! Versioned external process adapter. Plugins never receive a database handle.
use crate::{
    capabilities::*,
    domain::*,
    error::{Error, ErrorKind, Result},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    task::JoinHandle,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub protocol_version: u32,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}
impl Manifest {
    pub fn load(path: impl AsRef<Path>) -> Result<(Self, PathBuf)> {
        let path = std::fs::canonicalize(path)
            .map_err(|e| Error::new(ErrorKind::Configuration, e.to_string()))?;
        let contents = std::fs::read(&path)
            .map_err(|e| Error::new(ErrorKind::Configuration, e.to_string()))?;
        let manifest = serde_json::from_slice(&contents)
            .map_err(|e| Error::new(ErrorKind::Configuration, format!("invalid manifest: {e}")))?;
        Ok((manifest, path.parent().unwrap().to_path_buf()))
    }
}
#[derive(Debug, Clone)]
pub struct Limits {
    pub timeout: Duration,
    pub max_response_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            max_response_bytes: 32 * 1024 * 1024,
        }
    }
}

pub struct Plugin {
    identity: ProviderIdentity,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
    stderr_task: Option<JoinHandle<()>>,
    capabilities: BTreeMap<String, u32>,
    next_id: u64,
    limits: Limits,
}

impl Plugin {
    pub async fn start(
        manifest: Manifest,
        directory: PathBuf,
        instance_id: String,
        config: Value,
        limits: Limits,
    ) -> Result<Self> {
        if manifest.protocol_version != 1
            || manifest.id.is_empty()
            || manifest.version.is_empty()
            || manifest.command.is_empty()
            || instance_id.is_empty()
            || limits.timeout.is_zero()
            || limits.max_response_bytes == 0
        {
            return Err(Error::new(
                ErrorKind::Configuration,
                "invalid manifest, instance ID or limits",
            ));
        }
        let executable = Path::new(&manifest.command);
        let executable = if executable.is_relative() && executable.components().count() > 1 {
            directory.join(executable)
        } else {
            executable.to_path_buf()
        };
        let mut child = Command::new(executable)
            .args(&manifest.args)
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                Error::new(
                    ErrorKind::Configuration,
                    format!("could not start plugin: {e}"),
                )
            })?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().map(BufReader::new);
        let mut stderr = child.stderr.take().unwrap();
        let stderr_task = tokio::spawn(async move {
            // Drain without retaining arbitrary plugin diagnostics or configuration secrets.
            let mut buffer = [0_u8; 4096];
            while let Ok(n) = stderr.read(&mut buffer).await {
                if n == 0 {
                    break;
                }
            }
        });
        let mut plugin = Self {
            identity: ProviderIdentity {
                instance_id,
                plugin_id: manifest.id,
                plugin_version: manifest.version,
            },
            child: Some(child),
            stdin,
            stdout,
            stderr_task: Some(stderr_task),
            capabilities: BTreeMap::new(),
            next_id: 1,
            limits,
        };
        #[derive(Deserialize)]
        struct Handshake {
            protocol_version: u32,
            plugin_id: String,
            plugin_version: String,
            capabilities: BTreeMap<String, u32>,
        }
        let initialization = plugin
            .call::<Handshake>("initialize", json!({"protocol_version":1,"config":config}))
            .await;
        match initialization {
            Ok(result)
                if result.protocol_version == 1
                    && result.plugin_id == plugin.identity.plugin_id
                    && result.plugin_version == plugin.identity.plugin_version =>
            {
                plugin.capabilities = result.capabilities;
                Ok(plugin)
            }
            Ok(_) => {
                plugin.close().await?;
                Err(Error::new(
                    ErrorKind::Protocol,
                    "initialization identity or protocol mismatch",
                ))
            }
            Err(error) => {
                let _ = plugin.close().await;
                Err(error)
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some() && self.stdin.is_some()
    }
    pub fn capabilities(&self) -> &BTreeMap<String, u32> {
        &self.capabilities
    }
    pub async fn close(&mut self) -> Result<()> {
        self.stdin.take();
        self.stdout.take();
        if let Some(child) = self.child.as_mut() {
            let kill = child.start_kill();
            // Keep ownership in self across the await: cancellation must allow a
            // subsequent close to explicitly finish reaping the same child.
            let wait = child.wait().await;
            if let Some(task) = self.stderr_task.take() {
                task.abort();
            }
            if let Err(error) = wait {
                return Err(Error::new(ErrorKind::Unavailable, error.to_string()));
            }
            self.child.take();
            // start_kill may fail for an already-exited child; wait is authoritative.
            let _ = kill;
        }
        Ok(())
    }

    fn supports(&self, name: &str) -> Result<()> {
        if self.capabilities.get(name) == Some(&1) {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::Unsupported,
                format!("provider does not advertise {name} v1"),
            ))
        }
    }

    async fn call<T: DeserializeOwned>(&mut self, method: &str, params: Value) -> Result<T> {
        let mut guard = RequestGuard {
            plugin: self,
            completed: false,
        };
        let result = guard.plugin.exchange(method, params).await;
        guard.completed = true;
        result
    }

    async fn exchange<T: DeserializeOwned>(&mut self, method: &str, params: Value) -> Result<T> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| Error::new(ErrorKind::Protocol, "request ID exhausted"))?;
        let request =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let timeout = self.limits.timeout;
        let max = self.limits.max_response_bytes;
        // A complete source error leaves framing synchronized. Transport failures,
        // deadlines and protocol violations still invalidate the process.
        let mut source_error = false;
        let result = tokio::time::timeout(timeout, async {
            let stdin = self
                .stdin
                .as_mut()
                .ok_or_else(|| Error::new(ErrorKind::Unavailable, "plugin is closed"))?;
            stdin.write_all(&request).await.map_err(io_error)?;
            stdin.write_all(b"\n").await.map_err(io_error)?;
            stdin.flush().await.map_err(io_error)?;
            let stdout = self
                .stdout
                .as_mut()
                .ok_or_else(|| Error::new(ErrorKind::Unavailable, "plugin is closed"))?;
            let line = bounded_line(stdout, max).await?;
            let value: Value = serde_json::from_slice(&line)
                .map_err(|_| Error::new(ErrorKind::Protocol, "plugin returned invalid JSON"))?;
            if value.get("jsonrpc") != Some(&json!("2.0"))
                || value.get("id") != Some(&json!(id))
                || value.get("result").is_some() == value.get("error").is_some()
            {
                return Err(Error::new(
                    ErrorKind::Protocol,
                    "invalid response envelope or request ID",
                ));
            }
            if let Some(error) = value.get("error") {
                if error.get("code").and_then(Value::as_i64).is_none()
                    || error.get("message").and_then(Value::as_str).is_none()
                {
                    return Err(Error::new(ErrorKind::Protocol, "invalid error envelope"));
                }
                let code = error["code"].as_i64().unwrap();
                let fallback = match code {
                    -32601 => ErrorKind::Unsupported,
                    -32600 | -32602 => ErrorKind::InvalidRequest,
                    _ => ErrorKind::Protocol,
                };
                let kind = error
                    .pointer("/data/kind")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or(fallback);
                source_error = kind != ErrorKind::Protocol;
                return Err(Error {
                    kind,
                    message: error["message"].as_str().unwrap().to_owned(),
                    retry_after_seconds: error
                        .pointer("/data/retry_after_seconds")
                        .and_then(Value::as_u64),
                });
            }
            serde_json::from_value(value["result"].clone()).map_err(|_| {
                Error::new(
                    ErrorKind::Protocol,
                    "result does not match capability schema",
                )
            })
        })
        .await
        .unwrap_or_else(|_| {
            Err(Error::new(
                ErrorKind::Timeout,
                "plugin request exceeded deadline",
            ))
        });
        if result.as_ref().is_err_and(|e| {
            !source_error
                && matches!(
                    e.kind,
                    ErrorKind::Timeout | ErrorKind::Protocol | ErrorKind::Unavailable
                )
        }) {
            let _ = self.close().await;
        }
        result
    }

    async fn checked_page<T: DeserializeOwned + Validate>(
        &mut self,
        method: &str,
        query: &Query,
    ) -> Result<Page<T>> {
        query.validate()?;
        let page: Page<T> = self.call(method, serde_json::to_value(query)?).await?;
        if page.items.len() > query.page_size
            || page.next_cursor.as_ref().is_some_and(|c| c.is_empty())
            || page.items.iter().any(|item| item.validate().is_err())
        {
            let _ = self.close().await;
            return Err(Error::new(ErrorKind::Protocol, "invalid capability page"));
        }
        Ok(page)
    }
}
/// Dropping an in-flight future cannot leave unread bytes reusable by a later call.
struct RequestGuard<'a> {
    plugin: &'a mut Plugin,
    completed: bool,
}
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.plugin.stdin.take();
            self.plugin.stdout.take();
            if let Some(child) = self.plugin.child.as_mut() {
                let _ = child.start_kill();
            }
            if let Some(task) = self.plugin.stderr_task.take() {
                task.abort();
            }
            // Keep the handle so close() or the next request can await reaping.
        }
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }
        // Tokio's kill_on_drop terminates the child; callers should await close to reap explicitly.
    }
}
fn io_error(error: std::io::Error) -> Error {
    Error::new(ErrorKind::Unavailable, error.to_string())
}
async fn bounded_line(reader: &mut BufReader<ChildStdout>, max: usize) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(io_error)?;
        if available.is_empty() {
            return Err(Error::new(
                ErrorKind::Protocol,
                "plugin closed stdout before a complete response",
            ));
        }
        let end = available.iter().position(|&b| b == b'\n');
        let count = end.map_or(available.len(), |index| index + 1);
        if line.len().saturating_add(count) > max {
            return Err(Error::new(
                ErrorKind::Protocol,
                "response exceeds size limit",
            ));
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(line);
        }
    }
}
impl Provider for Plugin {
    fn identity(&self) -> &ProviderIdentity {
        &self.identity
    }
}
#[async_trait]
impl FilingsProvider for Plugin {
    async fn list_filings(&mut self, query: &Query) -> Result<Page<Filing>> {
        self.supports("filings")?;
        self.checked_page("filings.list", query).await
    }
    async fn fetch_document(&mut self, source_url: &str, max_bytes: usize) -> Result<Document> {
        self.supports("filings")?;
        if max_bytes == 0 {
            return Err(Error::new(
                ErrorKind::InvalidRequest,
                "document limit must be positive",
            ));
        }
        let document: Document = self
            .call(
                "filings.document",
                json!({"source_url":source_url,"max_bytes":max_bytes}),
            )
            .await?;
        let validation = document.bytes(max_bytes);
        if document.source_url != source_url || validation.is_err() {
            let message = if document.source_url == source_url
                && validation
                    .as_ref()
                    .is_err_and(|error| error.message == "document exceeds size limit")
            {
                "document exceeds size limit"
            } else {
                "document URL, encoding or size is invalid"
            };
            let _ = self.close().await;
            return Err(Error::new(ErrorKind::Protocol, message));
        }
        Ok(document)
    }
}
#[async_trait]
impl FundamentalsProvider for Plugin {
    async fn fetch_facts(&mut self, query: &Query) -> Result<Page<Fact>> {
        self.supports("fundamentals")?;
        self.checked_page("fundamentals.facts", query).await
    }
}

#[async_trait]
impl MarketDataProvider for Plugin {
    async fn fetch_prices(
        &mut self,
        query: &crate::market_data::PriceQuery,
    ) -> Result<crate::market_data::PricePage> {
        self.supports("market_data")?;
        query.validate()?;
        let page: crate::market_data::PricePage = self
            .call("market_data.daily", serde_json::to_value(query)?)
            .await?;
        if page.validate_for(query).is_err() {
            let _ = self.close().await;
            return Err(Error::new(
                ErrorKind::Protocol,
                "invalid market-data page, scope or coverage",
            ));
        }
        Ok(page)
    }
}

#[async_trait]
impl crate::resolution::CompanyResolutionProvider for Plugin {
    async fn search_companies(
        &mut self,
        request: &crate::resolution::SearchRequest,
    ) -> Result<crate::resolution::ResolutionPage> {
        self.supports("company_resolution")?;
        request.validate()?;
        let page: crate::resolution::ResolutionPage = self
            .call("company_resolution.search", serde_json::to_value(request)?)
            .await?;
        if page.validate_for(request).is_err() {
            let _ = self.close().await;
            return Err(Error::new(
                ErrorKind::Protocol,
                "invalid resolution page or query match",
            ));
        }
        Ok(page)
    }
    async fn lookup_company(
        &mut self,
        request: &crate::resolution::LookupRequest,
    ) -> Result<crate::resolution::Candidate> {
        self.supports("company_resolution")?;
        request.validate()?;
        let candidate: crate::resolution::Candidate = self
            .call("company_resolution.lookup", serde_json::to_value(request)?)
            .await?;
        if candidate.validate().is_err()
            || candidate.identifier != request.identifier
            || !candidate.match_reasons.is_empty()
        {
            let _ = self.close().await;
            return Err(Error::new(
                ErrorKind::Protocol,
                "invalid company lookup identity or record",
            ));
        }
        Ok(candidate)
    }
}

#[async_trait]
impl crate::instruments::InstrumentProvider for Plugin {
    async fn lookup_instrument(
        &mut self,
        query: &crate::instruments::InstrumentLookup,
    ) -> Result<crate::instruments::InstrumentMetadata> {
        query.validate()?;
        self.supports("instrument_lookup")?;
        let metadata: crate::instruments::InstrumentMetadata = self
            .call("instrument_lookup.lookup", serde_json::to_value(query)?)
            .await?;
        if metadata.validate_for(query).is_err() {
            let _ = self.close().await;
            return Err(Error::new(
                ErrorKind::Protocol,
                "invalid instrument metadata or source identity",
            ));
        }
        Ok(metadata)
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use std::{future::Future, task::Poll};

    #[tokio::test]
    async fn interrupted_close_retains_child_for_explicit_reaping() {
        let mut plugin = Plugin::start(
            Manifest {
                id: "fixture".into(),
                version: "1".into(),
                protocol_version: 1,
                command: "python3".into(),
                args: vec![format!(
                    "{}/tests/fixtures/plugin.py",
                    env!("CARGO_MANIFEST_DIR")
                )],
            },
            std::env::temp_dir(),
            "close-test".into(),
            json!({}),
            Limits::default(),
        )
        .await
        .unwrap();
        let interrupted = {
            let mut closing = std::pin::pin!(plugin.close());
            std::future::poll_fn(|cx| {
                Poll::Ready(match closing.as_mut().poll(cx) {
                    Poll::Pending => true,
                    Poll::Ready(result) => {
                        result.unwrap();
                        false
                    }
                })
            })
            .await
        };
        if interrupted {
            assert!(
                plugin.child.is_some(),
                "an interrupted wait lost the only child handle"
            );
        }
        plugin.close().await.unwrap();
        assert!(plugin.child.is_none());
    }
}
