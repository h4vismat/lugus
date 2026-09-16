use crate::{Error, Result, ToolSpec};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot},
    task::{JoinHandle, JoinSet},
};

const MAX_BODY: usize = 1024 * 1024;
const MAX_CONNECTIONS: usize = 16;
pub(super) struct Invocation {
    pub id: Value,
    pub name: String,
    pub arguments: Value,
    pub reply: oneshot::Sender<Value>,
}
pub(super) struct McpServer {
    pub config: Value,
    pub calls: mpsc::Receiver<Invocation>,
    task: JoinHandle<()>,
}
impl Drop for McpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl McpServer {
    pub async fn start_with_policy(
        tools: Vec<ToolSpec>,
        max_calls: usize,
        unlimited: bool,
    ) -> Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(io_error)?;
        let address = listener.local_addr().map_err(io_error)?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|_| Error::Process("cannot generate MCP authorization token".into()))?;
        let authorization = format!(
            "Bearer {}",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let config = json!({"mcpServers":{"lugus":{"type":"http", "url":format!("http://{address}/mcp"), "headers":{"Authorization":authorization}}}});
        let (sender, calls) = mpsc::channel(16);
        let state = Arc::new(State {
            authorization,
            tools,
            sender,
            requests: AtomicUsize::new(0),
            max_requests: if unlimited {
                isize::MAX as usize
            } else {
                max_calls.saturating_add(256).min(10_000)
            },
            unlimited,
        });
        let task = tokio::spawn(async move {
            // Dropping this owner aborts every connection, including handlers awaiting a tool.
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break; };
                        if connections.len() >= MAX_CONNECTIONS { continue; }
                        let state = state.clone();
                        connections.spawn(async move {
                            let service = service_fn(move |request| handle(request, state.clone()));
                            let connection = hyper::server::conn::http1::Builder::new().max_buf_size(32 * 1024).serve_connection(TokioIo::new(stream), service);
                            let _ = tokio::time::timeout(if unlimited { Duration::MAX } else { Duration::from_secs(300) }, connection).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self {
            config,
            calls,
            task,
        })
    }
    pub async fn stop(&mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}
struct State {
    authorization: String,
    tools: Vec<ToolSpec>,
    sender: mpsc::Sender<Invocation>,
    requests: AtomicUsize,
    max_requests: usize,
    unlimited: bool,
}
type HttpResponse = Response<Full<Bytes>>;
fn response(status: StatusCode, value: Option<Value>) -> HttpResponse {
    let mut result = Response::new(Full::new(Bytes::from(
        value.map(|v| v.to_string()).unwrap_or_default(),
    )));
    *result.status_mut() = status;
    result.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    result
}
fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id,"result":result})
}
fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id,"error":{"code":code,"message":message}})
}
async fn handle(
    request: Request<Incoming>,
    state: Arc<State>,
) -> std::result::Result<HttpResponse, Infallible> {
    Ok(handle_inner(request, state).await)
}
async fn handle_inner(request: Request<Incoming>, state: Arc<State>) -> HttpResponse {
    if request.headers().contains_key(hyper::header::ORIGIN) {
        return response(StatusCode::FORBIDDEN, None);
    }
    if request
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        != Some(state.authorization.as_str())
    {
        return response(StatusCode::UNAUTHORIZED, None);
    }
    if request.uri().path() != "/mcp" {
        return response(StatusCode::NOT_FOUND, None);
    }
    if request.method() != Method::POST {
        return response(StatusCode::METHOD_NOT_ALLOWED, None);
    }
    if state.requests.fetch_add(1, Ordering::Relaxed) >= state.max_requests {
        return response(StatusCode::TOO_MANY_REQUESTS, None);
    }
    if !request
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(';').next() == Some("application/json"))
    {
        return response(StatusCode::UNSUPPORTED_MEDIA_TYPE, None);
    }
    let body = match tokio::time::timeout(
        if state.unlimited {
            Duration::MAX
        } else {
            Duration::from_secs(5)
        },
        Limited::new(
            request.into_body(),
            if state.unlimited {
                isize::MAX as usize
            } else {
                MAX_BODY
            },
        )
        .collect(),
    )
    .await
    {
        Ok(Ok(body)) => body.to_bytes(),
        Ok(Err(_)) => return response(StatusCode::PAYLOAD_TOO_LARGE, None),
        Err(_) => return response(StatusCode::REQUEST_TIMEOUT, None),
    };
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return response(StatusCode::BAD_REQUEST, None);
    };
    let id = message.get("id").cloned();
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || !message.is_object() {
        return response(StatusCode::BAD_REQUEST, None);
    }
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(id) = id else {
        return if method == "notifications/initialized" || method == "notifications/cancelled" {
            response(StatusCode::ACCEPTED, None)
        } else {
            response(StatusCode::BAD_REQUEST, None)
        };
    };
    if !id.is_string() && !id.is_number() {
        return response(StatusCode::BAD_REQUEST, None);
    }
    let result = match method {
        "initialize" => {
            let requested = message
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let version = match requested {
                "2025-03-26" | "2025-06-18" | "2025-11-25" => requested,
                _ => "2025-11-25",
            };
            rpc_result(
                id,
                json!({"protocolVersion":version,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"lugus","version":env!("CARGO_PKG_VERSION")}}),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(
            id,
            json!({"tools":state.tools.iter().map(|t| json!({"name":t.name,"description":t.description,"inputSchema":t.input_schema})).collect::<Vec<_>>()}),
        ),
        "tools/call" => {
            let name = message
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = message
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            if !state.tools.iter().any(|t| t.name == name) || !arguments.is_object() {
                rpc_error(id, -32602, "Unknown tool or invalid arguments")
            } else {
                let (reply, receive) = oneshot::channel();
                let invocation = Invocation {
                    id: id.clone(),
                    name: name.into(),
                    arguments,
                    reply,
                };
                if state.sender.try_send(invocation).is_err() {
                    return response(StatusCode::SERVICE_UNAVAILABLE, None);
                }
                match receive.await {
                    Ok(result) => rpc_result(id, result),
                    Err(_) => return response(StatusCode::SERVICE_UNAVAILABLE, None),
                }
            }
        }
        _ => rpc_error(id, -32601, "Method not found"),
    };
    response(StatusCode::OK, Some(result))
}
fn io_error(error: std::io::Error) -> Error {
    Error::Process(error.to_string())
}
