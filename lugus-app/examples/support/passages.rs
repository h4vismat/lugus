use super::{AgentResult, failure, invoke};
use crate::{CliResult, read_json, value};
use lugus_agent::{runtime::RunRequest, tools::ToolExecutor};
use lugus_app::{passages::*, *};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

fn invalid() -> AppError {
    AppError::new(ErrorKind::InvalidInput, "invalid passage command", false)
}

fn index(value: &str) -> CliResult<usize> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid().into());
    }
    value.parse().map_err(|_| invalid().into())
}

pub async fn local(
    app: &Application,
    scope: &Scope,
    command: &str,
    args: &[&str],
) -> Option<CliResult<Value>> {
    if !matches!(
        command,
        "prepare-text"
            | "text-header"
            | "read-text"
            | "create-passage"
            | "read-passage"
            | "resolve-passage"
    ) {
        return None;
    }
    Some(local_command(app, scope, command, args).await)
}

async fn local_command(
    app: &Application,
    scope: &Scope,
    command: &str,
    args: &[&str],
) -> CliResult<Value> {
    match (command, args) {
        ("prepare-text", [dataset]) => value(app.prepare_text(scope, dataset).await?),
        ("text-header", [representation]) => value(app.text_header(scope, representation).await?),
        ("read-text", [representation, start, end]) => value(
            app.read_text(scope, representation, index(start)?, index(end)?)
                .await?,
        ),
        ("create-passage", [file]) => value(
            app.create_passage(scope, read_json::<CreatePassageRequest>(file).await?)
                .await?,
        ),
        ("read-passage", [passage]) => value(app.read_passage(scope, passage).await?),
        ("resolve-passage", [passage]) => value(app.resolve_passage(scope, passage).await?),
        _ => Err(invalid().into()),
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassageFixture {
    expected_passage_id: String,
    expected_quote: String,
    expected_document_checksum: String,
}

impl PassageFixture {
    pub fn validate(&self) -> Result<()> {
        lugus_app::conversations::validate_id(&self.expected_passage_id)?;
        if self.expected_quote.trim().is_empty()
            || self.expected_quote.len() > TextLimits::default().max_passage_bytes
            || self.expected_document_checksum.len() != 64
            || !self
                .expected_document_checksum
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub async fn run(
        &self,
        request: &RunRequest,
        tools: &dyn ToolExecutor,
        events: &mpsc::Sender<lugus_agent::runtime::RuntimeEvent>,
    ) -> AgentResult<Value> {
        if !request.context.is_empty()
            || request.instructions.contains(&self.expected_quote)
            || request.limits.max_tool_calls < 1
        {
            return Err(failure(
                "passage fixture received invalid runtime boundaries",
            ));
        }
        let context: Value = serde_json::from_str(&request.prompt)
            .map_err(|_| failure("invalid conversation context"))?;
        let reference = context["references"]
            .as_array()
            .and_then(|references| {
                references.iter().find(|reference| {
                    reference["reference"]["kind"] == "passage"
                        && reference["reference"]["id"] == self.expected_passage_id
                })
            })
            .ok_or_else(|| failure("expected selected passage absent"))?;
        let passage: Passage = serde_json::from_str(
            reference["serialized"]
                .as_str()
                .ok_or_else(|| failure("missing frozen passage"))?,
        )
        .map_err(|_| failure("invalid frozen passage"))?;
        if passage.id != self.expected_passage_id
            || passage.quote != self.expected_quote
            || passage.representation.document.checksum != self.expected_document_checksum
        {
            return Err(failure("frozen passage provenance differs from fixture"));
        }
        let resolved = invoke(
            request,
            tools,
            events,
            1,
            "lugus_resolve_passage",
            json!({"passage_id":passage.id}),
        )
        .await?;
        let resolved: PassageSource = serde_json::from_value(resolved)
            .map_err(|_| failure("invalid passage source receipt"))?;
        if resolved.passage.id != passage.id
            || resolved.passage.quote != passage.quote
            || resolved.passage.representation.document.checksum
                != passage.representation.document.checksum
            || resolved.sources.is_empty()
        {
            return Err(failure("resolved passage differs from frozen passage"));
        }
        Ok(json!({
            "subject_kind":"conversation",
            "answer":"Pinned filing passage and original source locations verified.",
            "receipts":{
                "passage":{
                    "id":passage.id,
                    "quote":passage.quote,
                    "document_checksum":passage.representation.document.checksum,
                },
                "source":{"source_count":resolved.sources.len()},
            },
        }))
    }
}
