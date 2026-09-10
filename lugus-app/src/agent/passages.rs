//! Strict host-scoped tools; no model argument can supply provenance or source mappings.
use super::*;
use crate::passages::CreatePassageRequest;
pub(super) const NAMES: &[&str] = &[
    "lugus_prepare_text",
    "lugus_text_header",
    "lugus_read_text",
    "lugus_create_passage",
    "lugus_read_passage",
    "lugus_resolve_passage",
];
#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    PrepareText {
        dataset_id: String,
    },
    TextHeader {
        representation_id: String,
    },
    ReadText {
        representation_id: String,
        start: usize,
        end: usize,
    },
    CreatePassage {
        request: CreatePassageRequest,
    },
    ReadPassage {
        passage_id: String,
    },
    ResolvePassage {
        passage_id: String,
    },
}
impl ResearchExecutor {
    pub(super) async fn passage_call(
        &self,
        scope: &Scope,
        call: &ToolCall,
    ) -> Result<(bool, Value)> {
        let mut args = call
            .arguments
            .as_object()
            .cloned()
            .ok_or_else(|| invalid("tool arguments must be an object"))?;
        if args.contains_key("operation") {
            return Err(invalid("operation is selected by tool name"));
        }
        args.insert(
            "operation".into(),
            json!(call.name.strip_prefix("lugus_").unwrap_or_default()),
        );
        let command: Command = serde_json::from_value(Value::Object(args))
            .map_err(|_| invalid("invalid passage tool arguments"))?;
        let operation = async {
            let app = &self.application;
            match command {
                Command::PrepareText { dataset_id } => {
                    self.value(app.prepare_text(scope, &dataset_id).await?)
                }
                Command::TextHeader { representation_id } => {
                    self.value(app.text_header(scope, &representation_id).await?)
                }
                Command::ReadText {
                    representation_id,
                    start,
                    end,
                } => self.value(app.read_text(scope, &representation_id, start, end).await?),
                Command::CreatePassage { request } => {
                    self.value(app.create_passage(scope, request).await?)
                }
                Command::ReadPassage { passage_id } => {
                    self.value(app.read_passage(scope, &passage_id).await?)
                }
                Command::ResolvePassage { passage_id } => {
                    self.value(app.resolve_passage(scope, &passage_id).await?)
                }
            }
        };
        let value = if let Some(mut cancel) = self.cancellation.clone() {
            tokio::select! {
                biased;
                _ = async { while !*cancel.borrow_and_update() { if cancel.changed().await.is_err() { break; } } } => return Err(AppError::new(ErrorKind::Cancelled, "passage operation cancelled", false)),
                result = operation => result?,
            }
        } else {
            operation.await?
        };
        Ok((true, value))
    }
}
pub(super) fn tool_specs(app: &Application) -> Vec<ToolSpec> {
    let range = || {
        vec![
            ("representation_id", id()),
            ("start", integer(app.text_limits().max_text_bytes)),
            ("end", integer(app.text_limits().max_text_bytes)),
        ]
    };
    let mut selection = range();
    selection.push((
        "expected_text",
        json!({"type":"string","minLength":1,"maxLength":app.text_limits().max_passage_bytes}),
    ));
    [
        (
            "prepare_text",
            "Prepare immutable local filing text from an owned document dataset.",
            vec![("dataset_id", id())],
        ),
        (
            "text_header",
            "Read immutable filing text provenance offline.",
            vec![("representation_id", id())],
        ),
        (
            "read_text",
            "Read a bounded UTF-8 byte range of prepared text offline.",
            range(),
        ),
        (
            "create_passage",
            "Pin an exact nonempty UTF-8 selection and its original source mappings.",
            vec![("request", object(selection))],
        ),
        (
            "read_passage",
            "Read an owned immutable passage offline.",
            vec![("passage_id", id())],
        ),
        (
            "resolve_passage",
            "Resolve a passage to pinned original source node excerpts offline.",
            vec![("passage_id", id())],
        ),
    ]
    .into_iter()
    .map(|(name, description, fields)| ToolSpec {
        name: format!("lugus_{name}"),
        description: description.into(),
        input_schema: object(fields),
    })
    .collect()
}
