//! Intent-before-effect and exact-result-before-return. Detached calls remain owned by the turn.
use super::{
    runtime::{failure, isolate},
    *,
};
use crate::{
    AppError, Application, ErrorKind, ResearchExecutor, Result,
    agent_contract::check_serialized_size,
};
use lugus_agent::{ToolCall, ToolExecutor, ToolResult};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use tokio::sync::watch;
struct Calls {
    closed: bool,
    next: u64,
    pending: BTreeMap<u64, watch::Receiver<bool>>,
    error: Option<AppError>,
    accepted: BTreeSet<String>,
}
struct Inner {
    app: Application,
    attempt: RunAttempt,
    research: ResearchExecutor,
    capacity: usize,
    max_calls: usize,
    cancel: watch::Sender<bool>,
    calls: Mutex<Calls>,
}
#[derive(Clone)]
pub(super) struct Journal {
    inner: Arc<Inner>,
}
impl Journal {
    pub fn new(
        app: Application,
        attempt: RunAttempt,
        research: ResearchExecutor,
        capacity: usize,
        max_calls: usize,
        cancel: watch::Sender<bool>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                app,
                attempt,
                research,
                capacity,
                max_calls,
                cancel,
                calls: Mutex::new(Calls {
                    closed: false,
                    next: 0,
                    pending: BTreeMap::new(),
                    error: None,
                    accepted: BTreeSet::new(),
                }),
            }),
        }
    }
    fn calls(&self) -> std::sync::MutexGuard<'_, Calls> {
        self.inner.calls.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn fail(&self, error: AppError) -> ToolResult {
        self.calls().error.get_or_insert(error.clone());
        self.inner.cancel.send_replace(true);
        error_result(&error)
    }
    pub async fn drain(&self) -> Result<()> {
        let pending: Vec<_> = {
            let mut calls = self.calls();
            calls.closed = true;
            if !calls.pending.is_empty() && !*self.inner.cancel.borrow() {
                calls.error.get_or_insert_with(|| {
                    failure(
                        ErrorKind::Unavailable,
                        "runtime ended with outstanding tool calls",
                    )
                });
                self.inner.cancel.send_replace(true);
            }
            calls.pending.values().cloned().collect()
        };
        for mut done in pending {
            while !*done.borrow_and_update() {
                if done.changed().await.is_err() {
                    return Err(failure(ErrorKind::Unavailable, "tool supervision stopped"));
                }
            }
        }
        self.calls().error.clone().map_or(Ok(()), Err)
    }
    async fn execute_owned(&self, call: ToolCall) -> Result<ToolResult> {
        validate_id(&call.call_id)?;
        validate_id(&call.run_id)?;
        if call.run_id != self.inner.attempt.run_id() {
            return Err(failure(
                ErrorKind::ScopeMismatch,
                "tool run identity does not match",
            ));
        }
        if !self
            .inner
            .research
            .tool_specs()
            .iter()
            .any(|s| s.name == call.name)
        {
            return Err(failure(
                ErrorKind::Unsupported,
                "tool was not offered for this turn",
            ));
        }
        check_serialized_size(&call, self.inner.app.limits().max_input_bytes)?;
        let arguments = serde_json::to_string(&call.arguments)
            .map_err(|_| failure(ErrorKind::InvalidInput, "tool arguments are invalid"))?;
        {
            let mut calls = self.calls();
            if !calls.accepted.contains(&call.call_id)
                && calls.accepted.len() >= self.inner.max_calls
            {
                return Err(failure(
                    ErrorKind::ResourceLimit,
                    "turn tool call limit is exhausted",
                ));
            }
            calls.accepted.insert(call.call_id.clone());
        }
        let intent = ToolIntent {
            call_id: call.call_id.clone(),
            name: call.name.clone(),
            arguments,
            result_capacity: self.inner.capacity,
        };
        let attempt = self.inner.attempt.clone();
        let begin = self
            .inner
            .app
            .conversation_effect(move |s| s.begin_tool(&attempt, &intent))
            .await?;
        if let BeginTool::Recorded(record) = begin {
            return match record.outcome {
                Some(ToolOutcome::Returned { result }) => Ok(result),
                Some(ToolOutcome::Failed { error }) => Err(error),
                None => Err(failure(ErrorKind::Conflict, "tool outcome is unknown")),
            };
        }
        let result = if *self.inner.cancel.borrow() {
            Err(failure(
                ErrorKind::Cancelled,
                "tool execution was cancelled",
            ))
        } else {
            isolate(self.inner.research.execute_recorded(call.clone()))
                .await
                .and_then(|result| result)
        };
        let outcome = match result {
            Ok(result) => {
                let returned = ToolOutcome::Returned { result };
                match check_serialized_size(&returned, self.inner.capacity) {
                    Ok(()) => returned,
                    Err(_) => ToolOutcome::Failed {
                        error: failure(
                            ErrorKind::ResourceLimit,
                            "tool result exceeds journal capacity",
                        ),
                    },
                }
            }
            Err(error) => ToolOutcome::Failed { error },
        };
        let attempt = self.inner.attempt.clone();
        let saved = self
            .inner
            .app
            .conversation_effect(move |s| s.finish_tool(&attempt, &call.call_id, &outcome))
            .await?;
        match saved.outcome {
            Some(ToolOutcome::Returned { result }) => Ok(result),
            Some(ToolOutcome::Failed { error }) => Err(error),
            None => Err(failure(ErrorKind::Storage, "tool result was not recorded")),
        }
    }
}
#[async_trait::async_trait]
impl ToolExecutor for Journal {
    async fn execute(&self, call: ToolCall) -> ToolResult {
        let (done, mut completion) = watch::channel(false);
        let (result, mut response) = watch::channel(None);
        let key = {
            let mut calls = self.calls();
            if calls.closed || calls.pending.len() >= self.inner.max_calls {
                drop(calls);
                return self.fail(failure(
                    ErrorKind::ResourceLimit,
                    "tool admission is closed or full",
                ));
            }
            let key = calls.next;
            calls.next += 1;
            calls.pending.insert(key, completion.clone());
            key
        };
        let journal = self.clone();
        tokio::spawn(async move {
            let value = match isolate(journal.execute_owned(call)).await {
                Ok(Ok(value)) => value,
                Ok(Err(error)) | Err(error) => journal.fail(error),
            };
            result.send_replace(Some(value));
            journal.calls().pending.remove(&key);
            done.send_replace(true);
        });
        loop {
            if let Some(value) = response.borrow_and_update().clone() {
                return value;
            }
            tokio::select! {
                _ = response.changed() => {},
                changed = completion.changed() => {
                    if changed.is_err() && response.borrow().is_none() { return self.fail(failure(ErrorKind::Unavailable, "tool supervisor stopped")); }
                }
            }
        }
    }
}
fn error_result(error: &AppError) -> ToolResult {
    ToolResult {
        success: false,
        content: serde_json::to_string(error).expect("bounded error serializes"),
    }
}
