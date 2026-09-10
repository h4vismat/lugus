pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("invalid configuration: {0}")]
    Configuration(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("runtime timed out")]
    Timeout,
    #[error("process error: {0}")]
    Process(String),
    #[error("process frame exceeds the {limit}-byte limit")]
    FrameTooLarge { limit: usize },
    #[error("process output ended before a complete frame")]
    UnexpectedEof,
    #[error("process output contains invalid UTF-8")]
    InvalidUtf8,
    #[error("process output contains malformed JSON: {0}")]
    MalformedJson(String),
    #[error("authentication is required")]
    AuthenticationRequired,
    #[error("runtime was cancelled")]
    Cancelled,
    #[error("runtime needs attention: {0}")]
    NeedsAttention(String),
    #[error("tool error: {0}")]
    Tool(String),
}
